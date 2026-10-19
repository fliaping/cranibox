//! Cranibox shared contracts (Phase 0, WS0).
//!
//! This crate only holds types, traits, constants and fixtures that several
//! workstreams depend on. It contains **no feature logic**. The contracts are
//! described in `docs/design/WORKPLAN.md` §1 and owned by WS0; other
//! workstreams request changes with a `CONTRACT-REQUEST:` note in their PR.
//!
//! | Module | Contract | Implemented by | Used by |
//! |---|---|---|---|
//! | [`roster`] | C1 roster and identity | WS6 | WS1, WS2, WS7 |
//! | [`net`] | C2 transport, mesh routes, pairing ticket | WS1 (iroh), WS0 (HTTP/Noise wrapper) | WS6, WS7 |
//! | [`llm`] | C3 model proxy, upstreams, sign-in, mgmt API, presets | WS2, WS3, WS4 | WS5, WS6, WS7 |
//! | [`agents`] | C4 agent adapters | WS5 | WS7 |
//! | [`config`] | C5 configuration schema and file locations | WS1, WS2, WS5, WS6 | all |
//! | [`app`] | C6 App API for the UI | WS7 | UI |
//! | [`vault`] | C7 vault records and sync messages | WS6 | WS2 (via `AccountStore`) |
//! | [`error`] | C8 unavailable errors and codes | WS2, WS6 | UI, agents |
//!
//! JSON conventions: field names are `snake_case`; unit enum variants are
//! `snake_case` strings; data-carrying enums are internally tagged with
//! `"kind"`. Optional fields serialize as `null` unless documented otherwise.
//! Public keys and signatures are lowercase hex; ciphertexts are standard
//! padded base64. Timestamps are Unix seconds (`u64`).

pub mod agents;
pub mod app;
pub mod canonical;
pub mod config;
pub mod error;
pub mod llm;
pub mod net;
pub mod roster;
pub mod vault;

#[cfg(feature = "mock")]
pub mod mock;

/// Stable node identifier, `"node-<32 hex>"` (see [`roster::NODE_ID_DOMAIN`]).
pub type NodeId = String;
/// iroh `EndpointId`: the node's ed25519 signing public key.
pub type EndpointId = [u8; 32];

/// Version of the contracts in this crate. Bumped by WS0 on every contract PR.
pub const CONTRACTS_VERSION: u32 = 1;
