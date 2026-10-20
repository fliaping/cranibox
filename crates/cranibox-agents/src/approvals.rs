//! Private, transport-neutral approval queue for native Agent hooks.
use crate::*;
use cranibox_contracts::app::*;
use serde::Deserialize;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Deserialize)]
struct Request {
    id: String,
    agent: AgentId,
    tool: String,
    detail: String,
    created_at: u64,
    expires_at: u64,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn directory(env: &Env) -> PathBuf {
    env.data.join("agents/approvals")
}
fn read(path: &Path) -> Option<Request> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 64 * 1024 {
        return None;
    }
    let req: Request = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    if req.tool.len() > 1024
        || req.detail.len() > 32768
        || req.created_at > now()
        || req.expires_at <= now()
        || req.expires_at > req.created_at.saturating_add(300)
    {
        return None;
    }
    Some(req)
}
fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
pub fn pending(env: &Env, node: &str) -> Result<Vec<ApprovalCard>> {
    let directory = directory(env);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(edit::error(e)),
    };
    let mut cards = Vec::new();
    for entry in entries.take(4096).flatten() {
        let path = entry.path();
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if path.extension().and_then(|s| s.to_str()) != Some("json")
            || !valid_id(id)
            || directory.join(format!("{id}.decision")).exists()
        {
            continue;
        }
        let Some(req) = read(&path) else { continue };
        if req.id != id {
            continue;
        }
        cards.push(ApprovalCard {
            id: format!("agent:{id}"),
            kind: ApprovalKind::AgentTool,
            from_node: None,
            on_node: node.into(),
            agent: Some(req.agent),
            title_key: "approval.agent_tool".into(),
            title_args: BTreeMap::from([("tool".into(), req.tool)]),
            detail: Some(req.detail),
            risk: Risk::High,
            created_at: req.created_at,
            expires_at: Some(req.expires_at),
            actions: vec![ApprovalAction::AllowOnce, ApprovalAction::Deny],
        });
    }
    cards.sort_by_key(|card| card.created_at);
    Ok(cards)
}
pub fn decide(env: &Env, id: &str, action: ApprovalAction) -> Result<()> {
    let id = id
        .strip_prefix("agent:")
        .filter(|id| valid_id(id))
        .ok_or_else(|| edit::error("invalid approval ID"))?;
    if !matches!(action, ApprovalAction::AllowOnce | ApprovalAction::Deny) {
        return Err(edit::error("approval only permits allow_once or deny"));
    }
    let directory = directory(env);
    // Serialize the check and publication; create_new also excludes other writers.
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(directory.join("decisions.lock"))
        .map_err(edit::error)?;
    fs2::FileExt::lock_exclusive(&lock).map_err(edit::error)?;
    let req = read(&directory.join(format!("{id}.json")))
        .filter(|r| r.id == id)
        .ok_or_else(|| edit::error("approval expired or withdrawn"))?;
    let path = directory.join(format!("{id}.decision"));
    if path.exists() {
        return Err(edit::error("approval already decided"));
    }
    let temp = directory.join(format!("{id}.decision.tmp"));
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp).map_err(edit::error)?;
    file.write_all(
        &serde_json::to_vec(&json!({"id":req.id,"decision":action})).map_err(edit::error)?,
    )
    .map_err(edit::error)?;
    file.sync_all().map_err(edit::error)?;
    fs::rename(temp, path).map_err(edit::error)?;
    Ok(())
}

/// Install the native hooks that have a verified blocking approval seam.
/// Other adapters keep their own permission prompts until a native bridge exists.
pub fn install(env: &Env, id: AgentId) -> Result<Applied> {
    applied::apply(id, env, edits(env, id)?)
}

pub(crate) fn edits(env: &Env, id: AgentId) -> Result<Vec<Edit>> {
    if !matches!(
        id,
        AgentId::DeepseekHarness | AgentId::Pi | AgentId::ClaudeCode
    ) {
        return Ok(vec![]);
    }
    let root = env.home.join(dir(id));
    let bridge = root.join("cranibox-approval.mjs");
    let source = include_str!("approval_bridge.mjs")
        .replace("__QUEUE_JSON__", &json!(directory(env)).to_string());
    let mut edits = vec![Edit::new(
        bridge.clone(),
        Format::Text,
        &["content"],
        json!(source),
    )];
    if id == AgentId::ClaudeCode {
        let file = root.join("settings.json");
        let mut hooks = match fs::read_to_string(&file) {
            Ok(text) => {
                let doc = edit::parse(&text, Format::Json)?;
                match doc.get("hooks").and_then(|h| h.get("PreToolUse")) {
                    None => vec![],
                    Some(Value::Array(hooks)) => hooks.clone(),
                    Some(_) => return Err(edit::error("PreToolUse hooks must be an array")),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(edit::error(e)),
        };
        // A stable entry makes repeated managed-model updates idempotent.
        #[cfg(unix)]
        let quote = |path: &Path| format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"));
        #[cfg(windows)]
        let quote = |path: &Path| format!("\"{}\"", path.to_string_lossy());
        let node = env
            .path
            .iter()
            .map(|p| p.join(if cfg!(windows) { "node.exe" } else { "node" }))
            .find(|p| super::executable(p))
            .unwrap_or_else(|| PathBuf::from("node"));
        let hook = json!({"matcher":"*","hooks":[{"type":"command","command":format!("{} {} --claude-hook",quote(&node),quote(&bridge)),"timeout":310}]});
        if !hooks.contains(&hook) {
            hooks.push(hook);
        }
        edits.push(Edit::new(
            file,
            Format::Json,
            &["hooks", "PreToolUse"],
            json!(hooks),
        ));
    } else if id == AgentId::Pi {
        edits.push(Edit::new(root.join("extensions/cranibox-approval.ts"), Format::Text, &["content"], json!("import { request } from '../cranibox-approval.mjs';\nexport default function(pi) { pi.on('tool_call', async event => { try { if (await request('pi', event.toolName, JSON.stringify(event.input))) return; } catch {} return { block: true, reason: 'Cranibox approval denied or expired' }; }); }\n")));
    } else {
        let plugin = root.join("cranibox-approval-plugin.mjs");
        edits.push(Edit::new(plugin.clone(), Format::Text, &["content"], json!("import { request } from './cranibox-approval.mjs';\nexport const name='cranibox-approval';\nexport const inject=['approval'];\nexport function apply(ctx) { ctx.on('approval/request', async req => { try { return await request('deepseek-harness', req.toolName, req.reason, req.signal) ? 'allowed-once' : 'rejected'; } catch { return 'unavailable'; } }, { prepend: true }); }\n")));
        // The wrapper loads this overlay in both Web and headless modes.
        edits.push(Edit::new(
            root.join("agent-workspace-mcp.cordis.yml"),
            Format::Cordis,
            &["cranibox-approval"],
            json!({"insert":[{"id":"cranibox-approval","name":plugin}]}),
        ));
    }
    Ok(edits)
}
