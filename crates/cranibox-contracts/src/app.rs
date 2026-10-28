//! C6 — App API, the UI's backend (`docs/design/WORKPLAN.md` §1 C6, ARCHITECTURE §8).
//!
//! Web: HTTP `/api/v1/app/*` served by `workspace-web` and the local control
//! plane of `workspace-node`. Tauri: IPC commands with the same names
//! (`app_status`, …) and the same JSON. Implemented by WS7 (`app_api.rs`);
//! `crate::mock::MockAppApi` and `fixtures/app_*.json` back the UI mocks.
use crate::{
    NodeId,
    agents::AgentId,
    llm::{AccountSummary, ModelEntry},
    net::{PairPurpose, PathKind},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// HTTP routes.
pub mod routes {
    pub const PREFIX: &str = "/api/v1/app";
    pub const STATUS: &str = "/api/v1/app/status";
    pub const DEVICES: &str = "/api/v1/app/devices";
    pub const PAIR_INVITE: &str = "/api/v1/app/pair/invite";
    pub const PAIR_ACCEPT: &str = "/api/v1/app/pair/accept";
    pub const PAIR_CONFIRM: &str = "/api/v1/app/pair/{flow}/confirm";
    pub const APPROVALS: &str = "/api/v1/app/approvals";
    pub const APPROVAL: &str = "/api/v1/app/approvals/{id}";
    /// Prefix proxied verbatim to the C3 mgmt API (`/accounts`, `/signin/*`, `/models`, `/presets`, …).
    pub const MODELS_PREFIX: &str = "/api/v1/app/models";
    pub const AGENTS: &str = "/api/v1/app/agents";
    pub const AGENT: &str = "/api/v1/app/agents/{id}";
    pub const CAPABILITIES: &str = "/api/v1/app/capabilities";
    /// `POST`, body optional; answers the updated [`super::CapabilityItem`] (or [`super::InstallPlan`]).
    pub const CAPABILITY_ACTION: &str = "/api/v1/app/capabilities/{id}/{action}";
    /// `GET` / `PUT` [`super::DeviceGrant`].
    pub const DEVICE_GRANT: &str = "/api/v1/app/devices/{node}/grant";
    /// SSE stream of [`super::AppEvent`].
    pub const EVENTS: &str = "/api/v1/app/events";
}

/// Tauri IPC command names (same request/response JSON as HTTP).
pub mod ipc {
    pub const STATUS: &str = "app_status";
    pub const DEVICES: &str = "app_devices";
    pub const PAIR_INVITE: &str = "app_pair_invite";
    pub const PAIR_ACCEPT: &str = "app_pair_accept";
    pub const PAIR_CONFIRM: &str = "app_pair_confirm";
    pub const APPROVALS: &str = "app_approvals";
    pub const APPROVAL_DECIDE: &str = "app_approval_decide";
    /// `{method, path, body}` forwarded to the mgmt API.
    pub const MODELS: &str = "app_models";
    pub const AGENTS: &str = "app_agents";
    pub const AGENT_UPDATE: &str = "app_agent_update";
    /// Subscribes to [`super::AppEvent`] (Tauri event channel `app://event`).
    pub const EVENTS: &str = "app_events";
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PillState {
    /// Hub online, models served through it.
    Connected,
    /// Hub offline; local fallback serving.
    Local,
    /// Something needs the user (approvals, sign-in).
    Attention,
    NoModels,
    Offline,
}

/// `GET /api/v1/app/status`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StatusPill {
    pub state: PillState,
    /// i18n key, e.g. `status.connected`.
    pub text_key: String,
    pub hub: Option<NodeId>,
    pub models_available: u32,
    pub approvals_pending: u32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Desktop,
    Laptop,
    Server,
    Phone,
    Unknown,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Badge {
    Hub,
    Claude,
    /// The device the UI runs on.
    This,
}

/// Capabilities this device granted to the listed device.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceCaps {
    pub exec: bool,
    pub terminals: bool,
    pub projects: bool,
    pub desktop: bool,
    pub computer: bool,
    pub browser: bool,
    pub messages: bool,
    pub activity: bool,
    pub spaces: bool,
    pub llm: bool,
    pub vault: bool,
}

/// One entry of `GET /api/v1/app/devices`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub node_id: NodeId,
    pub name: String,
    pub kind: DeviceKind,
    pub online: bool,
    pub path: PathKind,
    pub rtt_ms: Option<u32>,
    pub badges: Vec<Badge>,
    pub guest: bool,
    /// A member of this device's identity: shares model subscriptions and sees the
    /// same devices. A paired device that is not can be invited to join.
    #[serde(default)]
    pub same_identity: bool,
    pub caps: DeviceCaps,
    /// e.g. `macOS 26.6`, `Debian GNU/Linux 13 (trixie)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    /// Hardware model, e.g. `Mac16,8`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Unix seconds when the device last answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<u64>,
}

/// `POST /api/v1/app/pair/invite`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PairInviteReq {
    pub purpose: PairPurpose,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PairInvite {
    /// `cranibox://pair/…`
    pub link: String,
    pub web_link: Option<String>,
    pub qr_svg: String,
    pub expires_at: u64,
}
/// `POST /api/v1/app/pair/accept`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PairAcceptReq {
    pub link: String,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PairAccepted {
    pub flow_id: String,
    /// Pairing code shown on both devices, e.g. `1234-5678-9012`.
    pub sas: String,
}
/// `POST /api/v1/app/pair/{flow}/confirm`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PairConfirmReq {
    #[serde(rename = "match")]
    pub matches: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    Pairing,
    Message,
    Capability,
    AgentTool,
    ComputerUse,
    ModelWrite,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
    High,
}

/// Decision on a card. Pairing cards use `allow_once` = codes match,
/// `deny` = codes differ (decided by coordinator).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalAction {
    AllowOnce,
    AllowAlways,
    Deny,
}

/// Unified human-in-the-loop card. Expiry means `deny`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ApprovalCard {
    pub id: String,
    pub kind: ApprovalKind,
    pub from_node: Option<NodeId>,
    pub on_node: NodeId,
    pub agent: Option<AgentId>,
    /// i18n key; the frontend renders text from `title_key` + `title_args`.
    pub title_key: String,
    pub title_args: BTreeMap<String, String>,
    pub detail: Option<String>,
    pub risk: Risk,
    pub created_at: u64,
    pub expires_at: Option<u64>,
    pub actions: Vec<ApprovalAction>,
}

/// `POST /api/v1/app/approvals/{id}`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ApprovalDecision {
    pub decision: ApprovalAction,
}

/// One entry of `GET /api/v1/app/agents`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentInfo {
    pub id: AgentId,
    pub installed: bool,
    /// Cranibox owns this agent's model configuration.
    pub managed: bool,
    /// `<provider>/<model>`.
    pub model: Option<String>,
    pub recommended: bool,
}
/// `PUT /api/v1/app/agents/{id}`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentUpdate {
    pub managed: bool,
    pub model: Option<String>,
}

/// SSE event (`event:` = tag, `data:` = JSON of the variant payload).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum AppEvent {
    Access(crate::access::AccessView),
    Status(StatusPill),
    Approval(ApprovalCard),
    Device(Device),
    Account(AccountSummary),
}

/// Error body for every non-2xx App API answer.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{status} {code}: {message}")]
pub struct AppError {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
    #[serde(skip)]
    pub status: u16,
    /// Stable machine code (C8 codes, or `not_found`, `invalid`, `forbidden`).
    pub code: String,
    pub message: String,
}

/// App API surface (HTTP and Tauri IPC share it). WS7 implements the
/// aggregation layer; `MockAppApi` serves fixtures.
#[async_trait::async_trait]
pub trait AppApi: Send + Sync {
    async fn status(&self) -> Result<StatusPill, AppError>;
    async fn devices(&self) -> Result<Vec<Device>, AppError>;
    async fn pair_invite(&self, req: PairInviteReq) -> Result<PairInvite, AppError>;
    async fn pair_accept(&self, req: PairAcceptReq) -> Result<PairAccepted, AppError>;
    async fn pair_confirm(&self, flow: &str, req: PairConfirmReq) -> Result<(), AppError>;
    async fn approvals(&self) -> Result<Vec<ApprovalCard>, AppError>;
    async fn decide(&self, id: &str, req: ApprovalDecision) -> Result<(), AppError>;
    async fn agents(&self) -> Result<Vec<AgentInfo>, AppError>;
    async fn update_agent(&self, id: AgentId, req: AgentUpdate) -> Result<AgentInfo, AppError>;
    /// Models available to the agent model pickers (proxy of mgmt `GET /models`).
    async fn models(&self) -> Result<Vec<ModelEntry>, AppError>;
}

/// Platform of the device serving `GET /api/v1/app/capabilities`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Macos,
    Linux,
    Windows,
}

/// A local capability this device can share with paired devices.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityId {
    Terminal,
    Desktop,
    Computer,
    Browser,
}

/// Owner action on a capability: `POST /api/v1/app/capabilities/{id}/{action}`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityAction {
    /// Review an installation; answers [`InstallPlan`].
    InstallPlan,
    /// Run a reviewed plan once; body [`InstallReq`].
    Install,
    Start,
    Stop,
    /// Share the detected provider with paired devices.
    Enable,
    Disable,
    RequestPermissions,
    /// macOS Computer Use in the app: forget Cranibox's Screen Recording and
    /// Accessibility records, then ask again. A record kept from an install signed
    /// differently never matches, even with its switch on.
    ResetPermissions,
    OpenSettings,
    /// Browser only: share the owner's everyday Chrome (signed-in sessions) instead
    /// of the separate managed browser.
    UseProfile,
    Refresh,
}

/// The local program that provides a capability.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CapabilityProvider {
    /// `pty`, `screen_sharing`, `x11vnc`, `wayvnc`, `vnc`, `cua`, `http`, `chrome`, …
    pub kind: String,
    pub installed: bool,
    pub running: bool,
    pub version: Option<String>,
    pub detail: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionId {
    ScreenRecording,
    Accessibility,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Granted,
    Denied,
    Unknown,
    NotRequired,
}
/// Operating-system permission held by the provider (macOS TCC).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityPermission {
    pub id: PermissionId,
    pub state: PermissionState,
}

/// Whether paired devices may reach the provider (still subject to each grant).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CapabilitySharing {
    pub enabled: bool,
    /// Loopback provider address, when the provider has one.
    pub address: Option<String>,
    /// Browser: `profile` (the owner's Chrome), `managed` or `address`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CapabilityItem {
    pub id: CapabilityId,
    pub provider: CapabilityProvider,
    pub permissions: Vec<CapabilityPermission>,
    pub sharing: CapabilitySharing,
    /// Actions that apply in the current state.
    pub actions: Vec<CapabilityAction>,
    pub error: Option<String>,
}

/// `GET /api/v1/app/capabilities`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub platform: Platform,
    pub items: Vec<CapabilityItem>,
}

/// Answer of the `install_plan` action. The plan runs at most once, before `expires_at`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct InstallPlan {
    pub plan_id: String,
    /// Exact shell commands that `install` runs.
    pub commands: Vec<String>,
    /// Source of the installer, for review.
    pub url: Option<String>,
    /// Unix seconds.
    pub expires_at: u64,
}
/// Body of the `install` action.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct InstallReq {
    pub plan_id: String,
}

/// `GET`/`PUT /api/v1/app/devices/{node}/grant`: what that paired device may do here.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceGrant {
    pub permissions: DeviceCaps,
}
