//! C3 — model proxy (`docs/design/WORKPLAN.md` §1 C3, ARCHITECTURE §7).
//!
//! WS2 implements the proxy core, the API-key upstream and the mgmt API;
//! WS3 implements subscription sign-in and Chinese providers; WS4 implements
//! the Claude-node upstream. Agents (WS5) only ever talk to [`LLM_ADDR`].
use crate::{NodeId, error::ErrorCode, net::BoxBody};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};

/// Loopback-only listen address (IPv4). Also bound on [`LLM_ADDR_V6`].
pub const LLM_ADDR: &str = "127.0.0.1:3424";
pub const LLM_ADDR_V6: &str = "[::1]:3424";
pub const LLM_PORT: u16 = 3424;
/// Base URL agents are wired to.
pub const LLM_BASE_URL: &str = "http://127.0.0.1:3424";
/// Mgmt API Unix socket (named pipe on Windows), under the node's run directory.
pub const LLM_MGMT_SOCKET: &str = "cranibox-llm.sock";
/// Local per-agent key prefix (`cbx-<agent id>`). Identifies the agent; not a security boundary.
pub const LOCAL_KEY_PREFIX: &str = "cbx-";
/// Provider id of Claude subscriptions; models are `claude/<model>`.
pub const CLAUDE_PROVIDER: &str = "claude";

/// Wire protocol of a request or upstream.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Wire {
    /// OpenAI Chat Completions.
    Chat,
    /// OpenAI Responses.
    Responses,
    /// Anthropic Messages (also the conversion pivot).
    Messages,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Availability {
    Local,
    ViaHub,
    ViaNode { node: NodeId },
    ClaudeNodeOnly,
    Unavailable { code: ErrorCode },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ModelEntry {
    /// `"<provider>/<model>"`.
    pub id: String,
    pub display: String,
    pub wires: Vec<Wire>,
    pub native: Wire,
    pub context: Option<u32>,
    pub reasoning: bool,
    pub vision: bool,
    pub availability: Availability,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    ApiKey,
    #[serde(rename = "oauth")]
    OAuth,
    /// Claude subscription: lives on the Claude node only, never synced.
    #[serde(rename = "claude_oauth")]
    ClaudeOAuth,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AccountState {
    Ready,
    NeedsSignIn,
    QuotaExhausted { reset_at: Option<u64> },
    Error { message: String },
}

/// Account as shown to the UI and mgmt clients. Never carries secrets.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AccountSummary {
    pub id: String,
    /// Provider or preset id (`"openai"`, `"codex"`, `"zhipu-coding"`, `"custom"` …).
    pub provider: String,
    pub plan: Option<String>,
    pub label: String,
    pub kind: AccountKind,
    /// Node that added the account.
    pub home_node: NodeId,
    /// Data replicated to roster nodes (always `false` for `ClaudeOAuth`).
    pub sync: bool,
    pub enabled_on: Vec<NodeId>,
    /// Enabled on every current and future roster node.
    pub all_nodes: bool,
    pub state: AccountState,
}

/// Owned secret bytes. Not `Serialize`, redacted `Debug`, wiped on drop.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretBox(Vec<u8>);
impl SecretBox {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}
impl fmt::Debug for SecretBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretBox(<{} bytes>)", self.0.len())
    }
}
impl Drop for SecretBox {
    fn drop(&mut self) {
        for byte in self.0.iter_mut() {
            // SAFETY: `byte` is a valid, exclusively borrowed u8.
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
    }
}

/// Request in the proxy's canonical form (body is the wire-native JSON).
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalRequest {
    pub wire: Wire,
    pub body: serde_json::Value,
    pub stream: bool,
    pub session_id: Option<String>,
    /// Agent id derived from the local key (`cbx-<agent>`), if known.
    pub client_agent: Option<String>,
    /// Upstream model name, without the `<provider>/` prefix.
    pub model: String,
}

/// Context handed to an upstream for one call (decided by coordinator).
#[derive(Debug)]
pub struct UpstreamCx {
    pub account: AccountSummary,
    pub secret: SecretBox,
    /// Preset row this account was created from, if any.
    pub preset: Option<Preset>,
    /// Node executing the call.
    pub node: NodeId,
}

pub struct UpstreamResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// Wire of `body`; the proxy converts to the client's wire if needed.
    pub wire: Wire,
    pub body: BoxBody,
}

pub struct Refreshed {
    pub secret: SecretBox,
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpstreamError {
    #[error("upstream rejected credentials")]
    Auth,
    #[error("rate limited")]
    RateLimited { retry_after: Option<u64> },
    #[error("quota exhausted")]
    QuotaExhausted { reset_at: Option<u64> },
    #[error("network: {0}")]
    Network(String),
    #[error("upstream status {0}: {1}")]
    Status(u16, String),
    #[error("unsupported")]
    Unsupported,
}

/// Upstream adapter. WS2 defines the registry and implements `apikey`;
/// WS3 codex/kimi/gemini/copilot/cn; WS4 claude.
#[async_trait::async_trait]
pub trait Upstream: Send + Sync {
    fn provider(&self) -> &'static str;
    fn wires(&self) -> &[Wire];
    async fn send(
        &self,
        cx: &UpstreamCx,
        req: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError>;
    async fn refresh(
        &self,
        _cx: &UpstreamCx,
        _secret: &SecretBox,
    ) -> Result<Refreshed, UpstreamError> {
        Err(UpstreamError::Unsupported)
    }
    async fn models(&self, cx: &UpstreamCx) -> Result<Vec<ModelEntry>, UpstreamError>;
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct SignInOpts {
    /// `"cn"` or `"global"` for providers with regional endpoints.
    pub region: Option<String>,
    pub label: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignInStart {
    /// Open `url`; a loopback callback completes the flow.
    Browser { flow_id: String, url: String },
    /// Show `code`, user enters it at `url`; poll `complete` until done.
    DeviceCode {
        flow_id: String,
        code: String,
        url: String,
        expires_at: u64,
    },
    /// Open `url`, then paste the final callback URL into `complete`.
    PasteCallback { flow_id: String, url: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct SignInInput {
    pub callback_url: Option<String>,
    pub code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MgmtError {
    #[error("not found")]
    NotFound,
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("pending: sign-in not finished")]
    Pending,
    #[error("unavailable: {0:?}")]
    Unavailable(ErrorCode),
    #[error(transparent)]
    Upstream(#[from] UpstreamError),
    #[error("{0}")]
    Other(String),
}

/// Sign-in flows: implemented by WS3/WS4, called by WS2's mgmt.
#[async_trait::async_trait]
pub trait SignIn: Send + Sync {
    async fn start(&self, provider: &str, opts: SignInOpts) -> Result<SignInStart, MgmtError>;
    async fn complete(
        &self,
        flow_id: &str,
        input: SignInInput,
    ) -> Result<AccountSummary, MgmtError>;
}

/// `GET <mesh>/nodes/{node}/llm/health`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LlmHealth {
    pub version: String,
    pub hub: bool,
    pub accounts: Vec<HealthAccount>,
    pub models_etag: String,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct HealthAccount {
    pub id: String,
    pub provider: String,
    pub enabled_here: bool,
}

/// One row of `presets.toml` (`[[preset]]`). Built in, overridable from `<data>/llm/presets.d/`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Preset {
    pub id: String,
    pub label: String,
    /// `"cn"`, `"global"`.
    #[serde(default)]
    pub region: Vec<String>,
    pub chat: Option<String>,
    pub responses: Option<String>,
    pub anthropic: Option<String>,
    pub default_wire: Wire,
    #[serde(default)]
    pub models: Vec<String>,
    /// Upstream has no usable `/models`; use `models`.
    #[serde(default)]
    pub no_list: bool,
    /// Named provider quirks handled in code (e.g. `zhipu_envelope`).
    #[serde(default)]
    pub quirks: Vec<String>,
    pub key_hint: Option<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PresetFile {
    pub preset: Vec<Preset>,
}

/// Mgmt API (HTTP/1.1 + JSON over [`LLM_MGMT_SOCKET`]).
pub mod mgmt {
    use super::*;

    pub const STATUS: &str = "/status";
    pub const ACCOUNTS: &str = "/accounts";
    pub const ACCOUNT: &str = "/accounts/{id}";
    pub const ACCOUNT_ENABLE: &str = "/accounts/{id}/enable";
    pub const SIGNIN_START: &str = "/signin/start";
    pub const SIGNIN_COMPLETE: &str = "/signin/{flow}/complete";
    pub const MODELS: &str = "/models";
    pub const CLAUDE_SET_NODE: &str = "/claude/set-node";
    pub const PRESETS: &str = "/presets";
    /// SSE stream of [`MgmtEvent`].
    pub const EVENTS: &str = "/events";

    #[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum Mode {
        /// Requests go through the online Hub.
        Hub,
        /// Hub offline; serving with locally enabled accounts.
        Local,
        /// Nothing can serve.
        Offline,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
    pub struct Issue {
        pub code: ErrorCode,
        pub message: String,
        pub account_id: Option<String>,
    }

    /// `GET /status`.
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
    pub struct Status {
        pub mode: Mode,
        pub hub_online: bool,
        pub models_available: u32,
        pub issues: Vec<Issue>,
    }

    /// `POST /accounts`: API key for a preset or a custom provider.
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum NewAccount {
        ApiKey {
            preset: String,
            key: String,
            label: Option<String>,
            region: Option<String>,
        },
        Custom {
            label: String,
            key: String,
            /// Base URLs per wire; at least one.
            endpoints: BTreeMap<Wire, String>,
            models: Vec<String>,
        },
    }

    /// `POST /accounts/{id}/enable`. Exactly one of `node` / `all_nodes`.
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
    pub struct Enable {
        pub node: Option<NodeId>,
        pub all_nodes: Option<bool>,
        pub on: bool,
    }

    /// `POST /signin/start`.
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
    pub struct SignInStartReq {
        pub provider: String,
        pub region: Option<String>,
    }

    /// `POST /claude/set-node`.
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
    pub struct SetClaudeNode {
        pub node: NodeId,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
    #[serde(tag = "event", rename_all = "snake_case")]
    pub enum MgmtEvent {
        AccountChanged { account_id: String },
        ModeChanged { mode: Mode },
        ModelsChanged { etag: String },
    }

    /// Typed client view of the mgmt API. WS2 provides the socket client;
    /// [`crate::mock`] provides `MockLlmMgmt`; WS7's App API proxies it.
    #[async_trait::async_trait]
    pub trait LlmMgmt: Send + Sync {
        async fn status(&self) -> Result<Status, MgmtError>;
        async fn accounts(&self) -> Result<Vec<AccountSummary>, MgmtError>;
        async fn add_account(&self, account: NewAccount) -> Result<AccountSummary, MgmtError>;
        async fn delete_account(&self, id: &str) -> Result<(), MgmtError>;
        async fn enable(&self, id: &str, req: Enable) -> Result<AccountSummary, MgmtError>;
        async fn signin_start(&self, req: SignInStartReq) -> Result<SignInStart, MgmtError>;
        async fn signin_complete(
            &self,
            flow: &str,
            input: SignInInput,
        ) -> Result<AccountSummary, MgmtError>;
        async fn models(&self) -> Result<Vec<ModelEntry>, MgmtError>;
        async fn set_claude_node(&self, req: SetClaudeNode) -> Result<(), MgmtError>;
        async fn presets(&self) -> Result<Vec<Preset>, MgmtError>;
    }
}
