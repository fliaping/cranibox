//! One App API implementation shared by the browser and native IPC.
use crate::{local::LocalNode, models::Models};
use agent_workspace_runtime::peers::{self, config::Config};
use cranibox_contracts::{
    agents::*,
    app::*,
    llm::{mgmt::LlmMgmt, *},
    net::PathKind,
    roster::RosterView,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};
pub struct Service {
    local: Weak<LocalNode>,
    pub models: Arc<Models>,
    agent_updates: tokio::sync::Mutex<()>,
}
pub fn error(status: u16, code: &str, message: impl Into<String>) -> AppError {
    AppError {
        status,
        code: code.into(),
        message: message.into(),
    }
}
fn unavailable(e: impl std::fmt::Display) -> AppError {
    error(503, "unavailable", e.to_string())
}
impl Service {
    pub fn new(local: Weak<LocalNode>, models: Arc<Models>) -> Self {
        Self {
            local,
            models,
            agent_updates: tokio::sync::Mutex::new(()),
        }
    }
    fn local(&self) -> Result<Arc<LocalNode>, AppError> {
        self.local
            .upgrade()
            .ok_or_else(|| unavailable("Device is stopping"))
    }
    fn env(&self) -> Result<Env, AppError> {
        Ok(cranibox_agents::environment(
            crate::peer::home().map_err(unavailable)?,
            self.models.directory().to_path_buf(),
        ))
    }
    pub async fn model_call(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, AppError> {
        use cranibox_contracts::http::Method;
        if !path.starts_with('/')
            || path.contains("..")
            || path.contains('?')
            || !matches!(
                path.split('/').nth(1),
                Some("accounts" | "signin" | "models" | "presets" | "status" | "claude")
            )
        {
            return Err(error(400, "invalid", "Unknown model operation"));
        }
        if path == "/models" && method == "GET" {
            return serde_json::to_value(self.models().await?).map_err(unavailable);
        }
        if path == "/claude/set-node" && method == "POST" {
            let node = body
                .as_ref()
                .and_then(|v| v["node"].as_str())
                .ok_or_else(|| error(400, "invalid", "Choose a device"))?
                .to_owned();
            self.models
                .edit_members(|members| {
                    anyhow::ensure!(
                        members.iter().any(|m| m.node_id == node),
                        "unknown owner device"
                    );
                    for m in members {
                        m.is_claude_node = m.node_id == node;
                    }
                    Ok(())
                })
                .map_err(unavailable)?;
            return Ok(Value::Null);
        }
        let method: Method = method
            .parse()
            .map_err(|_| error(400, "invalid", "Invalid method"))?;
        cranibox_llm::mgmt::Client::new(self.models.directory().join("run").join(LLM_MGMT_SOCKET))
            .call(method, path, body)
            .await
            .map_err(|e| match e {
                MgmtError::NotFound => error(404, "not_found", "Not found"),
                MgmtError::Invalid(message) => error(400, "invalid", message),
                _ => unavailable(e),
            })
    }
    pub async fn device_update(&self, node: &str, input: Value) -> Result<(), AppError> {
        let local = self.local()?;
        let owner = self.models.roster.is_owner_node(node);
        if input["revoke"] == true {
            if node == local.id {
                return Err(error(400, "invalid", "Cannot remove this device"));
            }
            if owner {
                self.models
                    .edit_members(|m| {
                        m.retain(|m| m.node_id != node);
                        Ok(())
                    })
                    .map_err(unavailable)?;
            } else {
                local
                    .manage_peers(peers::Command::Revoke { node: node.into() })
                    .await
                    .map_err(unavailable)?;
            }
            return Ok(());
        }
        if !owner {
            let caps: peers::config::Permissions = serde_json::from_value(input["caps"].clone())
                .map_err(|_| error(400, "invalid", "Invalid capabilities"))?;
            if caps.llm || caps.vault {
                return Err(error(
                    403,
                    "forbidden",
                    "Guest devices cannot access subscriptions",
                ));
            }
            Config::update(local.config_path(), |c| {
                let grant = c
                    .grants
                    .get_mut(node)
                    .ok_or_else(|| anyhow::anyhow!("Guest device not found"))?;
                grant.permissions = caps;
                Ok(())
            })
            .map_err(unavailable)?;
            return Ok(());
        }
        self.models
            .edit_members(|members| {
                let member = members
                    .iter_mut()
                    .find(|m| m.node_id == node)
                    .ok_or_else(|| anyhow::anyhow!("Device not found"))?;
                if let Some(name) = input["name"].as_str() {
                    anyhow::ensure!(!name.trim().is_empty() && name.len() <= 128, "invalid name");
                    member.name = name.into();
                }
                if let Some(priority) = input["priority"].as_u64() {
                    member.priority = u8::try_from(priority)?;
                }
                Ok(())
            })
            .map_err(unavailable)
    }
}
#[async_trait::async_trait]
impl AppApi for Service {
    async fn status(&self) -> Result<StatusPill, AppError> {
        let hub = self.models.roster.hub().map(|m| m.node_id);
        let count = self
            .models()
            .await?
            .iter()
            .filter(|m| !matches!(m.availability, Availability::Unavailable { .. }))
            .count() as u32;
        let approvals = self.approvals().await?.len() as u32;
        let state = if approvals > 0 {
            PillState::Attention
        } else if count == 0 {
            PillState::NoModels
        } else if hub.as_ref().is_some_and(|n| {
            n != self.models.vault.node() && !self.models.router.health.get(n).available()
        }) {
            PillState::Local
        } else {
            PillState::Connected
        };
        let key = match state {
            PillState::Connected => "connected",
            PillState::Local => "local",
            PillState::Attention => "attention",
            PillState::NoModels => "no_models",
            PillState::Offline => "offline",
        };
        Ok(StatusPill {
            state,
            text_key: format!("status.{key}"),
            hub,
            models_available: count,
            approvals_pending: approvals,
        })
    }
    async fn devices(&self) -> Result<Vec<Device>, AppError> {
        let local = self.local()?;
        let config = Config::read(local.config_path()).map_err(unavailable)?;
        let mut devices = Vec::new();
        if let Some(roster) = self.models.roster.current() {
            for member in roster.members {
                let own = member.node_id == local.id;
                let path = self.models.router.transport.path(&member.node_id).await;
                let mut badges = Vec::new();
                if own {
                    badges.push(Badge::This);
                }
                if member.role == cranibox_contracts::roster::Role::Hub {
                    badges.push(Badge::Hub);
                }
                if member.is_claude_node {
                    badges.push(Badge::Claude);
                }
                let caps = DeviceCaps {
                    exec: true,
                    terminals: true,
                    projects: true,
                    desktop: true,
                    computer: true,
                    browser: true,
                    messages: true,
                    activity: true,
                    spaces: true,
                    llm: true,
                    vault: true,
                };
                devices.push(Device {
                    node_id: member.node_id.clone(),
                    name: member.name,
                    kind: if member.role == cranibox_contracts::roster::Role::Hub {
                        DeviceKind::Server
                    } else {
                        DeviceKind::Desktop
                    },
                    online: own
                        || (path.is_some()
                            && self.models.router.health.get(&member.node_id).available()),
                    path: path.as_ref().map_or(PathKind::Unknown, |p| p.kind),
                    rtt_ms: path.and_then(|p| p.rtt_ms),
                    badges,
                    guest: false,
                    caps,
                });
            }
        }
        for (node, grant) in config.grants {
            if devices.iter().any(|d| d.node_id == node) {
                continue;
            }
            let caps = serde_json::from_value(
                serde_json::to_value(grant.permissions).map_err(unavailable)?,
            )
            .map_err(unavailable)?;
            devices.push(Device {
                node_id: node.clone(),
                name: node,
                kind: DeviceKind::Unknown,
                online: false,
                path: PathKind::Unknown,
                rtt_ms: None,
                badges: vec![],
                guest: true,
                caps,
            });
        }
        Ok(devices)
    }
    async fn pair_invite(&self, _req: PairInviteReq) -> Result<PairInvite, AppError> {
        Err(error(
            503,
            "pairing_unavailable",
            "Encrypted pairing transport is not available on this build",
        ))
    }
    async fn pair_accept(&self, _req: PairAcceptReq) -> Result<PairAccepted, AppError> {
        Err(error(
            503,
            "pairing_unavailable",
            "Encrypted pairing transport is not available on this build",
        ))
    }
    async fn pair_confirm(&self, _flow: &str, _req: PairConfirmReq) -> Result<(), AppError> {
        Err(error(404, "not_found", "Pairing session not found"))
    }
    async fn approvals(&self) -> Result<Vec<ApprovalCard>, AppError> {
        let local = self.local()?;
        let held = local
            .manage_peers(peers::Command::Messages {
                command: peers::messages::Command::Held,
            })
            .await
            .map_err(unavailable)?;
        let mut cards = Vec::new();
        if let Some(items) = held["held"].as_array() {
            for item in items {
                let message = &item["message"];
                let Some(id) = message["id"].as_str() else {
                    continue;
                };
                let expires = message["expires_at"].as_u64();
                if expires.is_some_and(|at| at <= cranibox_llm::auth::now()) {
                    continue;
                }
                let sender = message["sender"].as_str().unwrap_or_default().to_owned();
                cards.push(ApprovalCard {
                    id: format!("message:{id}"),
                    kind: ApprovalKind::Message,
                    from_node: Some(sender.clone()),
                    on_node: local.id.clone(),
                    agent: None,
                    title_key: "approval.message".into(),
                    title_args: BTreeMap::from([("device".into(), sender)]),
                    detail: message["text"].as_str().map(str::to_owned),
                    risk: Risk::Medium,
                    created_at: message["created_at"].as_u64().unwrap_or_default(),
                    expires_at: expires,
                    actions: vec![ApprovalAction::AllowOnce, ApprovalAction::Deny],
                });
            }
        }
        cards.extend(
            cranibox_agents::approvals::pending(&self.env()?, &local.id).map_err(unavailable)?,
        );
        Ok(cards)
    }
    async fn decide(&self, id: &str, req: ApprovalDecision) -> Result<(), AppError> {
        let card = self
            .approvals()
            .await?
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| error(404, "not_found", "Approval expired or already decided"))?;
        if !card.actions.contains(&req.decision) {
            return Err(error(400, "invalid", "Action not offered"));
        }
        if id.starts_with("agent:") {
            return cranibox_agents::approvals::decide(&self.env()?, id, req.decision)
                .map_err(|e| error(409, "approval_conflict", e.to_string()));
        }
        let id = uuid::Uuid::parse_str(id.strip_prefix("message:").unwrap_or_default())
            .map_err(|_| error(400, "invalid", "Invalid approval"))?;
        let command = if req.decision == ApprovalAction::Deny {
            peers::messages::Command::Reject { id }
        } else {
            peers::messages::Command::Release { id }
        };
        self.local()?
            .manage_peers(peers::Command::Messages { command })
            .await
            .map_err(unavailable)?;
        Ok(())
    }
    async fn agents(&self) -> Result<Vec<AgentInfo>, AppError> {
        let env = self.env()?;
        let settings = std::fs::read(env.data.join("agent-selection.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<BTreeMap<String, Option<String>>>(&b).ok())
            .unwrap_or_default();
        Ok(cranibox_agents::registry()
            .iter()
            .map(|adapter| AgentInfo {
                id: adapter.id(),
                installed: adapter.detect(&env).installed,
                managed: settings.contains_key(adapter.id().as_str()),
                model: settings.get(adapter.id().as_str()).cloned().flatten(),
                recommended: adapter.id() == DEFAULT_AGENT,
            })
            .collect())
    }
    async fn update_agent(&self, id: AgentId, req: AgentUpdate) -> Result<AgentInfo, AppError> {
        let _guard = self.agent_updates.lock().await;
        let env = self.env()?;
        let adapter =
            cranibox_agents::adapter(id).ok_or_else(|| error(404, "not_found", "Unknown agent"))?;
        let models = self.models().await?;
        let selected = req.model.clone().or_else(|| {
            models
                .iter()
                .find(|m| !matches!(m.availability, Availability::Unavailable { .. }))
                .map(|m| m.id.clone())
        });
        if req.managed {
            let default = selected.ok_or_else(|| error(400, "no_models", "Add a model first"))?;
            if !models.iter().any(|m| {
                m.id == default && !matches!(m.availability, Availability::Unavailable { .. })
            }) {
                return Err(error(400, "invalid", "Model is unavailable"));
            }
            adapter
                .wire(
                    &env,
                    &LocalEndpoint::for_agent(id),
                    &ModelPick {
                        default,
                        small_fast: None,
                        models,
                    },
                )
                .map_err(unavailable)?;
            cranibox_agents::approvals::install(&env, id).map_err(unavailable)?;
            let local = self.local()?;
            let config = Config::read(local.config_path()).map_err(unavailable)?;
            let command = std::env::current_exe().map_err(unavailable)?;
            for service in ["devices", "computer", "browser"] {
                if service == "browser" && (config.browser.is_none() || config.browser_paused) {
                    continue;
                }
                let mut args = Vec::new();
                if command
                    .file_stem()
                    .is_some_and(|s| s == "cranibox" || s == "workspace-web")
                {
                    args.push("node".into());
                }
                args.extend([
                    "--config".into(),
                    local.config_path().to_string_lossy().into_owned(),
                    "app-mcp".into(),
                    "--data".into(),
                    env.data.to_string_lossy().into_owned(),
                    "--service".into(),
                    service.into(),
                ]);
                adapter
                    .register_mcp(
                        &env,
                        &McpSpec {
                            name: if service == "devices" {
                                "cranibox".into()
                            } else {
                                format!("cranibox-{service}")
                            },
                            command: command.to_string_lossy().into_owned(),
                            args,
                            env: BTreeMap::new(),
                        },
                    )
                    .map_err(unavailable)?;
            }
        } else {
            adapter.unwire(&env).map_err(unavailable)?;
        }
        let path = env.data.join("agent-selection.json");
        let mut settings: BTreeMap<String, Option<String>> = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if req.managed {
            settings.insert(id.as_str().into(), req.model.clone());
        } else {
            settings.remove(id.as_str());
        }
        agent_workspace_runtime::config::write_private(
            &path,
            &serde_json::to_vec(&settings).map_err(unavailable)?,
            true,
        )
        .map_err(unavailable)?;
        Ok(AgentInfo {
            id,
            installed: adapter.detect(&env).installed,
            managed: req.managed,
            model: req.model,
            recommended: id == DEFAULT_AGENT,
        })
    }
    async fn models(&self) -> Result<Vec<ModelEntry>, AppError> {
        self.models.models().await.map_err(unavailable)
    }
}

/// Transport-neutral dispatcher. Demo fixtures require an explicit process flag.
static MOCK: std::sync::LazyLock<cranibox_contracts::mock::MockAppApi> =
    std::sync::LazyLock::new(cranibox_contracts::mock::MockAppApi::fixture);
pub async fn dispatch(
    local: Option<Arc<LocalNode>>,
    method: &str,
    path: &str,
    body: Value,
) -> Result<Value, AppError> {
    let mock = std::env::var("CRANIBOX_APP_MOCK").as_deref() == Ok("1");
    let service = if mock {
        None
    } else {
        Some(
            local
                .ok_or_else(|| unavailable("Local device service unavailable"))?
                .app()
                .await
                .map_err(unavailable)?,
        )
    };
    let api: &dyn AppApi = service
        .as_ref()
        .map_or(&*MOCK as &dyn AppApi, |s| s.as_ref());
    fn input<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, AppError> {
        serde_json::from_value(value).map_err(|_| error(400, "invalid", "Invalid request"))
    }
    fn output<T: serde::Serialize>(value: T) -> Result<Value, AppError> {
        serde_json::to_value(value).map_err(unavailable)
    }
    match (method, path) {
        ("GET", "/status") => output(api.status().await?),
        ("GET", "/devices") => output(api.devices().await?),
        ("GET", "/approvals") => output(api.approvals().await?),
        ("GET", "/agents") => output(api.agents().await?),
        ("GET", "/tasks") => {
            let Some(service) = service else {
                return Ok(serde_json::json!([]));
            };
            let ledger = service
                .local()?
                .manage_peers(peers::Command::Tasks {
                    command: peers::tasks::Command::List {},
                })
                .await
                .map_err(unavailable)?;
            // Expose a small read-only summary, never credentials or run commands.
            let mut tasks: Vec<Value> = ["incoming", "outgoing"].iter()
                .flat_map(|direction| ledger[*direction].as_array().into_iter().flatten().map(move |item| {
                    serde_json::json!({
                        "id": item["task"]["id"], "title": item["task"]["title"],
                        "direction": direction, "state": item["current"]["state"],
                        "progress": item["current"]["progress"], "result": item["current"]["result"],
                        "questions": item["current"]["questions"],
                        "updated_at": item["current"]["updated_at"],
                    })
                })).collect();
            tasks.sort_by_key(|task| std::cmp::Reverse(task["updated_at"].as_u64().unwrap_or(0)));
            Ok(Value::Array(tasks))
        }
        ("POST", "/pair/invite") => output(api.pair_invite(input(body)?).await?),
        ("POST", "/pair/accept") => output(api.pair_accept(input(body)?).await?),
        _ => {
            if let Some(flow) = path
                .strip_prefix("/pair/")
                .and_then(|p| p.strip_suffix("/confirm"))
                && method == "POST"
            {
                api.pair_confirm(flow, input(body)?).await?;
                return Ok(Value::Null);
            }
            if let Some(id) = path.strip_prefix("/approvals/")
                && method == "POST"
            {
                api.decide(id, input(body)?).await?;
                return Ok(Value::Null);
            }
            if let Some(id) = path.strip_prefix("/agents/")
                && method == "PUT"
            {
                return output(
                    api.update_agent(
                        AgentId::parse(id)
                            .ok_or_else(|| error(404, "not_found", "Unknown agent"))?,
                        input(body)?,
                    )
                    .await?,
                );
            }
            if let Some(node) = path.strip_prefix("/devices/")
                && method == "PUT"
            {
                let service = service
                    .ok_or_else(|| error(503, "unavailable", "Demo devices are read only"))?;
                service.device_update(node, body).await?;
                return Ok(Value::Null);
            }
            if let Some(path) = path.strip_prefix("/models") {
                let path = if path.is_empty() { "/models" } else { path };
                if let Some(service) = service {
                    return service
                        .model_call(method, path, if body.is_null() { None } else { Some(body) })
                        .await;
                }
                let mgmt = &MOCK.mgmt;
                let result = match (method, path) {
                    ("GET", "/models") => output(mgmt.models().await.map_err(unavailable)?),
                    ("GET", "/accounts") => output(mgmt.accounts().await.map_err(unavailable)?),
                    ("GET", "/presets") => output(mgmt.presets().await.map_err(unavailable)?),
                    ("GET", "/status") => output(mgmt.status().await.map_err(unavailable)?),
                    ("POST", "/accounts") => {
                        output(mgmt.add_account(input(body)?).await.map_err(unavailable)?)
                    }
                    ("POST", "/signin/start") => {
                        output(mgmt.signin_start(input(body)?).await.map_err(unavailable)?)
                    }
                    ("POST", "/claude/set-node") => {
                        mgmt.set_claude_node(input(body)?)
                            .await
                            .map_err(unavailable)?;
                        Ok(Value::Null)
                    }
                    _ => {
                        if let Some(id) = path
                            .strip_prefix("/accounts/")
                            .and_then(|p| p.strip_suffix("/enable"))
                        {
                            {
                                if method != "POST" {
                                    return Err(error(405, "invalid", "POST required"));
                                }
                                output(mgmt.enable(id, input(body)?).await.map_err(unavailable)?)
                            }
                        } else if let Some(id) =
                            path.strip_prefix("/accounts/").filter(|p| !p.contains('/'))
                        {
                            if method != "DELETE" {
                                return Err(error(405, "invalid", "DELETE required"));
                            }
                            mgmt.delete_account(id).await.map_err(unavailable)?;
                            Ok(Value::Null)
                        } else if let Some(flow) = path
                            .strip_prefix("/signin/")
                            .and_then(|p| p.strip_suffix("/complete"))
                        {
                            {
                                if method != "POST" {
                                    return Err(error(405, "invalid", "POST required"));
                                }
                                output(
                                    mgmt.signin_complete(flow, input(body)?)
                                        .await
                                        .map_err(unavailable)?,
                                )
                            }
                        } else {
                            Err(error(404, "not_found", "Unknown model operation"))
                        }
                    }
                };
                return result;
            }
            Err(error(404, "not_found", "Unknown App operation"))
        }
    }
}

/// One change detector for HTTP SSE and native events. Summary data contains no credentials.
#[derive(Default)]
pub struct EventCursor(Option<Value>);
impl EventCursor {
    pub async fn poll(&mut self, local: Option<Arc<LocalNode>>) -> Result<Vec<AppEvent>, AppError> {
        let status = dispatch(local.clone(), "GET", "/status", Value::Null).await?;
        let devices = dispatch(local.clone(), "GET", "/devices", Value::Null).await?;
        let approvals = dispatch(local.clone(), "GET", "/approvals", Value::Null).await?;
        let accounts = dispatch(local, "GET", "/models/accounts", Value::Null).await?;
        let snapshot = serde_json::json!({"status":status,"devices":devices,"approvals":approvals,"accounts":accounts});
        if self.0.as_ref() == Some(&snapshot) {
            return Ok(Vec::new());
        }
        // Status also invalidates removed items, for which C6 has no tombstone event.
        let mut events = vec![AppEvent::Status(
            serde_json::from_value(status).map_err(unavailable)?,
        )];
        for (field, kind) in [
            ("devices", "device"),
            ("approvals", "approval"),
            ("accounts", "account"),
        ] {
            for item in snapshot[field].as_array().into_iter().flatten() {
                if self
                    .0
                    .as_ref()
                    .and_then(|old| old[field].as_array())
                    .is_some_and(|old| old.contains(item))
                {
                    continue;
                }
                events.push(
                    serde_json::from_value(serde_json::json!({"event":kind,"data":item}))
                        .map_err(unavailable)?,
                );
            }
        }
        self.0 = Some(snapshot);
        Ok(events)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn real_model_management_keeps_credentials_private_and_applies_enablement() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("peers.json");
        Config::update(&path, |_| Ok(())).unwrap();
        let identity = Config::identity(&path).unwrap();
        let dir = path.with_file_name("models");
        std::fs::create_dir_all(&dir).unwrap();
        // Use a fixture key directly; this test must never consult the user's keychain.
        agent_workspace_runtime::config::write_private(
            &dir.join("encryption.key"),
            identity.noise_private(),
            false,
        )
        .unwrap();
        let stop = CancellationToken::new();
        let models = Models::start_on(
            path,
            stop.clone(),
            Some(vec!["127.0.0.1:0".parse().unwrap()]),
        )
        .await
        .unwrap();
        let app = Service::new(Weak::new(), models.clone());
        let secret = "fake-app-model-management-key";
        let account = app
            .model_call(
                "POST",
                "/accounts",
                Some(json!({
                    "kind":"custom", "label":"App fixture", "key":secret,
                    "endpoints":{"chat":"http://127.0.0.1:9/v1"}, "models":["fixture-model"]
                })),
            )
            .await
            .unwrap();
        let id = account["id"].as_str().unwrap();
        let provider = account["provider"].as_str().unwrap();
        let model_id = format!("{provider}/fixture-model");
        let list = app.model_call("GET", "/accounts", None).await.unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert!(!list.to_string().contains(secret));
        let available = app.model_call("GET", "/models", None).await.unwrap();
        assert!(
            available
                .as_array()
                .unwrap()
                .iter()
                .any(|m| { m["id"] == model_id && m["availability"]["kind"] == "local" })
        );
        let enabled = app
            .model_call(
                "POST",
                &format!("/accounts/{id}/enable"),
                Some(json!({"node":models.vault.node(), "on":false, "all_nodes":null})),
            )
            .await
            .unwrap();
        assert_eq!(enabled["enabled_on"], json!([]));
        let unavailable = app.model_call("GET", "/models", None).await.unwrap();
        assert!(
            !unavailable
                .as_array()
                .unwrap()
                .iter()
                .any(|m| { m["id"] == model_id && m["availability"]["kind"] == "local" })
        );
        assert_eq!(
            app.model_call("GET", "/accounts/../status", None)
                .await
                .unwrap_err()
                .status,
            400
        );
        app.model_call("DELETE", &format!("/accounts/{id}"), None)
            .await
            .unwrap();
        assert_eq!(
            app.model_call("GET", "/accounts", None).await.unwrap(),
            json!([])
        );
        assert!(
            !std::fs::read(dir.join("vault.sqlite"))
                .unwrap()
                .windows(secret.len())
                .any(|bytes| bytes == secret.as_bytes())
        );
        stop.cancel();
    }
}
