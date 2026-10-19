use crate::common::*;
use fs2::FileExt;
use futures_util::{StreamExt, stream};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, env, fs, path::Path, time::UNIX_EPOCH};

pub fn agent_catalog() -> Vec<(&'static str, String, &'static str)> {
    cranibox_agents::registry()
        .iter()
        .map(|a| {
            (
                a.id().as_str(),
                a.display().name,
                cranibox_agents::binary(a.id()),
            )
        })
        .collect()
}
pub fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}
fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or_default()
}
fn object(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub async fn versions(ctx: &Context, paths: &[String]) -> Vec<String> {
    let cache_file = ctx.state().join("command-versions-v1.json");
    let cache = json_file(&cache_file);
    let timestamp = chrono::Utc::now().timestamp_millis() as f64 / 1000.;
    let ttl = env::var("AGENT_WORKSPACE_VERSION_CACHE_TTL")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(3600.)
        .max(30.);
    let results = stream::iter(paths.iter().map(|path| {
        let cache = &cache;
        async move {
            if path.is_empty() {
                return (String::new(), Value::Null);
            }
            let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.into());
            let fingerprint = fs::metadata(&resolved)
                .map(|metadata| {
                    format!(
                        "{}:{}:{}",
                        resolved.display(),
                        metadata
                            .modified()
                            .ok()
                            .and_then(|v| v.duration_since(UNIX_EPOCH).ok())
                            .map(|v| v.as_nanos())
                            .unwrap_or(0),
                        metadata.len()
                    )
                })
                .unwrap_or_else(|_| path.clone());
            let cached = &cache["entries"][path];
            if cached["fingerprint"] == fingerprint
                && cached["version"].is_string()
                && timestamp - cached["checked_at"].as_f64().unwrap_or(0.) <= ttl
            {
                return (text(&cached["version"]).to_owned(), cached.clone());
            }
            let result = ctx.probe(&[path, "--version"], 5.).await;
            let version = result
                .output()
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned();
            let entry = json!({"fingerprint":fingerprint,"version":version,"checked_at":timestamp});
            (version, entry)
        }
    }))
    .buffered(4)
    .collect::<Vec<_>>()
    .await;
    let entries: Map<String, Value> = paths
        .iter()
        .zip(&results)
        .filter(|(path, _)| !path.is_empty())
        .map(|(path, (_, value))| (path.clone(), value.clone()))
        .collect();
    if !entries.is_empty() {
        // Agents and browser probes run concurrently; merge under a shared file lock.
        let save = || -> anyhow::Result<()> {
            fs::create_dir_all(ctx.state())?;
            let lock = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(ctx.state().join("command-versions-v1.lock"))?;
            lock.lock_exclusive()?;
            let mut merged = object(json_file(&cache_file)["entries"].clone());
            merged.extend(entries);
            atomic_write(&cache_file, json!({"entries":merged}).to_string(), 0o600)
        };
        let _ = save();
    }
    results.into_iter().map(|(version, _)| version).collect()
}

fn writable(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
    }
    #[cfg(not(unix))]
    {
        fs::metadata(path).is_ok_and(|m| !m.permissions().readonly())
    }
}
pub async fn workspace(ctx: &Context) -> Value {
    let docker = command_path("docker");
    #[cfg(unix)]
    let socket = {
        use std::os::unix::fs::FileTypeExt;
        fs::metadata("/var/run/docker.sock").is_ok_and(|m| m.file_type().is_socket())
    };
    #[cfg(not(unix))]
    let socket = false;
    let (mode, state, label, detail, reachable, risk) = if socket {
        (
            "socket",
            "elevated",
            "Docker socket connected",
            "The Agent may control containers reachable through the mounted socket.",
            true,
            "high",
        )
    } else if !docker.is_empty() && ctx.probe(&[&docker, "info"], 4.).await.code == 0 {
        (
            "isolated",
            "ready",
            "Isolated Docker available",
            "The Agent can create containers in the workspace Docker daemon.",
            true,
            "medium",
        )
    } else if !docker.is_empty() {
        (
            "unavailable",
            "safe",
            "Docker disabled",
            "The CLI is installed, but no Docker daemon is reachable.",
            false,
            "low",
        )
    } else {
        (
            "disabled",
            "safe",
            "Docker disabled",
            "No Docker CLI or daemon is available inside this workspace.",
            false,
            "low",
        )
    };
    let sudo = ctx.sudo().await;
    #[cfg(unix)]
    let (uid, gid) = unsafe { (json!(libc::getuid()), json!(libc::getgid())) };
    #[cfg(not(unix))]
    let (uid, gid) = (Value::Null, Value::Null);
    let username = ctx.probe(&["id", "-un"], 1.).await;
    let username = if username.code == 0 {
        username.stdout.trim().to_owned()
    } else {
        env::var("USERNAME")
            .or_else(|_| env::var("USER"))
            .unwrap_or_default()
    };
    json!({"user":username,"uid":uid,"gid":gid,"home":env::var("HOME").unwrap_or_else(|_| ctx.root.display().to_string()),"root":ctx.workspace,"persistent_root":ctx.root,"writable":writable(&ctx.root),"sudo":{"available":sudo,"label":if sudo {"Passwordless container administration"} else {"Unavailable"}},"docker":{"mode":mode,"state":state,"label":label,"detail":detail,"cli":docker,"daemon_reachable":reachable,"risk":risk}})
}
pub async fn agents(ctx: &Context) -> Value {
    let mut definitions: Vec<Value> = cranibox_agents::registry().iter().map(|a| {
        let id=a.id();
        let mut row=json!({"id":id.as_str(),"label":a.display().name,"binary":cranibox_agents::binary(id),"install_name":id.as_str(),"launch":cranibox_agents::binary(id),"accent":"green","recommended":a.display().recommended});
        match id {
            cranibox_agents::AgentId::DeepseekHarness => {
                row["surface"] = json!("web");
                row["launch"] = json!("deepseek-harness web --no-open --port 3080");
                row["web_label"] = json!("Web UI");
                row["web_path"] = json!("/proxy/3080/");
                row["web_target"] = json!("127.0.0.1:3080");
                row["service"] = json!("deepseek-harness");
                row["web_service"] = json!("deepseek-harness");
            }
            cranibox_agents::AgentId::Codex => row["accent"] = json!("violet"),
            cranibox_agents::AgentId::ClaudeCode => row["accent"] = json!("amber"),
            cranibox_agents::AgentId::Hermes => {
                row["accent"] = json!("cyan");
                row["web_label"] = json!("Dashboard");
                row["web_path"] = json!("/proxy/9119/");
                row["web_target"] = json!("127.0.0.1:9119");
                row["web_service"] = json!("hermes-dashboard");
                row["launch"] = json!("hermes setup --portal");
            }
            cranibox_agents::AgentId::Openclaw => {
                row["accent"] = json!("rose");
                row["surface"] = json!("web");
                row["web_label"] = json!("Control UI");
                row["web_path"] = json!("/proxy/18789/");
                row["web_target"] = json!("127.0.0.1:18789");
                row["web_service"] = json!("openclaw-gateway");
                row["service"] = json!("openclaw-gateway");
                row["launch"] = json!("openclaw onboard");
            }
            cranibox_agents::AgentId::Pi => {}
        }
        row
    }).collect();
    let paths = definitions
        .iter()
        .map(|row| command_path(text(&row["binary"])))
        .collect::<Vec<_>>();
    let versions = versions(ctx, &paths).await;
    let selected = env::var("AGENT_WORKSPACE_AGENT")
        .unwrap_or_else(|_| cranibox_agents::DEFAULT_AGENT.as_str().into());
    for ((row, path), version) in definitions.iter_mut().zip(paths).zip(versions) {
        row["state"] = json!(if path.is_empty() { "missing" } else { "ready" });
        row["installed"] = json!(!path.is_empty());
        row["path"] = json!(path);
        row["version"] = json!(version);
        row["selected"] = json!(row["id"] == selected);
    }
    json!(definitions)
}
pub fn skills(ctx: &Context) -> Value {
    let mut rows: BTreeMap<String, Value> = BTreeMap::new();
    for (root, agent) in [
        (".agents/skills", ""),
        (".codex/skills", "codex"),
        (".claude/skills", "claude-code"),
        (".hermes/skills", "hermes"),
        (".dsh/skills", "deepseek-harness"),
    ] {
        for candidate in directories(ctx.root.join(root)) {
            let fallback = candidate.file_name().unwrap_or_default().to_string_lossy();
            if fallback.starts_with('.') || !candidate.join("SKILL.md").is_file() {
                continue;
            }
            let mut name = fallback.to_string();
            let mut description = String::new();
            let content = read(candidate.join("SKILL.md"));
            if content.starts_with("---") {
                for line in content
                    .lines()
                    .skip(1)
                    .take_while(|line| line.trim() != "---")
                {
                    if let Some((key, value)) = line.split_once(':') {
                        let value = value.trim().trim_matches(['\"', '\'']);
                        if key.trim() == "name" && valid_name(value) {
                            name = value.into();
                        }
                        if key.trim() == "description" && ![">", "|"].contains(&value) {
                            description = value.into();
                        }
                    }
                }
            }
            let shared = agent.is_empty();
            let row = rows.entry(name.clone()).or_insert_with(|| json!({"id":name,"name":name,"description":description,"path":candidate,"scope":if shared {"shared"} else {"agent"},"agents":[],"shared":shared}));
            if shared {
                row["path"] = json!(candidate);
                row["scope"] = json!("shared");
                row["shared"] = json!(true);
            }
            let available = row["agents"].as_array_mut().expect("agents list");
            for id in if shared {
                vec!["codex", "deepseek-harness"]
            } else {
                vec![agent]
            } {
                if !available.contains(&json!(id)) {
                    available.push(json!(id));
                }
            }
            if text(&row["description"]).is_empty() {
                row["description"] = json!(description);
            }
        }
    }
    let mut rows: Vec<_> = rows.into_values().collect();
    rows.sort_by_key(|row| text(&row["name"]).to_lowercase());
    for row in &mut rows {
        row["agents"]
            .as_array_mut()
            .unwrap()
            .sort_by_key(|id| agent_catalog().iter().position(|(agent, _, _)| id == agent));
    }
    json!({"manager":"npx skills","root":ctx.root.join(".agents/skills"),"count":rows.len(),"shared_count":rows.iter().filter(|row| row["shared"] == true).count(),"items":rows})
}

pub fn mcp_configs(ctx: &Context) -> Vec<Map<String, Value>> {
    let codex: Value = toml::from_str::<toml::Value>(&read(ctx.root.join(".codex/config.toml")))
        .ok()
        .and_then(|value| serde_json::to_value(value).ok())
        .unwrap_or(Value::Null);
    let hermes: Value =
        serde_yaml::from_str(&read(ctx.root.join(".hermes/config.yaml"))).unwrap_or(Value::Null);
    agent_catalog()
        .iter()
        .map(|(id, _, _)| match *id {
            "codex" => object(codex["mcp_servers"].clone()),
            "claude-code" => object(json_file(ctx.root.join(".claude.json"))["mcpServers"].clone()),
            "hermes" => object(hermes["mcp_servers"].clone()),
            "deepseek-harness" => {
                let mut servers = object(
                    json_file(
                        ctx.root
                            .join(".local/share/agent-workspace/deepseek-mcp.json"),
                    )["servers"]
                        .clone(),
                );
                let patch: Value = serde_yaml::from_str(&read(
                    ctx.root.join(".dsh/agent-workspace-mcp.cordis.yml"),
                ))
                .unwrap_or(Value::Null);
                for entry in patch
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|p| p["insert"].as_array().into_iter().flatten())
                {
                    let config = &entry["config"];
                    if entry["name"] == "@deepseek-ai/dsh-mcp-client"
                        && let Some(name) = config["serverName"].as_str()
                    {
                        servers.insert(name.to_owned(), config.clone());
                    }
                }
                servers
            }
            "pi" => object(json_file(ctx.root.join(".pi/agent/mcp.json"))["mcpServers"].clone()),
            "openclaw" => {
                let config: Value =
                    json5::from_str(&read(ctx.root.join(".openclaw/openclaw.json")))
                        .unwrap_or(Value::Null);
                object(config["mcp"]["servers"].clone())
            }
            _ => Map::new(),
        })
        .collect()
}
pub fn mcp_agents(ctx: &Context, name: &str) -> Vec<&'static str> {
    agent_catalog()
        .iter()
        .zip(mcp_configs(ctx))
        .filter(|(_, configs)| configs.contains_key(name))
        .map(|((id, _, _), _)| *id)
        .collect()
}
pub fn safe_url(value: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(value) else {
        return "Remote endpoint".into();
    };
    if !["http", "https"].contains(&url.scheme()) || url.host_str().is_none() {
        return "Remote endpoint".into();
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string().trim_end_matches('/').to_owned()
}
pub fn mcp(ctx: &Context) -> Value {
    let mut rows: BTreeMap<String, Value> = BTreeMap::new();
    let mut support = Vec::new();
    for ((id, label, binary), configs) in agent_catalog().iter().zip(mcp_configs(ctx)) {
        support.push(json!({"id":id,"label":label,"installed":!command_path(binary).is_empty(),"configured":configs.len()}));
        for (name, config) in configs {
            if !valid_name(&name) {
                continue;
            }
            let url = text(&config["url"]);
            let command = text(&config["command"]);
            let executable = Path::new(command)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            let args = array(&config["args"]);
            let (transport, detail, manager) = if !config.is_object() {
                ("unknown", "Configured".into(), "native")
            } else if !url.is_empty() {
                let transport = config["transport"]
                    .as_str()
                    .or_else(|| config["type"].as_str())
                    .unwrap_or("http");
                (transport, safe_url(url), "native")
            } else {
                (
                    "stdio",
                    format!(
                        "{} ({} argument{})",
                        if executable.is_empty() {
                            "STDIO command"
                        } else {
                            &executable
                        },
                        args.len(),
                        if args.len() == 1 { "" } else { "s" }
                    ),
                    if executable == "mcpm" && args.first() == Some(&json!("run")) {
                        "mcpm"
                    } else {
                        "native"
                    },
                )
            };
            let row = rows
                .entry(name.clone())
                .or_insert_with(|| json!({"id":name,"name":name,"agents":[],"global":false}));
            row["agents"].as_array_mut().unwrap().push(json!({"id":id,"label":label,"transport":transport,"detail":detail,"enabled":config["enabled"] != false,"manager":manager}));
        }
    }
    let mut rows: Vec<_> = rows.into_values().collect();
    rows.sort_by_key(|row| text(&row["name"]).to_lowercase());
    for row in &mut rows {
        let agents = array(&row["agents"]);
        let first = &agents[0];
        let transport = if agents.iter().all(|a| a["transport"] == first["transport"]) {
            first["transport"].clone()
        } else {
            json!("mixed")
        };
        let detail = if agents.iter().all(|a| a["detail"] == first["detail"]) {
            first["detail"].clone()
        } else {
            json!("Per-Agent configuration")
        };
        let global = agents.len() > 1;
        let managed = agents.iter().any(|a| a["manager"] == "mcpm");
        let enabled = agents.iter().any(|a| a["enabled"] == true);
        row["global"] = json!(global);
        row["managed"] = json!(managed);
        row["transport"] = transport;
        row["detail"] = detail;
        row["enabled"] = json!(enabled);
    }
    let mcpm = command_path("mcpm");
    json!({"manager":{"id":"mcpm","label":"MCPM","installed":!mcpm.is_empty(),"path":mcpm},"count":rows.len(),"global_count":rows.iter().filter(|r| r["global"] == true).count(),"items":rows,"agents":support})
}

pub fn user_service_process(ctx: &Context, name: &str) -> Option<u32> {
    let pid: u32 = read(ctx.root.join(format!(".local/run/user-systemd/{name}.pid")))
        .trim()
        .parse()
        .ok()?;
    if pid == 0 {
        return None;
    }
    #[cfg(unix)]
    {
        if pid > i32::MAX as u32 || unsafe { libc::kill(pid as i32, 0) } != 0 {
            return None;
        }
    }
    #[cfg(not(unix))]
    {
        None
    }
    #[cfg(unix)]
    Some(pid)
}
pub fn user_services(ctx: &Context) -> Vec<Value> {
    directories(ctx.units()).into_iter().filter(|path| path.extension().is_some_and(|ext| ext == "service") && path.is_file()).map(|unit| {
        let id = unit.file_stem().unwrap().to_string_lossy();
        let description = read(&unit).lines().find_map(|line| line.strip_prefix("Description=").map(str::to_owned)).unwrap_or_else(|| id.to_string());
        let pid = user_service_process(ctx, &id);
        json!({"id":id,"unit":unit.file_name().unwrap().to_string_lossy(),"name":description,"kind":"systemd","scope":"user","status":if pid.is_some() {"running"} else {"inactive"},"enabled":ctx.units().join("default.target.wants").join(unit.file_name().unwrap()).is_symlink(),"pid":pid,"log":ctx.logs().join(format!("{id}.log"))})
    }).collect()
}
pub async fn services(ctx: &Context) -> Value {
    let sudo = ctx.sudo().await;
    let paths = directories("/run/service")
        .into_iter()
        .filter(|path| {
            path.is_dir() && !path.file_name().unwrap().to_string_lossy().starts_with('.')
        })
        .collect::<Vec<_>>();
    let mut rows = user_services(ctx);
    let system = stream::iter(paths.iter().map(|path| async move {
        let dir = path.to_string_lossy();
        let args = if sudo { vec!["sudo", "-n", "s6-svstat", &dir] } else { vec!["s6-svstat", &dir] };
        let probe = ctx.probe(&args, 3.).await;
        let status = if probe.code != 0 {"unknown"} else if probe.output().starts_with("up ") {"running"} else {"stopped"};
        let id = path.file_name().unwrap().to_string_lossy();
        json!({"id":id,"unit":id,"name":id,"kind":"s6","scope":"system","status":status,"enabled":true,"pid":null,"detail":probe.output(),"log":""})
    })).buffered(8).collect::<Vec<_>>().await;
    rows.extend(system);
    json!(rows)
}
pub async fn ports(ctx: &Context) -> Value {
    let result = ctx
        .probe(&["lsof", "-nP", "-iTCP", "-sTCP:LISTEN"], 8.)
        .await;
    let rows = result.stdout.lines().skip(1).filter_map(|line| {
        let cols = line.split_whitespace().collect::<Vec<_>>();
        if cols.len() < 9 { return None; }
        Some(json!({"command":cols[0],"pid":cols[1].parse::<u32>().ok(),"user":cols[2],"address":cols[8]}))
    }).collect::<Vec<_>>();
    json!(rows)
}
pub fn routes(ctx: &Context) -> Value {
    let root = ctx.root.join("proxyctl");
    let caddy = root.join("bin/caddy").is_file();
    let configured = root.join("env").is_file();
    let vars = env_file(&root.join("env"));
    let get = |key: &str| vars.get(key).cloned().unwrap_or_default();
    let mut port = get("PROXY_PUBLIC_PORT");
    if port.is_empty()
        && let Some((_, candidate)) = get("CODE_SERVER_PROXY_DOMAIN").rsplit_once(':')
        && candidate.parse::<u16>().is_ok()
    {
        port = candidate.into();
    }
    let loaded = json_file(root.join("routes.json"));
    let loaded = if loaded.is_object() {
        &loaded["routes"]
    } else {
        &loaded
    };
    let scheme = vars
        .get("PROXY_PUBLIC_SCHEME")
        .map(String::as_str)
        .unwrap_or("https");
    let rows = array(loaded).iter().filter(|row| row.is_object()).map(|row| {
        let host = text(&row["host"]);
        json!({"name":row["name"].as_str().filter(|s| !s.is_empty()).unwrap_or(host),"host":host,"target":text(&row["target"]),"public_url":if host.is_empty() {String::new()} else { format!("{scheme}://{host}{}", if port.is_empty() {String::new()} else {format!(":{port}")}) }})
    }).collect::<Vec<_>>();
    json!({"installed":caddy && configured,"mode":if caddy && configured {"custom-domain"} else if caddy != configured {"incomplete"} else {"local"},"root_domain":get("PROXY_ROOT_DOMAIN"),"code_server_domain":get("CODE_SERVER_PROXY_DOMAIN"),"listen":get("PROXY_LISTEN"),"routes":rows})
}
pub fn bridge_url(ctx: &Context) -> String {
    env::var("AGENT_DESKTOP_BRIDGE_URL").unwrap_or_else(|_| {
        let port = env::var("AGENT_DESKTOP_BRIDGE_PORT")
            .unwrap_or_else(|_| read(ctx.root.join(".config/agent-workspace/desktop-bridge-port")));
        let port = port
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .unwrap_or(8765);
        format!("http://127.0.0.1:{port}")
    })
}
pub async fn desktop(ctx: &Context, rows: &Value) -> Value {
    let integration = extension_installed(ctx, "agent-workspace.selkies-desktop-");
    let running = array(rows)
        .iter()
        .any(|r| r["id"] == "svc-selkies" && r["status"] == "running");
    let installed = ctx.root.join("bin/agent-desktop-mcp").exists();
    let url = bridge_url(ctx);
    let mcp_agents = mcp_agents(ctx, "agent-desktop");
    let mut state = json!({"installed":installed,"running":array(rows).iter().any(|r| r["id"] == "agent-desktop-bridge" && r["status"] == "running"),"state":if installed {"stopped"} else {"missing"},"paused":false,"backend_available":false,"backend":"selkies-pixelflux-wayland","bridge_url":url,"native_url":"http://127.0.0.1:8764","native_bind":"0.0.0.0:8764","native_security":"internal upstream; never publish this port","loopback_only":true,"resolution":{"width":0,"height":0},"active_sessions":[],"capabilities":[],"mcp_agents":mcp_agents,"mcp_configured":!mcp_agents.is_empty(),"window_enumeration_complete":false,"browser_policy":"Use managed Chromium CDP for web tasks; use desktop Computer Use for native applications."});
    // The bridge returns structured diagnostics even when its desktop backend is unavailable.
    if let Ok(response) = ctx
        .http
        .get(format!("{url}/v1/status"))
        .timeout(std::time::Duration::from_millis(300))
        .send()
        .await
        && let Ok(value) = response.json::<Value>().await
        && let Some(values) = value.as_object()
    {
        state.as_object_mut().unwrap().extend(values.clone());
        state["installed"] = json!(installed);
        state["running"] = json!(true);
        state["mcp_agents"] = json!(mcp_agents);
        state["mcp_configured"] = json!(!mcp_agents.is_empty());
    }
    json!({"installed":integration,"runtime_available":Path::new("/run/service/svc-selkies").exists(),"running":running,"state":if integration && running {"ready"} else if !integration {"missing"} else {"stopped"},"service":"svc-selkies","local_url":"https://localhost:3001","code_server_path":"/proxy/3000/","install_target":"desktop","computer_use":state})
}
pub fn debug_port() -> u16 {
    env::var("AGENT_WORKSPACE_BROWSER_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v| *v >= 1024)
        .unwrap_or(9222)
}
pub fn browser_profile(ctx: &Context) -> std::path::PathBuf {
    env::var_os("AGENT_WORKSPACE_BROWSER_PROFILE")
        .map(Into::into)
        .unwrap_or_else(|| ctx.root.join(".config/agent-browser"))
}
pub async fn browser(ctx: &Context) -> Value {
    let binary = env::var("AGENT_WORKSPACE_BROWSER_BIN")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            let chromium = command_path("chromium");
            if chromium.is_empty() {
                command_path("google-chrome")
            } else {
                chromium
            }
        });
    let version = versions(ctx, std::slice::from_ref(&binary)).await.remove(0);
    let major = regex::Regex::new(r"(?:^|[^0-9])([0-9]+)(?:[.][0-9]+){2,3}")
        .unwrap()
        .captures(&version)
        .and_then(|c| c[1].parse::<u32>().ok())
        .unwrap_or(0);
    let profile = browser_profile(ctx);
    let legacy = env::var_os("AGENT_WORKSPACE_BROWSER_LEGACY_PROFILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| ctx.root.join(".config/chromium"));
    let desired = env::var("AGENT_WORKSPACE_BROWSER_PROFILE_NAME")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Agent Workspace (Managed)".into());
    let local = json_file(profile.join("Local State"));
    let directory = text(&local["profile"]["last_used"]);
    let valid = directory == "Default"
        || directory
            .strip_prefix("Profile ")
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()));
    let prefs = json_file(
        profile
            .join(if valid { directory } else { "Default" })
            .join("Preferences"),
    );
    let name = text(&prefs["profile"]["name"]).trim();
    let mut pids = Vec::new();
    let mut others = Vec::new();
    for path in directories("/proc") {
        let Some(pid) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let content = fs::read(path.join("cmdline")).unwrap_or_default();
        let args = content
            .split(|b| *b == 0)
            .filter(|v| !v.is_empty())
            .map(|v| String::from_utf8_lossy(v).to_string())
            .collect::<Vec<_>>();
        if let Some(first) = args.first() {
            let executable = Path::new(first.split_whitespace().next().unwrap_or(first))
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            if ["chromium", "chrome", "google-chrome", "wrapped-chromium"]
                .contains(&executable.as_ref())
                && !args.iter().any(|v| v.contains("--type="))
            {
                if args.contains(&format!("--user-data-dir={}", profile.display())) {
                    pids.push(pid);
                } else {
                    others.push(pid);
                }
            }
        }
    }
    pids.sort_unstable();
    others.sort_unstable();
    let port = debug_port();
    let remote = ctx
        .get_json(&format!("http://127.0.0.1:{port}/json/version"), 0.25)
        .await
        .is_ok_and(|value| !text(&value["webSocketDebuggerUrl"]).is_empty());
    let mcp_agents = mcp_agents(ctx, "chrome-devtools");
    let default = [
        ctx.root.join(".local/share/applications/chromium.desktop"),
        "/usr/share/applications/chromium.desktop".into(),
    ]
    .iter()
    .any(|p| read(p).contains("agent-workspace-browser"));
    let migration = !profile.exists() && legacy.join("Local State").is_file();
    let state = if binary.is_empty() || major < 136 {
        "unsupported"
    } else if mcp_agents.is_empty() {
        "needs-setup"
    } else if !others.is_empty() && (migration || pids.is_empty()) {
        "needs-restart"
    } else if pids.is_empty() {
        "not-running"
    } else if !remote {
        "needs-restart"
    } else {
        "ready"
    };
    json!({"installed":!binary.is_empty(),"supported":major >= 136,"state":state,"binary":binary,"version":version,"major_version":major,"profile":profile,"profile_name":if name.is_empty() {&desired} else {name},"desired_profile_name":desired,"profile_name_applied":name == desired,"running":!pids.is_empty(),"pids":pids,"other_browser_pids":others,"remote_debugging":remote,"debug_port":port,"mcp_agents":mcp_agents,"mcp_configured":!mcp_agents.is_empty(),"connection_mode":"managed-loopback-cdp","legacy_profile":legacy,"profile_exists":profile.exists(),"migration_pending":migration,"legacy_browser_running":!others.is_empty(),"managed_default":default,"exposed":false})
}
pub async fn tailscale(ctx: &Context) -> Value {
    let binary = command_path("tailscale");
    let mut base = json!({"installed":!binary.is_empty(),"state":if binary.is_empty() {"optional"} else {"needs-login"},"backend_state":"","mode":"userspace","hostname":"","dns_name":"","ips":[],"url":"","serve_enabled":false,"proxy":"127.0.0.1:1055"});
    if binary.is_empty() {
        return base;
    }
    if ctx.units().join("tailscaled-workspace.service").is_file()
        && user_service_process(ctx, "tailscaled-workspace").is_none()
    {
        base["backend_state"] = json!("Stopped");
        return base;
    }
    let result = ctx.probe(&[&binary, "status", "--json"], 6.).await;
    let loaded: Value = serde_json::from_str(&result.stdout).unwrap_or(Value::Null);
    if !loaded.is_object() {
        return base;
    }
    let backend = text(&loaded["BackendState"]);
    let node = &loaded["Self"];
    let dns = text(&node["DNSName"]).trim_end_matches('.');
    base["state"] = json!(if backend == "Running" {
        "connected"
    } else {
        "needs-login"
    });
    base["backend_state"] = json!(backend);
    base["hostname"] = json!(text(&node["HostName"]));
    base["dns_name"] = json!(dns);
    base["ips"] = json!(
        array(&node["TailscaleIPs"])
            .iter()
            .filter(|v| v.is_string())
            .collect::<Vec<_>>()
    );
    base["url"] = json!(if dns.is_empty() {
        String::new()
    } else {
        format!("https://{dns}")
    });
    if backend == "Running" {
        let result = ctx.probe(&[&binary, "serve", "status", "--json"], 6.).await;
        let served: Value = serde_json::from_str(&result.stdout).unwrap_or(Value::Null);
        base["serve_enabled"] = json!(served.as_object().is_some_and(|o| !o.is_empty()));
    }
    base
}
fn extension_installed(ctx: &Context, prefix: &str) -> bool {
    directories(ctx.root.join(".local/share/code-server/extensions"))
        .iter()
        .any(|p| p.file_name().unwrap().to_string_lossy().starts_with(prefix))
}
fn capabilities(ctx: &Context) -> Value {
    let routes = routes(ctx);
    let entries = [
        (
            "source",
            "Application source",
            if ctx.source.join(".git").is_dir() {
                "ready"
            } else {
                "missing"
            },
            ctx.source.display().to_string(),
        ),
        (
            "code-server",
            "code-server",
            if ctx.root.join("opt/code-server/bin/code-server").is_file() {
                "ready"
            } else {
                "missing"
            },
            ctx.root.join("opt/code-server").display().to_string(),
        ),
        (
            "control-center",
            "Control Center",
            if extension_installed(ctx, "agent-workspace.control-center-") {
                "ready"
            } else {
                "missing"
            },
            ctx.root
                .join(".local/share/code-server/extensions")
                .display()
                .to_string(),
        ),
        (
            "proxyctl",
            "Custom-domain routing",
            if routes["installed"] == true {
                "ready"
            } else if routes["mode"] == "incomplete" {
                "needs-attention"
            } else {
                "optional"
            },
            "Optional Caddy routing backend".into(),
        ),
        (
            "mcpm",
            "Global MCP manager",
            if command_path("mcpm").is_empty() {
                "optional"
            } else {
                "ready"
            },
            ctx.root.join("opt/mcpm").display().to_string(),
        ),
        (
            "desktop",
            "Selkies Desktop",
            if extension_installed(ctx, "agent-workspace.selkies-desktop-")
                && Path::new("/run/service/svc-selkies").exists()
            {
                "ready"
            } else {
                "optional"
            },
            "Image runtime + code-server integration".into(),
        ),
        (
            "tailscale",
            "Tailscale network",
            if command_path("tailscale").is_empty() {
                "optional"
            } else {
                "ready"
            },
            ctx.root.join("opt/tailscale").display().to_string(),
        ),
        (
            "custom-services",
            "Custom services",
            if ctx.root.join("custom-services.d").is_dir() {
                "ready"
            } else {
                "missing"
            },
            ctx.root.join("custom-services.d").display().to_string(),
        ),
    ];
    json!(entries.into_iter().map(|(id,label,state,detail)| json!({"id":id,"label":label,"state":state,"detail":detail})).collect::<Vec<_>>())
}
fn bootstrap(ctx: &Context) -> Value {
    let target = env::var("AGENT_WORKSPACE_BOOTSTRAP").unwrap_or_else(|_| "remote".into());
    let selected = env::var("AGENT_WORKSPACE_AGENT")
        .unwrap_or_else(|_| cranibox_agents::DEFAULT_AGENT.as_str().into());
    let marker = ctx.state().join("onboarding-v1.complete");
    json!({"target":target,"foundation_ready":ctx.state().join(format!("bootstrap-{target}.complete")).is_file(),"selected_agent":selected,"agent_ready":selected == "none" || ctx.state().join(format!("agent-{selected}.complete")).is_file(),"onboarding":{"version":1,"complete":marker.is_file(),"marker":marker}})
}
pub fn doctor(ctx: &Context) -> Value {
    let mut checks = ["bash", "curl", "git", "jq", "tar", "node", "python3"]
        .iter()
        .map(|name| {
            let path = command_path(name);
            json!({"id":name,"state":if path.is_empty() {"missing"} else {"ready"},"detail":path})
        })
        .collect::<Vec<_>>();
    checks.push(json!({"id":"config-root","state":if ctx.root.is_dir() {"ready"} else {"missing"},"detail":ctx.root}));
    checks.push(json!({"id":"config-writable","state":if writable(&ctx.root) {"ready"} else {"failed"},"detail":ctx.root}));
    json!({"healthy":checks.iter().all(|check| check["state"] == "ready"),"checks":checks})
}
pub fn locale(ctx: &Context) -> Value {
    let config = env::var_os("CODE_SERVER_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| ctx.root.join(".config/code-server/config.yaml"));
    let current = read(ctx.state().join("locale"));
    let current = normalize_locale(current.lines().next().unwrap_or_default())
        .or_else(|| {
            let value: Value = serde_yaml::from_str(&read(config)).unwrap_or(Value::Null);
            normalize_locale(text(&value["locale"]))
        })
        .unwrap_or("en");
    json!({"current":current,"label":if current == "en" {"English"} else {"简体中文"},"supported":[{"id":"en","label":"English"},{"id":"zh-cn","label":"简体中文"}],"restart_required":true})
}
pub fn normalize_locale(value: &str) -> Option<&'static str> {
    match value.trim().to_lowercase().as_str() {
        "en" | "en-us" | "english" => Some("en"),
        "zh" | "zh-cn" | "zh-hans" | "chinese" => Some("zh-cn"),
        _ => None,
    }
}
pub async fn snapshot(ctx: &Context) -> Value {
    let source = ctx.source.to_string_lossy();
    let revision_args = ["git", "-C", &source, "rev-parse", "--short", "HEAD"];
    let (workspace, agents, services, browser, tailscale, ports, revision) = tokio::join!(
        workspace(ctx),
        agents(ctx),
        services(ctx),
        browser(ctx),
        tailscale(ctx),
        ports(ctx),
        ctx.probe(&revision_args, 5.)
    );
    let skills = skills(ctx);
    let mcp = mcp(ctx);
    let network = routes(ctx);
    let diagnostics = doctor(ctx);
    let desktop = desktop(ctx, &services).await;
    let mut agents = agents;
    for agent in agents.as_array_mut().unwrap() {
        let target = text(&agent["web_target"]);
        if !target.is_empty() {
            let url = array(&network["routes"])
                .iter()
                .find(|r| {
                    text(&r["target"])
                        .trim_start_matches("http://")
                        .trim_start_matches("https://")
                        .trim_end_matches('/')
                        == target
                })
                .map(|r| r["public_url"].clone())
                .unwrap_or(json!(""));
            agent["web_url"] = url;
        }
        let id = text(&agent["web_service"]);
        if !id.is_empty() {
            let state = array(&services)
                .iter()
                .find(|r| r["id"] == id)
                .map(|r| r["status"].clone())
                .unwrap_or(json!("missing"));
            agent["web_state"] = state;
        }
    }
    let hostname = env::var("HOSTNAME")
        .or_else(|_| env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| read("/proc/sys/kernel/hostname").trim().to_owned());
    json!({"schema_version":1,"generated_at":now(),"revision":if revision.code == 0 {revision.stdout.trim()} else {""},"workspace":workspace,"bootstrap":bootstrap(ctx),"agents":agents,"skills":skills,"mcp":mcp,"services":services,"desktop":desktop,"browser":browser,"tailscale":tailscale,"ports":ports,"network":network,"capabilities":capabilities(ctx),"doctor":diagnostics,"access":{"hostname":hostname,"desktop_local":"https://localhost:3001","code_server_local":"https://localhost:8443","workspace":ctx.workspace},"locale":locale(ctx),"summary":{"ready_agents":array(&agents).iter().filter(|r| r["installed"] == true).count(),"global_skills":skills["shared_count"],"mcp_servers":mcp["count"],"running_services":array(&services).iter().filter(|r| r["status"] == "running").count(),"service_count":array(&services).len(),"healthy":diagnostics["healthy"]}})
}
pub fn print_text(name: &str, value: &Value) -> bool {
    match name {
        "agents" => {
            for row in array(value) {
                let detail = if row["installed"] == true {
                    format!("{} ({})", text(&row["path"]), text(&row["version"]))
                } else {
                    format!(
                        "install: workspacectl install agent {}",
                        text(&row["install_name"])
                    )
                };
                println!(
                    "{:<16} {:<10} {}",
                    text(&row["label"]),
                    if row["installed"] == true {
                        "ready"
                    } else {
                        "missing"
                    },
                    detail
                );
            }
        }
        "skills" | "mcp" => {
            if name == "skills" {
                println!(
                    "Global Skills: {} ({} shared in {})",
                    value["count"],
                    value["shared_count"],
                    text(&value["root"])
                );
            } else {
                println!(
                    "MCP servers: {} ({} on every installed Agent)",
                    value["count"], value["global_count"]
                );
            }
            let items = array(&value["items"]);
            if items.is_empty() {
                println!(
                    "  none; add one with: workspacectl {} add <{}>",
                    if name == "skills" { "skill" } else { "mcp" },
                    if name == "skills" {
                        "source"
                    } else {
                        "name> <options"
                    }
                );
            }
            for row in items {
                let agents = array(&row["agents"])
                    .iter()
                    .map(|agent| {
                        text(if name == "skills" {
                            agent
                        } else {
                            &agent["id"]
                        })
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                if name == "skills" {
                    println!(
                        "  {:<28} {:<7} {}",
                        text(&row["name"]),
                        if row["shared"] == true {
                            "shared"
                        } else {
                            "agent"
                        },
                        if agents.is_empty() { "-" } else { &agents }
                    );
                } else {
                    println!(
                        "  {:<24} {:<6} {:<28} {}",
                        text(&row["name"]),
                        text(&row["transport"]),
                        if agents.is_empty() { "-" } else { &agents },
                        text(&row["detail"])
                    );
                }
            }
        }
        "ports" => {
            println!("{:<16} {:>7} {:<8} ADDRESS", "COMMAND", "PID", "USER");
            for row in array(value) {
                println!(
                    "{:<16} {:>7} {:<8} {}",
                    text(&row["command"]),
                    row["pid"]
                        .as_i64()
                        .map(|pid| pid.to_string())
                        .unwrap_or_default(),
                    text(&row["user"]),
                    text(&row["address"])
                );
            }
        }
        _ => return false,
    }
    true
}

pub async fn payload(ctx: &Context, name: &str) -> Value {
    match name {
        "status" => snapshot(ctx).await,
        "info" => workspace(ctx).await,
        "agents" => agents(ctx).await,
        "skills" => skills(ctx),
        "mcp" => mcp(ctx),
        "locale" => locale(ctx),
        "browser" => browser(ctx).await,
        "desktop" => desktop(ctx, &services(ctx).await).await,
        "tailscale" => tailscale(ctx).await,
        "services" => services(ctx).await,
        "ports" => ports(ctx).await,
        "routes" => routes(ctx),
        "doctor" => doctor(ctx),
        _ => json!({"error":"unsupported inspect command"}),
    }
}
