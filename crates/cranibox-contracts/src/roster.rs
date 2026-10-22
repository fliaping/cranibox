//! C1 — roster and identity (`docs/design/WORKPLAN.md` §1 C1, ARCHITECTURE §4.2, §5.3).
//!
//! Implemented by WS6 (`workspace-runtime/src/peers/roster.rs`). Used by WS1
//! (relay allowlist), WS2 (llm access control) and WS7 (devices page).
use crate::NodeId;
use serde::{Deserialize, Serialize};

/// Current roster format.
pub const ROSTER_VERSION: u32 = 1;
/// Signature domain: `sig = ed25519(owner, ROSTER_SIG_DOMAIN ‖ canonical_json(roster without sig))`.
pub const ROSTER_SIG_DOMAIN: &[u8] = b"cranibox-roster-v1\0";
/// `NodeId = "node-" + hex(SHA256(NODE_ID_DOMAIN ‖ signing_pub)[..16])`.
pub const NODE_ID_DOMAIN: &[u8] = b"cranibox-node-id-v1\0";
/// Prefix of every NodeId.
pub const NODE_ID_PREFIX: &str = "node-";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The single always-on node (`cranibox serve`).
    Hub,
    Node,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RosterMember {
    pub node_id: NodeId,
    /// ed25519 public key, hex. Equals the iroh EndpointId.
    pub signing_key: String,
    /// X25519 public key (Noise static key and HPKE recipient), hex.
    pub x25519_pub: String,
    pub role: Role,
    /// Lower value = preferred for lease takeover and routing.
    pub priority: u8,
    pub is_claude_node: bool,
    pub name: String,
    pub added_at: u64,
    /// The member's signed public device identity (`PublicIdentity` JSON), so every
    /// member can pin every other one without pairing them separately.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Roster {
    /// Always [`ROSTER_VERSION`].
    pub version: u32,
    /// Owner ed25519 public key, hex.
    pub owner_pub: String,
    /// Monotonic; every membership change increments it.
    pub epoch: u64,
    pub members: Vec<RosterMember>,
    /// ed25519 signature by the owner key, hex. Empty while unsigned.
    pub sig: String,
}

impl Roster {
    /// The exact bytes the owner signs (`sig` is cleared first).
    pub fn signing_message(&self) -> Vec<u8> {
        let mut unsigned = self.clone();
        unsigned.sig.clear();
        crate::canonical::signing_message(ROSTER_SIG_DOMAIN, &unsigned).expect("roster serializes")
    }
    pub fn member(&self, node: &str) -> Option<&RosterMember> {
        self.members.iter().find(|m| m.node_id == node)
    }
}

/// Read-only view of the verified roster. Implemented by WS6.
pub trait RosterView: Send + Sync + 'static {
    /// The latest verified roster, if this node has one.
    fn current(&self) -> Option<Roster>;
    /// Whether `node` is a member of the owner's roster (owner device, not a guest).
    fn is_owner_node(&self, node: &str) -> bool;
    fn hub(&self) -> Option<RosterMember>;
    fn claude_node(&self) -> Option<RosterMember>;
    /// Notifies with the new epoch whenever the roster changes.
    fn subscribe(&self) -> tokio::sync::watch::Receiver<u64>;
}
