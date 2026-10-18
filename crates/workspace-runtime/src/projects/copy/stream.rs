//! Disk-backed snapshots and restartable, sequential chunk receipts.
use super::*;
use serde_json::{Value, json};
use std::io::{Seek, SeekFrom};

pub const CHUNK: usize = 1024 * 1024;
pub const MAX_BYTES: usize = 2 * 1024 * 1024 * 1024;
pub const MAX_FILE: usize = 1024 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 32768;
pub const MAX_MANIFEST: usize = 8 * 1024 * 1024;
const TTL: u64 = 24 * 60 * 60 * 1000;
#[cfg(test)]
mod tests;

#[derive(Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Export {
        id: String,
        replica_id: String,
    },
    Read {
        id: String,
        index: usize,
        offset: usize,
    },
    Release {
        id: String,
    },
    Begin {
        id: String,
        expected_root: PathBuf,
        digest: String,
        manifest: Package,
    },
    Write {
        id: String,
        digest: String,
        chunk: Chunk,
    },
    Status {
        id: String,
    },
    Finish {
        id: String,
        digest: String,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chunk {
    pub index: usize,
    pub offset: usize,
    pub data: String,
    pub sha256: String,
}
#[derive(Serialize, Deserialize)]
struct Export {
    id: String,
    caller: String,
    credential: String,
    manifest: Package,
}
#[derive(Serialize, Deserialize)]
struct Receiving {
    receipt: Receipt,
    expected_root: PathBuf,
    #[serde(skip)]
    manifest: Package,
    index: usize,
    offset: usize,
    bytes: usize,
}
fn record_path(state: &Path, group: &str, id: &str) -> Result<PathBuf> {
    Ok(state
        .with_file_name(group)
        .join(format!("{}.json", uuid::Uuid::parse_str(id)?)))
}
pub fn cache_path(state: &Path, id: &str) -> Result<PathBuf> {
    Ok(record_path(state, "project-copy-exports", id)?.with_extension("files"))
}
fn receiving_path(state: &Path, id: &str) -> Result<PathBuf> {
    record_path(state, "project-copy-stream-receipts", id)
}
fn load<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let file = fs::File::open(path).context("transfer record unavailable")?;
    ensure!(
        file.metadata()?.len() <= (MAX_MANIFEST + 32768) as u64,
        "transfer record too large"
    );
    Ok(serde_json::from_reader(file)?)
}
fn save<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() <= MAX_MANIFEST + 32768,
        "transfer record too large"
    );
    config::write_private(path, &bytes, true)
}
fn owner(caller: &str, token: &str, expected: &str, credential: &str) -> Result<()> {
    ensure!(
        caller == expected && config::matches(credential, &digest(token.as_bytes())),
        "transfer record unavailable"
    );
    Ok(())
}
pub fn blob(cache: &Path, sha256: &str) -> Result<PathBuf> {
    ensure!(
        sha256.len() == 64
            && sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid blob checksum"
    );
    Ok(cache.join(sha256))
}
fn open_regular(path: &Path, write: bool) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(write);
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
    ensure!(
        fs::canonicalize(path)? == path && !fs::symlink_metadata(path)?.file_type().is_symlink(),
        "transfer file became a link"
    );
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "transfer file is not regular");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(metadata.nlink() == 1, "transfer file became a hard link");
    }
    Ok(file)
}
pub fn verify_blob(
    path: &Path,
    expected: &super::File,
    permitted: &impl Fn() -> Result<()>,
) -> Result<()> {
    let mut file = open_regular(path, false)?;
    ensure!(
        file.metadata()?.len() == expected.size as u64,
        "transfer file size changed"
    );
    let mut buffer = vec![0u8; CHUNK];
    let mut hash = Sha256::new();
    let mut size = 0usize;
    loop {
        permitted()?;
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size += count;
        ensure!(size <= expected.size, "transfer file size changed");
        hash.update(&buffer[..count]);
    }
    ensure!(
        size == expected.size && format!("{:x}", hash.finalize()) == expected.sha256,
        "transfer file checksum mismatch"
    );
    Ok(())
}
pub(super) fn snapshot_file(
    input: &mut fs::File,
    cache: &Path,
    remaining: usize,
    permitted: &impl Fn() -> Result<()>,
) -> Result<(usize, String)> {
    let temp = cache.join(format!("{}.part", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut output = new_file(&temp, false)?;
        let mut hash = Sha256::new();
        let mut buffer = vec![0u8; CHUNK];
        let mut size = 0;
        loop {
            permitted()?;
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            size += count;
            ensure!(
                size <= MAX_FILE && size <= remaining,
                "snapshot exceeds 2 GiB total or 1 GiB per file"
            );
            output.write_all(&buffer[..count])?;
            hash.update(&buffer[..count]);
        }
        output.sync_all()?;
        drop(output);
        let sha256 = format!("{:x}", hash.finalize());
        let path = blob(cache, &sha256)?;
        if path.try_exists()? {
            fs::remove_file(&temp)?;
        } else {
            fs::rename(&temp, path)?;
        }
        Ok((size, sha256))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
fn new_file(path: &Path, executable: bool) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(if executable { 0o700 } else { 0o600 });
        let file = options.open(path)?;
        file.set_permissions(fs::Permissions::from_mode(if executable {
            0o700
        } else {
            0o600
        }))?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        let _ = executable;
        Ok(options.open(path)?)
    }
}
pub fn read_chunk(cache: &Path, manifest: &Package, index: usize, offset: usize) -> Result<Chunk> {
    let entry = manifest
        .files
        .get(index)
        .context("transfer file index out of range")?;
    ensure!(offset < entry.size, "transfer offset out of range");
    let mut input = open_regular(&blob(cache, &entry.sha256)?, false)?;
    ensure!(
        input.metadata()?.len() == entry.size as u64,
        "snapshot file size changed"
    );
    input.seek(SeekFrom::Start(offset as u64))?;
    let mut bytes = vec![0u8; CHUNK.min(entry.size - offset)];
    input.read_exact(&mut bytes)?;
    Ok(Chunk {
        index,
        offset,
        sha256: digest(&bytes),
        data: STANDARD.encode(bytes),
    })
}
pub fn cache_chunk(cache: &Path, manifest: &Package, chunk: Chunk) -> Result<usize> {
    let entry = manifest
        .files
        .get(chunk.index)
        .context("transfer file index out of range")?;
    let bytes = decode(&chunk)?;
    ensure!(
        chunk.offset <= entry.size && bytes.len() <= entry.size - chunk.offset,
        "transfer chunk exceeds file"
    );
    let path = blob(cache, &entry.sha256)?.with_extension("part");
    if !path.try_exists()? {
        new_file(&path, false)?.sync_all()?;
    }
    let mut output = open_regular(&path, true)?;
    ensure!(
        output.metadata()?.len() == chunk.offset as u64,
        "snapshot download offset changed"
    );
    output.seek(SeekFrom::End(0))?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    let next = chunk.offset + bytes.len();
    drop(output);
    if next == entry.size {
        verify_blob(&path, entry, &|| Ok(()))?;
        fs::rename(path, blob(cache, &entry.sha256)?)?;
    }
    Ok(next)
}
fn decode(chunk: &Chunk) -> Result<Vec<u8>> {
    ensure!(
        !chunk.data.is_empty() && chunk.data.len() <= CHUNK.div_ceil(3) * 4,
        "invalid transfer chunk size"
    );
    let bytes = STANDARD.decode(&chunk.data)?;
    ensure!(
        !bytes.is_empty() && bytes.len() <= CHUNK && digest(&bytes) == chunk.sha256,
        "transfer chunk checksum mismatch"
    );
    Ok(bytes)
}
fn export(
    state: &Path,
    store: &Store,
    node: &str,
    caller: &str,
    token: &str,
    selection: (String, String),
    permitted: &impl Fn(Option<&Path>) -> Result<()>,
) -> Result<Package> {
    let (id, replica) = selection;
    let path = record_path(state, "project-copy-exports", &id)?;
    let _index = config::lock_credentials(&path.with_file_name("index"))?;
    let _lock = config::lock_credentials(&path)?;
    permitted(None)?;
    if path.try_exists()? {
        let previous: Export = load(&path)?;
        owner(caller, token, &previous.caller, &previous.credential)?;
        ensure!(
            previous.id == id
                && previous.manifest.source == node
                && previous.manifest.replica_id == replica,
            "snapshot ID belongs to another replica"
        );
        ensure!(
            now().saturating_sub(previous.manifest.created_at) < TTL,
            "source snapshot expired"
        );
        previous.manifest.validate()?;
        return Ok(previous.manifest);
    }
    let mut count = 0;
    for entry in fs::read_dir(path.parent().unwrap())? {
        let candidate = entry?.path();
        if candidate.extension().and_then(|p| p.to_str()) != Some("json") {
            continue;
        }
        if let Ok(old) = load::<Export>(&candidate)
            && now().saturating_sub(old.manifest.created_at) >= TTL
            && let Ok(cache) = cache_path(state, &old.id)
        {
            let _old_lock = config::lock_credentials(&candidate)?;
            let _ = fs::remove_dir_all(cache);
            fs::remove_file(candidate)?;
        } else {
            count += 1;
        }
    }
    ensure!(
        count < 8,
        "source snapshot capacity reached; finish or discard an earlier transfer"
    );
    let cache = cache_path(state, &id)?;
    // A failed initial scan has no published manifest and may be prepared again.
    if cache.try_exists()? {
        fs::remove_dir_all(&cache)?;
    }
    private_directory(&cache)?;
    let result = (|| {
        let manifest = super::export_to(store, node, &replica, Some(&cache), || permitted(None))?;
        permitted(None)?;
        save(
            &path,
            &Export {
                id,
                caller: caller.into(),
                credential: digest(token.as_bytes()),
                manifest,
            },
        )?;
        Ok(load::<Export>(&path)?.manifest)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(cache);
    }
    result
}
impl Receiving {
    fn view(&self) -> Value {
        let mut result = self.receipt.view();
        result["protocol"] = json!(2);
        result["digest"] = json!(self.receipt.digest);
        result["progress"] = json!({"bytes":self.bytes,"total":self.manifest.files.iter().map(|f|f.size).sum::<usize>(),"file_index":self.index,"offset":self.offset,"files":self.manifest.files.len()});
        result
    }
    fn pending(&self) -> PathBuf {
        self.receipt.path.parent().unwrap().join(".pending")
    }
    fn check_directory(&self) -> Result<()> {
        ensure!(
            root(&self.expected_root)? == self.expected_root,
            "copy root changed"
        );
        let pending = self.pending();
        ensure!(
            fs::canonicalize(&pending)? == pending,
            "transfer staging directory changed"
        );
        for relative in &self.manifest.directories {
            let path = pending.join(relative);
            ensure!(
                fs::canonicalize(&path)? == path
                    && !fs::symlink_metadata(path)?.file_type().is_symlink(),
                "transfer staging directory changed"
            );
        }
        Ok(())
    }
    fn advance_empty(&mut self) -> Result<()> {
        while let Some(entry) = self.manifest.files.get(self.index) {
            if entry.size != 0 {
                break;
            }
            let path = self.pending().join(&entry.path);
            if !path.try_exists()? {
                new_file(&path, entry.executable)?.sync_all()?;
            }
            verify_blob(&path, entry, &|| Ok(()))?;
            self.index += 1;
        }
        Ok(())
    }
}
fn read_receiving(state: &Path, id: &str, caller: &str, token: &str) -> Result<Receiving> {
    let path = receiving_path(state, id)?;
    let mut record: Receiving = load(&path)?;
    owner(
        caller,
        token,
        &record.receipt.caller,
        &record.receipt.credential,
    )?;
    ensure!(record.receipt.id == id, "unexpected transfer receipt");
    record.manifest = load(&path.with_extension("manifest"))?;
    record.manifest.validate()?;
    ensure!(
        digest(&serde_json::to_vec(&record.manifest)?) == record.receipt.digest,
        "stored manifest checksum mismatch"
    );
    ensure!(
        record.index <= record.manifest.files.len() && record.bytes <= MAX_BYTES,
        "damaged transfer progress"
    );
    let expected_offset = record
        .manifest
        .files
        .get(record.index)
        .map_or(0, |file| file.size);
    ensure!(
        record.offset <= expected_offset
            && record.bytes
                == record.manifest.files[..record.index]
                    .iter()
                    .map(|file| file.size)
                    .sum::<usize>()
                    + record.offset,
        "inconsistent transfer progress"
    );
    Ok(record)
}
pub fn manage(
    state: &Path,
    store: &Store,
    node: &str,
    caller: &str,
    token: &str,
    command: Command,
    permitted: impl Fn(Option<&Path>) -> Result<()>,
) -> Result<Value> {
    permitted(None)?;
    match command {
        Command::Export { id, replica_id } => Ok(serde_json::to_value(export(
            state,
            store,
            node,
            caller,
            token,
            (id, replica_id),
            &permitted,
        )?)?),
        Command::Read { id, index, offset } => {
            let path = record_path(state, "project-copy-exports", &id)?;
            let _lock = config::lock_credentials(&path)?;
            let record: Export = load(&path)?;
            owner(caller, token, &record.caller, &record.credential)?;
            ensure!(
                record.id == id
                    && record.manifest.source == node
                    && now().saturating_sub(record.manifest.created_at) < TTL,
                "source snapshot expired or identity changed"
            );
            record.manifest.validate()?;
            let chunk = read_chunk(&cache_path(state, &id)?, &record.manifest, index, offset)?;
            permitted(None)?;
            Ok(serde_json::to_value(chunk)?)
        }
        Command::Release { id } => {
            let path = record_path(state, "project-copy-exports", &id)?;
            let _lock = config::lock_credentials(&path)?;
            if path.try_exists()? {
                let record: Export = load(&path)?;
                owner(caller, token, &record.caller, &record.credential)?;
                permitted(None)?;
                fs::remove_dir_all(cache_path(state, &id)?)?;
                fs::remove_file(path)?;
            }
            Ok(json!({"released":true}))
        }
        _ => receive_command(state, store, node, caller, token, command, &permitted),
    }
}
fn receive_command(
    state: &Path,
    store: &Store,
    node: &str,
    caller: &str,
    token: &str,
    command: Command,
    permitted: &impl Fn(Option<&Path>) -> Result<()>,
) -> Result<Value> {
    let id = match &command {
        Command::Begin { id, .. }
        | Command::Write { id, .. }
        | Command::Status { id }
        | Command::Finish { id, .. } => id,
        _ => unreachable!(),
    };
    let path = receiving_path(state, id)?;
    // Different copy IDs must share the capacity check across processes.
    let _index = if matches!(&command, Command::Begin { .. }) {
        Some(config::lock_credentials(&path.with_file_name("index"))?)
    } else {
        None
    };
    let _lock = config::lock_credentials(&path)?;
    if let Command::Begin {
        id,
        expected_root,
        digest: expected,
        manifest,
    } = command
    {
        manifest.validate()?;
        ensure!(
            manifest.version == 2 && digest(&serde_json::to_vec(&manifest)?) == expected,
            "snapshot checksum mismatch"
        );
        permitted(Some(&expected_root))?;
        if path.try_exists()? {
            let record = read_receiving(state, &id, caller, token)?;
            ensure!(
                record.receipt.digest == expected && record.expected_root == expected_root,
                "copy ID already used for another snapshot"
            );
            return Ok(record.view());
        }
        ensure!(
            !super::receipt_path(state, &id)?.try_exists()?,
            "copy ID already used by a legacy transfer"
        );
        ensure!(root(&expected_root)? == expected_root, "copy root changed");
        ensure!(
            fs::read_dir(path.parent().unwrap())?
                .filter_map(Result::ok)
                .filter(|e| e.path().extension().and_then(|p| p.to_str()) == Some("json"))
                .count()
                < 256,
            "copy receipt limit reached"
        );
        let container = expected_root.join(format!("cranibox-{id}"));
        let mut record = Receiving {
            receipt: Receipt {
                id,
                caller: caller.into(),
                credential: digest(token.as_bytes()),
                digest: expected,
                project_id: manifest.project_id.clone(),
                name: manifest.name.clone(),
                path: container.join("project"),
                state: "incomplete".into(),
                error: None,
            },
            expected_root,
            manifest,
            index: 0,
            offset: 0,
            bytes: 0,
        };
        config::write_private(
            &path.with_extension("manifest"),
            &serde_json::to_vec(&record.manifest)?,
            false,
        )?;
        save(&path, &record)?;
        let initialized: Result<()> = (|| {
            private_directory(&container)
                .context("copy destination already exists or is not writable")?;
            private_directory(&record.pending())?;
            for directory in &record.manifest.directories {
                permitted(Some(&record.expected_root))?;
                fs::create_dir_all(record.pending().join(directory))?;
            }
            record.check_directory()?;
            record.advance_empty()?;
            record.receipt.state = "receiving".into();
            Ok(())
        })();
        if let Err(error) = initialized {
            record.receipt.error = Some(error.to_string());
        }
        save(&path, &record)?;
        return Ok(record.view());
    }
    let mut record = read_receiving(state, id, caller, token)?;
    permitted(if record.receipt.state == "completed" {
        None
    } else {
        Some(&record.expected_root)
    })?;
    if matches!(command, Command::Status { .. }) {
        return Ok(record.view());
    }
    let expected = match &command {
        Command::Write { digest, .. } | Command::Finish { digest, .. } => digest,
        _ => unreachable!(),
    };
    ensure!(
        record.receipt.digest == *expected,
        "reviewed snapshot checksum changed"
    );
    if record.receipt.state == "completed" {
        return Ok(record.view());
    }
    ensure!(
        record.receipt.state != "incomplete",
        "transfer initialization failed; inspect the target directory"
    );
    if let Command::Write { chunk, .. } = command {
        ensure!(
            record.receipt.state == "receiving",
            "transfer is already being published"
        );
        record.check_directory()?;
        let bytes = decode(&chunk)?;
        let entry = record
            .manifest
            .files
            .get(chunk.index)
            .context("transfer file index out of range")?;
        ensure!(
            chunk.offset <= entry.size && bytes.len() <= entry.size - chunk.offset,
            "transfer chunk exceeds file"
        );
        let output_path = record.pending().join(&entry.path);
        if chunk.index < record.index || chunk.index == record.index && chunk.offset < record.offset
        {
            ensure!(
                chunk.index < record.index || chunk.offset + bytes.len() <= record.offset,
                "overlapping transfer chunk"
            );
            let mut file = open_regular(&output_path, false)?;
            file.seek(SeekFrom::Start(chunk.offset as u64))?;
            let mut existing = vec![0u8; bytes.len()];
            file.read_exact(&mut existing)?;
            ensure!(existing == bytes, "retry chunk differs from stored bytes");
            return Ok(record.view());
        }
        ensure!(
            chunk.index == record.index && chunk.offset == record.offset,
            "transfer offset out of order; query the target receipt"
        );
        if !output_path.try_exists()? {
            ensure!(record.offset == 0, "partial transfer file missing");
            new_file(&output_path, entry.executable)?.sync_all()?;
        }
        let mut output = open_regular(&output_path, true)?;
        ensure!(
            output.metadata()?.len() >= record.offset as u64,
            "partial transfer file was truncated"
        );
        // Bytes written before a crash but not acknowledged by the durable receipt
        // are overwritten only inside this still-private staging file.
        output.set_len(record.offset as u64)?;
        output.seek(SeekFrom::Start(record.offset as u64))?;
        output.write_all(&bytes)?;
        output.sync_all()?;
        drop(output);
        permitted(Some(&record.expected_root))?;
        record.offset += bytes.len();
        record.bytes += bytes.len();
        if record.offset == entry.size {
            verify_blob(&output_path, entry, &|| {
                permitted(Some(&record.expected_root))
            })?;
            record.index += 1;
            record.offset = 0;
            record.advance_empty()?;
        }
    } else {
        ensure!(
            record.index == record.manifest.files.len() && record.offset == 0,
            "transfer still has missing files"
        );
        if record.receipt.state == "receiving" || record.receipt.state == "publishing" {
            let pending = record.pending();
            let published = !pending.try_exists()?
                && record.receipt.path.try_exists()?
                && record.receipt.state == "publishing";
            let directory = if published {
                &record.receipt.path
            } else {
                &pending
            };
            ensure!(
                fs::canonicalize(directory)? == *directory,
                "copy destination changed"
            );
            for file in &record.manifest.files {
                verify_blob(&directory.join(&file.path), file, &|| {
                    permitted(Some(&record.expected_root))
                })?;
            }
            permitted(Some(&record.expected_root))?;
            ensure!(
                root(&record.expected_root)? == record.expected_root,
                "copy root changed"
            );
            if !published {
                record.check_directory()?;
                ensure!(
                    !record.receipt.path.try_exists()?,
                    "copy destination already exists"
                );
                record.receipt.state = "publishing".into();
                save(&path, &record)?;
                fs::rename(pending, &record.receipt.path)?;
            }
            record.receipt.state = "copied".into();
            save(&path, &record)?;
        }
        super::finish(store, node, &mut record.receipt, &|| {
            permitted(Some(&record.expected_root))
        })?;
    }
    permitted(Some(&record.expected_root))?;
    save(&path, &record)?;
    Ok(record.view())
}
