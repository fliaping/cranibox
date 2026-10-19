//! Loopback model subscription proxy (C3).
//!
//! Module ownership (docs/design/WORKPLAN.md). Only touch modules you own; WS3
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

use cranibox_contracts::{
    llm::{SignIn, Upstream},
    vault::AccountStore,
};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

/// The caller supplies the C7 vault. No credentials are persisted by this crate.
pub struct Config {
    pub node: String,
    pub listen: Vec<SocketAddr>,
    pub run_dir: PathBuf,
    pub store: Arc<dyn AccountStore>,
    pub upstreams: Vec<Arc<dyn Upstream>>,
    pub sign_in: Option<Arc<dyn SignIn>>,
    pub presets: Vec<cranibox_contracts::llm::Preset>,
    pub routing: Option<Arc<dyn route::Router>>,
}
impl Config {
    pub fn new(node: String, run_dir: PathBuf, store: Arc<dyn AccountStore>) -> Self {
        Self {
            sign_in: Some(Arc::new(auth::OAuth::new(
                node.clone(),
                store.clone(),
                auth::OAuthConfig::default(),
            ))),
            node,
            run_dir,
            store,
            listen: vec![
                LLM_ADDR.parse().expect("constant address"),
                cranibox_contracts::llm::LLM_ADDR_V6
                    .parse()
                    .expect("constant address"),
            ],
            upstreams: upstream::registry(),
            presets: catalog::presets(),
            routing: None,
        }
    }
}
pub use server::engine::CallError as ProxyError;
pub use server::{Handle, spawn};
