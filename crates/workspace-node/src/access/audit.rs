//! The hash-chained audit log, stored in segments. `access-audit.jsonl` is the active
//! segment; a full segment is renamed to `access-audit.<first>-<last>.jsonl` unchanged and
//! the next one starts with a checkpoint whose `prev` is the hash of the archived last
//! line, so the chain stays verifiable across segments. Old archives are deleted after
//! `RETAIN_SEGMENTS` or `RETAIN_SECS`; the oldest kept segment then starts with a
//! checkpoint that points at deleted history.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    sync::{LazyLock, Mutex},
};
pub(super) const SEGMENT: usize = 5_000;
pub(super) const RETAIN_SEGMENTS: usize = 20;
pub(super) const RETAIN_SECS: u64 = 180 * 86400;
fn hash(line: &str) -> String {
    format!("{:x}", Sha256::digest(line.as_bytes()))
}
fn checkpoint(index: u64, at: u64, prev: String) -> AuditEvent {
    AuditEvent {
        index,
        at,
        event: EventKind::Checkpoint,
        from: None,
        name: None,
        capability: None,
        session_key: None,
        basis: None,
        result: prev.clone(),
        prev,
        operation: None,
        status: None,
        transport: None,
        detail: None,
        review: None,
    }
}
/// One segment, verified on its own: entries, first broken index, and the hash of its
/// last line.
pub(super) struct Segment {
    pub entries: Vec<AuditEvent>,
    pub broken: Option<u64>,
    pub last_hash: Option<String>,
}
#[cfg(test)]
pub(super) fn read(path: &Path) -> Result<(Vec<AuditEvent>, Option<u64>)> {
    let segment = read_segment(path)?;
    Ok((segment.entries, segment.broken))
}
fn read_segment(path: &Path) -> Result<Segment> {
    let text = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Segment {
                entries: vec![],
                broken: None,
                last_hash: None,
            });
        }
        Err(e) => return Err(e).context("read access audit"),
    };
    let mut entries = Vec::new();
    let mut previous = String::new();
    let mut next = 0;
    let mut broken = None;
    let mut last_hash = None;
    for (offset, line) in text.lines().enumerate() {
        let entry: AuditEvent = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(_) => {
                broken.get_or_insert(next);
                break;
            }
        };
        if offset == 0 && entry.event == EventKind::Checkpoint {
            if entry.prev.len() != 64
                || !entry.prev.bytes().all(|b| b.is_ascii_hexdigit())
                || entry.result != entry.prev
            {
                broken.get_or_insert(entry.index);
            }
        } else if entry.prev != previous || entry.index != next {
            broken.get_or_insert(entry.index);
        }
        previous = hash(line);
        last_hash = Some(previous.clone());
        next = entry.index + 1;
        entries.push(entry);
    }
    if !text.is_empty() && !text.ends_with(char::from(10)) {
        broken.get_or_insert(next.saturating_sub(1));
    }
    Ok(Segment {
        entries,
        broken,
        last_hash,
    })
}
/// Archived segments next to `active`, oldest first.
pub(super) fn archives(active: &Path) -> Vec<PathBuf> {
    let Some(dir) = active.parent() else {
        return vec![];
    };
    let mut found: Vec<(u64, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let range = name.strip_prefix("access-audit.")?.strip_suffix(".jsonl")?;
            let (first, last) = range.split_once('-')?;
            let first: u64 = first.parse().ok()?;
            last.parse::<u64>().ok()?;
            Some((first, entry.path()))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, p)| p).collect()
}
/// Segments newest first, each verified and linked to the next older one as it is read.
pub(super) struct History {
    paths: Vec<PathBuf>,
    newer_first: Option<AuditEvent>,
    pub broken: Option<u64>,
}
impl History {
    pub fn open(active: &Path) -> Self {
        let mut paths = archives(active);
        paths.push(active.to_owned());
        paths.reverse();
        Self {
            paths,
            newer_first: None,
            broken: None,
        }
    }
    /// The next older segment's entries, oldest first within the segment.
    pub fn next_segment(&mut self) -> Result<Option<Vec<AuditEvent>>> {
        if self.paths.is_empty() {
            return Ok(None);
        }
        let path = self.paths.remove(0);
        let segment = read_segment(&path)?;
        if let Some(b) = segment.broken {
            self.broken.get_or_insert(b);
        }
        // The newer segment's checkpoint must commit to this segment's last line.
        if let Some(first) = &self.newer_first {
            let linked = first.event == EventKind::Checkpoint
                && segment.last_hash.as_deref() == Some(first.prev.as_str())
                && segment.entries.last().map(|e| e.index + 1) == Some(first.index);
            if !linked {
                self.broken.get_or_insert(first.index);
            }
        }
        self.newer_first = segment.entries.first().cloned();
        Ok(Some(segment.entries))
    }
}
/// Cached tail of the active segment: (file length, entries, next index, last hash).
type Tail = (u64, usize, u64, String);
static TAIL: LazyLock<Mutex<std::collections::HashMap<PathBuf, Tail>>> =
    LazyLock::new(Default::default);
fn write_line(path: &Path, line: &str) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).context("open access audit")?;
    writeln!(file, "{line}").context("append access audit")?;
    file.sync_all()?;
    Ok(())
}
/// Appends one event. The caller holds the access-policy lock.
pub(super) fn append(path: &Path, mut event: AuditEvent) -> Result<()> {
    let length = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let cached = TAIL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(path)
        .filter(|(len, ..)| *len == length && length > 0)
        .cloned();
    let (mut count, mut next, mut last) = match cached {
        Some((_, count, next, last)) => (count, next, Some(last)),
        None => {
            let segment = read_segment(path)?;
            ensure!(
                segment.broken.is_none(),
                "access audit chain broken at {:?}",
                segment.broken
            );
            (
                segment.entries.len(),
                segment.entries.last().map_or(0, |e| e.index + 1),
                segment.last_hash,
            )
        }
    };
    if count >= SEGMENT
        && let Some(previous) = last.clone()
    {
        // The full segment moves aside byte-for-byte; the new one commits to it.
        let first = next - count as u64;
        let archive =
            path.with_file_name(format!("access-audit.{first:010}-{:010}.jsonl", next - 1));
        std::fs::rename(path, &archive).context("archive access audit")?;
        let line = serde_json::to_string(&checkpoint(next, event.at, previous))?;
        write_line(path, &line)?;
        count = 1;
        next += 1;
        last = Some(hash(&line));
        retain(path, event.at);
    }
    event.index = next;
    event.prev = last.unwrap_or_default();
    let line = serde_json::to_string(&event)?;
    write_line(path, &line)?;
    let length = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    TAIL.lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(path.to_owned(), (length, count + 1, next + 1, hash(&line)));
    Ok(())
}
/// Deletes archives beyond the retention limits, oldest first.
fn retain(active: &Path, at: u64) {
    let archives = archives(active);
    let excess = archives.len().saturating_sub(RETAIN_SEGMENTS);
    for (position, archive) in archives.iter().enumerate() {
        let old = std::fs::metadata(archive)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .is_some_and(|t| at.saturating_sub(t.as_secs()) > RETAIN_SECS);
        if position < excess || old {
            let _ = std::fs::remove_file(archive);
        }
    }
}
#[cfg(test)]
pub(super) fn forget_cache() {
    TAIL.lock().unwrap_or_else(|e| e.into_inner()).clear();
}
