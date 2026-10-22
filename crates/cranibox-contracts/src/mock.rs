//! In-memory implementations for parallel development (feature `mock`).
//!
//! Seeded from `fixtures/*.json`, which are the same files the WS7 frontend
//! mock serves. No I/O, no network, fake keys only.
use crate::{
    NodeId,
    agents::AgentId,
    app::*,
    llm::{mgmt::*, *},
    roster::{Role, Roster, RosterMember, RosterView},
    vault::{AccountStore, StoreError},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

/// Raw fixture JSON, keyed by file stem.
pub mod fixtures {
    macro_rules! fixtures {
        ($($name:ident => $file:literal),* $(,)?) => {
            $(pub const $name: &str = include_str!(concat!("../fixtures/", $file));)*
            /// `(file name, JSON)` for every JSON fixture.
            pub const ALL: &[(&str, &str)] = &[$(($file, $name)),*];
        };
    }
    fixtures! {
        ROSTER => "roster.json",
        PAIR_TICKET => "pair_ticket.json",
        IROH_CONFIG => "iroh_config.json",
        LLM_HEALTH => "llm_health.json",
        LLM_STATUS => "llm_status.json",
        LLM_ACCOUNTS => "llm_accounts.json",
        LLM_MODELS => "llm_models.json",
        LLM_SIGNIN_START => "llm_signin_start.json",
        LLM_NEW_ACCOUNT => "llm_new_account.json",
        LLM_EVENTS => "llm_events.json",
        AGENTS_APPLIED => "agents_applied.json",
        APP_STATUS => "app_status.json",
        APP_DEVICES => "app_devices.json",
        APP_APPROVALS => "app_approvals.json",
        APP_AGENTS => "app_agents.json",
        APP_PAIR_INVITE => "app_pair_invite.json",
        APP_PAIR_ACCEPTED => "app_pair_accepted.json",
        APP_EVENTS => "app_events.json",
        APP_CAPABILITIES => "app_capabilities.json",
        APP_INSTALL_PLAN => "app_install_plan.json",
        APP_DEVICE_GRANT => "app_device_grant.json",
        VAULT_PULL_REQ => "vault_pull_req.json",
        VAULT_PULL_RESP => "vault_pull_resp.json",
        VAULT_REFRESH_REQUEST => "vault_refresh_request.json",
        ERROR_UNAVAILABLE => "error_unavailable.json",
    }
    pub const PRESETS_TOML: &str = include_str!("../fixtures/presets.example.toml");
}

fn load<T: serde::de::DeserializeOwned>(json: &str) -> T {
    serde_json::from_str(json).expect("contract fixture matches its type")
}

/// Roster view over a fixed roster; `set` bumps subscribers.
pub struct MockRoster {
    roster: Mutex<Option<Roster>>,
    epoch: tokio::sync::watch::Sender<u64>,
}
impl MockRoster {
    pub fn new(roster: Option<Roster>) -> Self {
        let epoch = roster.as_ref().map_or(0, |r| r.epoch);
        Self {
            roster: Mutex::new(roster),
            epoch: tokio::sync::watch::channel(epoch).0,
        }
    }
    /// The `fixtures/roster.json` roster.
    pub fn fixture() -> Self {
        Self::new(Some(load(fixtures::ROSTER)))
    }
    pub fn set(&self, roster: Option<Roster>) {
        let epoch = roster.as_ref().map_or(0, |r| r.epoch);
        *self.roster.lock().unwrap() = roster;
        self.epoch.send_replace(epoch);
    }
}
impl RosterView for MockRoster {
    fn current(&self) -> Option<Roster> {
        self.roster.lock().unwrap().clone()
    }
    fn is_owner_node(&self, node: &str) -> bool {
        self.current().is_some_and(|r| r.member(node).is_some())
    }
    fn hub(&self) -> Option<RosterMember> {
        self.current()?
            .members
            .into_iter()
            .find(|m| m.role == Role::Hub)
    }
    fn claude_node(&self) -> Option<RosterMember> {
        self.current()?
            .members
            .into_iter()
            .find(|m| m.is_claude_node)
    }
    fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.epoch.subscribe()
    }
}

/// Thread-safe in-memory [`AccountStore`] for WS2 development.
#[derive(Default)]
pub struct MemoryAccountStore {
    accounts: Mutex<BTreeMap<String, (AccountSummary, Option<SecretBox>)>>,
}
impl MemoryAccountStore {
    pub fn new() -> Self {
        Self::default()
    }
}
impl AccountStore for MemoryAccountStore {
    fn list(&self) -> Vec<AccountSummary> {
        self.accounts
            .lock()
            .unwrap()
            .values()
            .map(|(a, _)| a.clone())
            .collect()
    }
    fn get(&self, id: &str) -> Option<AccountSummary> {
        self.accounts
            .lock()
            .unwrap()
            .get(id)
            .map(|(a, _)| a.clone())
    }
    fn get_secret(&self, id: &str) -> Result<SecretBox, StoreError> {
        match self.accounts.lock().unwrap().get(id) {
            None => Err(StoreError::NotFound),
            Some((_, None)) => Err(StoreError::NoSecret),
            Some((_, Some(secret))) => Ok(secret.clone()),
        }
    }
    fn put(&self, account: AccountSummary, secret: Option<SecretBox>) -> Result<(), StoreError> {
        let mut accounts = self.accounts.lock().unwrap();
        let secret = secret.or_else(|| accounts.get(&account.id).and_then(|(_, s)| s.clone()));
        accounts.insert(account.id.clone(), (account, secret));
        Ok(())
    }
    fn remove(&self, id: &str) -> Result<(), StoreError> {
        self.accounts
            .lock()
            .unwrap()
            .remove(id)
            .map(|_| ())
            .ok_or(StoreError::NotFound)
    }
}

/// Fixture-backed mgmt API. Mutations change in-memory state only.
pub struct MockLlmMgmt {
    pub node: NodeId,
    accounts: Mutex<Vec<AccountSummary>>,
    claude_node: Mutex<Option<NodeId>>,
}
impl MockLlmMgmt {
    pub fn fixture(node: impl Into<NodeId>) -> Self {
        Self {
            node: node.into(),
            accounts: Mutex::new(load(fixtures::LLM_ACCOUNTS)),
            claude_node: Mutex::new(None),
        }
    }
    pub fn claude_node(&self) -> Option<NodeId> {
        self.claude_node.lock().unwrap().clone()
    }
}
#[async_trait::async_trait]
impl LlmMgmt for MockLlmMgmt {
    async fn status(&self) -> Result<Status, MgmtError> {
        Ok(load(fixtures::LLM_STATUS))
    }
    async fn accounts(&self) -> Result<Vec<AccountSummary>, MgmtError> {
        Ok(self.accounts.lock().unwrap().clone())
    }
    async fn add_account(&self, account: NewAccount) -> Result<AccountSummary, MgmtError> {
        let (provider, label) = match account {
            NewAccount::ApiKey { preset, label, .. } => (preset.clone(), label.unwrap_or(preset)),
            NewAccount::Custom { label, .. } => ("custom".into(), label),
        };
        let mut accounts = self.accounts.lock().unwrap();
        let summary = AccountSummary {
            id: format!("acc-mock-{}", accounts.len() + 1),
            provider,
            plan: None,
            label,
            kind: AccountKind::ApiKey,
            home_node: self.node.clone(),
            sync: true,
            enabled_on: vec![self.node.clone()],
            all_nodes: false,
            state: AccountState::Ready,
        };
        accounts.push(summary.clone());
        Ok(summary)
    }
    async fn delete_account(&self, id: &str) -> Result<(), MgmtError> {
        let mut accounts = self.accounts.lock().unwrap();
        let before = accounts.len();
        accounts.retain(|a| a.id != id);
        if accounts.len() == before {
            return Err(MgmtError::NotFound);
        }
        Ok(())
    }
    async fn enable(&self, id: &str, req: Enable) -> Result<AccountSummary, MgmtError> {
        let mut accounts = self.accounts.lock().unwrap();
        let account = accounts
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or(MgmtError::NotFound)?;
        match (req.node, req.all_nodes) {
            (Some(node), None) => {
                account.enabled_on.retain(|n| *n != node);
                if req.on {
                    account.enabled_on.push(node);
                }
            }
            (None, Some(true)) => account.all_nodes = req.on,
            _ => {
                return Err(MgmtError::Invalid(
                    "exactly one of node or all_nodes".into(),
                ));
            }
        }
        Ok(account.clone())
    }
    async fn signin_start(&self, req: SignInStartReq) -> Result<SignInStart, MgmtError> {
        let starts: Vec<SignInStart> = load(fixtures::LLM_SIGNIN_START);
        let index = if req.provider == "kimi-code" { 1 } else { 0 };
        Ok(starts[index].clone())
    }
    async fn signin_complete(
        &self,
        flow: &str,
        _input: SignInInput,
    ) -> Result<AccountSummary, MgmtError> {
        let mut account = self
            .add_account(NewAccount::ApiKey {
                preset: "codex".into(),
                key: String::new(),
                label: Some(format!("Signed in ({flow})")),
                region: None,
            })
            .await?;
        account.kind = AccountKind::OAuth;
        Ok(account)
    }
    async fn models(&self) -> Result<Vec<ModelEntry>, MgmtError> {
        Ok(load(fixtures::LLM_MODELS))
    }
    async fn set_claude_node(&self, req: SetClaudeNode) -> Result<(), MgmtError> {
        *self.claude_node.lock().unwrap() = Some(req.node);
        Ok(())
    }
    async fn presets(&self) -> Result<Vec<Preset>, MgmtError> {
        // The mock avoids a TOML dependency; the example rows are mirrored here.
        Ok(vec![Preset {
            id: "deepseek".into(),
            label: "DeepSeek".into(),
            region: vec!["cn".into(), "global".into()],
            chat: Some("https://api.deepseek.com".into()),
            responses: None,
            anthropic: Some("https://api.deepseek.com/anthropic".into()),
            default_wire: Wire::Chat,
            models: vec![],
            no_list: false,
            quirks: vec![],
            key_hint: None,
        }])
    }
}

/// Fixture-backed App API for UI development.
pub struct MockAppApi {
    approvals: Mutex<Vec<ApprovalCard>>,
    agents: Mutex<Vec<AgentInfo>>,
    capabilities: Mutex<Capabilities>,
    grants: Mutex<BTreeMap<NodeId, DeviceGrant>>,
    pub mgmt: Arc<MockLlmMgmt>,
}
impl MockAppApi {
    pub fn fixture() -> Self {
        let devices: Vec<Device> = load(fixtures::APP_DEVICES);
        Self {
            approvals: Mutex::new(load(fixtures::APP_APPROVALS)),
            agents: Mutex::new(load(fixtures::APP_AGENTS)),
            capabilities: Mutex::new(load(fixtures::APP_CAPABILITIES)),
            grants: Mutex::new(
                devices
                    .iter()
                    .filter(|d| !d.badges.contains(&Badge::This))
                    .map(|d| {
                        (
                            d.node_id.clone(),
                            DeviceGrant {
                                permissions: d.caps,
                            },
                        )
                    })
                    .collect(),
            ),
            mgmt: Arc::new(MockLlmMgmt::fixture(devices[0].node_id.clone())),
        }
    }
    /// `GET /capabilities`.
    pub fn capabilities(&self) -> Capabilities {
        self.capabilities.lock().unwrap().clone()
    }
    /// `POST /capabilities/{id}/{action}`: the updated item, or an [`InstallPlan`].
    pub fn capability_action(
        &self,
        id: CapabilityId,
        action: CapabilityAction,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, AppError> {
        let mut capabilities = self.capabilities.lock().unwrap();
        let item = capabilities
            .items
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or_else(not_found)?;
        if !item.actions.contains(&action) {
            return Err(invalid("action does not apply in the current state"));
        }
        let plan: InstallPlan = load(fixtures::APP_INSTALL_PLAN);
        match action {
            CapabilityAction::InstallPlan => {
                return serde_json::to_value(plan).map_err(|e| invalid(e.to_string()));
            }
            CapabilityAction::Install => {
                let req: InstallReq = serde_json::from_value(body)
                    .map_err(|_| invalid("install requires a reviewed plan_id"))?;
                if req.plan_id != plan.plan_id {
                    return Err(invalid("install plan expired or already used"));
                }
                item.provider.installed = true;
            }
            CapabilityAction::Start => item.provider.running = true,
            CapabilityAction::Stop => {
                item.provider.running = false;
                if id == CapabilityId::Browser {
                    item.sharing = CapabilitySharing {
                        enabled: false,
                        address: None,
                    };
                }
            }
            CapabilityAction::Enable => item.sharing.enabled = true,
            CapabilityAction::Disable => item.sharing.enabled = false,
            CapabilityAction::RequestPermissions => {
                for permission in &mut item.permissions {
                    permission.state = PermissionState::Granted;
                }
            }
            CapabilityAction::OpenSettings | CapabilityAction::Refresh => {}
        }
        if id == CapabilityId::Browser && item.provider.running {
            item.sharing = CapabilitySharing {
                enabled: true,
                address: Some("127.0.0.1:9333".into()),
            };
        }
        item.actions = mock_actions(item);
        serde_json::to_value(&*item).map_err(|e| invalid(e.to_string()))
    }
    /// `GET /devices/{node}/grant`.
    pub fn device_grant(&self, node: &str) -> Result<DeviceGrant, AppError> {
        self.grants
            .lock()
            .unwrap()
            .get(node)
            .copied()
            .ok_or_else(not_found)
    }
    /// `PUT /devices/{node}/grant`.
    pub fn set_device_grant(
        &self,
        node: &str,
        grant: DeviceGrant,
    ) -> Result<DeviceGrant, AppError> {
        let mut grants = self.grants.lock().unwrap();
        let current = grants.get_mut(node).ok_or_else(not_found)?;
        if grant.permissions == DeviceCaps::default() {
            return Err(invalid(
                "a grant needs at least one capability; remove the device instead",
            ));
        }
        *current = grant;
        Ok(grant)
    }
}
fn mock_actions(item: &CapabilityItem) -> Vec<CapabilityAction> {
    use CapabilityAction::*;
    let mut actions = vec![];
    match item.id {
        CapabilityId::Terminal => {}
        CapabilityId::Desktop => {
            actions.push(if item.sharing.enabled {
                Disable
            } else {
                Enable
            });
            actions.push(OpenSettings);
        }
        CapabilityId::Computer => {
            if !item.provider.installed {
                actions.extend([InstallPlan, Install]);
            } else {
                actions.push(if item.provider.running { Stop } else { Start });
                actions.push(if item.sharing.enabled {
                    Disable
                } else {
                    Enable
                });
                actions.extend([RequestPermissions, OpenSettings]);
            }
        }
        CapabilityId::Browser => {
            if item.provider.installed {
                actions.push(if item.provider.running { Stop } else { Start });
            }
        }
    }
    actions.push(Refresh);
    actions
}
fn invalid(message: impl Into<String>) -> AppError {
    AppError {
        status: 400,
        code: "invalid".into(),
        message: message.into(),
    }
}
fn not_found() -> AppError {
    AppError {
        status: 404,
        code: "not_found".into(),
        message: "not found".into(),
    }
}
#[async_trait::async_trait]
impl AppApi for MockAppApi {
    async fn status(&self) -> Result<StatusPill, AppError> {
        let mut pill: StatusPill = load(fixtures::APP_STATUS);
        pill.approvals_pending = self.approvals.lock().unwrap().len() as u32;
        Ok(pill)
    }
    async fn devices(&self) -> Result<Vec<Device>, AppError> {
        Ok(load(fixtures::APP_DEVICES))
    }
    async fn pair_invite(&self, _req: PairInviteReq) -> Result<PairInvite, AppError> {
        Ok(load(fixtures::APP_PAIR_INVITE))
    }
    async fn pair_accept(&self, _req: PairAcceptReq) -> Result<PairAccepted, AppError> {
        Ok(load(fixtures::APP_PAIR_ACCEPTED))
    }
    async fn pair_confirm(&self, flow: &str, _req: PairConfirmReq) -> Result<(), AppError> {
        let accepted: PairAccepted = load(fixtures::APP_PAIR_ACCEPTED);
        if flow != accepted.flow_id {
            return Err(not_found());
        }
        Ok(())
    }
    async fn approvals(&self) -> Result<Vec<ApprovalCard>, AppError> {
        Ok(self.approvals.lock().unwrap().clone())
    }
    async fn decide(&self, id: &str, req: ApprovalDecision) -> Result<(), AppError> {
        let mut approvals = self.approvals.lock().unwrap();
        let card = approvals
            .iter()
            .position(|c| c.id == id)
            .ok_or_else(not_found)?;
        if !approvals[card].actions.contains(&req.decision) {
            return Err(AppError {
                status: 400,
                code: "invalid".into(),
                message: "action not offered on this card".into(),
            });
        }
        approvals.remove(card);
        Ok(())
    }
    async fn agents(&self) -> Result<Vec<AgentInfo>, AppError> {
        Ok(self.agents.lock().unwrap().clone())
    }
    async fn update_agent(&self, id: AgentId, req: AgentUpdate) -> Result<AgentInfo, AppError> {
        let mut agents = self.agents.lock().unwrap();
        let agent = agents
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(not_found)?;
        agent.managed = req.managed;
        agent.model = req.model;
        Ok(agent.clone())
    }
    async fn models(&self) -> Result<Vec<ModelEntry>, AppError> {
        self.mgmt.models().await.map_err(|e| AppError {
            status: 503,
            code: "unavailable".into(),
            message: e.to_string(),
        })
    }
}
