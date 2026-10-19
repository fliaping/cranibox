//! Target-owned grants and explicitly configured direct routes. Never export secret values.
use super::identity::{self, Identity, PublicIdentity};
use crate::{
    client::{Api, ApiOptions},
    config,
    protocol::valid_node_id,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
    pub browser: bool,
    #[serde(default)]
    pub messages: bool,
    /// Read this node's active task, message and collaboration content, including other senders.
    #[serde(default)]
    pub activity: bool,
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
/// A peer key pinned by the first authenticated pairing and, optionally, confirmed by
/// both owners comparing the pairing code.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Pinned {
    pub identity: PublicIdentity,
    #[serde(default)]
    pub verified: bool,
    pub pinned_at: u64,
}
impl Pinned {
    pub fn new(identity: PublicIdentity) -> Self {
        Self {
            identity,
            verified: false,
            pinned_at: identity::now(),
        }
    }
    fn view(&self, own: Option<&PublicIdentity>) -> serde_json::Value {
        serde_json::json!({"fingerprint":self.identity.fingerprint(),"key_derived_id":self.identity.derived(),
            "verified":self.verified,"pinned_at":self.pinned_at,
            "pairing_code":own.map(|own| identity::pairing_code(own, &self.identity))})
    }
}
fn is_false(value: &bool) -> bool {
    !*value
}
/// What happens to agent-to-agent content (device messages) arriving from a peer.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Inbound {
    /// Deliver to the local mailbox and Agents.
    #[default]
    Accept,
    /// Keep in a human approval queue; Agents do not see it until released.
    Hold,
    /// Reject without storing.
    Refuse,
}
impl Inbound {
    fn is_accept(&self) -> bool {
        *self == Self::Accept
    }
}
/// Target-issued capability grant. Only a digest of the bearer secret is stored.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    /// Legacy plaintext secret, accepted only while reading and migrated immediately.
    #[serde(default, skip_serializing)]
    token: Option<String>,
    /// Random per-grant salt (hex). Empty only for grants migrated from plaintext, whose
    /// digest must stay equal to the credential hash already bound in existing ledgers.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub salt: String,
    /// SHA-256(salt || secret), hex. Also the stable credential binding for ledgers.
    #[serde(default)]
    pub credential: String,
    pub permissions: Permissions,
    /// Caller key pinned on first end-to-end pairing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<Pinned>,
    /// Refuse capabilities until both owners verified the pairing code.
    #[serde(default, skip_serializing_if = "is_false")]
    pub require_verified: bool,
    /// Inbound content policy. Grants written before this field existed accept, so
    /// their signed bytes are unchanged; newly issued grants hold for a human.
    #[serde(default, skip_serializing_if = "Inbound::is_accept")]
    pub inbound: Inbound,
    /// Ed25519 signature by the issuing (target) node over this record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}
fn credential_digest(salt: &[u8], secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt);
    hasher.update(secret.as_bytes());
    identity::hex(&hasher.finalize())
}
impl Grant {
    /// Issue a new grant for a freshly generated secret.
    pub fn issue(secret: &str, permissions: Permissions) -> Self {
        let mut salt = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::rng(), &mut salt);
        Self {
            token: None,
            salt: identity::hex(&salt),
            credential: credential_digest(&salt, secret),
            permissions,
            peer: None,
            require_verified: false,
            inbound: Inbound::Hold,
            signature: None,
        }
    }
    /// Constant-time check of a presented bearer secret against the stored digest.
    pub fn verifies(&self, secret: &str) -> bool {
        let Ok(salt) = decode_salt(&self.salt) else {
            return false;
        };
        config::matches(&credential_digest(&salt, secret), &self.credential)
    }
    /// Whether two views describe the same issued credential (rotation changes it).
    pub fn same_credential(&self, other: &Grant) -> bool {
        config::matches(&self.credential, &other.credential)
    }
    fn migrate(&mut self) {
        if let Some(token) = self.token.take() {
            // Equal to the SHA-256 credential hash existing ledgers already store.
            self.salt.clear();
            self.credential = credential_digest(&[], &token);
            self.signature = None;
        }
    }
    fn signed_bytes(&self, owner: &str, node: &str) -> Vec<u8> {
        let mut unsigned = self.clone();
        unsigned.signature = None;
        let mut message = b"cranibox-grant-v1\0".to_vec();
        message.extend_from_slice(owner.as_bytes());
        message.push(0);
        message.extend_from_slice(node.as_bytes());
        message.push(0);
        message.extend_from_slice(&serde_json::to_vec(&unsigned).expect("serialize grant"));
        message
    }
    fn signature_valid(&self, owner: &PublicIdentity, node: &str) -> bool {
        use ed25519_dalek::Verifier;
        let Some(signature) = &self.signature else {
            return false;
        };
        let (Ok(key), Ok(bytes)) = (owner.verifying_key(), identity::unhex::<64>(signature)) else {
            return false;
        };
        key.verify(
            &self.signed_bytes(&owner.node_id, node),
            &ed25519_dalek::Signature::from_bytes(&bytes),
        )
        .is_ok()
    }
    /// Whether capabilities may be used now (pairing verification when required).
    pub fn usable(&self) -> bool {
        !self.require_verified || self.peer.as_ref().is_some_and(|p| p.verified)
    }
}
fn decode_salt(salt: &str) -> Result<Vec<u8>> {
    if salt.is_empty() {
        return Ok(vec![]);
    }
    Ok(identity::unhex::<16>(salt)?.to_vec())
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
    /// Target identity pinned on first contact (trust on first use, then verified).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_identity: Option<Pinned>,
    /// Peer configuration this route was read from; locates the local identity.
    #[serde(skip)]
    pub config_path: Option<PathBuf>,
}
impl Route {
    /// Fields that describe where and with which secret to connect, excluding the
    /// pinned identity, so pinning a key does not invalidate queued work.
    pub fn binding_view(&self) -> Self {
        let mut route = self.clone();
        route.target_identity = None;
        route.config_path = None;
        route
    }
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
        let api = Api::direct(
            ApiOptions {
                url: self.url.clone(),
                token_file: Some(self.token_file.clone()),
                credentials: PathBuf::new(),
                allow_insecure: self.allow_insecure,
            },
            target.into(),
        )?
        .with_paths(self.endpoints())?;
        Ok(match &self.config_path {
            Some(path) => api.with_identity(path.clone(), self.target_identity.clone()),
            None => api,
        })
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
    pub browser: Option<super::browser::Provider>,
    #[serde(default)]
    pub browser_epoch: u64,
    #[serde(default)]
    pub browser_paused: bool,
    #[serde(default)]
    pub copy_root: Option<PathBuf>,
    #[serde(default)]
    pub grants: BTreeMap<String, Grant>,
    #[serde(default)]
    pub routes: BTreeMap<String, Route>,
    /// This node's public identity, used to verify the grant signatures below.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<PublicIdentity>,
}
/// Version 2: grants store digests and carry signatures by `identity`.
pub const VERSION: u32 = 2;
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
            browser: None,
            browser_epoch: 0,
            browser_paused: false,
            copy_root: None,
            grants: BTreeMap::new(),
            routes: BTreeMap::new(),
            identity: None,
        }
    }
    /// A new node derives its NodeID from a freshly generated key.
    fn create(path: &Path) -> Result<Self> {
        let identity = Identity::create_new(path)?;
        let mut value = Self::new();
        value.node_id = identity.node_id().to_owned();
        Ok(value)
    }
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.version, 1 | VERSION) && valid_node_id(&self.node_id),
            "invalid peer identity or version"
        );
        if let Some(identity) = &self.identity {
            identity.verify()?;
            ensure!(
                identity.node_id == self.node_id,
                "peer configuration identity mismatch"
            );
        }
        // After migration every grant must stay signed; stripping a signature is tampering.
        ensure!(
            self.version == 1 || self.identity.is_some(),
            "signed peer configuration requires its identity"
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
                grant.token.is_none()
                    && grant.credential.len() == 64
                    && grant.credential.bytes().all(|b| b.is_ascii_hexdigit())
                    && decode_salt(&grant.salt).is_ok()
                    && tokens.insert(&grant.credential),
                "invalid or duplicate peer credential"
            );
            if let Some(identity) = self.identity.as_ref().filter(|_| self.version >= VERSION) {
                ensure!(
                    grant.signature_valid(identity, node),
                    "peer grant signature is invalid; existing data was preserved"
                );
            }
            if let Some(peer) = &grant.peer {
                peer.identity.verify()?;
                ensure!(
                    peer.identity.node_id == *node,
                    "pinned peer key belongs to another node"
                );
            }
            ensure!(
                grant.permissions.exec
                    || grant.permissions.terminals
                    || grant.permissions.projects
                    || grant.permissions.desktop
                    || grant.permissions.computer
                    || grant.permissions.browser
                    || grant.permissions.messages
                    || grant.permissions.activity
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
            if let Some(pinned) = &route.target_identity {
                pinned.identity.verify()?;
                ensure!(
                    pinned.identity.node_id == *node,
                    "pinned target key belongs to another node"
                );
            }
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
        if let Some(browser) = &self.browser {
            browser.validate()?;
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
        let mut value: Self =
            serde_json::from_reader(file).context("invalid peer configuration")?;
        // Plaintext secrets from version 1 are replaced in memory before any use.
        for grant in value.grants.values_mut() {
            grant.migrate();
        }
        value.validate()?;
        if value.identity.is_some()
            || value.version >= VERSION
            || identity::path_for(path).try_exists()?
        {
            value.local_identity(path)?;
        }
        for route in value.routes.values_mut() {
            route.config_path = Some(path.to_path_buf());
        }
        Ok(value)
    }
    /// The private identity file anchors this node's public key and migration floor.
    /// Reading a migrated configuration must never create or repair key material.
    fn local_identity(&self, path: &Path) -> Result<Identity> {
        let identity = Identity::load(&identity::path_for(path))
            .context("load original node identity; restore its key file from backup if missing")?;
        ensure!(
            identity.node_id() == self.node_id,
            "node identity file belongs to another NodeID; existing data was preserved"
        );
        ensure!(
            self.version >= identity.minimum_config_version(),
            "peer configuration downgrade rejected; restore the signed configuration"
        );
        ensure!(
            self.identity
                .as_ref()
                .is_none_or(|public| public == identity.public()),
            "peer configuration identity changed; existing data was preserved"
        );
        Ok(identity)
    }
    /// Sign unsigned grants with the node key and record the public identity. Grants
    /// whose content changed were re-validated by the caller before signing.
    fn seal(&mut self, path: &Path) -> Result<Identity> {
        let identity = if self.identity.is_some() || self.version >= VERSION {
            self.local_identity(path)?
        } else {
            Identity::ensure(path, &self.node_id)?
        };
        let public = identity.public().clone();
        if self.identity.as_ref() != Some(&public) {
            ensure!(
                self.identity.is_none(),
                "peer configuration identity changed; existing data was preserved"
            );
            self.identity = Some(public.clone());
        }
        for (node, grant) in self.grants.iter_mut() {
            if !grant.signature_valid(&public, node) {
                grant.signature = None;
                grant.signature = Some(identity.sign(&grant.signed_bytes(&public.node_id, node)));
            }
        }
        self.version = VERSION;
        Ok(identity)
    }
    /// Load this node's private identity (creating one for a legacy NodeID).
    pub fn identity(path: &Path) -> Result<Identity> {
        let _lock = config::lock_credentials(path)?;
        let value = Self::read(path)?;
        if value.identity.is_some() || value.version >= VERSION {
            value.local_identity(path)
        } else {
            Identity::ensure(path, &value.node_id)
        }
    }
    pub fn update(path: &Path, change: impl FnOnce(&mut Self) -> Result<()>) -> Result<Self> {
        let _lock = config::lock_credentials(path)?;
        let mut value = if path.exists() {
            Self::read(path)?
        } else {
            Self::create(path)?
        };
        change(&mut value)?;
        // Validate the change before signing it, then validate the signed result.
        let mut unsigned = value.clone();
        unsigned.version = 1;
        unsigned.validate()?;
        let mut identity = value.seal(path)?;
        value.validate()?;
        let bytes = serde_json::to_vec_pretty(&value)?;
        ensure!(bytes.len() <= 128 * 1024, "peer configuration too large");
        config::write_private(path, &bytes, true)?;
        identity.record_config_version(path, VERSION)?;
        Ok(value)
    }
    pub fn public_view(&self) -> serde_json::Value {
        serde_json::json!({"node_id":self.node_id, "listener":self.listener, "desktop":self.desktop, "computer":self.computer, "computer_paused":self.computer_paused, "browser":self.browser, "browser_paused":self.browser_paused, "copy_root":self.copy_root,
            "identity":self.identity.as_ref().map(|i| serde_json::json!({"fingerprint":i.fingerprint(),"key_derived_id":i.derived(),"public":i})),
            "grants":self.grants.iter().map(|(id, grant)| serde_json::json!({"node":id,"permissions":grant.permissions,
                "require_verified":grant.require_verified,"inbound":grant.inbound,"signed":grant.signature.is_some(),
                "peer":grant.peer.as_ref().map(|p| p.view(self.identity.as_ref()))})).collect::<Vec<_>>(),
            "routes":self.routes.iter().map(|(id, route)| serde_json::json!({"node":id,"url":route.url,"allow_private_http":route.allow_insecure,
                "identity":route.target_identity.as_ref().map(|p| p.view(self.identity.as_ref())),
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
                && !legacy.browser
                && !legacy.messages
                && !legacy.activity
                && !legacy.spaces,
            "old grants must not acquire new capabilities"
        );
        let mut value = Config::new();
        let token = config::token();
        value.grants.insert(
            "caller".into(),
            Grant::issue(&token, Permissions::default()),
        );
        assert!(value.validate().is_err());
        value.grants.get_mut("caller").unwrap().permissions.activity = true;
        assert!(value.validate().is_ok());
        value.grants.get_mut("caller").unwrap().permissions.activity = false;
        value.grants.get_mut("caller").unwrap().permissions.exec = true;
        assert!(value.validate().is_ok());
        assert!(!value.public_view().to_string().contains(&token));
        assert!(!serde_json::to_string(&value).unwrap().contains(&token));
        value.listener = Some(Listener {
            address: "0.0.0.0:9000".parse().unwrap(),
            allow_insecure: false,
        });
        assert!(value.validate().is_err());
        value.listener.as_mut().unwrap().allow_insecure = true;
        assert!(value.validate().is_ok());
    }

    fn legacy_file(root: &Path, token: &str) -> PathBuf {
        let path = root.join("peers.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({"version":1,"node_id":"node-legacy",
                "grants":{"caller":{"token":token,"permissions":{"messages":true}}},"routes":{}}))
            .unwrap(),
        )
        .unwrap();
        path
    }
    #[test]
    fn new_nodes_derive_their_id_from_a_persistent_key() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("peers.json");
        let value = Config::update(&path, |_| Ok(())).unwrap();
        assert!(value.node_id.starts_with(identity::PREFIX));
        let public = value.identity.clone().unwrap();
        assert!(public.derived() && public.node_id == value.node_id);
        assert_eq!(Config::identity(&path).unwrap().public(), &public);
        assert_eq!(value.version, VERSION);
    }
    #[test]
    fn plaintext_grants_migrate_to_digests_and_keep_ledger_bindings() {
        let root = tempfile::tempdir().unwrap();
        let token = config::token();
        let path = legacy_file(root.path(), &token);
        let read = Config::read(&path).unwrap();
        let grant = &read.grants["caller"];
        // Same value existing task/execution/copy ledgers stored for this credential.
        assert_eq!(
            grant.credential,
            format!("{:x}", Sha256::digest(token.as_bytes()))
        );
        assert!(grant.verifies(&token) && !grant.verifies(&config::token()));
        let saved = Config::update(&path, |_| Ok(())).unwrap();
        assert_eq!(saved.node_id, "node-legacy", "legacy NodeIDs are kept");
        assert_eq!(saved.version, VERSION);
        assert!(saved.grants["caller"].signature.is_some());
        let bytes = std::fs::read_to_string(&path).unwrap();
        assert!(
            !bytes.contains(&token),
            "no plaintext secret remains on disk"
        );
        assert!(Config::read(&path).unwrap().grants["caller"].verifies(&token));
        // New grants are salted.
        let issued = Grant::issue(&token, Permissions::default());
        assert_eq!(issued.salt.len(), 32);
        assert_ne!(issued.credential, grant.credential);
        assert!(issued.verifies(&token));
    }
    #[test]
    fn signed_grants_reject_tampering_and_signature_stripping() {
        let root = tempfile::tempdir().unwrap();
        let path = legacy_file(root.path(), &config::token());
        Config::update(&path, |_| Ok(())).unwrap();
        let original = std::fs::read(&path).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        value["grants"]["caller"]["permissions"]["exec"] = true.into();
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(
            Config::read(&path).is_err(),
            "escalated permissions must be rejected"
        );
        assert!(Config::update(&path, |_| Ok(())).is_err());
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        value["grants"]["caller"]
            .as_object_mut()
            .unwrap()
            .remove("signature");
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(
            Config::read(&path).is_err(),
            "unsigned grants are refused after migration"
        );
        std::fs::write(&path, &original).unwrap();
        assert!(Config::read(&path).is_ok());
    }
    #[test]
    fn migrated_configuration_requires_its_original_private_key() {
        let root = tempfile::tempdir().unwrap();
        let path = legacy_file(root.path(), &config::token());
        Config::update(&path, |_| Ok(())).unwrap();
        let original = std::fs::read(&path).unwrap();
        let key_path = identity::path_for(&path);
        let key = std::fs::read(&key_path).unwrap();
        std::fs::remove_file(&key_path).unwrap();
        assert!(Config::read(&path).is_err());
        assert!(Config::identity(&path).is_err());
        assert!(Config::update(&path, |_| Ok(())).is_err());
        assert!(
            !key_path.exists(),
            "a missing migrated key must not be replaced"
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::write(&key_path, &key).unwrap();
        assert!(
            Config::read(&path).is_ok(),
            "restoring the original key recovers the node"
        );
    }
    #[test]
    fn self_signed_replacement_cannot_reauthorize_the_node() {
        let root = tempfile::tempdir().unwrap();
        let path = legacy_file(root.path(), &config::token());
        let mut forged = Config::update(&path, |_| Ok(())).unwrap();
        let key_path = identity::path_for(&path);
        let key = std::fs::read(&key_path).unwrap();
        // A different key can sign a valid identity for the same legacy NodeID.
        // Trust must come from the local key file, not the editable configuration.
        let other = tempfile::tempdir().unwrap();
        forged.version = 1;
        forged.identity = None;
        forged.grants.get_mut("caller").unwrap().permissions.exec = true;
        forged.seal(&other.path().join("peers.json")).unwrap();
        forged.validate().unwrap();
        let bytes = serde_json::to_vec(&forged).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert!(Config::read(&path).is_err());
        assert!(Config::identity(&path).is_err());
        assert!(Config::update(&path, |_| Ok(())).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read(&key_path).unwrap(), key);
    }
    #[test]
    fn migrated_configuration_cannot_bypass_signatures_by_downgrading() {
        let root = tempfile::tempdir().unwrap();
        let path = legacy_file(root.path(), &config::token());
        Config::update(&path, |_| Ok(())).unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["version"] = 1.into();
        value.as_object_mut().unwrap().remove("identity");
        value["grants"]["caller"]["permissions"]["exec"] = true.into();
        value["grants"]["caller"]
            .as_object_mut()
            .unwrap()
            .remove("signature");
        let bytes = serde_json::to_vec(&value).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert!(Config::read(&path).is_err());
        assert!(Config::update(&path, |_| Ok(())).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    #[test]
    fn interrupted_legacy_migration_can_reuse_its_key() {
        let root = tempfile::tempdir().unwrap();
        let path = legacy_file(root.path(), &config::token());
        // A crash after key creation and before the configuration save is recoverable.
        let identity = Identity::ensure(&path, "node-legacy").unwrap();
        let public = identity.public().clone();
        // Identity files written by the first identity release had no checkpoint.
        let key_path = identity::path_for(&path);
        let mut key: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&key_path).unwrap()).unwrap();
        key.as_object_mut()
            .unwrap()
            .remove("minimum_config_version");
        std::fs::write(&key_path, serde_json::to_vec(&key).unwrap()).unwrap();
        assert_eq!(Config::read(&path).unwrap().version, 1);
        let saved = Config::update(&path, |_| Ok(())).unwrap();
        assert_eq!(saved.identity.as_ref(), Some(&public));
        assert_eq!(Config::read(&path).unwrap().version, VERSION);
        let saved_key: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&key_path).unwrap()).unwrap();
        assert_eq!(saved_key["minimum_config_version"], VERSION);
        assert_eq!(saved_key["signing_seed"], key["signing_seed"]);
        assert_eq!(saved_key["noise_private"], key["noise_private"]);
        assert_eq!(saved_key["created_at"], key["created_at"]);
    }
}
