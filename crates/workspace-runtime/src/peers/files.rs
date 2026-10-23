//! Whole-file transfer between devices: `cranibox push` / `cranibox pull`.
//! Files move in sequential chunks; an upload lands beside its destination as
//! `<path>.cranibox-part` and replaces it atomically only after its SHA-256
//! matches. Access follows the `exec` grant, which can already write any file the
//! target account can.
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
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadChunk {
    pub path: String,
    pub offset: u64,
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
}
