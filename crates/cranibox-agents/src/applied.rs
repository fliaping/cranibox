//! Durable, reversible edits with per-agent ownership and crash-safe file writes.
use crate::{
    edit::{self, Format},
    *,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Clone, Serialize, Deserialize)]
struct Change {
    keys: Vec<String>,
    before: Option<Value>,
    after: Value,
}
#[derive(Clone, Serialize, Deserialize)]
struct File {
    path: PathBuf,
    format: Format,
    original: Option<String>,
    hash: String,
    changes: Vec<Change>,
}
#[derive(Default, Serialize, Deserialize)]
struct Journal {
    agents: BTreeMap<AgentId, Vec<File>>,
}
pub(crate) struct Edit {
    pub path: PathBuf,
    pub format: Format,
    pub keys: Vec<String>,
    pub value: Value,
}
impl Edit {
    pub fn new(path: PathBuf, format: Format, keys: &[&str], value: Value) -> Self {
        Self {
            path,
            format,
            keys: keys.iter().map(|s| (*s).into()).collect(),
            value,
        }
    }
}
fn io(e: impl std::fmt::Display) -> AgentError {
    AgentError::Io(e.to_string())
}
pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| io("missing parent"))?;
    fs::create_dir_all(parent).map_err(io)?;
    let tmp = path.with_extension(format!("cranibox-{}.tmp", std::process::id()));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&tmp).map_err(io)?;
    let result = (|| {
        file.write_all(bytes).map_err(io)?;
        file.sync_all().map_err(io)?;
        fs::rename(&tmp, path).map_err(io)?;
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(io)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
fn hash(s: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(s))
}
fn location(env: &Env) -> PathBuf {
    env.data.join("agents/applied.json")
}
fn locked(env: &Env) -> Result<(fs::File, Journal)> {
    let path = location(env);
    fs::create_dir_all(path.parent().unwrap()).map_err(io)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("lock"))
        .map_err(io)?;
    lock.lock_exclusive().map_err(io)?;
    let journal = match fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).map_err(edit::error)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Journal::default(),
        Err(e) => return Err(io(e)),
    };
    Ok((lock, journal))
}
fn public(files: &[File]) -> Applied {
    Applied {
        files: files
            .iter()
            .map(|f| AppliedFile {
                path: f.path.clone(),
                keys: f.changes.iter().map(|c| c.keys.join(".")).collect(),
                before_hash: f.original.as_deref().map(hash),
                after_hash: f.hash.clone(),
            })
            .collect(),
    }
}
pub(crate) fn apply(id: AgentId, env: &Env, edits: Vec<Edit>) -> Result<Applied> {
    let (_lock, mut journal) = locked(env)?;
    for e in edits {
        let files = journal.agents.entry(id).or_default();
        let text = match fs::read_to_string(&e.path) {
            Ok(s) => Some(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(io(e)),
        };
        let current = text.as_deref().unwrap_or("");
        let doc = edit::parse(current, e.format)?;
        if files
            .iter()
            .find(|f| f.path == e.path)
            .is_some_and(|f| f.hash != hash(current))
        {
            return Err(edit::error(
                "configuration drift detected; review before applying a new model",
            ));
        }
        if !files.iter().any(|f| f.path == e.path) {
            if let Some(original) = &text {
                let backup = PathBuf::from(format!("{}.before-cranibox", e.path.display()));
                if backup.exists() {
                    return Err(edit::error("untracked backup already exists"));
                }
                write(&backup, original.as_bytes())?;
            }
            files.push(File {
                path: e.path.clone(),
                format: e.format,
                original: text.clone(),
                hash: hash(current),
                changes: vec![],
            });
        }
        let file = files.iter_mut().find(|f| f.path == e.path).unwrap();
        if let Some(change) = file.changes.iter_mut().find(|c| c.keys == e.keys) {
            change.after = e.value.clone();
        } else {
            file.changes.push(Change {
                keys: e.keys.clone(),
                before: edit::get(&doc, &e.keys).cloned(),
                after: e.value.clone(),
            });
        }
        let rendered = edit::render(current, e.format, &e.keys, Some(e.value.clone()))?;
        file.hash = hash(&rendered);
        // Persist the undo record before mutating the user's file.
        let bytes = serde_json::to_vec_pretty(&journal).map_err(edit::error)?;
        write(&location(env), &bytes)?;
        let rendered = if id == AgentId::Openclaw {
            openclaw_set(env, &e, &rendered).unwrap_or(rendered)
        } else {
            rendered
        };
        write(&e.path, rendered.as_bytes())?;
        let files = journal.agents.get_mut(&id).unwrap();
        files.iter_mut().find(|f| f.path == e.path).unwrap().hash = hash(&rendered);
        write(
            &location(env),
            &serde_json::to_vec_pretty(&journal).map_err(edit::error)?,
        )?;
        // Reborrow after journaling (also keeps the next edit based on disk).
    }
    Ok(public(
        journal
            .agents
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or_default(),
    ))
}
/// Prefer the agent's supported writer, but accept its result only if every
/// unrelated value survived. Unsupported CLI versions fall back to JSON5.
fn openclaw_set(env: &Env, edit: &Edit, expected: &str) -> Option<String> {
    if edit.path != env.home.join(".openclaw/openclaw.json") {
        return None;
    }
    let binary = crate::adapter(AgentId::Openclaw)?.detect(env).binary?;
    let mut child = Command::new(binary)
        .args([
            "config",
            "set",
            &edit.keys.join("."),
            &edit.value.to_string(),
            "--json",
        ])
        .env("HOME", &env.home)
        .env("OPENCLAW_CONFIG_PATH", &edit.path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let text = fs::read_to_string(&edit.path).ok()?;
                return (edit::parse(&text, edit.format).ok()?
                    == edit::parse(expected, edit.format).ok()?)
                .then_some(text);
            }
            Ok(None) if started.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}
pub(crate) fn clean_openclaw_cache(env: &Env) -> Result<()> {
    let path = env.home.join(".openclaw/agents/main/agent/models.json");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(io(e)),
    };
    let doc = edit::parse(&text, Format::Json5)?;
    let keys = vec!["providers".into(), "cranibox".into()];
    let Some(provider) = edit::get(&doc, &keys) else {
        return Ok(());
    };
    let endpoint = LocalEndpoint::for_agent(AgentId::Openclaw);
    // This is a generated cache, not the user's configuration. Only discard a
    // provider carrying our exact endpoint and local identification key.
    if provider["baseUrl"] == format!("{}/v1", endpoint.base) && provider["apiKey"] == endpoint.key
    {
        let rendered = edit::render(&text, Format::Json5, &keys, None)?;
        write(&path, rendered.as_bytes())?;
    }
    Ok(())
}
pub(crate) fn drift(id: AgentId, env: &Env) -> Result<Drift> {
    let (_lock, journal) = locked(env)?;
    Ok(Drift {
        files: journal
            .agents
            .get(&id)
            .into_iter()
            .flatten()
            .map(|f| DriftFile {
                path: f.path.clone(),
                state: match fs::read_to_string(&f.path) {
                    Ok(s) if hash(&s) == f.hash => DriftState::Clean,
                    Ok(_) => DriftState::Modified,
                    Err(_) => DriftState::Missing,
                },
            })
            .collect(),
    })
}
pub(crate) fn restore(id: AgentId, env: &Env) -> Result<Restored> {
    restore_where(id, env, |_, _| true)
}
pub(crate) fn restore_where(
    id: AgentId,
    env: &Env,
    select: impl Fn(&Path, &[String]) -> bool,
) -> Result<Restored> {
    let (_lock, mut journal) = locked(env)?;
    let mut result = Restored::default();
    for f in journal.agents.get(&id).cloned().unwrap_or_default() {
        let changes: Vec<_> = f
            .changes
            .iter()
            .filter(|c| select(&f.path, &c.keys))
            .collect();
        if changes.is_empty() {
            continue;
        }
        let all = changes.len() == f.changes.len();
        let clean = fs::read_to_string(&f.path).is_ok_and(|s| hash(&s) == f.hash);
        let mode = match fs::read_to_string(&f.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => RestoreMode::Missing,
            Err(e) => return Err(io(e)),
            Ok(s) if all && hash(&s) == f.hash => {
                if let Some(original) = &f.original {
                    write(&f.path, original.as_bytes())?;
                } else {
                    fs::remove_file(&f.path).map_err(io)?;
                }
                RestoreMode::Original
            }
            Ok(mut s) => {
                for change in changes.iter().rev() {
                    let doc = edit::parse(&s, f.format)?;
                    if edit::get(&doc, &change.keys) == Some(&change.after) {
                        s = edit::render(&s, f.format, &change.keys, change.before.clone())?;
                    } else if let Some(Value::Array(current)) = edit::get(&doc, &change.keys)
                        && let Value::Array(after) = &change.after
                        && let Some(before) = match &change.before {
                            None => Some(&[][..]),
                            Some(Value::Array(items)) => Some(items.as_slice()),
                            _ => None,
                        }
                        && after.starts_with(before)
                    {
                        // Native hooks are arrays. If the user appended hooks
                        // later, remove only the exact entries we appended.
                        let added = &after[before.len()..];
                        let remaining: Vec<_> = current
                            .iter()
                            .filter(|item| !added.contains(item))
                            .cloned()
                            .collect();
                        let value = (!remaining.is_empty() || change.before.is_some())
                            .then_some(Value::Array(remaining));
                        s = edit::render(&s, f.format, &change.keys, value)?;
                    }
                }
                write(&f.path, s.as_bytes())?;
                RestoreMode::KeysRemoved
            }
        };
        let backup = PathBuf::from(format!("{}.before-cranibox", f.path.display()));
        if all && backup.exists() {
            fs::remove_file(backup).map_err(io)?;
        }
        let files = journal.agents.get_mut(&id).unwrap();
        if all {
            files.retain(|file| file.path != f.path);
        } else if let Some(file) = files.iter_mut().find(|file| file.path == f.path) {
            file.changes.retain(|c| !select(&file.path, &c.keys));
            if clean && let Ok(text) = fs::read_to_string(&file.path) {
                file.hash = hash(&text);
            }
        }
        result.files.push(RestoredFile {
            path: f.path.clone(),
            mode,
        });
    }
    if journal.agents.get(&id).is_some_and(Vec::is_empty) {
        journal.agents.remove(&id);
    }
    write(
        &location(env),
        &serde_json::to_vec_pretty(&journal).map_err(edit::error)?,
    )?;
    Ok(result)
}
