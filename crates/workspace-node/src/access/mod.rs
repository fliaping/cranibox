//! Access policy is target-owned and independent of transport grants.
use agent_workspace_runtime::{
    config,
    peers::config::{Config, Permissions},
};
use anyhow::{Context, Result, ensure};
pub use cranibox_contracts::access::*;
use cranibox_contracts::roster::RosterView;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
pub(crate) mod api;
mod audit;
pub(crate) mod cli;
#[cfg(test)]
mod tests;

pub fn now() -> u64 {
    agent_workspace_runtime::peers::identity::now()
}
#[derive(Clone)]
pub struct Store {
    peers: PathBuf,
}
#[derive(Default, Serialize, Deserialize)]
struct State {
    #[serde(flatten)]
    policy: AccessPolicy,
    #[serde(default)]
    requests: BTreeMap<String, AccessRequest>,
    #[serde(default)]
    approvals: BTreeMap<String, Approval>,
    #[serde(default)]
    cooldown: BTreeMap<String, u64>,
    #[serde(default)]
    sessions: BTreeMap<String, ActiveSession>,
    #[serde(default)]
    refusals: BTreeMap<String, u64>,
    #[serde(default)]
    jobs: BTreeMap<String, JobAccess>,
    /// The grant credential each peer's modes were last derived from (see `reconcile`).
    #[serde(default)]
    derived: BTreeMap<String, String>,
}
// A job's approval also covers observing/cancelling that job, never starting another one.
#[derive(Serialize, Deserialize)]
struct JobAccess {
    session: ActiveSession,
    expires_at: Option<u64>,
}
#[derive(Clone, Debug)]
pub struct Decision {
    pub basis: String,
}
#[derive(Debug)]
pub enum Denied {
    Off {
        capability: Capability,
        target_name: String,
        hint: String,
    },
    ApprovalRequired {
        request_id: String,
        capability: Capability,
        expires_at: u64,
    },
    Cooldown {
        capability: Capability,
        expires_at: u64,
    },
    Unavailable(anyhow::Error),
}
impl Denied {
    pub fn response(self) -> (axum::http::StatusCode, axum::Json<serde_json::Value>) {
        use axum::{Json, http::StatusCode};
        use serde_json::json;
        let body = match self {
            Self::Off {
                capability,
                target_name,
                hint,
            } => {
                json!({"error":"capability_off","capability":capability,"target_name":target_name,"hint":hint})
            }
            Self::ApprovalRequired {
                request_id,
                capability,
                expires_at,
            } => {
                json!({"error":"approval_required","request_id":request_id,"capability":capability,"expires_at":expires_at})
            }
            Self::Cooldown {
                capability,
                expires_at,
            } => {
                json!({"error":"approval_cooldown","capability":capability,"expires_at":expires_at})
            }
            Self::Unavailable(_) => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({"error":"access_store_unavailable"})),
                );
            }
        };
        (StatusCode::FORBIDDEN, Json(body))
    }
}

/// The mode a grant implies: its capabilities allowed (remote updates are never part of a
/// grant), everything else off.
fn grant_mode(p: &Permissions, capability: Capability) -> Mode {
    if capability != Capability::Update && granted(p, capability) {
        Mode::Allow
    } else {
        Mode::Off
    }
}
pub fn granted(p: &Permissions, capability: Capability) -> bool {
    match capability {
        Capability::Messages => p.messages,
        Capability::Terminals => p.terminals,
        Capability::Exec => p.exec,
        Capability::Projects => p.projects,
        Capability::Desktop => p.desktop,
        Capability::Computer => p.computer,
        Capability::Browser => p.browser,
        Capability::Spaces => p.spaces,
        Capability::Activity => p.activity,
        Capability::Llm => p.llm,
        Capability::Vault => p.vault,
        // Legacy update admission is verified roster membership, checked by the handler.
        // Do not re-sign existing transport grants merely to add the target policy layer.
        Capability::Update => true,
    }
}
impl Store {
    pub fn new(peers: &Path) -> Self {
        Self {
            peers: peers.to_owned(),
        }
    }
    fn path(&self) -> PathBuf {
        self.peers.with_file_name("access-policy.json")
    }
    fn name(&self, node: &str) -> String {
        crate::app::known::load(&self.peers)
            .get(node)
            .map(|k| k.display(node))
            .or_else(|| {
                agent_workspace_runtime::peers::roster::RosterStore::load(
                    self.peers.with_file_name("models").join("roster.json"),
                )
                .ok()?
                .current()?
                .members
                .iter()
                .find(|m| m.node_id == node)
                .map(|m| m.name.clone())
                .filter(|name| !name.is_empty())
            })
            .unwrap_or_else(|| node.to_owned())
    }
    fn migrate(&self) -> Result<State> {
        let config = Config::read(&self.peers)?;
        let roster_path = self.peers.with_file_name("models").join("roster.json");
        let roster = if roster_path.exists() {
            Some(agent_workspace_runtime::peers::roster::RosterStore::load(
                roster_path,
            )?)
        } else {
            None
        };
        let mut state = State::default();
        let mut nodes: std::collections::BTreeSet<_> = config.grants.keys().cloned().collect();
        if let Some(roster) = roster.as_ref().and_then(|r| r.current()) {
            nodes.extend(
                roster
                    .members
                    .iter()
                    .filter(|m| m.node_id != config.node_id)
                    .map(|m| m.node_id.clone()),
            );
        }
        for node in nodes {
            let owner = roster.as_ref().is_some_and(|r| r.is_owner_node(&node));
            let modes = Capability::ALL
                .into_iter()
                .map(|c| {
                    let mode = if owner {
                        if matches!(c, Capability::Messages | Capability::Activity) {
                            Mode::Allow
                        } else {
                            Mode::Ask
                        }
                    } else {
                        config
                            .grants
                            .get(&node)
                            .map_or(Mode::Off, |g| grant_mode(&g.permissions, c))
                    };
                    (c, mode)
                })
                .collect();
            if let Some(grant) = config.grants.get(&node) {
                state.derived.insert(node.clone(), grant.credential.clone());
            }
            state.policy.peers.insert(node, modes);
        }
        Ok(state)
    }
    /// A grant issued after the migration (by the CLI, the app or pairing) is the owner's
    /// explicit choice for a non-owner peer: its capabilities become allowed and the others
    /// off. Modes changed later stay until the peer's credential changes again. Owner
    /// devices keep their modes.
    fn reconcile(&self, state: &mut State, at: u64) -> Result<()> {
        let Ok(config) = Config::read(&self.peers) else {
            return Ok(());
        };
        state
            .derived
            .retain(|node, _| config.grants.contains_key(node));
        let mut roster = None;
        for (node, grant) in &config.grants {
            if state.derived.get(node) == Some(&grant.credential) {
                continue;
            }
            let roster = match &roster {
                Some(roster) => roster,
                None => roster.insert({
                    let path = self.peers.with_file_name("models").join("roster.json");
                    path.exists()
                        .then(|| agent_workspace_runtime::peers::roster::RosterStore::load(path))
                        .transpose()?
                }),
            };
            if !roster.as_ref().is_some_and(|r| r.is_owner_node(node)) {
                for capability in Capability::ALL {
                    let mode = grant_mode(&grant.permissions, capability);
                    if state.policy.mode(node, capability) == mode {
                        continue;
                    }
                    self.event(
                        EventKind::PolicyChanged,
                        Some(node),
                        Some(capability),
                        None,
                        None,
                        &serde_json::to_string(&mode)?,
                        at,
                    )?;
                    state
                        .policy
                        .peers
                        .entry(node.clone())
                        .or_insert_with(defaults)
                        .insert(capability, mode);
                }
            }
            state.derived.insert(node.clone(), grant.credential.clone());
        }
        Ok(())
    }
    fn transaction<T>(&self, at: u64, edit: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
        let path = self.path();
        let _lock = config::lock_credentials(&path).context("lock access policy")?;
        let mut state: State = if path.exists() {
            serde_json::from_slice(&std::fs::read(&path).context("read access policy")?)
                .context("decode access policy")?
        } else {
            self.migrate()?
        };
        let before = serde_json::to_vec(&state)?;
        self.reconcile(&mut state, at)?;
        self.expire(&mut state, at)?;
        let result = edit(&mut state);
        let after = serde_json::to_vec(&state)?;
        if before != after || !path.exists() {
            config::write_private(&path, &after, true).context("save access policy")?;
        }
        result
    }
    pub fn initialize(&self) -> Result<()> {
        self.transaction(now(), |state| {
            // A process restart ended transports; only terminals retain reconnect grace.
            let sessions = std::mem::take(&mut state.sessions);
            for session in sessions.into_values() {
                self.end(state, &session, now())?;
            }
            Ok(())
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn event(
        &self,
        kind: EventKind,
        node: Option<&str>,
        capability: Option<Capability>,
        key: Option<&str>,
        basis: Option<&str>,
        result: &str,
        at: u64,
    ) -> Result<()> {
        audit::append(
            &self.peers.with_file_name("access-audit.jsonl"),
            AuditEvent {
                index: 0,
                at,
                event: kind,
                from: node.map(str::to_owned),
                name: node.map(|n| self.name(n)),
                capability,
                session_key: key.map(str::to_owned),
                basis: basis.map(str::to_owned),
                result: result.into(),
                prev: String::new(),
            },
        )
    }
    fn expire(&self, state: &mut State, at: u64) -> Result<()> {
        for r in state
            .requests
            .values_mut()
            .filter(|r| r.state == RequestState::Pending && r.expires_at <= at)
        {
            self.event(
                EventKind::Expired,
                Some(&r.from),
                Some(r.capability),
                Some(&r.session_key),
                Some(&r.id),
                "request",
                at,
            )?;
            r.state = RequestState::Expired;
        }
        let expired: Vec<_> = state
            .approvals
            .values()
            .filter(|a| a.expires_at.is_some_and(|e| e <= at))
            .cloned()
            .collect();
        for a in expired {
            self.event(
                EventKind::Expired,
                Some(&a.from),
                Some(a.capability),
                a.session_key.as_deref(),
                Some(&a.id),
                "approval",
                at,
            )?;
            state.approvals.remove(&a.id);
            if let Some(request) = state.requests.get_mut(&a.id) {
                request.state = RequestState::Expired;
            }
        }
        state.cooldown.retain(|_, e| *e > at);
        state
            .jobs
            .retain(|_, job| job.expires_at.is_none_or(|e| e > at));
        state.refusals.retain(|_, minute| *minute >= at / 60);
        state.requests.retain(|_, r| {
            r.state == RequestState::Pending || r.expires_at.saturating_add(86400) > at
        });
        Ok(())
    }
    fn decision(state: &State, node: &str, capability: Capability, key: &str) -> Option<Decision> {
        let mode = state.policy.mode(node, capability);
        if mode == Mode::Off {
            return None;
        }
        if mode == Mode::Allow && !capability.requires_approval() {
            return Some(Decision {
                basis: "allow".into(),
            });
        }
        state
            .approvals
            .values()
            .find(|a| {
                a.from == node
                    && a.capability == capability
                    && (a.scope == ApprovalScope::Until || a.session_key.as_deref() == Some(key))
            })
            .map(|a| Decision {
                basis: a.id.clone(),
            })
    }
    fn refused(
        &self,
        state: &mut State,
        node: &str,
        capability: Capability,
        reason: &str,
        at: u64,
    ) -> Result<()> {
        if state
            .refusals
            .insert(format!("{node}:{capability}:{reason}"), at / 60)
            != Some(at / 60)
        {
            self.event(
                EventKind::Refused,
                Some(node),
                Some(capability),
                None,
                None,
                reason,
                at,
            )?;
        }
        Ok(())
    }
    pub fn authorize(
        &self,
        node: &str,
        capability: Capability,
        key: &str,
    ) -> std::result::Result<Decision, Denied> {
        self.authorize_at(node, capability, key, now())
    }
    fn authorize_at(
        &self,
        node: &str,
        capability: Capability,
        key: &str,
        at: u64,
    ) -> std::result::Result<Decision, Denied> {
        self.transaction(at, |state| {
            ensure!(
                !key.is_empty() && key.len() <= 512,
                "invalid access session key"
            );
            if let Some(decision) = Self::decision(state, node, capability, key) {
                return Ok(Ok(decision));
            }
            if state.policy.mode(node, capability) == Mode::Off {
                self.refused(state, node, capability, "off", at)?;
                let target_name = crate::device_info::local().name.clone();
                let hint = format!(
                    "On {target_name}, open Devices, select {}, and allow {capability} on its Access tab.",
                    self.name(node)
                );
                return Ok(Err(Denied::Off {
                    capability,
                    target_name,
                    hint,
                }));
            }
            if let Some(expires_at) = state.cooldown.get(node).copied() {
                self.refused(state, node, capability, "cooldown", at)?;
                return Ok(Err(Denied::Cooldown {
                    capability,
                    expires_at,
                }));
            }
            if let Some(r) = state
                .requests
                .values()
                .find(|r| r.from == node && r.state == RequestState::Pending)
                .cloned()
            {
                self.refused(state, node, capability, "approval_pending", at)?;
                return Ok(Err(Denied::ApprovalRequired {
                    request_id: r.id.clone(),
                    capability: r.capability,
                    expires_at: r.expires_at,
                }));
            }
            let request = AccessRequest {
                id: uuid::Uuid::new_v4().to_string(),
                from: node.into(),
                name: self.name(node),
                capability,
                session_key: key.into(),
                created_at: at,
                expires_at: at + 120,
                state: RequestState::Pending,
                path: Config::read(&self.peers).ok().and_then(|c| {
                    crate::app::paths::load(&self.peers)
                        .get(&c.node_id)
                        .and_then(|row| row.get(node))
                        .and_then(|p| serde_json::to_value(p).ok())
                }),
            };
            self.event(
                EventKind::RequestCreated,
                Some(node),
                Some(capability),
                Some(key),
                Some(&request.id),
                "pending",
                at,
            )?;
            let denied = Denied::ApprovalRequired {
                request_id: request.id.clone(),
                capability,
                expires_at: request.expires_at,
            };
            state.requests.insert(request.id.clone(), request);
            Ok(Err(denied))
        })
        .map_err(Denied::Unavailable)?
    }
    pub fn permitted(&self, node: &str, capability: Capability, key: &str) -> bool {
        self.transaction(now(), |state| {
            Ok(Self::decision(state, node, capability, key).is_some())
        })
        .unwrap_or(false)
    }
    pub fn is_enabled(&self, node: &str, capability: Capability) -> bool {
        self.transaction(now(), |state| {
            Ok(state.policy.mode(node, capability) != Mode::Off)
        })
        .unwrap_or(false)
    }
    pub fn change(&self, node: &str, changes: BTreeMap<Capability, Mode>) -> Result<()> {
        ensure!(
            agent_workspace_runtime::protocol::valid_node_id(node),
            "invalid node ID"
        );
        ensure!(!changes.is_empty(), "empty policy change");
        self.transaction(now(), |state| {
            for (capability, mode) in changes {
                self.event(
                    EventKind::PolicyChanged,
                    Some(node),
                    Some(capability),
                    None,
                    None,
                    &serde_json::to_string(&mode)?,
                    now(),
                )?;
                state
                    .policy
                    .peers
                    .entry(node.into())
                    .or_insert_with(defaults)
                    .insert(capability, mode);
                state
                    .approvals
                    .retain(|_, a| a.from != node || a.capability != capability);
                state
                    .jobs
                    .retain(|_, j| j.session.from != node || j.session.capability != capability);
                for request in state.requests.values_mut().filter(|r| {
                    r.from == node
                        && r.capability == capability
                        && matches!(r.state, RequestState::Pending | RequestState::Approved)
                }) {
                    request.state = RequestState::Denied;
                }
            }
            Ok(())
        })
    }
    pub fn decide(&self, id: &str, input: DecideRequest) -> Result<AccessRequest> {
        self.decide_at(id, input, now())
    }
    fn decide_at(&self, id: &str, input: DecideRequest, at: u64) -> Result<AccessRequest> {
        self.transaction(at, |state| {
            let r = state
                .requests
                .get_mut(id)
                .context("unknown access request")?;
            ensure!(
                r.state == RequestState::Pending,
                "request already decided or expired"
            );
            if input.decision == ApprovalDecision::Deny {
                self.event(
                    EventKind::Denied,
                    Some(&r.from),
                    Some(r.capability),
                    Some(&r.session_key),
                    Some(id),
                    "local",
                    at,
                )?;
                r.state = RequestState::Denied;
                state.cooldown.insert(r.from.clone(), at + 60);
            } else {
                let scope = input.scope.context("approval scope required")?;
                let (scope, expires_at) = match scope {
                    DurationScope::Session => (ApprovalScope::Session, Some(at + 120)),
                    DurationScope::Minutes30 => (ApprovalScope::Until, Some(at + 1800)),
                    DurationScope::Hours8 => (ApprovalScope::Until, Some(at + 28800)),
                };
                self.event(
                    EventKind::Approved,
                    Some(&r.from),
                    Some(r.capability),
                    Some(&r.session_key),
                    Some(id),
                    "local",
                    at,
                )?;
                r.state = RequestState::Approved;
                state.approvals.insert(
                    id.into(),
                    Approval {
                        id: id.into(),
                        from: r.from.clone(),
                        capability: r.capability,
                        scope,
                        session_key: (scope == ApprovalScope::Session)
                            .then(|| r.session_key.clone()),
                        expires_at,
                        approved_by: "local".into(),
                        approved_at: at,
                    },
                );
            }
            Ok(r.clone())
        })
    }
    pub fn request(&self, id: &str, requester: &str) -> Result<Option<AccessRequest>> {
        self.transaction(now(), |state| {
            Ok(state
                .requests
                .get(id)
                .filter(|r| r.from == requester)
                .cloned())
        })
    }
    pub fn revoke(&self, id: Option<&str>, emergency: bool) -> Result<()> {
        self.transaction(now(), |state| {
            self.event(
                if emergency {
                    EventKind::EmergencyDisconnect
                } else {
                    EventKind::Denied
                },
                None,
                None,
                None,
                id,
                "revoked",
                now(),
            )?;
            state
                .approvals
                .retain(|key, _| id.is_some_and(|id| id != key));
            state
                .jobs
                .retain(|_, j| !emergency && id.is_some_and(|id| id != j.session.basis));
            for r in state.requests.values_mut().filter(|r| {
                matches!(r.state, RequestState::Pending | RequestState::Approved)
                    && (emergency || id.is_none() || id == Some(r.id.as_str()))
            }) {
                r.state = RequestState::Denied;
            }
            // Clearing the session records makes their live guards fail even in allow mode.
            let ended: Vec<_> = state
                .sessions
                .values()
                .filter(|s| emergency || id.is_none() || id == Some(s.basis.as_str()))
                .cloned()
                .collect();
            for session in ended {
                state.sessions.remove(&session.id);
                self.end(state, &session, now())?;
            }
            Ok(())
        })
    }
    pub fn start(&self, node: &str, capability: Capability, key: &str) -> Result<Session> {
        let session = self.transaction(now(), |state| {
            let decision = Self::decision(state, node, capability, key)
                .context("access changed before session start")?;
            if state
                .approvals
                .get(&decision.basis)
                .is_some_and(|a| a.scope == ApprovalScope::Session)
            {
                ensure!(
                    !state.sessions.values().any(|s| s.basis == decision.basis),
                    "session already attached"
                );
            }
            let session = ActiveSession {
                id: uuid::Uuid::new_v4().to_string(),
                from: node.into(),
                name: self.name(node),
                capability,
                session_key: key.into(),
                started_at: now(),
                basis: decision.basis,
            };
            self.event(
                EventKind::SessionStarted,
                Some(node),
                Some(capability),
                Some(key),
                Some(&session.basis),
                "started",
                now(),
            )?;
            if let Some(a) = state
                .approvals
                .get_mut(&session.basis)
                .filter(|a| a.scope == ApprovalScope::Session)
            {
                a.expires_at = None;
            }
            state.sessions.insert(session.id.clone(), session.clone());
            Ok(session)
        })?;
        Ok(Session {
            store: self.clone(),
            session,
        })
    }
    fn end(&self, state: &mut State, session: &ActiveSession, at: u64) -> Result<()> {
        self.event(
            EventKind::SessionEnded,
            Some(&session.from),
            Some(session.capability),
            Some(&session.session_key),
            Some(&session.basis),
            "ended",
            at,
        )?;
        for job in state
            .jobs
            .values_mut()
            .filter(|j| j.session.id == session.id)
        {
            job.expires_at = Some(job.expires_at.map_or(at + 600, |e| e.min(at + 600)));
        }
        if let Some(a) = state
            .approvals
            .get_mut(&session.basis)
            .filter(|a| a.scope == ApprovalScope::Session)
        {
            if session.capability == Capability::Terminals {
                a.expires_at = Some(at + 60);
            } else {
                state.approvals.remove(&session.basis);
                if let Some(r) = state.requests.get_mut(&session.basis) {
                    r.state = RequestState::Expired;
                }
            }
        }
        Ok(())
    }
    pub fn job_decision(
        &self,
        node: &str,
        capability: Capability,
        id: &str,
    ) -> Result<Option<Decision>> {
        self.transaction(now(), |state| {
            Ok(state
                .jobs
                .get(&format!("{node}:{capability}:{id}"))
                .filter(|_| state.policy.mode(node, capability) != Mode::Off)
                .map(|j| Decision {
                    basis: j.session.basis.clone(),
                }))
        })
    }
    pub fn view(&self) -> Result<AccessView> {
        self.transaction(now(), |state| {
            let config = Config::read(&self.peers)?;
            let mut nodes: std::collections::BTreeSet<_> = config
                .grants
                .keys()
                .chain(config.routes.keys())
                .chain(state.policy.peers.keys())
                .cloned()
                .collect();
            nodes.extend(crate::app::known::load(&self.peers).into_keys());
            if let Ok(roster) = agent_workspace_runtime::peers::roster::RosterStore::load(
                self.peers.with_file_name("models").join("roster.json"),
            ) && let Some(roster) = roster.current()
            {
                nodes.extend(roster.members.iter().map(|m| m.node_id.clone()));
            }
            nodes.remove(&config.node_id);
            Ok(AccessView {
                peers: nodes
                    .into_iter()
                    .map(|node| PeerPolicy {
                        name: self.name(&node),
                        capabilities: Capability::ALL
                            .into_iter()
                            .map(|c| (c, state.policy.mode(&node, c)))
                            .collect(),
                        node,
                    })
                    .collect(),
                pending: state
                    .requests
                    .values()
                    .filter(|r| r.state == RequestState::Pending)
                    .cloned()
                    .collect(),
                approvals: state.approvals.values().cloned().collect(),
                sessions: state.sessions.values().cloned().collect(),
            })
        })
    }
    pub fn audit(
        &self,
        limit: usize,
        before: Option<u64>,
        node: Option<&str>,
        capability: Option<Capability>,
    ) -> Result<AuditView> {
        let _lock = config::lock_credentials(&self.path())?;
        let (entries, broken_index) =
            audit::read(&self.peers.with_file_name("access-audit.jsonl"))?;
        Ok(AuditView {
            entries: entries
                .into_iter()
                .rev()
                .filter(|e| {
                    before.is_none_or(|b| e.index < b)
                        && node.is_none_or(|n| e.from.as_deref() == Some(n))
                        && capability.is_none_or(|c| e.capability == Some(c))
                })
                .take(limit.min(10000))
                .collect(),
            verified: broken_index.is_none(),
            broken_index,
        })
    }
}
pub struct Session {
    store: Store,
    session: ActiveSession,
}
impl Session {
    pub fn bind_job(&self, id: &str) -> Result<()> {
        self.store.transaction(now(), |state| {
            ensure!(
                state.sessions.contains_key(&self.session.id),
                "session revoked"
            );
            let expires_at = state
                .approvals
                .get(&self.session.basis)
                .filter(|a| a.scope == ApprovalScope::Until)
                .and_then(|a| a.expires_at);
            state.jobs.insert(
                format!("{}:{}:{id}", self.session.from, self.session.capability),
                JobAccess {
                    session: self.session.clone(),
                    expires_at,
                },
            );
            Ok(())
        })
    }
    pub async fn revoked(&self) {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(250));
        loop {
            interval.tick().await;
            if !self.valid() {
                return;
            }
        }
    }
    pub fn valid(&self) -> bool {
        self.store
            .transaction(now(), |s| {
                Ok(s.sessions.contains_key(&self.session.id)
                    && Store::decision(
                        s,
                        &self.session.from,
                        self.session.capability,
                        &self.session.session_key,
                    )
                    .is_some())
            })
            .unwrap_or(false)
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let result = self.store.transaction(now(), |state| {
            if state.sessions.remove(&self.session.id).is_some() {
                self.store.end(state, &self.session, now())?;
            }
            Ok(())
        });
        if let Err(error) = result {
            eprintln!("end access session: {error:#}");
        }
    }
}

#[cfg(test)]
pub(crate) fn allow_fixture(peers: &Path, node: &str, capabilities: &[Capability]) {
    let store = Store::new(peers);
    store
        .change(
            node,
            capabilities.iter().map(|c| (*c, Mode::Allow)).collect(),
        )
        .unwrap();
    for capability in capabilities.iter().filter(|c| c.requires_approval()) {
        let Denied::ApprovalRequired { request_id, .. } =
            store.authorize(node, *capability, "fixture").unwrap_err()
        else {
            panic!("approval required");
        };
        store
            .decide(
                &request_id,
                DecideRequest {
                    decision: ApprovalDecision::Approve,
                    scope: Some(DurationScope::Hours8),
                },
            )
            .unwrap();
    }
}
