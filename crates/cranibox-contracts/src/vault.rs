//! C7 — vault records and sync messages (`docs/design/WORKPLAN.md` §1 C7,
//! `docs/design/tasks/06-multi-node-vault.md` data model).
//!
//! Implemented by WS6. WS2 uses accounts only through [`AccountStore`]
//! (Phase 0 ships `crate::mock::MemoryAccountStore`).
use crate::{
    NodeId,
    llm::{AccountKind, AccountSummary, SecretBox},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Signature domain: `sig = ed25519(writer, VAULT_SIG_DOMAIN ‖ canonical_json(record without sig))`.
pub const VAULT_SIG_DOMAIN: &[u8] = b"cranibox-vault-v1\0";
/// HPKE suite for DEK wraps (RFC 9180 ids: KEM 0x0020, KDF 0x0001, AEAD 0x0003).
pub const HPKE_SUITE: &str = "DHKEM(X25519, HKDF-SHA256) + HKDF-SHA256 + ChaCha20Poly1305";
/// Record AEAD (per-record DEK).
pub const RECORD_AEAD: &str = "XChaCha20-Poly1305";
/// Sync protocol id carried in health and pull responses.
pub const VAULT_PROTOCOL: &str = "vault/1";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncPolicy {
    Synced,
    /// Fixed for `ClaudeOAuth`; wraps contain only the home node.
    NeverSync,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub provider: String,
    pub kind: AccountKind,
    pub label: String,
    pub home_node: NodeId,
    pub sync: SyncPolicy,
    pub version: u64,
    pub writer: NodeId,
    pub updated_at: u64,
    /// base64(XChaCha20-Poly1305(DEK, secret, aad = id ‖ version ‖ gen)), nonce prefixed.
    pub sealed: String,
    /// NodeId → base64(HPKE(X25519 of that node, DEK)).
    pub wraps: BTreeMap<NodeId, String>,
    /// Credential generation; bumped by every OAuth refresh.
    #[serde(rename = "gen")]
    pub generation: u64,
    pub sig: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Enablement {
    pub account_id: String,
    pub enabled_on: BTreeSet<NodeId>,
    pub all_nodes: bool,
    pub version: u64,
    pub writer: NodeId,
    pub sig: String,
}

/// Refresh lease (only between nodes in `enabled_on`, only synced OAuth).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Lease {
    pub account_id: String,
    pub holder: NodeId,
    pub epoch: u64,
    pub expires_at: u64,
    pub sig: String,
}
/// Wire form of a lease announcement (`POST …/vault/lease`).
pub type LeaseMsg = Lease;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", content = "record", rename_all = "snake_case")]
pub enum SignedRecord {
    Account(Account),
    Enablement(Enablement),
    Lease(Lease),
}
impl SignedRecord {
    /// Key used in [`PullReq::since`]: `"<kind>:<account id>"`.
    pub fn key(&self) -> String {
        match self {
            Self::Account(a) => format!("account:{}", a.id),
            Self::Enablement(e) => format!("enablement:{}", e.account_id),
            Self::Lease(l) => format!("lease:{}", l.account_id),
        }
    }
}

/// `POST …/vault/pull`. `since` maps [`SignedRecord::key`] → highest version
/// (accounts: `version`, enablements: `version`, leases: `epoch`) already held.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct PullReq {
    pub since: BTreeMap<String, u64>,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct PullResp {
    pub records: Vec<SignedRecord>,
}
/// `POST …/vault/push`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Push {
    pub records: Vec<SignedRecord>,
}
/// `POST …/vault/refresh-request`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RefreshRequest {
    pub account_id: String,
    #[serde(rename = "gen")]
    pub generation: u64,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RefreshResponse {
    pub record: SignedRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("account not found")]
    NotFound,
    #[error("secret not available on this node")]
    NoSecret,
    #[error("{0}")]
    Other(String),
}

/// The only vault surface WS2 uses. WS6 replaces the memory implementation.
pub trait AccountStore: Send + Sync {
    fn list(&self) -> Vec<AccountSummary>;
    fn get(&self, id: &str) -> Option<AccountSummary>;
    fn get_secret(&self, id: &str) -> Result<SecretBox, StoreError>;
    /// Insert or replace. `secret: None` keeps the stored secret.
    fn put(&self, account: AccountSummary, secret: Option<SecretBox>) -> Result<(), StoreError>;
    fn remove(&self, id: &str) -> Result<(), StoreError>;
}
