//! Bounded code snapshots. No processes, credentials, links, or source deletion.
use crate::{
    config,
    projects::{Command, Store},
    protocol::valid_node_id,
};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const MAX_WIRE: usize = 24 * 1024 * 1024;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_FILE: usize = 4 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
pub mod stream;

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub path: String,
    pub size: usize,
    pub sha256: String,
    pub executable: bool,
    pub data: String,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub version: u32,
    pub source: String,
    pub project_id: String,
    pub replica_id: String,
    pub name: String,
    pub created_at: u64,
    pub directories: Vec<String>,
    pub files: Vec<File>,
    pub skipped: Vec<String>,
}
impl Package {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.version, 1 | 2) && valid_node_id(&self.source),
            "unsupported code snapshot"
        );
        uuid::Uuid::parse_str(&self.project_id)?;
        uuid::Uuid::parse_str(&self.replica_id)?;
        ensure!(
            !self.name.trim().is_empty()
                && self.name.chars().count() <= 80
                && !self.name.chars().any(char::is_control),
            "invalid project name"
        );
        ensure!(
            self.directories.len() + self.files.len() + self.skipped.len()
                <= if self.version == 2 {
                    stream::MAX_ENTRIES
                } else {
                    MAX_ENTRIES
                },
            "snapshot entry limit exceeded"
        );
        let mut paths = BTreeSet::new();
        let mut dirs = BTreeSet::new();
        for directory in &self.directories {
            portable(directory)?;
            ensure!(
                paths.insert(directory.to_lowercase()),
                "duplicate snapshot path"
            );
            dirs.insert(directory.as_str());
        }
        let mut total = 0usize;
        for file in &self.files {
            portable(&file.path)?;
            ensure!(
                paths.insert(file.path.to_lowercase()),
                "duplicate snapshot path"
            );
            ensure!(
                file.size
                    <= if self.version == 2 {
                        stream::MAX_FILE
                    } else {
                        MAX_FILE
                    },
                "file exceeds snapshot file limit"
            );
            total = total
                .checked_add(file.size)
                .context("snapshot size overflow")?;
            ensure!(
                total
                    <= if self.version == 2 {
                        stream::MAX_BYTES
                    } else {
                        MAX_BYTES
                    },
                "code snapshot exceeds total size limit"
            );
            ensure!(
                file.sha256.len() == 64
                    && file
                        .sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "invalid file checksum"
            );
            if self.version == 2 {
                ensure!(
                    file.data.is_empty(),
                    "stream manifest must not contain file contents"
                );
                continue;
            }
            ensure!(
                file.data.len() <= MAX_FILE.div_ceil(3) * 4,
                "file exceeds 4 MiB"
            );
            let bytes = STANDARD
                .decode(&file.data)
                .context("invalid file encoding")?;
            ensure!(
                bytes.len() == file.size && digest(&bytes) == file.sha256,
                "file checksum mismatch"
            );
        }
        for path in self
            .directories
            .iter()
            .chain(self.files.iter().map(|f| &f.path))
        {
            if let Some((parent, _)) = path.rsplit_once('/') {
                ensure!(dirs.contains(parent), "snapshot parent directory missing");
            }
        }
        for skipped in &self.skipped {
            ensure!(
                skipped.len() <= 4096 && !skipped.chars().any(char::is_control),
                "invalid skipped path"
            );
        }
        ensure!(
            serde_json::to_vec(self)?.len()
                <= if self.version == 2 {
                    stream::MAX_MANIFEST
                } else {
                    MAX_WIRE - 4096
                },
            "snapshot metadata exceeds limit"
        );
        Ok(())
    }
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({"source":self.source,"project_id":self.project_id,"name":self.name,"directories":self.directories,
            "created_at":self.created_at,"bytes":self.files.iter().map(|f|f.size).sum::<usize>(),
            "files":self.files.iter().map(|f|serde_json::json!({"path":f.path,"size":f.size,"sha256":f.sha256})).collect::<Vec<_>>(),"skipped":self.skipped})
    }
}
fn portable(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty()
            && path.len() <= 1024
            && !path.contains('\\')
            && !path.chars().any(char::is_control),
        "snapshot path is not portable"
    );
    for part in path.split('/') {
        let base = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        ensure!(
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.len() <= 240
                && !part.ends_with(['.', ' '])
                && !part.contains([':', '*', '?', '"', '<', '>', '|'])
                && !["CON", "PRN", "AUX", "NUL"].contains(&base.as_str())
                && !(base.len() == 4
                    && (base.starts_with("COM") || base.starts_with("LPT"))
                    && matches!(base.as_bytes()[3], b'1'..=b'9')),
            "snapshot path is not portable"
        );
    }
    Ok(())
}
fn excluded(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        ".git",
        "node_modules",
        "target",
        "dist",
        "build",
        "coverage",
        ".venv",
        "venv",
        ".cache",
        "__pycache__",
        ".ssh",
        ".aws",
        ".config",
        ".npmrc",
        ".pypirc",
        ".netrc",
        ".ds_store",
    ]
    .contains(&name.as_str())
        || name == ".env"
        || name.starts_with(".env.")
        || [".pem", ".key", ".p12", ".pfx"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}
pub fn export(
    store: &Store,
    owner: &str,
    replica_id: &str,
    permitted: impl Fn() -> Result<()>,
) -> Result<Package> {
    export_to(store, owner, replica_id, None, permitted)
}
pub(super) fn export_to(
    store: &Store,
    owner: &str,
    replica_id: &str,
    cache: Option<&Path>,
    permitted: impl Fn() -> Result<()>,
) -> Result<Package> {
    let catalog = store.apply_owned(Command::List {}, owner)?;
    let project = catalog
        .projects
        .iter()
        .find(|p| p.replicas.iter().any(|r| r.id == replica_id))
        .context("project replica unavailable")?;
    let replica = project
        .replicas
        .iter()
        .find(|r| r.id == replica_id)
        .unwrap();
    let root = fs::canonicalize(&replica.path).context("source directory unavailable")?;
    ensure!(root.is_dir(), "source must be a directory");
    let mut package = Package {
        version: if cache.is_some() { 2 } else { 1 },
        source: owner.into(),
        project_id: project.id.clone(),
        replica_id: replica.id.clone(),
        name: project.name.clone(),
        created_at: now(),
        directories: vec![],
        files: vec![],
        skipped: vec![],
    };
    let mut pending = vec![(root.clone(), String::new(), 0)];
    let mut entries = 0usize;
    let mut total = 0usize;
    let max_file = if cache.is_some() {
        stream::MAX_FILE
    } else {
        MAX_FILE
    };
    let max_bytes = if cache.is_some() {
        stream::MAX_BYTES
    } else {
        MAX_BYTES
    };
    let max_entries = if cache.is_some() {
        stream::MAX_ENTRIES
    } else {
        MAX_ENTRIES
    };
    while let Some((directory, prefix, depth)) = pending.pop() {
        permitted()?;
        ensure!(
            depth <= 32 && fs::canonicalize(&directory)? == directory,
            "source directory changed or nesting too deep"
        );
        ensure!(
            !fs::symlink_metadata(&directory)?.file_type().is_symlink(),
            "source directory became a link"
        );
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            entries += 1;
            ensure!(
                entries <= max_entries,
                "snapshot entry limit exceeded; exclude dependencies and generated files"
            );
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("source contains a non-UTF-8 filename"))?;
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let metadata = fs::symlink_metadata(entry.path())?;
            if excluded(&name)
                || metadata.file_type().is_symlink()
                || !metadata.is_dir() && !metadata.is_file()
            {
                package.skipped.push(relative);
                continue;
            }
            portable(&relative)?;
            let full = root.join(&relative);
            ensure!(fs::canonicalize(&full)? == full, "source path changed");
            if metadata.is_dir() {
                package.directories.push(relative.clone());
                pending.push((full, relative, depth + 1));
            } else {
                ensure!(
                    metadata.len() <= max_file as u64,
                    "file exceeds snapshot file limit: {relative}"
                );
                let mut options = fs::OpenOptions::new();
                options.read(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::OpenOptionsExt;
                    options.custom_flags(0x00200000);
                }
                let mut file = options.open(&full)?;
                let before = file.metadata()?;
                ensure!(before.is_file(), "source is no longer a regular file");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    ensure!(
                        before.nlink() == 1,
                        "hard-linked source files are not supported: {relative}"
                    );
                }
                let mut bytes = vec![];
                let (size, sha256) = if let Some(cache) = cache {
                    stream::snapshot_file(&mut file, cache, max_bytes - total, &permitted)?
                } else {
                    (&mut file)
                        .take((max_file + 1) as u64)
                        .read_to_end(&mut bytes)?;
                    (bytes.len(), digest(&bytes))
                };
                let after = file.metadata()?;
                ensure!(
                    size <= max_file
                        && before.len() == after.len()
                        && before.modified()? == after.modified()?
                        && size as u64 == after.len()
                        && fs::canonicalize(&full)? == full,
                    "source file changed; pause writers and prepare again: {relative}"
                );
                total += size;
                ensure!(total <= max_bytes, "code snapshot exceeds total size limit");
                #[cfg(unix)]
                let executable = {
                    use std::os::unix::fs::PermissionsExt;
                    before.permissions().mode() & 0o111 != 0
                };
                #[cfg(not(unix))]
                let executable = false;
                package.files.push(File {
                    path: relative,
                    size,
                    sha256,
                    executable,
                    data: STANDARD.encode(bytes),
                });
            }
        }
    }
    package.directories.sort();
    package.files.sort_by(|a, b| a.path.cmp(&b.path));
    package.skipped.sort();
    package.validate()?;
    permitted()?;
    Ok(package)
}
pub fn root(path: &Path) -> Result<PathBuf> {
    ensure!(
        path.is_absolute(),
        "copy root must be an existing absolute directory"
    );
    let root = fs::canonicalize(path).context("copy root is unavailable")?;
    ensure!(
        root.is_dir() && root.parent().is_some(),
        "choose a project directory, not a filesystem root"
    );
    Ok(root)
}
fn private_directory(path: &Path) -> Result<()> {
    let builder = &mut fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    pub id: String,
    pub expected_root: PathBuf,
    pub digest: String,
    pub package: Package,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub id: String,
    pub caller: String,
    pub credential: String,
    pub digest: String,
    pub project_id: String,
    pub name: String,
    pub path: PathBuf,
    pub state: String,
    pub error: Option<String>,
}
fn receipt_path(state: &Path, id: &str) -> Result<PathBuf> {
    let id = uuid::Uuid::parse_str(id)?.to_string();
    Ok(state
        .with_file_name("project-copy-receipts")
        .join(format!("{id}.json")))
}
fn save(path: &Path, receipt: &Receipt) -> Result<()> {
    config::write_private(path, &serde_json::to_vec(receipt)?, true)
}
pub fn status(state: &Path, id: &str, caller: &str, token: &str) -> Result<Receipt> {
    let path = receipt_path(state, id)?;
    let file = fs::File::open(path).context("copy receipt unavailable")?;
    ensure!(file.metadata()?.len() <= 16384, "copy receipt too large");
    let receipt: Receipt = serde_json::from_reader(file)?;
    ensure!(
        receipt.caller == caller && config::matches(&receipt.credential, &digest(token.as_bytes())),
        "copy receipt unavailable"
    );
    Ok(receipt)
}
impl Receipt {
    pub fn view(&self) -> serde_json::Value {
        serde_json::json!({"id":self.id,"state":self.state,"project_id":self.project_id,"path":self.path,"error":self.error})
    }
}
pub fn receive(
    state: &Path,
    store: &Store,
    owner: &str,
    caller: &str,
    token: &str,
    request: Import,
    permitted: impl Fn() -> Result<()>,
) -> Result<Receipt> {
    let path = receipt_path(state, &request.id)?;
    let _lock = config::lock_credentials(&path)?;
    request.package.validate()?;
    ensure!(
        request.package.version == 1,
        "use the chunked transfer endpoint for this snapshot"
    );
    ensure!(
        digest(&serde_json::to_vec(&request.package)?) == request.digest,
        "snapshot checksum mismatch"
    );
    permitted()?;
    if path.exists() {
        let mut receipt = status(state, &request.id, caller, token)?;
        ensure!(
            receipt.digest == request.digest,
            "copy ID already used for a different snapshot"
        );
        if receipt.state == "copied" {
            finish(store, owner, &mut receipt, &permitted)?;
            save(&path, &receipt)?;
        }
        return Ok(receipt);
    }
    let directory = root(&request.expected_root)?;
    ensure!(directory == request.expected_root, "copy root changed");
    ensure!(
        fs::read_dir(path.parent().unwrap())?.count() < 256,
        "copy receipt limit reached"
    );
    let container = directory.join(format!("cranibox-{}", uuid::Uuid::parse_str(&request.id)?));
    let destination = container.join("project");
    let mut receipt = Receipt {
        id: request.id,
        caller: caller.into(),
        credential: digest(token.as_bytes()),
        digest: request.digest,
        project_id: request.package.project_id.clone(),
        name: request.package.name.clone(),
        path: destination.clone(),
        state: "incomplete".into(),
        error: None,
    };
    save(&path, &receipt)?;
    let result: Result<()> = (|| {
        private_directory(&container)
            .context("copy destination already exists or is not writable")?;
        let staging = container.join(".pending");
        private_directory(&staging)?;
        for relative in &request.package.directories {
            permitted()?;
            fs::create_dir_all(staging.join(relative))?;
        }
        for file in &request.package.files {
            permitted()?;
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(if file.executable { 0o700 } else { 0o600 });
            }
            let mut output = options.open(staging.join(&file.path))?;
            output.write_all(&STANDARD.decode(&file.data)?)?;
            output.sync_all()?;
        }
        permitted()?;
        ensure!(
            fs::canonicalize(&directory)? == directory && !destination.exists(),
            "copy destination changed"
        );
        fs::rename(&staging, &destination)?;
        receipt.state = "copied".into();
        save(&path, &receipt)?;
        finish(store, owner, &mut receipt, &permitted)?;
        Ok(())
    })();
    if let Err(error) = result {
        receipt.error = Some(error.to_string());
    }
    save(&path, &receipt)?;
    Ok(receipt)
}
fn finish(
    store: &Store,
    owner: &str,
    receipt: &mut Receipt,
    permitted: &impl Fn() -> Result<()>,
) -> Result<()> {
    permitted()?;
    store.apply_owned(
        Command::Register {
            project_id: Some(receipt.project_id.clone()),
            name: receipt.name.clone(),
            node: Some(owner.into()),
            path: receipt
                .path
                .to_str()
                .context("destination is not UTF-8")?
                .into(),
        },
        owner,
    )?;
    receipt.state = "completed".into();
    receipt.error = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Package {
        Package {
            version: 1,
            source: "source".into(),
            project_id: uuid::Uuid::new_v4().to_string(),
            replica_id: uuid::Uuid::new_v4().to_string(),
            name: "Code".into(),
            created_at: now(),
            directories: vec!["src".into()],
            files: vec![File {
                path: "src/main.rs".into(),
                size: 4,
                sha256: digest(b"code"),
                executable: false,
                data: STANDARD.encode(b"code"),
            }],
            skipped: vec![],
        }
    }
    #[test]
    fn paths_and_checksums_reject_ambiguous_or_escaping_files() {
        for path in [
            "../outside",
            "/root",
            "C:/file",
            "a\\b",
            "src/../secret",
            "CON.txt",
            "dir/aux",
            "a.",
            "a ",
            "a//b",
            "./a",
        ] {
            let mut value = sample();
            value.files[0].path = path.into();
            assert!(value.validate().is_err(), "{path}");
        }
        let mut value = sample();
        value.files[0].data = STANDARD.encode(b"evil");
        assert!(value.validate().is_err());
        let mut value = sample();
        value.directories.push("SRC".into());
        assert!(value.validate().is_err());
        let mut value = sample();
        value.files[0].path = "absent/main.rs".into();
        assert!(value.validate().is_err());
        let mut value = sample();
        value.files[0].size = MAX_FILE + 1;
        assert!(value.validate().is_err());
    }
    #[test]
    fn receipt_is_durable_private_idempotent_and_bound_to_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let directory = root(temp.path()).unwrap();
        let state = temp.path().join("peers.json");
        let store = Store::new(temp.path().join("projects.json"));
        let package = sample();
        let project_id = package.project_id.clone();
        let id = uuid::Uuid::new_v4().to_string();
        let bytes = serde_json::to_vec(&package).unwrap();
        let request = || Import {
            id: id.clone(),
            expected_root: directory.clone(),
            digest: digest(&bytes),
            package: serde_json::from_slice(&bytes).unwrap(),
        };
        let first = receive(
            &state,
            &store,
            "target",
            "caller",
            "secret",
            request(),
            || Ok(()),
        )
        .unwrap();
        assert_eq!(first.state, "completed");
        assert_eq!(fs::read(first.path.join("src/main.rs")).unwrap(), b"code");
        fs::write(first.path.join("src/main.rs"), b"local edits").unwrap();
        let second = receive(
            &state,
            &store,
            "target",
            "caller",
            "secret",
            request(),
            || Ok(()),
        )
        .unwrap();
        assert_eq!(first.path, second.path);
        assert_eq!(
            fs::read(second.path.join("src/main.rs")).unwrap(),
            b"local edits"
        );
        let catalog = store.apply_owned(Command::List {}, "target").unwrap();
        assert_eq!(catalog.projects[0].id, project_id);
        assert_eq!(catalog.projects[0].replicas.len(), 1);
        assert!(catalog.projects[0].replicas[0].sessions.is_empty());
        assert!(status(&state, &id, "other", "secret").is_err());
        assert!(status(&state, &id, "caller", "rotated").is_err());
        assert!(
            receive(
                &state,
                &store,
                "target",
                "caller",
                "secret",
                request(),
                || anyhow::bail!("revoked")
            )
            .is_err()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(receipt_path(&state, &id).unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn conflicts_and_revocation_keep_partial_results_without_overwriting() {
        let temp = tempfile::tempdir().unwrap();
        let directory = root(temp.path()).unwrap();
        let state = temp.path().join("peers.json");
        let store = Store::new(temp.path().join("projects.json"));
        let package = sample();
        let bytes = serde_json::to_vec(&package).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let container = temp.path().join(format!("cranibox-{id}"));
        fs::create_dir(&container).unwrap();
        fs::write(container.join("keep"), b"existing").unwrap();
        let first = receive(
            &state,
            &store,
            "target",
            "caller",
            "secret",
            Import {
                id,
                expected_root: directory.clone(),
                digest: digest(&bytes),
                package,
            },
            || Ok(()),
        )
        .unwrap();
        assert_eq!(first.state, "incomplete");
        assert_eq!(fs::read(container.join("keep")).unwrap(), b"existing");
        assert!(!first.path.exists());
        let checks = std::cell::Cell::new(0);
        let package = sample();
        let id = uuid::Uuid::new_v4().to_string();
        let second = receive(
            &state,
            &store,
            "target",
            "caller",
            "secret",
            Import {
                id,
                expected_root: directory,
                digest: digest(&serde_json::to_vec(&package).unwrap()),
                package,
            },
            || {
                checks.set(checks.get() + 1);
                ensure!(checks.get() < 3, "revoked during copy");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(second.state, "incomplete");
        assert!(!second.path.exists());
        assert!(second.path.parent().unwrap().join(".pending").is_dir());
    }
    #[test]
    fn export_includes_uncommitted_files_but_excludes_credentials_and_links() {
        let temp = tempfile::tempdir().unwrap();
        let code = temp.path().join("code");
        fs::create_dir(&code).unwrap();
        fs::write(code.join("main.rs"), b"uncommitted").unwrap();
        fs::write(code.join(".env"), b"secret").unwrap();
        fs::create_dir(code.join(".git")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(temp.path(), code.join("outside")).unwrap();
        let store = Store::new(temp.path().join("projects.json"));
        let catalog = store
            .apply_owned(
                Command::Register {
                    name: "Code".into(),
                    node: Some("source".into()),
                    path: code.to_str().unwrap().into(),
                    project_id: None,
                },
                "source",
            )
            .unwrap();
        let replica = &catalog.projects[0].replicas[0].id;
        let value = export(&store, "source", replica, || Ok(())).unwrap();
        assert_eq!(value.files.len(), 1);
        assert_eq!(value.files[0].path, "main.rs");
        assert!(value.skipped.contains(&".env".into()));
        assert!(value.skipped.contains(&".git".into()));
        assert!(export(&store, "wrong-owner", replica, || Ok(())).is_err());
        assert!(export(&store, "source", replica, || anyhow::bail!("revoked")).is_err());
    }
}
