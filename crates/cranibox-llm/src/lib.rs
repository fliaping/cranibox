//! Loopback model subscription proxy (C3). Phase 0 skeleton.
//!
//! Module ownership (docs/design/OWNERS.md). Only touch modules you own; WS3
//! and WS4 may each add their own lines to the registry in [`upstream`].
//!
//! | Module | Owner |
//! |---|---|
//! | `server`, `proto`, `catalog`, `mgmt`, `route::local`, `upstream` (registry), `upstream::apikey`, `upstream/presets.toml` | WS2 |
//! | `auth`, `upstream::{codex, kimi, gemini, copilot, cn}` | WS3 |
//! | `upstream::claude` | WS4 |
//! | `vault`, `route::{remote, health}` | WS6 |
pub use cranibox_contracts::llm::{LLM_ADDR, LLM_MGMT_SOCKET};

pub mod auth;
pub mod catalog;
pub mod mgmt;
pub mod proto;
pub mod route;
pub mod server;
pub mod upstream;
pub mod vault;
