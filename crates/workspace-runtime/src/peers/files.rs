//! File and folder transfer between devices: `cranibox push` / `cranibox pull`.
//! Files move in sequential chunks; an upload lands beside its destination as
//! `<path>.cranibox-part` and replaces it atomically only after its SHA-256
//! matches. A folder moves as its files, listed by [`list`]. Access follows the
//! `exec` grant, which can already read and write any file the target account can.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Digest;
use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

/// Raw bytes per request; base64 and JSON stay well inside [`MAX_WIRE`].
pub const CHUNK: usize = 4 * 1024 * 1024;
/// Request and response bound for the file routes.
pub const MAX_WIRE: usize = 8 * 1024 * 1024;
pub const MAX_FILE: u64 = 4 * 1024 * 1024 * 1024;
/// Most entries one folder listing returns; a larger tree is refused, not cut short.
pub const MAX_ENTRIES: usize = 100_000;
const PART: &str = ".cranibox-part";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteChunk {
    /// Absolute destination path on the target device.
    pub path: String,
    pub offset: u64,
    /// Base64 bytes of this chunk.
    pub data: String,
    /// Size of the whole file.
    pub size: u64,
    /// Lowercase hex SHA-256 of the whole file.
    pub sha256: String,
    #[serde(default)]
    pub overwrite: bool,
    /// Unix permission bits for the finished file.
    #[serde(default)]
    pub mode: Option<u32>,
    /// Create missing parent folders (folder uploads). Left out when false, so
    /// single-file uploads still reach devices that predate it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub parents: bool,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadChunk {
    pub path: String,
    pub offset: u64,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListRequest {
    /// Absolute path of a file or folder on the target device.
    pub path: String,
}

fn destination(path: &str) -> Result<PathBuf> {
    let path = PathBuf::from(path);
    ensure!(
        path.is_absolute() && !path.as_os_str().is_empty() && path.file_name().is_some(),
        "Use an absolute file path"
    );
    ensure!(
        !path.to_string_lossy().contains('\0') && path.as_os_str().len() <= 4096,
        "Invalid file path"
    );
    Ok(path)
}
fn part(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(PART);
    path.with_file_name(name)
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
pub fn digest_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Target side of an upload chunk. Chunks must arrive in order; the first one
/// (offset 0) starts over.
pub fn write(request: WriteChunk) -> Result<Value> {
    let path = destination(&request.path)?;
    ensure!(request.size <= MAX_FILE, "File is larger than 4 GiB");
    ensure!(valid_digest(&request.sha256), "Invalid SHA-256");
    let data = STANDARD
        .decode(request.data.as_bytes())
        .context("Invalid chunk encoding")?;
    ensure!(data.len() <= CHUNK, "Chunk too large");
    let end = request
        .offset
        .checked_add(data.len() as u64)
        .filter(|end| *end <= request.size)
        .context("Chunk exceeds the file size")?;
    if !request.overwrite && path.exists() {
        bail!(
            "{} already exists; pass --overwrite to replace it",
            path.display()
        );
    }
    let temporary = part(&path);
    let mut file = if request.offset == 0 {
        let parent = path.parent().context("Use an absolute file path")?;
        if request.parents {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Cannot create {}", parent.display()))?;
        }
        ensure!(parent.is_dir(), "{} does not exist", parent.display());
        std::fs::File::create(&temporary)?
    } else {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&temporary)
            .context("The upload was not started or was interrupted; send it again")?;
        ensure!(
            file.metadata()?.len() == request.offset,
            "The upload was interrupted; send it again"
        );
        file
    };
    file.seek(SeekFrom::Start(request.offset))?;
    file.write_all(&data)?;
    file.sync_data()?;
    drop(file);
    if end < request.size {
        return Ok(json!({"received": end, "done": false}));
    }
    if digest_file(&temporary)? != request.sha256 {
        let _ = std::fs::remove_file(&temporary);
        bail!("The received file does not match its SHA-256; nothing was replaced");
    }
    #[cfg(unix)]
    if let Some(mode) = request.mode {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(mode & 0o7777))?;
    }
    std::fs::rename(&temporary, &path)?;
    Ok(json!({"received": end, "done": true, "path": path}))
}

/// What a download of `path` would copy: one file, or a folder's files and
/// subfolders relative to it ('/'-separated). Symbolic links and special files are
/// skipped, never followed, so a listing cannot leave the folder.
pub fn list(request: ListRequest) -> Result<Value> {
    let root = destination(&request.path)?;
    let metadata = std::fs::symlink_metadata(&root)
        .with_context(|| format!("Cannot open {}", root.display()))?;
    if metadata.is_file() {
        return Ok(json!({"kind": "file", "size": metadata.len()}));
    }
    ensure!(
        metadata.is_dir(),
        "{} is not a file or folder",
        root.display()
    );
    let (mut files, mut folders, mut skipped) = (Vec::new(), Vec::new(), Vec::new());
    let mut total = 0u64;
    let mut pending = vec![String::new()];
    while let Some(relative) = pending.pop() {
        let folder = if relative.is_empty() {
            root.clone()
        } else {
            root.join(&relative)
        };
        let mut entries = std::fs::read_dir(&folder)
            .with_context(|| format!("Cannot read {}", folder.display()))?
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                skipped.push(json!({"path": format!("{relative}/{}", entry.file_name().to_string_lossy()).trim_start_matches('/'), "reason": "name"}));
                continue;
            };
            let path = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            let kind = entry.file_type()?;
            if kind.is_dir() {
                folders.push(json!(path));
                pending.push(path);
            } else if kind.is_file() {
                let size = entry.metadata()?.len();
                ensure!(size <= MAX_FILE, "{path} is larger than 4 GiB");
                total += size;
                files.push(json!({"path": path, "size": size}));
            } else {
                skipped.push(json!({"path": path, "reason": if kind.is_symlink() { "link" } else { "special" }}));
            }
            ensure!(
                files.len() + folders.len() + skipped.len() <= MAX_ENTRIES,
                "{} has more than {MAX_ENTRIES} entries; copy a smaller folder or an archive",
                root.display()
            );
        }
    }
    Ok(
        json!({"kind": "folder", "files": files, "folders": folders, "skipped": skipped, "bytes": total}),
    )
}

/// Target side of a download chunk. The last chunk carries the whole file's SHA-256.
pub fn read(request: ReadChunk) -> Result<Value> {
    let path = destination(&request.path)?;
    let mut file =
        std::fs::File::open(&path).with_context(|| format!("Cannot open {}", path.display()))?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "{} is not a file", path.display());
    let size = metadata.len();
    ensure!(size <= MAX_FILE, "File is larger than 4 GiB");
    ensure!(request.offset <= size, "Offset is past the end of the file");
    file.seek(SeekFrom::Start(request.offset))?;
    let mut data = Vec::with_capacity(CHUNK.min((size - request.offset) as usize));
    file.take(CHUNK as u64).read_to_end(&mut data)?;
    let end = request.offset + data.len() as u64;
    let mut answer = json!({"size": size, "data": STANDARD.encode(&data), "eof": end >= size});
    if end >= size {
        answer["sha256"] = json!(digest_file(&path)?);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            answer["mode"] = json!(metadata.permissions().mode() & 0o7777);
        }
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upload_lands_atomically_only_after_the_digest_matches() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("app.dmg");
        let bytes: Vec<u8> = (0..(CHUNK + 1000)).map(|i| (i % 251) as u8).collect();
        let sha = hex(&sha2::Sha256::digest(&bytes));
        let chunk = |offset: usize, len: usize, sha: &str| WriteChunk {
            path: target.display().to_string(),
            offset: offset as u64,
            data: STANDARD.encode(&bytes[offset..offset + len]),
            size: bytes.len() as u64,
            sha256: sha.into(),
            overwrite: false,
            mode: Some(0o755),
            parents: false,
        };
        let first = write(chunk(0, CHUNK, &sha)).unwrap();
        assert_eq!(first["done"], false);
        assert!(!target.exists(), "nothing lands before the last chunk");
        // Out-of-order chunks are refused rather than leaving a hole.
        assert!(write(chunk(CHUNK - 1, 1001, &sha)).is_err());
        write(chunk(CHUNK, 1000, &sha)).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
        assert!(!part(&target).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&target).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        // An existing file needs overwrite; a wrong digest replaces nothing.
        assert!(write(chunk(0, CHUNK, &sha)).is_err());
        let mut bad = chunk(0, bytes.len(), &"0".repeat(64));
        bad.data = STANDARD.encode(&bytes[..CHUNK]);
        bad.size = CHUNK as u64;
        bad.overwrite = true;
        assert!(write(bad).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
        assert!(
            write(WriteChunk {
                path: "relative".into(),
                ..chunk(0, 1, &sha)
            })
            .is_err()
        );

        let mut offset = 0;
        let mut copy = Vec::new();
        loop {
            let answer = read(ReadChunk {
                path: target.display().to_string(),
                offset,
            })
            .unwrap();
            copy.extend(STANDARD.decode(answer["data"].as_str().unwrap()).unwrap());
            offset = copy.len() as u64;
            if answer["eof"] == true {
                assert_eq!(answer["sha256"], sha);
                break;
            }
        }
        assert_eq!(copy, bytes);
    }
    #[test]
    fn folder_uploads_create_parents_and_listings_stay_inside_the_folder() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("a/b/c.txt");
        let upload = |parents| WriteChunk {
            path: nested.display().to_string(),
            offset: 0,
            data: STANDARD.encode(b"hi"),
            size: 2,
            sha256: hex(&sha2::Sha256::digest(b"hi")),
            overwrite: false,
            mode: None,
            parents,
        };
        assert!(
            write(upload(false)).is_err(),
            "a missing folder needs parents"
        );
        assert_eq!(write(upload(true)).unwrap()["done"], true);
        assert_eq!(std::fs::read(&nested).unwrap(), b"hi");
        // Old devices reject unknown fields, so a plain upload must not send it.
        let plain = serde_json::to_value(upload(false)).unwrap();
        assert!(plain.get("parents").is_none());

        std::fs::create_dir(root.path().join("empty")).unwrap();
        std::fs::write(root.path().join("top.bin"), [0u8; 5]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc", root.path().join("escape")).unwrap();
        let listing = list(ListRequest {
            path: root.path().display().to_string(),
        })
        .unwrap();
        assert_eq!(listing["kind"], "folder");
        assert_eq!(
            listing["files"],
            json!([{"path": "top.bin", "size": 5}, {"path": "a/b/c.txt", "size": 2}])
        );
        assert_eq!(listing["folders"], json!(["a", "empty", "a/b"]));
        assert_eq!(listing["bytes"], 7);
        #[cfg(unix)]
        assert_eq!(
            listing["skipped"],
            json!([{"path": "escape", "reason": "link"}])
        );
        let file = list(ListRequest {
            path: nested.display().to_string(),
        })
        .unwrap();
        assert_eq!(file, json!({"kind": "file", "size": 2}));
        assert!(
            list(ListRequest {
                path: "relative".into()
            })
            .is_err()
        );
    }
}
