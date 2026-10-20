//! Six agent adapters, sharing reversible edits and one ordered registry.
pub use cranibox_contracts::agents::*;
mod applied;
pub mod approvals;
mod edit;
mod sessions;
use applied::Edit;
use edit::Format;
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf};
pub const DSH_VERSION: &str = "0.1.1-rc.2";
pub const DSH_SHA256: &str = "47ec05f45ada5ab87779ae18a90456b5ebff5421dc0ff5c179677d65e1c16057";
struct Adapter(AgentId);
static ADAPTERS: [Adapter; 6] = [
    Adapter(AgentId::DeepseekHarness),
    Adapter(AgentId::Codex),
    Adapter(AgentId::ClaudeCode),
    Adapter(AgentId::Pi),
    Adapter(AgentId::Hermes),
    Adapter(AgentId::Openclaw),
];
static REGISTRY: [&'static dyn AgentAdapter; 6] = [
    &ADAPTERS[0],
    &ADAPTERS[1],
    &ADAPTERS[2],
    &ADAPTERS[3],
    &ADAPTERS[4],
    &ADAPTERS[5],
];
pub fn registry() -> &'static [&'static dyn AgentAdapter] {
    &REGISTRY
}
pub fn adapter(id: AgentId) -> Option<&'static dyn AgentAdapter> {
    registry().iter().copied().find(|a| a.id() == id)
}
pub fn binary(id: AgentId) -> &'static str {
    match id {
        AgentId::ClaudeCode => "claude",
        _ => id.as_str(),
    }
}
pub fn environment(home: PathBuf, data: PathBuf) -> Env {
    Env {
        home,
        data,
        path: std::env::var_os("PATH")
            .map(|p| {
                std::env::split_paths(&p)
                    .filter(|p| p.is_absolute())
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Remove a registered MCP entry without undoing managed models or other servers.
pub fn unregister_mcp(id: AgentId, env: &Env, name: &str) -> Result<Restored> {
    let keys: Vec<String> = match id {
        AgentId::DeepseekHarness => vec![format!("cranibox-mcp-{name}")],
        AgentId::Codex | AgentId::Hermes => vec!["mcp_servers".into(), name.into()],
        AgentId::ClaudeCode | AgentId::Pi => vec!["mcpServers".into(), name.into()],
        AgentId::Openclaw => vec!["mcp".into(), "servers".into(), name.into()],
    };
    applied::restore_where(id, env, |_, owned| owned == keys)
}
fn dir(id: AgentId) -> &'static str {
    match id {
        AgentId::DeepseekHarness => ".dsh",
        AgentId::Codex => ".codex",
        AgentId::ClaudeCode => ".claude",
        AgentId::Pi => ".pi/agent",
        AgentId::Hermes => ".hermes",
        AgentId::Openclaw => ".openclaw",
    }
}
fn executable(path: &std::path::Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    true
}
impl AgentAdapter for Adapter {
    fn id(&self) -> AgentId {
        self.0
    }
    fn display(&self) -> AgentDisplay {
        AgentDisplay {
            name: match self.0 {
                AgentId::DeepseekHarness => "DeepSeek Harness",
                AgentId::Codex => "Codex",
                AgentId::ClaudeCode => "Claude Code",
                AgentId::Pi => "Pi",
                AgentId::Hermes => "Hermes Agent",
                AgentId::Openclaw => "OpenClaw",
            }
            .into(),
            icon: "bot".into(),
            recommended: self.0 == DEFAULT_AGENT,
        }
    }
    fn detect(&self, env: &Env) -> Detection {
        let binary = env.path.iter().filter(|p| p.is_absolute()).find_map(|p| {
            let b = p.join(binary(self.0));
            if executable(&b) {
                Some(b)
            } else {
                #[cfg(windows)]
                {
                    let b = b.with_extension("cmd");
                    if executable(&b) {
                        return Some(b);
                    }
                }
                None
            }
        });
        Detection {
            installed: binary.is_some(),
            binary,
            version: None,
            config_dir: Some(env.home.join(dir(self.0))),
        }
    }
    fn install_plan(&self, _env: &Env) -> InstallPlan {
        let (package, version, method) = match self.0 {
            AgentId::DeepseekHarness => ("@deepseek-ai/dsh", DSH_VERSION, InstallMethod::Npm),
            AgentId::Codex => ("@openai/codex", "", InstallMethod::Manual),
            AgentId::ClaudeCode => ("@anthropic-ai/claude-code", "", InstallMethod::Manual),
            AgentId::Pi => ("@mariozechner/pi-coding-agent", "", InstallMethod::Manual),
            AgentId::Hermes => ("hermes-agent", "", InstallMethod::Manual),
            AgentId::Openclaw => ("openclaw", "", InstallMethod::Manual),
        };
        InstallPlan {
            package: package.into(),
            version: version.into(),
            method,
            integrity: (self.0 == DEFAULT_AGENT).then(|| format!("sha256:{DSH_SHA256}")),
        }
    }
    fn wire(&self, env: &Env, ep: &LocalEndpoint, pick: &ModelPick) -> Result<Applied> {
        if ep != &LocalEndpoint::for_agent(self.0) {
            return Err(edit::error(
                "only the Cranibox loopback endpoint is supported",
            ));
        }
        if !pick.default.contains('/')
            || (!pick.models.is_empty() && !pick.models.iter().any(|m| m.id == pick.default))
        {
            return Err(edit::error("select an available provider/model"));
        }
        let mut edits = Vec::new();
        let root = env.home.join(dir(self.0));
        let model_list:Vec<_>=pick.models.iter().map(|m|json!({"id":m.id,"name":m.display,"contextWindow":m.context.unwrap_or(128000),"reasoning":m.reasoning,"input":if m.vision {vec!["text","image"]}else {vec!["text"]},"maxTokens":8192,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0}})).collect();
        let mut add = |path: PathBuf, fmt, keys: &[&str], value| {
            edits.push(Edit::new(path, fmt, keys, value))
        };
        match self.0 {
            AgentId::DeepseekHarness => {
                let mut profiles = vec![root.join("profiles/web"), root.join("profiles/headless")];
                if let Ok(entries) = std::fs::read_dir(root.join("profiles")) {
                    for e in entries.flatten() {
                        if e.path().is_dir() && !profiles.contains(&e.path()) {
                            profiles.push(e.path());
                        }
                    }
                }
                for profile in profiles {
                    for (name, api, path) in [
                        ("cranibox", "openai-completions", "/v1"),
                        ("cranibox-responses", "openai-responses", "/v1"),
                        ("cranibox-anthropic", "anthropic-messages", ""),
                    ] {
                        add(
                            profile.join("cordis.patch.yml"),
                            Format::Cordis,
                            &["llm-pi-ai", "config", "providers", name],
                            json!({"displayName":"Cranibox","apiKeyEnv":"CRANIBOX_GATEWAY_KEY","api":api,"baseURL":format!("{}{path}",ep.base),"models":model_list}),
                        );
                    }
                    add(
                        profile.join("cordis.patch.yml"),
                        Format::Cordis,
                        &["agent-default-model", "config"],
                        json!({"provider":"cranibox", "model":pick.default}),
                    );
                }
                add(
                    root.join(".env"),
                    Format::Env,
                    &["CRANIBOX_GATEWAY_KEY"],
                    json!(ep.key),
                );
            }
            AgentId::Codex => {
                let file = root.join("config.toml");
                for (key, value) in [
                    ("model_provider", json!("cranibox")),
                    ("model", json!(pick.default)),
                    (
                        "model_catalog_json",
                        json!(root.join("cranibox-models.json")),
                    ),
                ] {
                    add(file.clone(), Format::Toml, &[key], value);
                }
                add(
                    file,
                    Format::Toml,
                    &["model_providers", "cranibox"],
                    json!({"name":"Cranibox","base_url":format!("{}/v1",ep.base),"wire_api":"responses","experimental_bearer_token":ep.key}),
                );
                add(root.join("cranibox-models.json"),Format::Json,&["models"],json!(pick.models.iter().map(|m|json!({"slug":m.id,"display_name":m.display,"description":m.display,"context_window":m.context.unwrap_or(128000),"supports_reasoning_summaries":m.reasoning,"supported_in_api":true,"visibility":"list","support_verbosity":false,"default_verbosity":null,"default_reasoning_summary":"none","supports_parallel_tool_calls":true,"apply_patch_tool_type":"freeform","experimental_supported_tools":[],"effective_context_window_percent":95,"default_reasoning_level":if m.reasoning {"medium"} else {"none"},"supported_reasoning_levels":if m.reasoning {vec![json!({"effort":"medium","description":"Balanced reasoning"})]} else {vec![]},"shell_type":"shell_command","priority":0,"base_instructions":"You are a helpful coding assistant.","truncation_policy":{"mode":"tokens","limit":10000},"input_modalities":if m.vision {vec!["text","image"]}else {vec!["text"]}})).collect::<Vec<_>>()));
            }
            AgentId::ClaudeCode => {
                let file = root.join("settings.json");
                let fast = pick.small_fast.as_ref().unwrap_or(&pick.default);
                for (k, v) in [
                    ("ANTHROPIC_BASE_URL", ep.base.as_str()),
                    ("ANTHROPIC_AUTH_TOKEN", ep.key.as_str()),
                    ("ANTHROPIC_DEFAULT_OPUS_MODEL", &pick.default),
                    ("ANTHROPIC_DEFAULT_SONNET_MODEL", &pick.default),
                    ("ANTHROPIC_DEFAULT_HAIKU_MODEL", fast),
                    ("ANTHROPIC_SMALL_FAST_MODEL", fast),
                    ("CLAUDE_CODE_SUBAGENT_MODEL", &pick.default),
                ] {
                    add(file.clone(), Format::Json, &["env", k], json!(v));
                }
                add(file, Format::Json, &["model"], json!(pick.default));
            }
            AgentId::Pi => {
                add(
                    root.join("models.json"),
                    Format::Json,
                    &["providers", "cranibox"],
                    json!({"name":"Cranibox","baseUrl":format!("{}/v1",ep.base),"api":"openai-completions","apiKey":ep.key,"compat":{"sendSessionAffinityHeaders":true},"models":model_list}),
                );
                add(
                    root.join("settings.json"),
                    Format::Json,
                    &["defaultProvider"],
                    json!("cranibox"),
                );
                add(
                    root.join("settings.json"),
                    Format::Json,
                    &["defaultModel"],
                    json!(pick.default),
                );
            }
            AgentId::Hermes => {
                let mut files = vec![root.join("config.yaml")];
                if let Ok(entries) = std::fs::read_dir(root.join("profiles")) {
                    for e in entries.flatten() {
                        if e.path().is_dir() {
                            files.push(e.path().join("config.yaml"));
                        }
                    }
                }
                for file in files {
                    add(
                        file.clone(),
                        Format::Yaml,
                        &["providers", "cranibox"],
                        json!({"base_url":format!("{}/v1",ep.base),"api_key":ep.key}),
                    );
                    add(
                        file.clone(),
                        Format::Yaml,
                        &["model", "provider"],
                        json!("cranibox"),
                    );
                    add(
                        file,
                        Format::Yaml,
                        &["model", "default"],
                        json!(pick.default),
                    );
                }
            }
            AgentId::Openclaw => {
                let file = root.join("openclaw.json");
                add(
                    file.clone(),
                    Format::Json5,
                    &["models", "mode"],
                    json!("merge"),
                );
                add(
                    file.clone(),
                    Format::Json5,
                    &["models", "providers", "cranibox"],
                    json!({"baseUrl":format!("{}/v1",ep.base),"apiKey":ep.key,"api":"openai-completions","models":model_list}),
                );
                add(
                    file,
                    Format::Json5,
                    &["agents", "defaults", "model", "primary"],
                    json!(format!("cranibox/{}", pick.default)),
                );
            }
        }
        let applied = applied::apply(self.0, env, edits)?;
        if self.0 == AgentId::Openclaw {
            applied::clean_openclaw_cache(env)?;
        }
        Ok(applied)
    }
    fn unwire(&self, env: &Env) -> Result<Restored> {
        let restored = applied::restore(self.0, env)?;
        if self.0 == AgentId::Openclaw {
            applied::clean_openclaw_cache(env)?;
        }
        Ok(restored)
    }
    fn drift(&self, env: &Env) -> Result<Drift> {
        applied::drift(self.0, env)
    }
    fn headless(&self, task: &HeadlessTask) -> Result<Launch> {
        if task.prompt.trim().is_empty() {
            return Err(edit::error("task instructions are required"));
        }
        let args: Vec<String> = match self.0 {
            AgentId::DeepseekHarness => vec!["headless".into(), task.prompt.clone()],
            AgentId::Codex => vec![
                "exec".into(),
                "--sandbox".into(),
                "workspace-write".into(),
                "--color".into(),
                "never".into(),
                "-".into(),
            ],
            AgentId::ClaudeCode => vec![
                "--print".into(),
                "--output-format".into(),
                if task.json { "json" } else { "text" }.into(),
            ],
            AgentId::Pi => vec![
                "--print".into(),
                "--mode".into(),
                if task.json { "json" } else { "text" }.into(),
                task.prompt.clone(),
            ],
            AgentId::Hermes => vec!["chat".into(), "--query".into(), task.prompt.clone()],
            AgentId::Openclaw => vec![
                "agent".into(),
                "--local".into(),
                "--agent".into(),
                "main".into(),
                "--message".into(),
                task.prompt.clone(),
            ],
        };
        let mut launch = Launch {
            program: binary(self.0).into(),
            args,
            env: BTreeMap::new(),
            stdin: matches!(self.0, AgentId::Codex | AgentId::ClaudeCode)
                .then(|| task.prompt.clone()),
        };
        if let Some(session) = &task.session_id {
            match self.0 {
                AgentId::ClaudeCode => launch.args.extend(["--resume".into(), session.clone()]),
                AgentId::Pi => launch.args.extend(["--session".into(), session.clone()]),
                AgentId::Openclaw => launch.args.extend(["--session-id".into(), session.clone()]),
                _ => return Err(AgentError::Unsupported),
            }
        }
        if self.0 == AgentId::Openclaw && task.json {
            launch.args.push("--json".into());
        }
        Ok(launch)
    }
    fn sessions(&self, roots: &Roots) -> Vec<SessionRef> {
        sessions::discover(self.0, roots)
    }
    fn register_mcp(&self, env: &Env, mcp: &McpSpec) -> Result<Applied> {
        if mcp.name.is_empty() || mcp.name.contains('.') {
            return Err(edit::error("invalid MCP name"));
        }
        let config = json!({"command":mcp.command,"args":mcp.args,"env":mcp.env});
        if self.0 == AgentId::DeepseekHarness {
            if !mcp
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                || mcp.name.len() > 32
            {
                return Err(edit::error("invalid DSH MCP server name"));
            }
            let id = format!("cranibox-mcp-{}", mcp.name);
            return applied::apply(
                self.0,
                env,
                vec![Edit::new(
                    env.home.join(".dsh/agent-workspace-mcp.cordis.yml"),
                    Format::Cordis,
                    &[&id],
                    json!({"insert":[{"id":id,"name":"@deepseek-ai/dsh-mcp-client","config":{"serverName":mcp.name,"transport":"stdio","command":mcp.command,"args":mcp.args,"env":mcp.env}}]}),
                )],
            );
        }
        if self.0 == AgentId::Pi {
            return applied::apply(
                self.0,
                env,
                vec![
                    Edit::new(
                        env.home.join(".pi/agent/mcp.json"),
                        Format::Json,
                        &["mcpServers", &mcp.name],
                        config,
                    ),
                    Edit::new(
                        env.home.join(".pi/agent/extensions/cranibox-mcp.ts"),
                        Format::Text,
                        &["content"],
                        json!(include_str!("pi_mcp.ts")),
                    ),
                ],
            );
        }
        let (file, format, keys) = match self.0 {
            AgentId::Codex => (
                env.home.join(".codex/config.toml"),
                Format::Toml,
                vec!["mcp_servers", &mcp.name],
            ),
            AgentId::ClaudeCode => (
                env.home.join(".claude.json"),
                Format::Json,
                vec!["mcpServers", &mcp.name],
            ),
            AgentId::Hermes => (
                env.home.join(".hermes/config.yaml"),
                Format::Yaml,
                vec!["mcp_servers", &mcp.name],
            ),
            AgentId::Pi => unreachable!(),
            AgentId::Openclaw => (
                env.home.join(".openclaw/openclaw.json"),
                Format::Json5,
                vec!["mcp", "servers", &mcp.name],
            ),
            AgentId::DeepseekHarness => unreachable!(),
        };
        applied::apply(self.0, env, vec![Edit::new(file, format, &keys, config)])
    }
    fn caps(&self) -> AgentCaps {
        AgentCaps {
            web_ui: matches!(
                self.0,
                AgentId::DeepseekHarness | AgentId::Hermes | AgentId::Openclaw
            ),
            acp: matches!(self.0, AgentId::Codex | AgentId::ClaudeCode),
            rpc: self.0 == AgentId::Pi,
            hooks: matches!(
                self.0,
                AgentId::ClaudeCode | AgentId::DeepseekHarness | AgentId::Pi
            ),
            approval: matches!(
                self.0,
                AgentId::DeepseekHarness | AgentId::Pi | AgentId::ClaudeCode
            ),
        }
    }
}
