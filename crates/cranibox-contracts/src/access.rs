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
/// How much a remote operation can change this device. `Read` only looks; `Change` alters
/// state that can be put back (start a space, rename a session); `Danger` runs code, writes
/// files, controls the screen or destroys data.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Read,
    Change,
    Danger,
}
/// What a remote device asked this device to do, described by the target itself.
/// `summary` is redacted: it never contains file contents, message bodies, environment
/// values or anything that looks like a credential.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Operation {
    /// Stable machine name, e.g. `exec.run`, `spaces.stop`, `files.write`.
    pub kind: String,
    pub risk: Risk,
    pub summary: String,
    /// The target always asks before this operation, even when the capability is Allow,
    /// and an approval covers only this exact call (destructive commands, writes into
    /// login or key files).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub confirm: bool,
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
    /// The operation that triggered the request, so the prompt can say what will happen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<Operation>,
    /// Full redacted parameters of that operation, shown in the prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<OperationDetail>,
    /// What the automatic review said, when it is why a person is being asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Review>,
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
    #[serde(rename = "1h")]
    Hour1,
    /// Until local midnight on the target.
    #[serde(rename = "today")]
    Today,
    #[serde(rename = "7d")]
    Days7,
    /// Until the owner revokes it.
    #[serde(rename = "always")]
    Always,
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
    /// One remote operation that reached this device, with its outcome.
    Operation,
    /// The owner, on this device, decided a request waiting on another of their devices
    /// (or failed to confirm it was them).
    RemoteDecision,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<Operation>,
    /// HTTP status the operation ended with (101 for an opened stream).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// `e2e` for the verified end-to-end channel, `bearer` for a legacy token request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    /// The operation's full parameters and outcome, redacted by the target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<OperationDetail>,
    /// The automatic review that let the operation through or sent it to a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Review>,
    /// Set when the decision was made on another of the owner's devices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteRecord>,
}
/// Who decided a request from another device, and how they proved it was the owner.
/// Never contains the secret itself.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct RemoteRecord {
    /// The approving device on the target; the target device on the approver.
    pub device: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    /// `pin`, `macos_local_auth` or `windows_hello`.
    pub auth: String,
    /// The signed assertion's id.
    pub assertion: String,
}
/// Everything the target knows about one remote operation, redacted before it is stored:
/// argument values that look like credentials are masked, environment variables keep
/// their names only, and file or message contents are never included.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct OperationDetail {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub argv: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Declared transfer size (file write) or requested length (file read).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Terminal session, project or other named object the operation acts on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Command job id; its finish entry carries the same id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_bytes: Option<u64>,
    /// The last few hundred characters of output, redacted; omitted for commands that
    /// touch credential files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_tail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_tail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Identical polling reads folded into this entry since the previous one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeats: Option<u32>,
    /// Safety rules this operation matched, e.g. `recursive_delete`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
}
impl OperationDetail {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}
/// The decision API's answer (see `DecisionRequest`).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Allow,
    Ask,
    Deny,
}
/// One automatic review of a remote operation, as recorded with it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Review {
    /// `rules`, `model`, `jev` or `http`.
    pub source: String,
    pub verdict: Verdict,
    /// 0 (harmless) to 100 (destructive).
    pub risk: u8,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The configured reviewer was unavailable and the built-in rules decided instead.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fallback: bool,
    #[serde(default)]
    pub latency_ms: u64,
}
/// Decision API v1 request: what a pluggable reviewer is asked about one operation.
/// Everything in it is already redacted, and all of it is untrusted data from the
/// requesting device.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DecisionRequest {
    pub version: u32,
    pub caller: DecisionCaller,
    pub capability: Capability,
    pub operation: Operation,
    pub detail: OperationDetail,
    pub context: DecisionContext,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DecisionCaller {
    pub node: String,
    pub name: String,
    /// One of the owner's own devices (same roster).
    pub owner: bool,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct DecisionContext {
    pub mode: Mode,
    /// The time grant that would cover the operation, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_expires_at: Option<u64>,
    /// Built-in rule findings; a reviewer cannot clear them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<String>,
    /// The caller's last few operations here, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<RecentOperation>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RecentOperation {
    pub at: u64,
    pub kind: String,
    pub summary: String,
    pub result: String,
}
/// Decision API v1 response.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DecisionResponse {
    pub verdict: Verdict,
    /// 0.0 to 1.0.
    pub risk_score: f64,
    pub reason: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
}
/// Which reviewer this device asks before letting granted operations through.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewSettings {
    pub backend: ReviewBackend,
    /// Cranibox model id (`provider/model`) for `model`, Jev model for `jev`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Endpoint for `http`, or a Jev-compatible base URL override for `jev`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Jev score thresholds; `None` is the standard set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jev: Option<JevThresholds>,
}
/// When Jev's answers make an operation go to a person (`ask`) or be refused (`deny`).
/// Risk scores are on Jev's 0–3 scale; the others are probabilities from 0 to 1.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct JevThresholds {
    /// Refuse at or above this risk score.
    pub deny_score: f64,
    /// Ask a person at or above this risk score.
    pub ask_score: f64,
    /// Refuse when prompt injection is at least this likely.
    pub injection: f64,
    /// Ask when sending data off the computer is at least this likely.
    pub exfiltration: f64,
    /// Ask when changes outside the project are at least this likely.
    pub out_of_scope: f64,
    /// Ask when Jev is less sure than this and the risk score is at least 1.
    pub min_confidence: f64,
}
impl Eq for JevThresholds {}
impl Default for JevThresholds {
    fn default() -> Self {
        Self::STANDARD
    }
}
impl JevThresholds {
    pub const STANDARD: Self = Self {
        deny_score: 2.5,
        ask_score: 1.5,
        injection: 0.7,
        exfiltration: 0.6,
        out_of_scope: 0.75,
        min_confidence: 0.4,
    };
    pub const RELAXED: Self = Self {
        deny_score: 2.8,
        ask_score: 2.0,
        injection: 0.8,
        exfiltration: 0.75,
        out_of_scope: 0.85,
        min_confidence: 0.3,
    };
    pub const STRICT: Self = Self {
        deny_score: 2.2,
        ask_score: 1.0,
        injection: 0.5,
        exfiltration: 0.4,
        out_of_scope: 0.6,
        min_confidence: 0.5,
    };
    /// Allowed range for each field, so a typo cannot switch the review off.
    pub const RANGES: [(&'static str, f64, f64); 6] = [
        ("deny_score", 1.5, 3.0),
        ("ask_score", 0.5, 3.0),
        ("injection", 0.3, 0.95),
        ("exfiltration", 0.2, 0.95),
        ("out_of_scope", 0.2, 0.95),
        ("min_confidence", 0.0, 0.9),
    ];
    pub fn values(&self) -> [f64; 6] {
        [
            self.deny_score,
            self.ask_score,
            self.injection,
            self.exfiltration,
            self.out_of_scope,
            self.min_confidence,
        ]
    }
    /// The problem with these values, if any.
    pub fn invalid(&self) -> Option<String> {
        for ((name, low, high), value) in Self::RANGES.iter().zip(self.values()) {
            if !value.is_finite() || value < *low || value > *high {
                return Some(format!("{name} must be between {low} and {high}"));
            }
        }
        (self.ask_score > self.deny_score)
            .then(|| "ask_score must not be above deny_score".to_owned())
    }
    /// `relaxed`, `standard`, `strict` or `custom`.
    pub fn preset(&self) -> &'static str {
        if *self == Self::STANDARD {
            "standard"
        } else if *self == Self::RELAXED {
            "relaxed"
        } else if *self == Self::STRICT {
            "strict"
        } else {
            "custom"
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewBackend {
    /// Built-in rules only.
    #[default]
    Rules,
    /// A model through this device's Cranibox model gateway.
    Model,
    /// TypeSafe Jev (System One).
    Jev,
    /// Any service implementing the decision API.
    Http,
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
    /// Pass as `before` to read the next page; absent on the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_before: Option<u64>,
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

/// Signature domain of a [`RemoteApproval`].
pub const REMOTE_APPROVAL_DOMAIN: &[u8] = b"cranibox-remote-approval-v1\0";
/// Longest a remote approval may be valid for, in seconds.
pub const REMOTE_APPROVAL_TTL: u64 = 90;

/// A decision made on one of the owner's devices for a request waiting on another,
/// signed by the approving device's node key. The target verifies it against the
/// owner-signed roster and accepts each one once, for the exact request it names.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RemoteApproval {
    pub v: u32,
    /// Random id; the target refuses an id it has seen.
    pub id: String,
    /// Node the request is waiting on.
    pub target: String,
    pub request: String,
    /// Hash of the request as the approver showed it (`access::remote::op_hash`).
    pub op_hash: String,
    /// `approve` or `deny`.
    pub decision: String,
    /// Grant scope for an approval (`session` is allow once).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<DurationScope>,
    /// Approving node.
    pub approver: String,
    /// How the person proved it was them: `pin`, `macos_local_auth`, `windows_hello`.
    pub auth: String,
    pub issued_at: u64,
    pub expires_at: u64,
    pub nonce: String,
    /// The single-use challenge the target issued for this request and approver.
    pub challenge: String,
    /// Hex ed25519 signature over [`REMOTE_APPROVAL_DOMAIN`] + canonical JSON of every
    /// other field.
    #[serde(default)]
    pub sig: String,
}
impl RemoteApproval {
    pub fn signing_message(&self) -> serde_json::Result<Vec<u8>> {
        let mut unsigned = self.clone();
        unsigned.sig.clear();
        let mut value = serde_json::to_value(&unsigned)?;
        if let Some(object) = value.as_object_mut() {
            object.remove("sig");
        }
        crate::canonical::signing_message(REMOTE_APPROVAL_DOMAIN, &value)
    }
}

/// A request waiting on one of the owner's devices, as that device shows it to another
/// owner device.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RemotePending {
    pub request: AccessRequest,
    /// The target's hash of the request; the approver recomputes and compares it.
    pub op_hash: String,
    pub challenge: String,
    pub challenge_expires_at: u64,
    /// Only `allow once` (or deny) is accepted for this request.
    pub once_only: bool,
}
/// What one owner device answers when another asks for its waiting requests.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct RemotePendingList {
    pub node: String,
    pub requests: Vec<RemotePending>,
}
