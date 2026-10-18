//! Target-owned grants and explicitly configured direct routes. Never export secret values.
use crate::{
    client::{Api, ApiOptions},
    config,
    protocol::valid_node_id,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    #[serde(default)]
    pub exec: bool,
    #[serde(default)]
    pub terminals: bool,
    #[serde(default)]
    pub projects: bool,
    #[serde(default)]
    pub desktop: bool,
    #[serde(default)]
    pub computer: bool,
    #[serde(default)]
    pub messages: bool,
    #[serde(default)]
    pub spaces: bool,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesktopProtocol {
    #[default]
    Vnc,
    Selkies,
}
impl DesktopProtocol {
    fn is_vnc(&self) -> bool {
        *self == Self::Vnc
    }
}
/// The owner selects a local desktop server. Peer requests cannot choose a destination.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Desktop {
    pub address: SocketAddr,
    #[serde(default, skip_serializing_if = "DesktopProtocol::is_vnc")]
    pub protocol: DesktopProtocol,
}
impl Desktop {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.address.ip().is_loopback() && self.address.port() != 0,
            "Desktop sharing must use a loopback address and a nonzero port"
        );
        Ok(())
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub token: String,
    pub permissions: Permissions,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub url: String,
    #[serde(default)]
    pub allow_insecure: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_forward_lock: Option<PathBuf>,
}
impl Endpoint {
    pub(crate) fn validate(&self) -> Result<()> {
        let url = config::check_url(&self.url, false, self.allow_insecure)?;
        ensure!(url.path() == "/", "invalid route origin");
        if let Some(lease) = &self.local_forward_lock {
            ensure!(
                lease.is_absolute()
                    && url.scheme() == "http"
                    && url.host_str() == Some("127.0.0.1"),
                "managed network routes require a local forward lease and IPv4 loopback URL"
            );
        }
        Ok(())
    }
    pub fn public_view(&self) -> serde_json::Value {
        serde_json::json!({"url":self.url,"allow_private_http":self.allow_insecure,
            "managed":self.local_forward_lock.is_some()})
    }
}
pub const MAX_PATHS: usize = 4;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub url: String,
    pub token_file: PathBuf,
    #[serde(default)]
    pub allow_insecure: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_forward_lock: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<Endpoint>,
}
impl Route {
    pub fn endpoints(&self) -> Vec<Endpoint> {
        let mut endpoints = vec![Endpoint {
            url: self.url.clone(),
            allow_insecure: self.allow_insecure,
            local_forward_lock: self.local_forward_lock.clone(),
        }];
        endpoints.extend(self.alternatives.clone());
        endpoints
    }
    pub(super) fn set_endpoints(&mut self, mut endpoints: Vec<Endpoint>) {
        let first = endpoints.remove(0);
        self.url = first.url;
        self.allow_insecure = first.allow_insecure;
        self.local_forward_lock = first.local_forward_lock;
        self.alternatives = endpoints;
    }
    pub fn api(&self, target: &str) -> Result<Api> {
        Api::direct(
            ApiOptions {
                url: self.url.clone(),
                token_file: Some(self.token_file.clone()),
                credentials: PathBuf::new(),
                allow_insecure: self.allow_insecure,
            },
            target.into(),
        )?
        .with_paths(self.endpoints())
    }
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Listener {
    pub address: SocketAddr,
    #[serde(default)]
    pub allow_insecure: bool,
}
impl Listener {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.address.ip().is_loopback() || self.allow_insecure,
            "non-loopback HTTP listener requires --allow-private-http on a trusted private network or behind TLS"
        );
        Ok(())
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub node_id: String,
    #[serde(default)]
    pub listener: Option<Listener>,
    #[serde(default)]
    pub desktop: Option<Desktop>,
    #[serde(default)]
    pub desktop_epoch: u64,
    #[serde(default)]
    pub computer: Option<super::computer::Provider>,
    #[serde(default)]
    pub computer_epoch: u64,
    #[serde(default)]
    pub computer_paused: bool,
    #[serde(default)]
    pub copy_root: Option<PathBuf>,
    #[serde(default)]
    pub grants: BTreeMap<String, Grant>,
    #[serde(default)]
    pub routes: BTreeMap<String, Route>,
}
impl Config {
    pub fn new() -> Self {
        Self {
            version: 1,
            node_id: format!("node-{}", uuid::Uuid::new_v4().simple()),
            listener: None,
            desktop: None,
            desktop_epoch: 0,
            computer: None,
            computer_epoch: 0,
            computer_paused: false,
            copy_root: None,
            grants: BTreeMap::new(),
            routes: BTreeMap::new(),
        }
    }
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && valid_node_id(&self.node_id),
            "invalid peer identity or version"
        );
        ensure!(
            self.grants.len() <= 128 && self.routes.len() <= 128,
            "peer limit reached"
        );
        let mut tokens = BTreeSet::new();
        for (node, grant) in &self.grants {
            ensure!(
                valid_node_id(node) && node != &self.node_id,
                "invalid grant node"
            );
            ensure!(
                grant.token.len() == 64
                    && grant.token.bytes().all(|b| b.is_ascii_hexdigit())
                    && tokens.insert(&grant.token),
                "invalid or duplicate peer token"
            );
            ensure!(
                grant.permissions.exec
                    || grant.permissions.terminals
                    || grant.permissions.projects
                    || grant.permissions.desktop
                    || grant.permissions.computer
                    || grant.permissions.messages
                    || grant.permissions.spaces,
                "grant has no capabilities"
            );
        }
        for (node, route) in &self.routes {
            ensure!(
                valid_node_id(node) && node != &self.node_id,
                "invalid route node"
            );
            ensure!(route.token_file.is_absolute(), "invalid route token file");
            ensure!(
                route.alternatives.len() < MAX_PATHS,
                "at most four paths per node"
            );
            let mut origins = BTreeSet::new();
            for endpoint in route.endpoints() {
                endpoint.validate()?;
                let origin = config::check_url(&endpoint.url, false, endpoint.allow_insecure)?
                    .origin()
                    .ascii_serialization();
                ensure!(origins.insert(origin), "duplicate node path");
            }
        }
        if let Some(listener) = &self.listener {
            listener.validate()?;
        }
        if let Some(desktop) = &self.desktop {
            desktop.validate()?;
        }
        if let Some(computer) = &self.computer {
            computer.validate()?;
        }
        if let Some(root) = &self.copy_root {
            ensure!(
                root.is_absolute() && root.parent().is_some(),
                "invalid project copy root"
            );
        }
        Ok(())
    }
    pub fn read(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path).context("read peer configuration")?;
        ensure!(
            file.metadata()?.len() <= 128 * 1024,
            "peer configuration too large"
        );
        let value: Self = serde_json::from_reader(file).context("invalid peer configuration")?;
        value.validate()?;
        Ok(value)
    }
    pub fn update(path: &Path, change: impl FnOnce(&mut Self) -> Result<()>) -> Result<Self> {
        let _lock = config::lock_credentials(path)?;
        let mut value = if path.exists() {
            Self::read(path)?
        } else {
            Self::new()
        };
        change(&mut value)?;
        value.validate()?;
        let bytes = serde_json::to_vec_pretty(&value)?;
        ensure!(bytes.len() <= 128 * 1024, "peer configuration too large");
        config::write_private(path, &bytes, true)?;
        Ok(value)
    }
    pub fn public_view(&self) -> serde_json::Value {
        serde_json::json!({"node_id":self.node_id, "listener":self.listener, "desktop":self.desktop, "computer":self.computer, "computer_paused":self.computer_paused, "copy_root":self.copy_root,
            "grants":self.grants.iter().map(|(id, grant)| serde_json::json!({"node":id,"permissions":grant.permissions})).collect::<Vec<_>>(),
            "routes":self.routes.iter().map(|(id, route)| serde_json::json!({"node":id,"url":route.url,"allow_private_http":route.allow_insecure,
                "paths":route.endpoints().iter().map(Endpoint::public_view).collect::<Vec<_>>()
            })).collect::<Vec<_>>()})
    }
}
impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

pub fn path_for(settings: &Path) -> PathBuf {
    settings.with_file_name("peers.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_protocol_is_explicit_and_legacy_vnc_stays_compatible() {
        let old = serde_json::json!({"address":"127.0.0.1:5900"});
        let desktop: Desktop = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(desktop.protocol, DesktopProtocol::Vnc);
        assert_eq!(serde_json::to_value(desktop).unwrap(), old);
        let desktop: Desktop = serde_json::from_value(
            serde_json::json!({"address":"127.0.0.1:8081","protocol":"selkies"}),
        )
        .unwrap();
        assert_eq!(desktop.protocol, DesktopProtocol::Selkies);
        assert!(desktop.validate().is_ok());
        assert!(
            serde_json::from_value::<Desktop>(
                serde_json::json!({"address":"127.0.0.1:8081","protocol":"http"})
            )
            .is_err()
        );
    }
    #[test]
    fn identity_persists_and_corruption_is_not_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("peers.json");
        let first = Config::update(&path, |_| Ok(())).unwrap();
        let second = Config::update(&path, |_| Ok(())).unwrap();
        assert_eq!(first.node_id, second.node_id);
        assert_ne!(first.node_id, Config::new().node_id);
        std::fs::write(&path, b"broken").unwrap();
        assert!(Config::update(&path, |_| Ok(())).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"broken");
    }
    #[test]
    fn scopes_and_private_network_are_explicit_and_public_view_hides_secrets() {
        let legacy: Permissions =
            serde_json::from_value(serde_json::json!({"exec":true,"terminals":true})).unwrap();
        assert!(
            !legacy.projects
                && !legacy.desktop
                && !legacy.computer
                && !legacy.messages
                && !legacy.spaces,
            "old grants must not acquire new capabilities"
        );
        let mut value = Config::new();
        let token = config::token();
        value.grants.insert(
            "caller".into(),
            Grant {
                token: token.clone(),
                permissions: Permissions::default(),
            },
        );
        assert!(value.validate().is_err());
        value.grants.get_mut("caller").unwrap().permissions.exec = true;
        assert!(value.validate().is_ok());
        assert!(!value.public_view().to_string().contains(&token));
        value.listener = Some(Listener {
            address: "0.0.0.0:9000".parse().unwrap(),
            allow_insecure: false,
        });
        assert!(value.validate().is_err());
        value.listener.as_mut().unwrap().allow_insecure = true;
        assert!(value.validate().is_ok());
    }
}
