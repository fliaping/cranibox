//! C4 — agent adapters (`docs/design/WORKPLAN.md` §1 C4, ARCHITECTURE §6).
//!
//! Implemented by WS5 in `cranibox-agents`; `cranibox_agents::registry()`
//! returns adapters in [`AgentId::ALL`] order. The default agent is
//! [`DEFAULT_AGENT`] (`deepseek-harness`).
use crate::llm::{LLM_BASE_URL, LOCAL_KEY_PREFIX, ModelEntry};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum AgentId {
    DeepseekHarness,
    Codex,
    ClaudeCode,
    Pi,
    Hermes,
    Openclaw,
}
impl AgentId {
    /// Fixed registry order.
    pub const ALL: [AgentId; 6] = [
        Self::DeepseekHarness,
        Self::Codex,
        Self::ClaudeCode,
        Self::Pi,
        Self::Hermes,
        Self::Openclaw,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DeepseekHarness => "deepseek-harness",
            Self::Codex => "codex",
            Self::ClaudeCode => "claude-code",
            Self::Pi => "pi",
            Self::Hermes => "hermes",
            Self::Openclaw => "openclaw",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.as_str() == value)
    }
}
pub const DEFAULT_AGENT: AgentId = AgentId::DeepseekHarness;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentDisplay {
    pub name: String,
    /// Icon id in `design-system`.
    pub icon: String,
    pub recommended: bool,
}

/// Where the adapter looks for things. Built by the caller; tests use temp dirs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Env {
    pub home: PathBuf,
    /// Cranibox data directory (`<data>`, see C5).
    pub data: PathBuf,
    /// Directories searched for executables, in order.
    pub path: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Detection {
    pub installed: bool,
    pub version: Option<String>,
    pub binary: Option<PathBuf>,
    pub config_dir: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallMethod {
    Npm,
    Pip,
    Binary,
    /// Cannot be installed automatically; show instructions.
    Manual,
}

/// Pinned, verifiable install (no `@latest`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct InstallPlan {
    pub method: InstallMethod,
    pub package: String,
    pub version: String,
    /// `sha256:<hex>` or an npm/SRI integrity string.
    pub integrity: Option<String>,
}

/// The local proxy endpoint an agent is wired to.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LocalEndpoint {
    /// Always [`LLM_BASE_URL`].
    pub base: String,
    /// `cbx-<agent id>`.
    pub key: String,
}
impl LocalEndpoint {
    pub fn for_agent(agent: AgentId) -> Self {
        Self {
            base: LLM_BASE_URL.into(),
            key: format!("{LOCAL_KEY_PREFIX}{}", agent.as_str()),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ModelPick {
    /// `<provider>/<model>`.
    pub default: String,
    pub small_fast: Option<String>,
    pub models: Vec<ModelEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AppliedFile {
    pub path: PathBuf,
    /// Dotted key paths we wrote (e.g. `model_providers.cranibox`).
    pub keys: Vec<String>,
    /// `sha256:<hex>` of the file before our first write; `None` if it did not exist.
    pub before_hash: Option<String>,
    pub after_hash: String,
}

/// Record of a reversible write; persisted in `<data>/agents/applied.json`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub files: Vec<AppliedFile>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestoreMode {
    /// File unchanged since our write: restored byte for byte.
    Original,
    /// File edited by the user: only our keys were removed.
    KeysRemoved,
    /// File no longer exists; nothing to do.
    Missing,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RestoredFile {
    pub path: PathBuf,
    pub mode: RestoreMode,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Restored {
    pub files: Vec<RestoredFile>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DriftState {
    Clean,
    /// Our keys were changed or removed by someone else.
    Modified,
    Missing,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DriftFile {
    pub path: PathBuf,
    pub state: DriftState,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Drift {
    pub files: Vec<DriftFile>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct HeadlessTask {
    pub prompt: String,
    pub cwd: PathBuf,
    pub session_id: Option<String>,
    pub json: bool,
}

/// A process to spawn (no shell interpolation).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub stdin: Option<String>,
}

/// Session discovery roots (replaces `Roots` in `peers/discovery.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roots {
    pub home: PathBuf,
    pub extra: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionRef {
    pub agent: AgentId,
    pub id: String,
    pub cwd: Option<PathBuf>,
    pub title: Option<String>,
    pub updated_at: Option<u64>,
    pub path: Option<PathBuf>,
}

/// Cranibox MCP server registration (computer, browser, devices, approvals).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct McpSpec {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AgentCaps {
    pub web_ui: bool,
    pub acp: bool,
    pub rpc: bool,
    pub hooks: bool,
    pub approval: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AgentError {
    #[error("agent is not installed")]
    NotInstalled,
    #[error("unsupported by this agent")]
    Unsupported,
    #[error("config: {0}")]
    Config(String),
    #[error("io: {0}")]
    Io(String),
}
pub type Result<T, E = AgentError> = std::result::Result<T, E>;

/// One adapter per agent. Writes must be reversible (see ARCHITECTURE §6.2).
pub trait AgentAdapter: Send + Sync {
    fn id(&self) -> AgentId;
    fn display(&self) -> AgentDisplay;
    fn detect(&self, env: &Env) -> Detection;
    fn install_plan(&self, env: &Env) -> InstallPlan;
    // `env` is passed explicitly (decided by coordinator) so registry
    // adapters can be `&'static` and tests can point at temp dirs.
    fn wire(&self, env: &Env, ep: &LocalEndpoint, pick: &ModelPick) -> Result<Applied>;
    fn unwire(&self, env: &Env) -> Result<Restored>;
    /// Check only; never rewrites on startup.
    fn drift(&self, env: &Env) -> Result<Drift>;
    fn headless(&self, task: &HeadlessTask) -> Result<Launch>;
    fn sessions(&self, roots: &Roots) -> Vec<SessionRef>;
    fn register_mcp(&self, env: &Env, mcp: &McpSpec) -> Result<Applied>;
    fn caps(&self) -> AgentCaps;
}
