//! Target-owned device access policy. Times are Unix seconds.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Messages,
    Terminals,
    Exec,
    Projects,
    Desktop,
    Computer,
    Browser,
    Update,
    Spaces,
    Activity,
    Llm,
    Vault,
}
impl Capability {
    pub const ALL: [Self; 12] = [
        Self::Messages,
        Self::Terminals,
        Self::Exec,
        Self::Projects,
        Self::Desktop,
        Self::Computer,
        Self::Browser,
        Self::Update,
        Self::Spaces,
        Self::Activity,
        Self::Llm,
        Self::Vault,
    ];
    pub fn requires_approval(self) -> bool {
        matches!(self, Self::Update | Self::Llm | Self::Vault)
    }
}
impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = serde_json::to_value(self).map_err(|_| std::fmt::Error)?;
        f.write_str(name.as_str().ok_or(std::fmt::Error)?)
    }
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Off,
    Ask,
    Allow,
}
pub fn defaults() -> BTreeMap<Capability, Mode> {
    Capability::ALL
        .into_iter()
        .map(|c| {
            (
                c,
                if c == Capability::Messages {
                    Mode::Allow
                } else {
                    Mode::Off
                },
            )
        })
        .collect()
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AccessPolicy {
    pub peers: BTreeMap<String, BTreeMap<Capability, Mode>>,
}
impl AccessPolicy {
    pub fn mode(&self, node: &str, capability: Capability) -> Mode {
        self.peers
            .get(node)
            .and_then(|p| p.get(&capability))
            .copied()
            .unwrap_or_else(|| defaults()[&capability])
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequestState {
    Pending,
    Approved,
    Denied,
    Expired,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AccessRequest {
    pub id: String,
    pub from: String,
    pub name: String,
    pub capability: Capability,
    pub session_key: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub state: RequestState,
    pub path: Option<serde_json::Value>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalScope {
    Session,
    Until,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum DurationScope {
    #[serde(rename = "session")]
    Session,
    #[serde(rename = "30m")]
    Minutes30,
    #[serde(rename = "8h")]
    Hours8,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Deny,
    Approve,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DecideRequest {
    pub decision: ApprovalDecision,
    pub scope: Option<DurationScope>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Approval {
    pub id: String,
    pub from: String,
    pub capability: Capability,
    pub scope: ApprovalScope,
    pub session_key: Option<String>,
    pub expires_at: Option<u64>,
    pub approved_by: String,
    pub approved_at: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ActiveSession {
    pub id: String,
    pub from: String,
    pub name: String,
    pub capability: Capability,
    pub session_key: String,
    pub started_at: u64,
    pub basis: String,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    SessionStarted,
    SessionEnded,
    RequestCreated,
    Approved,
    Denied,
    Expired,
    PolicyChanged,
    Refused,
    EmergencyDisconnect,
    Checkpoint,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AuditEvent {
    pub index: u64,
    pub at: u64,
    pub event: EventKind,
    pub from: Option<String>,
    pub name: Option<String>,
    pub capability: Option<Capability>,
    pub session_key: Option<String>,
    pub basis: Option<String>,
    pub result: String,
    pub prev: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PeerPolicy {
    pub node: String,
    pub name: String,
    pub capabilities: BTreeMap<Capability, Mode>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct AccessView {
    pub peers: Vec<PeerPolicy>,
    pub pending: Vec<AccessRequest>,
    pub approvals: Vec<Approval>,
    pub sessions: Vec<ActiveSession>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AuditView {
    pub entries: Vec<AuditEvent>,
    pub verified: bool,
    pub broken_index: Option<u64>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum Denied {
    CapabilityOff {
        capability: Capability,
        target_name: String,
        hint: String,
    },
    ApprovalRequired {
        request_id: String,
        capability: Capability,
        expires_at: u64,
    },
    ApprovalCooldown {
        capability: Capability,
        expires_at: u64,
    },
}
