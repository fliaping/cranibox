use super::*;
use sha2::{Digest, Sha256};
use std::io::Write;
const KEEP: usize = 10_000;
fn hash(line: &str) -> String {
    format!("{:x}", Sha256::digest(line.as_bytes()))
}

pub(super) fn read(path: &Path) -> Result<(Vec<AuditEvent>, Option<u64>)> {
    let text = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), None)),
        Err(e) => return Err(e).context("read access audit"),
    };
    let mut entries = Vec::new();
    let mut previous = String::new();
    let mut next = 0;
    let mut broken = None;
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
        next = entry.index + 1;
        entries.push(entry);
    }
    if !text.is_empty() && !text.ends_with(char::from(10)) {
        broken.get_or_insert(next.saturating_sub(1));
    }
    Ok((entries, broken))
}
pub(super) fn append(path: &Path, mut event: AuditEvent) -> Result<()> {
    let (entries, broken) = read(path)?;
    ensure!(broken.is_none(), "access audit chain broken at {broken:?}");
    let text = if path.exists() {
        std::fs::read_to_string(path)?
    } else {
        String::new()
    };
    if let Some(last) = entries.last() {
        event.index = last.index + 1;
        event.prev = hash(text.lines().last().context("empty audit tail")?);
    }
    let line = serde_json::to_string(&event)?;
    if entries.len() < KEEP {
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
    } else {
        let lines: Vec<_> = text.lines().chain(std::iter::once(line.as_str())).collect();
        let start = lines.len() - (KEEP - 1);
        let dropped = hash(lines[start - 1]);
        let checkpoint = AuditEvent {
            index: entries[start].index - 1,
            at: event.at,
            event: EventKind::Checkpoint,
            from: None,
            name: None,
            capability: None,
            session_key: None,
            basis: None,
            result: dropped.clone(),
            prev: dropped,
            operation: None,
            status: None,
            transport: None,
        };
        let checkpoint = serde_json::to_string(&checkpoint)?;
        // Commit to the dropped prefix and re-link the retained suffix; indexes stay stable.
        let mut output = checkpoint.clone();
        output.push(char::from(10));
        let mut prev = hash(&checkpoint);
        for line in &lines[start..] {
            let mut entry: AuditEvent = serde_json::from_str(line)?;
            entry.prev = prev;
            let encoded = serde_json::to_string(&entry)?;
            prev = hash(&encoded);
            output.push_str(&encoded);
            output.push(char::from(10));
        }
        config::write_private(path, output.as_bytes(), true)?;
    }
    Ok(())
}
