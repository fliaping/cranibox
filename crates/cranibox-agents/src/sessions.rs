use crate::*;
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
};
pub(crate) fn discover(id: AgentId, roots: &Roots) -> Vec<SessionRef> {
    let default = roots
        .home
        .join(super::dir(id))
        .join(if id == AgentId::ClaudeCode {
            "projects"
        } else if id == AgentId::Openclaw {
            "agents/main/sessions"
        } else {
            "sessions"
        });
    let mut paths = vec![default];
    paths.extend(roots.extra.clone());
    let mut out = vec![];
    if id == AgentId::Hermes {
        hermes(&roots.home.join(".hermes/state.db"), &mut out);
    }
    for path in paths {
        walk(id, &path, 5, &mut out);
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
    out.dedup_by(|a, b| a.id == b.id);
    out.truncate(64);
    out
}
fn hermes(path: &Path, out: &mut Vec<SessionRef>) {
    use rusqlite::{Connection, OpenFlags};
    // Hermes owns this database: discovery must never create or migrate it.
    let Ok(db) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return;
    };
    let _ = db.busy_timeout(std::time::Duration::from_millis(100));
    let Ok(mut query) = db.prepare(
        "SELECT id, substr(title, 1, 512), CAST(COALESCE(ended_at, started_at) AS INTEGER), cwd FROM sessions WHERE length(id) BETWEEN 1 AND 256 ORDER BY COALESCE(ended_at, started_at) DESC LIMIT 64",
    ) else {
        return;
    };
    let Ok(rows) = query.query_map([], |row| {
        Ok(SessionRef {
            agent: AgentId::Hermes,
            id: row.get(0)?,
            cwd: row.get::<_, Option<String>>(3)?.map(PathBuf::from),
            title: row.get(1)?,
            updated_at: row.get(2)?,
            path: Some(path.to_owned()),
        })
    }) else {
        return;
    };
    out.extend(rows.flatten());
}
fn walk(id: AgentId, path: &Path, depth: u8, out: &mut Vec<SessionRef>) {
    if depth == 0 || out.len() >= 4096 {
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten().take(4096) {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let p = entry.path();
        if kind.is_dir() {
            walk(id, &p, depth - 1, out);
            continue;
        }
        if !kind.is_file() || !p.extension().is_some_and(|s| s == "jsonl" || s == "json") {
            continue;
        }
        let Ok(file) = fs::File::open(&p) else {
            continue;
        };
        // Never read an unbounded transcript or return its message content.
        use std::io::Read;
        let reader = BufReader::new(file.take(65536));
        for line in reader.lines().take(32).map_while(std::result::Result::ok) {
            let Ok(mut value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value["type"] == "session_meta" {
                value = value["payload"].clone();
            }
            let session = value["sessionId"].as_str().or_else(|| value["id"].as_str());
            if let Some(session) = session.filter(|s| !s.is_empty() && s.len() <= 256) {
                out.push(SessionRef {
                    agent: id,
                    id: session.into(),
                    cwd: value["cwd"].as_str().map(PathBuf::from),
                    title: None,
                    updated_at: entry
                        .metadata()
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs()),
                    path: Some(p.clone()),
                });
                break;
            }
        }
    }
}
