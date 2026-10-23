//! One App API implementation shared by the browser and native IPC.
mod capabilities;
mod known;
pub mod preferences;
use crate::{local::LocalNode, models::Models};
use agent_workspace_runtime::peers::{self, config::Config};
use cranibox_contracts::{
    agents::*,
    app::*,
    llm::{mgmt::LlmMgmt, *},
    net::PathKind,
    roster::RosterView,
};
use cranibox_net::pair::{FlowState, Side};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};
pub struct Service {
    local: Weak<LocalNode>,
    pub models: Arc<Models>,
    agent_updates: tokio::sync::Mutex<()>,
    /// First time each inviter-side pairing flow was seen, for stable approval cards.
    pair_seen: std::sync::Mutex<BTreeMap<String, u64>>,
    /// Local capability providers, created on first use.
    capabilities: std::sync::OnceLock<capabilities::Service>,
    /// Recent reachability of paired devices that have no open connection.
    reach: Arc<std::sync::Mutex<BTreeMap<String, Reach>>>,
    /// Requests this device sent to servers: request ID → (server, polling secret).
    server_requests: std::sync::Mutex<BTreeMap<String, (url::Url, String)>>,
    /// Devices approved through a server-address request; their pairing confirms itself.
    auto_confirm: Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>,
}
#[derive(Clone, Copy)]
struct Reach {
    checked: std::time::Instant,
    online: bool,
    probing: bool,
}
/// How long a probe result stands before the device is probed again.
const REACH_TTL: std::time::Duration = std::time::Duration::from_secs(15);
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
            pair_seen: Default::default(),
            capabilities: Default::default(),
            reach: Default::default(),
            server_requests: Default::default(),
            auto_confirm: Default::default(),
        }
    }
    fn local(&self) -> Result<Arc<LocalNode>, AppError> {
        self.local
            .upgrade()
            .ok_or_else(|| unavailable("Device is stopping"))
    }
    /// Last known reachability of a paired device; refreshes it in the background with
    /// an authenticated identity probe when the result is older than [`REACH_TTL`].
    fn reachable(&self, node: &str) -> bool {
        let mut cache = self.reach.lock().unwrap();
        let entry = cache.get(node).copied();
        let fresh = entry.is_some_and(|r| r.checked.elapsed() < REACH_TTL);
        if !fresh && !entry.is_some_and(|r| r.probing) {
            cache.insert(
                node.to_owned(),
                Reach {
                    checked: entry.map_or_else(std::time::Instant::now, |r| r.checked),
                    online: entry.is_some_and(|r| r.online),
                    probing: true,
                },
            );
            let (local, reach, node) = (self.local.clone(), self.reach.clone(), node.to_owned());
            tokio::spawn(async move {
                let online = match local.upgrade() {
                    Some(local) => match tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        local.manage_peers(peers::Command::Probe { node: node.clone() }),
                    )
                    .await
                    {
                        Ok(Ok(answer)) => {
                            let now = cranibox_llm::auth::now();
                            let _ = known::update(local.config_path(), &node, |k| {
                                k.learn(&answer, now)
                            });
                            true
                        }
                        _ => false,
                    },
                    None => false,
                };
                reach.lock().unwrap().insert(
                    node,
                    Reach {
                        checked: std::time::Instant::now(),
                        online,
                        probing: false,
                    },
                );
            });
        }
        entry.is_some_and(|r| r.online)
    }
    /// `POST /pair/server {url}`: pair through a server's public address or pairing link.
    pub async fn pair_server(&self, url: &str) -> Result<Value, AppError> {
        let invalid = |e: anyhow::Error| error(400, "invalid", format!("{e:#}"));
        let (base, link) = crate::pair_url::classify(url).map_err(invalid)?;
        if let Some(link) = link {
            let accepted = self.pair_accept(PairAcceptReq { link }).await?;
            return serde_json::to_value(accepted).map_err(unavailable);
        }
        let server = crate::pair_url::server_identity(&base)
            .await
            .map_err(invalid)?;
        let local = self.local()?;
        if server["node_id"] == local.id.as_str() {
            return Err(error(400, "invalid", "That address is this device"));
        }
        let identity = local
            .manage_peers(peers::Command::Identity)
            .await
            .map_err(unavailable)?;
        let me = serde_json::json!({
            "node_id": local.id,
            "name": crate::device_info::local().name,
            "fingerprint": identity["identity"]["fingerprint"],
            "endpoint_id": identity["identity"]["public"]["signing_key"],
        });
        let (id, secret) = crate::pair_url::request(&base, me.clone())
            .await
            .map_err(|e| error(409, "pairing_failed", format!("{e:#}")))?;
        let mut requests = self.server_requests.lock().unwrap();
        while requests.len() >= 8 {
            let Some(oldest) = requests.keys().next().cloned() else {
                break;
            };
            requests.remove(&oldest);
        }
        requests.insert(id.clone(), (base, secret));
        Ok(serde_json::json!({
            "request_id": id,
            "server": {"name": server["name"], "node_id": server["node_id"], "fingerprint": server["fingerprint"]},
            "fingerprint": me["fingerprint"],
        }))
    }
    /// `GET /pair/server/{id}`: once the server's owner approves, pair with its invitation.
    pub async fn pair_server_status(&self, id: &str) -> Result<Value, AppError> {
        let (base, secret) = self
            .server_requests
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| error(404, "not_found", "Pairing request not found"))?;
        let status = crate::pair_url::poll(&base, id, &secret)
            .await
            .map_err(|e| error(409, "pairing_failed", format!("{e:#}")))?;
        match status["state"].as_str() {
            Some("approved") => {
                self.server_requests.lock().unwrap().remove(id);
                let link = status["link"].as_str().unwrap_or_default().to_owned();
                let accepted = self.pair_accept(PairAcceptReq { link }).await?;
                let mut value = serde_json::to_value(accepted).map_err(unavailable)?;
                value["state"] = "accepted".into();
                Ok(value)
            }
            Some(state @ ("pending" | "rejected")) => Ok(serde_json::json!({"state": state})),
            _ => Err(error(
                409,
                "pairing_failed",
                "Unexpected answer from the server",
            )),
        }
    }
    /// The owner approved a server-address request: invite that device and confirm its
    /// pairing automatically (the approval is the owner's consent).
    async fn approve_server_request(&self, id: &str, approve: bool) -> Result<(), AppError> {
        let local = self.local()?;
        if !approve {
            local
                .pair_requests
                .decide(id, crate::pair_url::State::Rejected)
                .ok_or_else(|| error(404, "not_found", "Approval expired or already decided"))?;
            return Ok(());
        }
        let iroh = self.pairing().await?;
        let invite = iroh
            .pair_invite(PairInviteReq {
                purpose: cranibox_contracts::net::PairPurpose::OwnerDevice,
            })
            .map_err(|e| error(409, "pairing_failed", format!("{e:#}")))?;
        let request = local
            .pair_requests
            .decide(id, crate::pair_url::State::Approved { link: invite.link })
            .ok_or_else(|| error(404, "not_found", "Approval expired or already decided"))?;
        self.confirm_inviter(iroh, request.node_id, crate::pair_url::REQUEST_TTL_SECS);
        Ok(())
    }
    /// Confirm this device's invitation once `node` reaches the code comparison: the
    /// owner already consented (an approved request or an invitation they sent).
    fn confirm_inviter(&self, iroh: crate::peer::transport_iroh::Handle, node: String, ttl: u64) {
        self.auto_confirm.lock().unwrap().insert(node.clone());
        let auto = self.auto_confirm.clone();
        tokio::spawn(async move {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(ttl);
            while std::time::Instant::now() < deadline {
                if let Some(flow) = iroh.pair_flows().into_iter().find(|f| {
                    f.side == Side::Inviter
                        && f.state == FlowState::Comparing
                        && f.peer_node_id == node
                }) {
                    let _ = iroh
                        .pair_confirm(&flow.flow_id, PairConfirmReq { matches: true })
                        .await;
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            auto.lock().unwrap().remove(&node);
        });
    }
    async fn pairing(&self) -> Result<crate::peer::transport_iroh::Handle, AppError> {
        self.local()?.iroh().await.ok_or_else(|| {
            error(
                503,
                "pairing_unavailable",
                "The iroh network is not running on this device",
            )
        })
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
    fn capability_service(&self, local: &LocalNode) -> &capabilities::Service {
        self.capabilities
            .get_or_init(|| capabilities::Service::new(local.config_path()))
    }
    /// `GET /capabilities`.
    pub async fn capabilities(&self) -> Result<Capabilities, AppError> {
        let local = self.local()?;
        self.capability_service(&local).list(local.as_ref()).await
    }
    /// `POST /capabilities/{id}/{action}`.
    pub async fn capability_action(
        &self,
        id: CapabilityId,
        action: CapabilityAction,
        body: Value,
    ) -> Result<Value, AppError> {
        let local = self.local()?;
        self.capability_service(&local)
            .act(local.as_ref(), id, action, body)
            .await
    }
    /// `GET /devices/{node}/grant`.
    pub fn device_grant(&self, node: &str) -> Result<DeviceGrant, AppError> {
        read_grant(self.local()?.config_path(), node)
    }
    /// `PUT /devices/{node}/grant`. Re-signed by [`Config::update`].
    pub fn set_device_grant(
        &self,
        node: &str,
        grant: DeviceGrant,
    ) -> Result<DeviceGrant, AppError> {
        let owner = self.models.roster.is_owner_node(node);
        write_grant(self.local()?.config_path(), node, owner, grant)
    }
    pub async fn device_update(&self, node: &str, input: Value) -> Result<(), AppError> {
        let local = self.local()?;
        if let Some(name) = input.get("name") {
            let name = name
                .as_str()
                .map(str::trim)
                .ok_or_else(|| error(400, "invalid", "Name must be text"))?;
            if name.chars().count() > 64 {
                return Err(error(400, "invalid", "Use at most 64 characters"));
            }
            known::update(local.config_path(), node, |k| {
                k.alias = (!name.is_empty()).then(|| name.to_owned());
            })
            .map_err(unavailable)?;
            if input.get("caps").is_none() && input.get("revoke").is_none() {
                return Ok(());
            }
        }
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
                let models = self.models.clone();
                let node = node.to_owned();
                tokio::spawn(async move { models.notify_removed(&node).await });
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
        let known = known::load(local.config_path());
        let now = cranibox_llm::auth::now();
        let this = crate::device_info::local();
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
                let remembered = known.get(&member.node_id).cloned().unwrap_or_default();
                // Also keeps what the device shares current, even while it is reachable.
                let probed = !own && self.reachable(&member.node_id);
                // Our own devices allow everything; whether one actually shares its
                // desktop, Computer Use or browser is what it last reported.
                let caps = DeviceCaps {
                    exec: true,
                    terminals: true,
                    projects: true,
                    desktop: own || remembered.offers("desktop"),
                    computer: own || remembered.offers("computer"),
                    browser: own || remembered.offers("browser"),
                    messages: true,
                    activity: true,
                    spaces: true,
                    llm: true,
                    vault: true,
                };
                // Reachable is online; the model route's health only matters for models
                // (a member that has not taken this identity yet refuses them).
                let online = own
                    || probed
                    || (path.is_some()
                        && self.models.router.health.get(&member.node_id).available());
                devices.push(Device {
                    node_id: member.node_id.clone(),
                    name: if own {
                        remembered
                            .alias
                            .clone()
                            .unwrap_or_else(|| this.name.clone())
                    } else {
                        remembered.alias.clone().unwrap_or(member.name)
                    },
                    kind: if own {
                        known::Known {
                            kind: Some(this.kind.into()),
                            ..Default::default()
                        }
                        .kind()
                    } else if member.role == cranibox_contracts::roster::Role::Hub {
                        DeviceKind::Server
                    } else {
                        remembered.kind()
                    },
                    online,
                    path: path.as_ref().map_or(PathKind::Unknown, |p| p.kind),
                    rtt_ms: path.and_then(|p| p.rtt_ms),
                    badges,
                    guest: false,
                    same_identity: true,
                    caps,
                    os_version: if own {
                        this.os_version.clone()
                    } else {
                        remembered.os_version
                    },
                    model: if own {
                        this.model.clone()
                    } else {
                        remembered.model
                    },
                    last_seen: if online {
                        Some(now)
                    } else {
                        remembered.last_seen
                    },
                });
            }
        }
        let iroh = local.iroh().await;
        for (node, grant) in config.grants {
            if devices.iter().any(|d| d.node_id == node) {
                continue;
            }
            // A guest pairing grants messages only; an owner device gets full access.
            let guest = !(grant.permissions.exec && grant.permissions.terminals);
            let caps = serde_json::from_value(
                serde_json::to_value(grant.permissions).map_err(unavailable)?,
            )
            .map_err(unavailable)?;
            let path = match &iroh {
                Some(iroh) => iroh.peer_path(&node).await,
                None => None,
            };
            let online = path.is_some() || self.reachable(&node);
            let remembered = known.get(&node).cloned().unwrap_or_default();
            devices.push(Device {
                name: remembered.display(&node),
                kind: remembered.kind(),
                node_id: node,
                online,
                path: path.map_or(PathKind::Unknown, |p| p.kind),
                rtt_ms: path.and_then(|p| p.rtt_ms),
                badges: vec![],
                guest,
                same_identity: false,
                caps,
                os_version: remembered.os_version,
                model: remembered.model,
                last_seen: if online {
                    Some(now)
                } else {
                    remembered.last_seen
                },
            });
        }
        Ok(devices)
    }
    async fn pair_invite(&self, req: PairInviteReq) -> Result<PairInvite, AppError> {
        self.pairing()
            .await?
            .pair_invite(req)
            .map_err(|e| error(409, "pairing_failed", format!("{e:#}")))
    }
    async fn pair_accept(&self, req: PairAcceptReq) -> Result<PairAccepted, AppError> {
        self.pairing()
            .await?
            .pair_accept(req)
            .await
            .map_err(|e| error(409, "pairing_failed", format!("{e:#}")))
    }
    async fn pair_confirm(&self, flow: &str, req: PairConfirmReq) -> Result<(), AppError> {
        let iroh = self.pairing().await?;
        if iroh.pair_flows().iter().all(|f| f.flow_id != flow) {
            return Err(error(404, "not_found", "Pairing session not found"));
        }
        let matches = req.matches;
        match iroh
            .pair_confirm(flow, req)
            .await
            .map_err(|e| error(409, "pairing_failed", format!("{e:#}")))?
        {
            FlowState::Paired => Ok(()),
            FlowState::Rejected if !matches => Ok(()),
            FlowState::Rejected => Err(error(
                409,
                "pairing_rejected",
                "The other device rejected the pairing code",
            )),
            FlowState::Failed(reason) => Err(error(409, "pairing_failed", reason)),
            FlowState::Comparing => Err(error(
                409,
                "pairing_failed",
                "The other device did not answer",
            )),
        }
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
        // The joiner confirms in its pairing sheet; the inviter confirms here.
        if let Some(iroh) = local.iroh().await {
            let auto = self.auto_confirm.lock().unwrap().clone();
            let flows: Vec<_> = iroh
                .pair_flows()
                .into_iter()
                .filter(|f| {
                    f.side == Side::Inviter
                        && f.state == FlowState::Comparing
                        && !auto.contains(&f.peer_node_id)
                })
                .collect();
            let mut seen = self.pair_seen.lock().unwrap();
            seen.retain(|id, _| flows.iter().any(|f| &f.flow_id == id));
            for flow in flows {
                let created_at = *seen
                    .entry(flow.flow_id.clone())
                    .or_insert_with(cranibox_llm::auth::now);
                cards.push(ApprovalCard {
                    id: format!("pair:{}", flow.flow_id),
                    kind: ApprovalKind::Pairing,
                    from_node: Some(flow.peer_node_id.clone()),
                    on_node: local.id.clone(),
                    agent: None,
                    title_key: "approval.pairing".into(),
                    title_args: BTreeMap::from([
                        ("code".into(), flow.sas),
                        ("device".into(), flow.peer_node_id),
                    ]),
                    detail: None,
                    risk: Risk::Medium,
                    created_at,
                    expires_at: None,
                    actions: vec![ApprovalAction::AllowOnce, ApprovalAction::Deny],
                });
            }
        }
        // Devices asking to join through this device's public address.
        for entry in local.pair_requests.pending() {
            cards.push(ApprovalCard {
                id: format!("pairreq:{}", entry.id),
                kind: ApprovalKind::Pairing,
                from_node: Some(entry.request.node_id.clone()),
                on_node: local.id.clone(),
                agent: None,
                title_key: "approval.pair_request".into(),
                title_args: BTreeMap::from([
                    ("device".into(), entry.request.name.clone()),
                    ("code".into(), entry.request.fingerprint.clone()),
                ]),
                detail: None,
                risk: Risk::Medium,
                created_at: entry.created_at,
                expires_at: Some(entry.created_at + crate::pair_url::REQUEST_TTL_SECS),
                actions: vec![ApprovalAction::AllowOnce, ApprovalAction::Deny],
            });
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
        if let Some(request) = id.strip_prefix("pairreq:") {
            return self
                .approve_server_request(request, req.decision != ApprovalAction::Deny)
                .await;
        }
        if let Some(flow) = id.strip_prefix("pair:") {
            let matches = req.decision != ApprovalAction::Deny;
            return self.pair_confirm(flow, PairConfirmReq { matches }).await;
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

fn caps(p: &peers::config::Permissions) -> DeviceCaps {
    DeviceCaps {
        exec: p.exec,
        terminals: p.terminals,
        projects: p.projects,
        desktop: p.desktop,
        computer: p.computer,
        browser: p.browser,
        messages: p.messages,
        activity: p.activity,
        spaces: p.spaces,
        llm: p.llm,
        vault: p.vault,
    }
}
fn read_grant(path: &std::path::Path, node: &str) -> Result<DeviceGrant, AppError> {
    let config = Config::read(path).map_err(unavailable)?;
    let grant = config
        .grants
        .get(node)
        .ok_or_else(|| error(404, "not_found", "This device has no grant on this device"))?;
    Ok(DeviceGrant {
        permissions: caps(&grant.permissions),
    })
}
/// Replace what a paired device may do here. Guests never reach subscriptions.
fn write_grant(
    path: &std::path::Path,
    node: &str,
    owner: bool,
    grant: DeviceGrant,
) -> Result<DeviceGrant, AppError> {
    let p = grant.permissions;
    if !owner && (p.llm || p.vault) {
        return Err(error(
            403,
            "forbidden",
            "Guest devices cannot access subscriptions",
        ));
    }
    if p == DeviceCaps::default() {
        return Err(error(
            400,
            "invalid",
            "A grant needs at least one capability; remove the device instead",
        ));
    }
    read_grant(path, node)?;
    Config::update(path, |c| {
        let grant = c
            .grants
            .get_mut(node)
            .ok_or_else(|| anyhow::anyhow!("Device not found"))?;
        grant.permissions = peers::config::Permissions {
            exec: p.exec,
            terminals: p.terminals,
            projects: p.projects,
            desktop: p.desktop,
            computer: p.computer,
            browser: p.browser,
            messages: p.messages,
            activity: p.activity,
            spaces: p.spaces,
            llm: p.llm,
            vault: p.vault,
        };
        Ok(())
    })
    .map_err(unavailable)?;
    read_grant(path, node)
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
    route(service, method, path, body).await
}
/// Routes one call to the real service, or to the fixtures when `service` is `None`.
async fn route(
    service: Option<Arc<Service>>,
    method: &str,
    path: &str,
    body: Value,
) -> Result<Value, AppError> {
    let api: &dyn AppApi = service
        .as_ref()
        .map_or(&*MOCK as &dyn AppApi, |s| s.as_ref());
    fn input<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, AppError> {
        serde_json::from_value(value).map_err(|_| error(400, "invalid", "Invalid request"))
    }
    fn output<T: serde::Serialize>(value: T) -> Result<Value, AppError> {
        serde_json::to_value(value).map_err(unavailable)
    }
    if let Some(rest) = path.strip_prefix("/pair/server") {
        let service = service
            .as_ref()
            .ok_or_else(|| error(503, "unavailable", "Demo devices cannot pair"))?;
        return match (method, rest.strip_prefix('/')) {
            ("POST", None) if rest.is_empty() => {
                service
                    .pair_server(body["url"].as_str().unwrap_or_default())
                    .await
            }
            ("GET", Some(id)) => service.pair_server_status(id).await,
            _ => Err(error(405, "invalid", "Unknown pairing request")),
        };
    }
    if path == "/preferences" {
        let service = service
            .as_ref()
            .ok_or_else(|| error(503, "unavailable", "Demo devices keep no preferences"))?;
        let peers = service.local()?.config_path().to_path_buf();
        return match method {
            "GET" => output(preferences::read(&peers)),
            "PUT" => output(
                preferences::update(&peers, &body)
                    .map_err(|e| error(400, "invalid", format!("{e:#}")))?,
            ),
            _ => Err(error(405, "invalid", "GET or PUT required")),
        };
    }
    if let Some(rest) = path.strip_prefix("/identity") {
        let service = service
            .as_ref()
            .ok_or_else(|| error(503, "unavailable", "Demo devices have no identity"))?;
        let conflict = |e: anyhow::Error| error(409, "identity", format!("{e:#}"));
        return match (method, rest) {
            ("GET", "") => Ok(service.models.identity()),
            ("POST", "/recovery") => {
                let passphrase = body["passphrase"].as_str();
                let models = service.models.clone();
                let passphrase = passphrase.map(str::to_owned);
                // Argon2id takes a moment; keep it off the async workers.
                let code = tokio::task::spawn_blocking(move || {
                    models.create_recovery(passphrase.as_deref())
                })
                .await
                .map_err(unavailable)?
                .map_err(conflict)?;
                Ok(serde_json::json!({"code": code, "identity": service.models.identity()}))
            }
            ("POST", "/rotate") => {
                let remove: Vec<String> =
                    serde_json::from_value(body["remove"].clone()).unwrap_or_default();
                let models = service.models.clone();
                let code = body["code"].as_str().unwrap_or_default().to_owned();
                tokio::task::spawn_blocking(move || models.rotate_owner(&code, &remove))
                    .await
                    .map_err(unavailable)?
                    .map_err(conflict)?;
                Ok(service.models.identity())
            }
            _ => Err(error(405, "invalid", "Unknown identity operation")),
        };
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
        ("GET", "/capabilities") => match service {
            Some(service) => output(service.capabilities().await?),
            None => output(MOCK.capabilities()),
        },
        _ => {
            if let Some((id, action)) = path
                .strip_prefix("/capabilities/")
                .and_then(|p| p.split_once('/'))
                && method == "POST"
            {
                let id: CapabilityId = serde_json::from_value(Value::from(id))
                    .map_err(|_| error(404, "not_found", "Unknown capability"))?;
                let action: CapabilityAction = serde_json::from_value(Value::from(action))
                    .map_err(|_| error(404, "not_found", "Unknown capability action"))?;
                return match service {
                    Some(service) => service.capability_action(id, action, body).await,
                    None => MOCK.capability_action(id, action, body),
                };
            }
            if let Some(node) = path
                .strip_prefix("/devices/")
                .and_then(|p| p.strip_suffix("/grant"))
                .filter(|n| !n.contains('/'))
            {
                return match (method, service) {
                    ("GET", Some(service)) => output(service.device_grant(node)?),
                    ("GET", None) => output(MOCK.device_grant(node)?),
                    ("PUT", Some(service)) => output(service.set_device_grant(node, input(body)?)?),
                    ("PUT", None) => output(MOCK.set_device_grant(node, input(body)?)?),
                    _ => Err(error(405, "invalid", "GET or PUT required")),
                };
            }
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

#[cfg(test)]
mod pairing_tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    async fn node(root: &std::path::Path, stop: &CancellationToken) -> (Arc<LocalNode>, Service) {
        let path = root.join("peers.json");
        std::fs::create_dir_all(root).unwrap();
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
        let local = Arc::new(LocalNode::start(path.clone()).await.unwrap());
        let models = Models::start_on(
            path,
            stop.clone(),
            Some(vec!["127.0.0.1:0".parse().unwrap()]),
        )
        .await
        .unwrap();
        let app = Service::new(Arc::downgrade(&local), models);
        (local, app)
    }

    /// Both sides of iroh pairing through the App API the desktop and web UIs call:
    /// the joiner confirms in its sheet, the inviter confirms from its approval card.
    /// Needs network access for the iroh relay, so it is opt-in.
    #[tokio::test]
    #[ignore]
    async fn real_pairing_through_app_api() {
        let root = tempfile::tempdir().unwrap();
        let stop = CancellationToken::new();
        let (a, a_app) = node(&root.path().join("a"), &stop).await;
        let (b, b_app) = node(&root.path().join("b"), &stop).await;
        let invite = a_app
            .pair_invite(PairInviteReq {
                purpose: cranibox_contracts::net::PairPurpose::OwnerDevice,
            })
            .await
            .unwrap();
        assert!(invite.link.starts_with("cranibox://pair/"));
        let accepted = b_app
            .pair_accept(PairAcceptReq { link: invite.link })
            .await
            .unwrap();
        let card = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                let cards = a_app.approvals().await.unwrap();
                if let Some(card) = cards.into_iter().find(|c| c.kind == ApprovalKind::Pairing) {
                    return card;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        })
        .await
        .expect("inviter never showed a pairing approval");
        assert_eq!(card.title_args["code"], accepted.sas);
        assert_eq!(card.from_node.as_deref(), Some(b.id.as_str()));
        let again = a_app.approvals().await.unwrap();
        assert_eq!(again[0].created_at, card.created_at, "stable card");
        let (joiner, inviter) = tokio::join!(
            b_app.pair_confirm(&accepted.flow_id, PairConfirmReq { matches: true }),
            a_app.decide(
                &card.id,
                ApprovalDecision {
                    decision: ApprovalAction::AllowOnce
                }
            ),
        );
        joiner.unwrap();
        inviter.unwrap();
        let cards = a_app.approvals().await.unwrap();
        assert!(cards.iter().all(|c| c.kind != ApprovalKind::Pairing));
        for (node, peer) in [(&a, &b.id), (&b, &a.id)] {
            let config = Config::read(node.config_path()).unwrap();
            assert!(
                config.grants.contains_key(peer),
                "{} has no grant for {peer}",
                node.id
            );
        }
        // A repeated confirm of a finished flow is idempotent; an unknown flow is not found.
        b_app
            .pair_confirm(&accepted.flow_id, PairConfirmReq { matches: true })
            .await
            .unwrap();
        let unknown = b_app
            .pair_confirm("no-such-flow", PairConfirmReq { matches: true })
            .await;
        assert_eq!(unknown.unwrap_err().status, 404);
        stop.cancel();
    }
}

#[cfg(test)]
mod grant_tests {
    use super::*;

    #[tokio::test]
    async fn grants_round_trip_stay_signed_and_keep_guests_from_subscriptions() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("peers.json");
        let store = peers::Store::new(path.clone());
        store.manage(peers::Command::Identity).await.unwrap();
        let caller = "node-0a1b2c3d4e5f60718293a4b5c6d7e8f9";
        store
            .manage(peers::Command::Grant {
                node: caller.into(),
                permissions: peers::config::Permissions {
                    exec: true,
                    terminals: true,
                    ..Default::default()
                },
                inbound: None,
                require_verified: None,
            })
            .await
            .unwrap();
        let grant = read_grant(&path, caller).unwrap();
        let value = serde_json::to_value(grant).unwrap();
        // Every capability is present, including the ones omitted from signed bytes.
        assert_eq!(value["permissions"].as_object().unwrap().len(), 11);
        assert_eq!(value["permissions"]["llm"], false);
        assert!(grant.permissions.exec && !grant.permissions.computer);

        let mut wanted = grant;
        wanted.permissions.computer = true;
        wanted.permissions.exec = false;
        assert_eq!(write_grant(&path, caller, false, wanted).unwrap(), wanted);
        // The re-signed configuration still validates (Config::read checks signatures).
        let config = Config::read(&path).unwrap();
        assert!(config.grants[caller].signature.is_some());
        assert!(config.grants[caller].permissions.computer);
        assert_eq!(read_grant(&path, caller).unwrap(), wanted);

        let mut subscriptions = wanted;
        subscriptions.permissions.llm = true;
        assert_eq!(
            write_grant(&path, caller, false, subscriptions)
                .unwrap_err()
                .status,
            403
        );
        assert_eq!(
            write_grant(&path, caller, true, subscriptions).unwrap(),
            subscriptions
        );
        assert_eq!(
            write_grant(&path, caller, true, DeviceGrant::default())
                .unwrap_err()
                .status,
            400
        );
        assert_eq!(read_grant(&path, "node-unknown").unwrap_err().status, 404);
        assert_eq!(
            write_grant(&path, "node-unknown", true, wanted)
                .unwrap_err()
                .status,
            404
        );
    }

    #[tokio::test]
    async fn mock_dispatch_serves_capabilities_and_grants() {
        let list = route(None, "GET", "/capabilities", Value::Null)
            .await
            .unwrap();
        assert_eq!(list["items"].as_array().unwrap().len(), 4);
        let plan = route(
            None,
            "POST",
            "/capabilities/computer/install_plan",
            Value::Null,
        )
        .await
        .unwrap();
        assert!(plan["plan_id"].is_string());
        assert_eq!(
            route(None, "POST", "/capabilities/nope/start", Value::Null)
                .await
                .unwrap_err()
                .status,
            404
        );
        let node = "node-0a1b2c3d4e5f60718293a4b5c6d7e8f9";
        let grant = route(None, "GET", &format!("/devices/{node}/grant"), Value::Null)
            .await
            .unwrap();
        let updated = route(
            None,
            "PUT",
            &format!("/devices/{node}/grant"),
            grant.clone(),
        )
        .await
        .unwrap();
        assert_eq!(updated, grant);
    }
}
