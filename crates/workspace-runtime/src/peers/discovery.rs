//! Passive discovery of running Codex and Claude Code sessions.
//!
//! Nothing registers or sends heartbeats: this reads the transcript files both tools
//! already write and treats a recently modified transcript as a running session.
//! - Claude Code: `$CLAUDE_CONFIG_DIR` or `~/.claude`, `projects/<dir>/<session>.jsonl`;
//!   lines carry `sessionId`, `cwd`, `gitBranch`, `type` and `timestamp`.
//! - Codex: `$CODEX_HOME` or `~/.codex`, `sessions/YYYY/MM/DD/rollout-*.jsonl`; the first
//!   line is `session_meta` with `payload.id`, `payload.cwd` and `payload.git.branch`.
//!
//! The result is a local, read-only snapshot for the device owner. Files are only read
//! in bounded slices; content other than the routing fields is never returned.
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A transcript untouched for this long is not considered a running session.
pub const RUNNING_WINDOW: Duration = Duration::from_secs(30 * 60);
/// A transcript written within this window is reported as working right now.
const WORKING_WINDOW: Duration = Duration::from_secs(90);
const HEAD_BYTES: u64 = 64 * 1024;
const TAIL_BYTES: u64 = 64 * 1024;
const MAX_FILES: usize = 4096;
const MAX_SESSIONS: usize = 64;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Discovered {
    /// Session UUID, usable as a message or task `target_session`.
    pub id: String,
    pub harness: &'static str,
    pub cwd: Option<String>,
    /// Repository directory name (git top level), when the cwd is inside one.
    pub repo: Option<String>,
    pub branch: Option<String>,
    /// `working` (transcript written in the last 90 s) or `idle`.
    pub activity: &'static str,
    /// Kind of the last transcript entry, such as `user`, `assistant` or `tool`.
    pub last_event: Option<String>,
    pub last_active: u64,
}

fn epoch(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn valid_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| !id.is_nil()) && value.len() == 36
}
fn read_slice(path: &Path, from_end: bool, limit: u64) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut buffer = Vec::new();
    if from_end && len > limit {
        file.seek(SeekFrom::Start(len - limit)).ok()?;
    }
    file.take(limit).read_to_end(&mut buffer).ok()?;
    let mut text = String::from_utf8_lossy(&buffer).into_owned();
    if from_end && len > limit {
        // Drop the partial first line.
        text = text.split_once('\n').map(|(_, rest)| rest.to_owned())?;
    }
    Some(text)
}
fn lines(text: &str) -> impl DoubleEndedIterator<Item = serde_json::Value> + '_ {
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
}
fn string(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 4096)
        .map(str::to_owned)
}
/// Git top-level directory name and current branch for `cwd`, read from `.git`.
fn git(cwd: &Path) -> (Option<String>, Option<String>) {
    let mut dir = Some(cwd);
    while let Some(current) = dir {
        let marker = current.join(".git");
        if marker.exists() {
            let repo = current
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
            let head_dir = if marker.is_file() {
                // Worktrees and submodules: "gitdir: <path>".
                std::fs::read_to_string(&marker).ok().and_then(|s| {
                    s.trim()
                        .strip_prefix("gitdir:")
                        .map(|p| current.join(p.trim()))
                })
            } else {
                Some(marker)
            };
            let branch = head_dir
                .and_then(|dir| std::fs::read_to_string(dir.join("HEAD")).ok())
                .and_then(|head| {
                    head.trim()
                        .strip_prefix("ref: refs/heads/")
                        .map(str::to_owned)
                });
            return (repo, branch);
        }
        dir = current.parent();
    }
    (None, None)
}
fn transcripts(
    root: &Path,
    depth: usize,
    out: &mut Vec<(PathBuf, SystemTime)>,
    cutoff: SystemTime,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() && depth > 0 {
            transcripts(&path, depth - 1, out, cutoff);
        } else if kind.is_file()
            && path.extension().is_some_and(|e| e == "jsonl")
            && let Ok(modified) = entry.metadata().and_then(|m| m.modified())
            && modified >= cutoff
        {
            out.push((path, modified));
        }
    }
}
fn finish(
    id: String,
    harness: &'static str,
    cwd: Option<String>,
    branch: Option<String>,
    last_event: Option<String>,
    modified: SystemTime,
    now: SystemTime,
) -> Discovered {
    let (repo, git_branch) = cwd
        .as_deref()
        .map(|cwd| git(Path::new(cwd)))
        .unwrap_or_default();
    let working = now.duration_since(modified).unwrap_or_default() < WORKING_WINDOW;
    Discovered {
        id,
        harness,
        cwd,
        repo,
        // The transcript records the branch at write time; the work tree is current.
        branch: git_branch.or(branch),
        activity: if working { "working" } else { "idle" },
        last_event,
        last_active: epoch(modified),
    }
}
fn claude(path: &Path, modified: SystemTime, now: SystemTime) -> Option<Discovered> {
    let tail = read_slice(path, true, TAIL_BYTES)?;
    let mut entries = lines(&tail).rev();
    let last = entries.next()?;
    let routed = std::iter::once(last.clone())
        .chain(entries)
        .find(|v| v.get("sessionId").is_some() && v.get("cwd").is_some())?;
    let id = string(&routed["sessionId"])
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))?;
    if !valid_uuid(&id) {
        return None;
    }
    let event = string(&last["type"]).map(|kind| {
        let tool = last["message"]["content"].as_array().is_some_and(|parts| {
            parts
                .iter()
                .any(|p| matches!(p["type"].as_str(), Some("tool_use" | "tool_result")))
        });
        if tool { "tool".to_owned() } else { kind }
    });
    Some(finish(
        id,
        "claude-code",
        string(&routed["cwd"]),
        string(&routed["gitBranch"]),
        event,
        modified,
        now,
    ))
}
fn codex(path: &Path, modified: SystemTime, now: SystemTime) -> Option<Discovered> {
    let head = read_slice(path, false, HEAD_BYTES)?;
    let meta = lines(&head).find(|v| v["type"] == "session_meta")?;
    let payload = &meta["payload"];
    let id = string(&payload["id"])?;
    if !valid_uuid(&id) {
        return None;
    }
    let tail = read_slice(path, true, TAIL_BYTES).unwrap_or_default();
    let event = lines(&tail).next_back().and_then(|last| {
        let kind = last["payload"]["type"].as_str().or(last["type"].as_str())?;
        Some(match kind {
            "function_call" | "function_call_output" | "custom_tool_call" | "local_shell_call" => {
                "tool".to_owned()
            }
            "message" => string(&last["payload"]["role"]).unwrap_or_else(|| "message".into()),
            other => other.chars().take(40).collect(),
        })
    });
    Some(finish(
        id,
        "codex",
        string(&payload["cwd"]),
        string(&payload["git"]["branch"]),
        event,
        modified,
        now,
    ))
}

/// Locations scanned, from the environment the node runs in.
pub struct Roots {
    pub claude: PathBuf,
    pub codex: PathBuf,
}
impl Roots {
    pub fn from_env(home: &Path) -> Self {
        let pick = |var: &str, default: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(default))
        };
        Self {
            claude: pick("CLAUDE_CONFIG_DIR", ".claude"),
            codex: pick("CODEX_HOME", ".codex"),
        }
    }
}

/// Running sessions, most recently active first.
pub fn discover(roots: &Roots, now: SystemTime) -> Vec<Discovered> {
    let cutoff = now.checked_sub(RUNNING_WINDOW).unwrap_or(UNIX_EPOCH);
    let mut found = Vec::new();
    let mut files = Vec::new();
    transcripts(&roots.claude.join("projects"), 1, &mut files, cutoff);
    found.extend(files.drain(..).filter_map(|(p, m)| claude(&p, m, now)));
    transcripts(&roots.codex.join("sessions"), 3, &mut files, cutoff);
    found.extend(files.drain(..).filter_map(|(p, m)| codex(&p, m, now)));
    found.sort_by(|a, b| b.last_active.cmp(&a.last_active).then(a.id.cmp(&b.id)));
    // One entry per session (Claude can split a session across files).
    let mut seen = std::collections::BTreeSet::new();
    found.retain(|s| seen.insert(s.id.clone()));
    found.truncate(MAX_SESSIONS);
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn write(path: &Path, lines: &[serde_json::Value]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = std::fs::File::create(path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
    }
    #[test]
    fn finds_running_claude_and_codex_sessions_with_repo_branch_and_activity() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("work/cranibox");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/feat/x\n").unwrap();
        let cwd = repo.join("crates").to_string_lossy().into_owned();
        std::fs::create_dir_all(&cwd).unwrap();
        let claude_id = "11111111-1111-4111-8111-111111111111";
        let codex_id = "22222222-2222-4222-8222-222222222222";
        let roots = Roots {
            claude: root.path().join(".claude"),
            codex: root.path().join(".codex"),
        };
        write(
            &roots
                .claude
                .join(format!("projects/-work-cranibox/{claude_id}.jsonl")),
            &[
                serde_json::json!({"type":"user","sessionId":claude_id,"cwd":cwd,"gitBranch":"old","message":{"content":"secret prompt"}}),
                serde_json::json!({"type":"assistant","sessionId":claude_id,"cwd":cwd,"message":{"content":[{"type":"tool_use","name":"Bash"}]}}),
            ],
        );
        write(
            &roots
                .codex
                .join("sessions/2026/10/19/rollout-2026-10-19T08-00-00-x.jsonl"),
            &[
                serde_json::json!({"type":"session_meta","payload":{"id":codex_id,"cwd":"/nonexistent/elsewhere","git":{"branch":"main"}}}),
                serde_json::json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":"secret answer"}}),
            ],
        );
        // Stale and malformed transcripts are ignored.
        let stale = roots
            .claude
            .join("projects/-old/33333333-3333-4333-8333-333333333333.jsonl");
        write(
            &stale,
            &[
                serde_json::json!({"type":"user","sessionId":"33333333-3333-4333-8333-333333333333","cwd":"/"}),
            ],
        );
        std::fs::File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(3 * 3600))
            .unwrap();
        write(
            &roots.claude.join("projects/-bad/not-a-session.jsonl"),
            &[serde_json::json!({"type":"user","sessionId":"x","cwd":"/"})],
        );
        let now = SystemTime::now();
        let found = discover(&roots, now);
        assert_eq!(found.len(), 2, "{found:?}");
        let claude = found.iter().find(|s| s.harness == "claude-code").unwrap();
        assert_eq!(claude.id, claude_id);
        assert_eq!(claude.repo.as_deref(), Some("cranibox"));
        assert_eq!(
            claude.branch.as_deref(),
            Some("feat/x"),
            "work tree beats the transcript"
        );
        assert_eq!(
            (claude.activity, claude.last_event.as_deref()),
            ("working", Some("tool"))
        );
        let codex = found.iter().find(|s| s.harness == "codex").unwrap();
        assert_eq!(
            (
                codex.id.as_str(),
                codex.branch.as_deref(),
                codex.repo.as_deref()
            ),
            (codex_id, Some("main"), None)
        );
        assert_eq!(codex.last_event.as_deref(), Some("assistant"));
        let json = serde_json::to_string(&found).unwrap();
        assert!(
            !json.contains("secret"),
            "transcript content is never returned"
        );
        // Later, the same files read as idle.
        let later = discover(&roots, now + Duration::from_secs(600));
        assert!(later.iter().all(|s| s.activity == "idle"));
        assert!(discover(&roots, now + RUNNING_WINDOW + Duration::from_secs(5)).is_empty());
    }
    #[test]
    fn missing_roots_yield_nothing() {
        let root = tempfile::tempdir().unwrap();
        let roots = Roots {
            claude: root.path().join("none"),
            codex: root.path().join("none2"),
        };
        assert!(discover(&roots, SystemTime::now()).is_empty());
    }
}
