//! C5 — configuration schema and file locations
//! (`docs/design/WORKPLAN.md` §1 C5, ARCHITECTURE §9).
//!
//! `<cfg>` is the directory holding `peers.json`. `<data>` defaults to `<cfg>`.
//! Every new file lives under `<data>`; `llm/` and `agents/` are subdirectories.
use serde::{Deserialize, Serialize};

pub const PEERS_FILE: &str = "peers.json";
pub const IDENTITY_FILE: &str = "node-identity.json";
/// Network settings file read and written by `workspace-node/src/network/config.rs`.
/// The [`IrohConfig`] section is added under the top-level key [`IROH_SECTION`].
pub const NETWORK_FILE: &str = "networks.json";
pub const IROH_SECTION: &str = "iroh";
pub const INVITATIONS_FILE: &str = "network-invitations.json";
pub const ROSTER_FILE: &str = "roster.json";
pub const LLM_DIR: &str = "llm";
pub const VAULT_DB: &str = "llm/vault.db";
pub const PRESETS_OVERRIDE_DIR: &str = "llm/presets.d";
pub const CLAUDE_DIR: &str = "llm/claude";
pub const AGENTS_DIR: &str = "agents";
pub const APPLIED_FILE: &str = "agents/applied.json";
/// Suffix of byte-for-byte backups taken before the first agent config write.
pub const BACKUP_SUFFIX: &str = ".before-cranibox";
pub const APPROVALS_DB: &str = "approvals.db";

/// `cranibox serve` (Hub) flags added by WS1.
pub const SERVE_FLAG_RELAY: &str = "--relay";
pub const SERVE_FLAG_NO_RELAY: &str = "--no-relay";
/// `--public-url https://hub.example.com`: used for tickets and the relay self-check.
pub const SERVE_FLAG_PUBLIC_URL: &str = "--public-url";
/// Path of the embedded relay on the control-center HTTPS port.
pub const RELAY_PATH: &str = "/relay";

/// `"iroh": { "enabled": true, "relays": [...], "n0_fallback": true, "bind_port": 0 }`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IrohConfig {
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Own relays first (e.g. `https://hub.example.com/relay`), in order.
    #[serde(default)]
    pub relays: Vec<String>,
    /// Fall back to the public n0 relays (D4: on by default).
    #[serde(default = "yes")]
    pub n0_fallback: bool,
    /// UDP port; 0 = ephemeral.
    #[serde(default)]
    pub bind_port: u16,
}
fn yes() -> bool {
    true
}
impl Default for IrohConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            relays: Vec::new(),
            n0_fallback: true,
            bind_port: 0,
        }
    }
}
