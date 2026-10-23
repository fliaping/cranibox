//! Device-owned model service, signed roster and encrypted subscription synchronization.
use agent_workspace_runtime::peers::{
    Store,
    config::Config,
    identity::Identity,
    roster::{OwnerKey, RosterStore},
    transport::DirectTransport,
};
use anyhow::{Context, Result, ensure};
use cranibox_contracts::{
    llm::{AccountKind, Availability, ModelEntry, mgmt::LlmMgmt},
    roster::{Role, RosterMember, RosterView},
    vault::{AccountStore, RefreshRequest, RefreshResponse},
};
use cranibox_llm::{
    route::remote::RemoteRouter,
    vault::{Vault, VaultKeys, coordinated::Coordinated, lease::RefreshCoordinator},
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;
fn live_models()
-> &'static std::sync::Mutex<std::collections::HashMap<PathBuf, std::sync::Weak<Models>>> {
    static LIVE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<PathBuf, std::sync::Weak<Models>>>,
    > = std::sync::OnceLock::new();
    LIVE.get_or_init(Default::default)
}
/// The running model service of the node whose peer configuration is `peers`.
pub fn live(peers: &Path) -> Option<Arc<Models>> {
    live_models().lock().unwrap().get(peers)?.upgrade()
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
/// The recovery signing key behind a recovery code's seed.
fn recovery_key(seed: &[u8; 32]) -> ed25519_dalek::SigningKey {
    use sha2::Digest;
    let digest = sha2::Sha256::digest([b"cranibox-recovery-key-v1\0".as_slice(), seed].concat());
    ed25519_dalek::SigningKey::from_bytes(&digest.into())
}
/// Shortest recovery passphrase: it guards the identity against offline guessing.
pub const MIN_PASSPHRASE: usize = 12;
fn check_passphrase(passphrase: &str) -> Result<()> {
    let length = passphrase.chars().count();
    ensure!(
        length >= MIN_PASSPHRASE,
        "Use at least {MIN_PASSPHRASE} characters for the recovery passphrase"
    );
    ensure!(length <= 256, "The recovery passphrase is too long");
    let distinct: std::collections::BTreeSet<char> = passphrase.chars().collect();
    ensure!(
        distinct.len() >= 5,
        "The recovery passphrase is too simple; mix more characters"
    );
    Ok(())
}
/// The recovery seed behind a passphrase: Argon2id (64 MiB, 3 passes) with a
/// per-identity salt, so a passphrase cannot be guessed cheaply.
fn passphrase_seed(passphrase: &str, salt: &[u8]) -> Result<zeroize::Zeroizing<[u8; 32]>> {
    let params = argon2::Params::new(64 * 1024, 3, 1, Some(32))
        .map_err(|e| anyhow::anyhow!("recovery parameters: {e}"))?;
    let mut seed = zeroize::Zeroizing::new([0u8; 32]);
    argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
        .hash_password_into(passphrase.trim().as_bytes(), salt, seed.as_mut())
        .map_err(|e| anyhow::anyhow!("recovery passphrase: {e}"))?;
    Ok(seed)
}
fn sign_cert(
    recovery: &ed25519_dalek::SigningKey,
    owner_pub: &str,
    serial: u64,
    recovery_salt: Option<String>,
) -> cranibox_contracts::roster::OwnerCert {
    use ed25519_dalek::Signer;
    let mut cert = cranibox_contracts::roster::OwnerCert {
        recovery_pub: hex32(recovery.verifying_key().as_bytes()),
        owner_pub: owner_pub.to_owned(),
        serial,
        recovery_salt,
        sig: String::new(),
    };
    cert.sig = recovery
        .sign(&cert.signing_message())
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    cert
}
/// 56 base32 characters in groups of four: the seed plus a 4-character checksum.
fn recovery_code(seed: &[u8; 32]) -> String {
    use sha2::Digest;
    let check = sha2::Sha256::digest([b"cranibox-recovery-check\0".as_slice(), seed].concat());
    let text = data_encoding::BASE32_NOPAD.encode(&[seed.as_slice(), &check[..3]].concat());
    text.as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("-")
}
fn parse_recovery_code(code: &str) -> Result<zeroize::Zeroizing<[u8; 32]>> {
    use sha2::Digest;
    let text: String = code
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let bytes = data_encoding::BASE32_NOPAD
        .decode(text.as_bytes())
        .map_err(|_| anyhow::anyhow!("That is not a recovery code"))?;
    ensure!(bytes.len() == 35, "That is not a recovery code");
    let mut seed = zeroize::Zeroizing::new([0u8; 32]);
    seed.copy_from_slice(&bytes[..32]);
    let check =
        sha2::Sha256::digest([b"cranibox-recovery-check\0".as_slice(), seed.as_slice()].concat());
    ensure!(check[..3] == bytes[32..], "The recovery code has a typo");
    Ok(seed)
}

/// `cbo-xxxx-xxxx-xxxx-xxxx`: short, stable code of an identity (its recovery or owner key).
pub fn identity_code(owner_pub: &str) -> String {
    use sha2::Digest;
    let digest =
        sha2::Sha256::digest([b"cranibox-identity-v1\0".as_slice(), owner_pub.as_bytes()].concat());
    let text = data_encoding::BASE32_NOPAD
        .encode(&digest[..10])
        .to_ascii_lowercase();
    format!(
        "cbo-{}-{}-{}-{}",
        &text[0..4],
        &text[4..8],
        &text[8..12],
        &text[12..16]
    )
}

pub struct Models {
    pub roster: Arc<RosterStore>,
    pub vault: Arc<Vault>,
    pub router: Arc<RemoteRouter>,
    pub proxy: cranibox_llm::Handle,
    upstreams: Vec<Arc<Coordinated>>,
    coordinator: Arc<RefreshCoordinator>,
    path: PathBuf,
    owner_path: PathBuf,
    stop: CancellationToken,
    /// This device's vault X25519 key, for secrets shared with fellow roster members.
    vault_private: zeroize::Zeroizing<[u8; 32]>,
}
impl Models {
    pub async fn start(path: PathBuf, stop: CancellationToken) -> Result<Arc<Self>> {
        Self::start_on(path, stop, None).await
    }
    pub(crate) async fn start_on(
        path: PathBuf,
        stop: CancellationToken,
        listen: Option<Vec<std::net::SocketAddr>>,
    ) -> Result<Arc<Self>> {
        let config = Config::read(&path)?;
        let identity = Arc::new(Identity::load(
            &agent_workspace_runtime::peers::identity::path_for(&path),
        )?);
        let public = identity.public().clone();
        let signer = identity.clone();
        let dir = path.with_file_name("models");
        std::fs::create_dir_all(&dir)?;
        let key_path = dir.join("encryption.key");
        // Preserve keys of pre-keychain vaults; new vaults get a dedicated key
        // from the desktop secret service, with a private headless fallback.
        let encryption = if dir.join("roster.json").exists() && !key_path.exists() {
            let roster = RosterStore::load(dir.join("roster.json"))?;
            let old = VaultKeys::new(
                config.node_id.clone(),
                public.signing_key.clone(),
                *identity.noise_private(),
                Arc::new(|_| String::new()),
            );
            if roster
                .current()
                .and_then(|r| r.member(&config.node_id).cloned())
                .is_some_and(|m| m.x25519_pub == old.encryption_public())
            {
                agent_workspace_runtime::config::write_private(
                    &key_path,
                    identity.noise_private(),
                    false,
                )?;
            }
            cranibox_llm::vault::keys::load_or_create(
                &config.node_id,
                &key_path,
                Some(&cranibox_llm::vault::keys::PlatformKeyStore),
            )?
        } else {
            cranibox_llm::vault::keys::load_or_create(
                &config.node_id,
                &key_path,
                Some(&cranibox_llm::vault::keys::PlatformKeyStore),
            )?
        };
        let vault_private: [u8; 32] = encryption
            .expose()
            .try_into()
            .context("invalid vault key")?;
        let keys = VaultKeys::new(
            config.node_id.clone(),
            public.signing_key.clone(),
            encryption
                .expose()
                .try_into()
                .context("invalid vault key")?,
            Arc::new(move |m| signer.sign(m)),
        );
        let owner_path = dir.join("owner.key");
        let roster_path = dir.join("roster.json");
        if !roster_path.with_extension("owner").exists() {
            let owner = OwnerKey::load_or_create(&owner_path)?;
            RosterStore::pin(&roster_path, &owner.public())?;
        }
        let roster = Arc::new(RosterStore::load(roster_path)?);
        if roster.current().is_none() {
            let owner = OwnerKey::load_or_create(&owner_path)?;
            roster.issue(
                &owner,
                vec![RosterMember {
                    node_id: config.node_id.clone(),
                    signing_key: public.signing_key,
                    x25519_pub: keys.encryption_public(),
                    role: if std::env::var("CRANIBOX_NODE_ROLE").as_deref() == Ok("hub") {
                        Role::Hub
                    } else {
                        Role::Node
                    },
                    priority: 128,
                    is_claude_node: false,
                    name: crate::device_info::local().name.clone(),
                    added_at: cranibox_llm::auth::now(),
                    identity: serde_json::to_value(identity.public()).ok(),
                }],
            )?;
        }
        if let Some(member) = roster
            .current()
            .and_then(|r| r.member(&config.node_id).cloned())
        {
            ensure!(
                member.x25519_pub == keys.encryption_public(),
                "vault key does not match the pinned roster"
            );
            // Rosters from older versions named this device "My device"; other
            // members show that name. A name the owner chose is kept.
            if (member.name.trim().is_empty() || member.name == "My device")
                && owner_path.exists()
                && let Some(mut current) = roster.current()
            {
                for m in &mut current.members {
                    if m.node_id == config.node_id {
                        m.name = crate::device_info::local().name.chars().take(64).collect();
                    }
                }
                let owner = OwnerKey::load_or_create(&owner_path)?;
                if let Err(error) = roster.issue(&owner, current.members) {
                    eprintln!("roster: could not update this device's name: {error:#}");
                }
            }
        }
        let vault = Arc::new(Vault::open(
            &dir.join("vault.sqlite"),
            keys,
            roster.clone(),
        )?);
        let transport = Arc::new(DirectTransport::new(Store::new(path.clone())));
        let router = Arc::new(RemoteRouter::new(
            config.node_id,
            roster.clone(),
            vault.clone(),
            transport.clone(),
        ));
        let coordinator = Arc::new(RefreshCoordinator::new(vault.clone()));
        let upstreams: Vec<_> = cranibox_llm::upstream::registry()
            .into_iter()
            .map(|inner| {
                Arc::new(Coordinated {
                    inner,
                    vault: vault.clone(),
                    coordinator: coordinator.clone(),
                    transport: transport.clone(),
                })
            })
            .collect();
        let mut proxy_config =
            cranibox_llm::Config::new(vault.node().into(), dir.join("run"), vault.clone());
        proxy_config.upstreams = upstreams
            .iter()
            .cloned()
            .map(|u| u as Arc<dyn cranibox_contracts::llm::Upstream>)
            .collect();
        proxy_config.routing = Some(router.clone());
        if let Some(listen) = listen {
            proxy_config.listen = listen;
        }
        let proxy = cranibox_llm::spawn(proxy_config)
            .await
            .context("start local model proxy")?;
        let service = Arc::new(Self {
            roster,
            vault,
            router,
            proxy,
            upstreams,
            coordinator,
            path,
            owner_path,
            stop: stop.child_token(),
            vault_private: zeroize::Zeroizing::new(vault_private),
        });
        live_models()
            .lock()
            .unwrap()
            .insert(service.path.clone(), Arc::downgrade(&service));
        let weak = Arc::downgrade(&service);
        let cancel = service.stop.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {_ = cancel.cancelled()=>break,_ = interval.tick()=>{let Some(service)=weak.upgrade()else{break};let _=service.tick().await;}}
            }
        });
        Ok(service)
    }
    async fn tick(&self) -> Result<()> {
        use futures_util::{StreamExt, stream};
        self.trust_members().await;
        let roster = self.roster.current().context("no roster")?;
        let mut reachable = BTreeSet::new();
        // A dead peer must not serialize all other health probes behind its
        // timeout. Probe every device before doing the slower anti-entropy work.
        let peers: Vec<_> = roster
            .members
            .into_iter()
            .filter(|m| m.node_id != self.vault.node())
            .collect();
        let probes = stream::iter(peers.into_iter().map(|peer| async move {
            let responds = self.router.probe(&peer.node_id).await;
            (peer, responds)
        }))
        .buffer_unordered(16)
        .collect::<Vec<_>>()
        .await;
        let mut online = Vec::new();
        for (peer, responds) in probes {
            if responds {
                if self.router.health.get(&peer.node_id).available() {
                    reachable.insert(peer.node_id.clone());
                }
                online.push(peer.node_id.clone());
            }
        }
        let at = cranibox_llm::auth::now();
        for account in self
            .vault
            .list()
            .iter()
            .filter(|a| a.kind == AccountKind::OAuth)
        {
            let _ = self
                .coordinator
                .maintain_lease(&account.id, at, &reachable)
                .await;
        }
        stream::iter(online.into_iter().map(|node| async move {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                let _ = self.exchange_roster(&node).await;
                let _ = cranibox_llm::vault::sync::exchange(
                    &self.vault,
                    self.router.transport.as_ref(),
                    &node,
                )
                .await;
            })
            .await;
        }))
        .buffer_unordered(16)
        .collect::<Vec<_>>()
        .await;
        Ok(())
    }
    /// Tell a device just removed from the identity, so it stops acting as a member
    /// instead of waiting for a sync that will never come. Best effort.
    pub async fn notify_removed(&self, node: &str) {
        if let Err(error) = self.exchange_roster(node).await {
            eprintln!("roster: could not tell {node} it was removed: {error:#}");
        }
    }
    async fn exchange_roster(&self, node: &str) -> Result<()> {
        use cranibox_contracts::net::{BoxBody, routes};
        use futures_util::StreamExt;
        let url = routes::vault(node, "/roster");
        let request = axum::http::Request::get(&url).body(BoxBody::empty())?;
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.router.transport.request(&node.to_owned(), request),
        )
        .await??;
        ensure!(response.status().is_success(), "roster unavailable");
        let mut body = response.into_body().into_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) =
            tokio::time::timeout(std::time::Duration::from_secs(5), body.next()).await?
        {
            let chunk = chunk.map_err(|_| anyhow::anyhow!("roster response interrupted"))?;
            ensure!(bytes.len() + chunk.len() <= 1024 * 1024, "roster too large");
            bytes.extend_from_slice(&chunk);
        }
        let remote: cranibox_contracts::roster::Roster = serde_json::from_slice(&bytes)?;
        let local = self.roster.current().context("no roster")?;
        // Verify even an equal epoch; a peer may only introduce a different owner with a
        // newer certificate from this identity's recovery key.
        match self.roster.check(&remote) {
            Ok(true) => return self.install_roster(remote),
            Ok(false) => {}
            // The peer still runs a roster from before our rotation: send it ours.
            Err(_) if remote.owner_pub != local.owner_pub && local.owner_cert.is_some() => {
                let request = axum::http::Request::post(&url)
                    .header("content-type", "application/json")
                    .body(BoxBody::full(serde_json::to_vec(&local)?))?;
                let response = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    self.router.transport.request(&node.to_owned(), request),
                )
                .await??;
                ensure!(response.status().is_success(), "roster rejected");
                return Ok(());
            }
            Err(error) => return Err(error),
        }
        if remote.epoch > local.epoch {
            self.install_roster(remote)?;
        } else if local.epoch > remote.epoch {
            let request = axum::http::Request::post(&url)
                .header("content-type", "application/json")
                .body(BoxBody::full(serde_json::to_vec(&local)?))?;
            let response = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                self.router.transport.request(&node.to_owned(), request),
            )
            .await??;
            ensure!(response.status().is_success(), "roster rejected");
        } else {
            ensure!(local == remote, "conflicting roster epoch");
        }
        Ok(())
    }
    pub async fn refresh(&self, request: RefreshRequest) -> Result<RefreshResponse> {
        let account = self
            .vault
            .get(&request.account_id)
            .context("account not found")?;
        let upstream = self
            .upstreams
            .iter()
            .find(|u| cranibox_contracts::llm::Upstream::provider(u.as_ref()) == account.provider)
            .context("provider unavailable")?;
        Ok(upstream.refresh_record(request).await?)
    }
    pub async fn models(&self) -> Result<Vec<ModelEntry>> {
        let mut models = self.proxy.mgmt.models().await?;
        for model in &mut models {
            let provider = model.id.split_once('/').map_or("", |(p, _)| p);
            if let Some(node) = self.router.candidates(provider).into_iter().next() {
                model.availability = if node == self.vault.node() {
                    Availability::Local
                } else {
                    Availability::ViaNode { node }
                };
            }
        }
        Ok(models)
    }
    /// This device's identity code and roster summary. Once a recovery code exists the
    /// identity is the recovery key, so the code survives owner-key rotations.
    pub fn identity(&self) -> serde_json::Value {
        let owner = self.roster.owner();
        let recovery = self.roster.identity();
        let roster = self.roster.current();
        serde_json::json!({
            "code": identity_code(recovery.as_deref().unwrap_or(&owner)),
            "owner_pub": owner,
            "recovery": recovery.is_some(),
            "recovery_kind": roster.as_ref().and_then(|r| r.owner_cert.as_ref()).map(|c| if c.recovery_salt.is_some() { "passphrase" } else { "code" }),
            "epoch": roster.as_ref().map(|r| r.epoch),
            "members": roster.map_or(0, |r| r.members.len()),
        })
    }
    /// Create this identity's recovery key, either from a passphrase the owner chose
    /// (`Some`, nothing to show) or as a generated code returned once. The recovery
    /// key certifies the current owner key and never stays on any device.
    pub fn create_recovery(&self, passphrase: Option<&str>) -> Result<Option<String>> {
        ensure!(
            self.roster.identity().is_none(),
            "this identity already has a recovery code"
        );
        let (seed, salt, code) = match passphrase {
            Some(passphrase) => {
                check_passphrase(passphrase)?;
                let mut salt = [0u8; 16];
                rand::RngCore::fill_bytes(&mut rand::rng(), &mut salt);
                (
                    passphrase_seed(passphrase, &salt)?,
                    Some(hex_bytes(&salt)),
                    None,
                )
            }
            None => {
                let mut seed = zeroize::Zeroizing::new([0u8; 32]);
                rand::RngCore::fill_bytes(&mut rand::rng(), seed.as_mut());
                let code = recovery_code(&seed);
                (seed, None, Some(code))
            }
        };
        let recovery = recovery_key(&seed);
        let owner = OwnerKey::load_or_create(&self.owner_path)?;
        let roster = self.roster.current().context("no roster")?;
        let cert = sign_cert(&recovery, &owner.public(), 1, salt);
        self.roster.pin_identity(&cert.recovery_pub)?;
        self.roster
            .issue_with(&owner, roster.members, Some(cert), Default::default())?;
        Ok(code)
    }
    /// The recovery key from a generated code or, for a passphrase identity, the passphrase.
    fn recovery_from(&self, secret: &str) -> Result<ed25519_dalek::SigningKey> {
        if let Ok(seed) = parse_recovery_code(secret) {
            return Ok(recovery_key(&seed));
        }
        let salt = self
            .roster
            .current()
            .and_then(|r| r.owner_cert)
            .and_then(|c| c.recovery_salt)
            .context("That is not this identity's recovery code")?;
        let salt = (0..salt.len() / 2)
            .map(|i| u8::from_str_radix(&salt[i * 2..i * 2 + 2], 16))
            .collect::<Result<Vec<u8>, _>>()
            .context("invalid recovery salt")?;
        Ok(recovery_key(&*passphrase_seed(secret, &salt)?))
    }
    /// Lost a device: with the recovery code, replace the owner key, remove `remove`
    /// from the identity and hand the new key to every remaining member.
    pub fn rotate_owner(&self, code: &str, remove: &[String]) -> Result<()> {
        let recovery = self.recovery_from(code)?;
        let recovery_pub = hex32(recovery.verifying_key().as_bytes());
        ensure!(
            self.roster.identity().as_deref() == Some(recovery_pub.as_str()),
            "That recovery code or passphrase does not belong to this identity"
        );
        ensure!(
            !remove.iter().any(|n| n == self.vault.node()),
            "cannot remove this device"
        );
        let roster = self.roster.current().context("no roster")?;
        let serial = roster.owner_cert.as_ref().map_or(0, |c| c.serial) + 1;
        let salt = roster
            .owner_cert
            .as_ref()
            .and_then(|c| c.recovery_salt.clone());
        let staged = self.owner_path.with_extension("key.new");
        let _ = std::fs::remove_file(&staged);
        let key = OwnerKey::load_or_create(&staged)?;
        let cert = sign_cert(&recovery, &key.public(), serial, salt);
        let members: Vec<_> = roster
            .members
            .into_iter()
            .filter(|m| !remove.contains(&m.node_id))
            .collect();
        let mut sealed = std::collections::BTreeMap::new();
        for member in members.iter().filter(|m| m.node_id != self.vault.node()) {
            sealed.insert(
                member.node_id.clone(),
                serde_json::to_value(key.seal_for(member)?)?,
            );
        }
        let removed = self.roster.issue_with(&key, members, Some(cert), sealed)?;
        if self.owner_path.exists() {
            std::fs::rename(
                &self.owner_path,
                self.owner_path.with_extension("key.rotated"),
            )?;
        }
        std::fs::rename(&staged, &self.owner_path)?;
        Config::update(&self.path, |c| {
            for node in &removed {
                c.grants.remove(node);
                c.routes.remove(node);
            }
            Ok(())
        })?;
        self.vault.rotate_for_roster()?;
        Ok(())
    }
    /// Secret this device shares with a fellow roster member: both derive it from the
    /// vault keys listed in the signed roster, so nothing is exchanged.
    fn member_secret(&self, member: &RosterMember) -> Result<String> {
        use sha2::Digest;
        let public: [u8; 32] = (0..32)
            .map(|i| {
                u8::from_str_radix(member.x25519_pub.get(i * 2..i * 2 + 2).unwrap_or("zz"), 16)
            })
            .collect::<Result<Vec<u8>, _>>()?
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid member key"))?;
        let shared = x25519_dalek::StaticSecret::from(*self.vault_private)
            .diffie_hellman(&x25519_dalek::PublicKey::from(public));
        ensure!(shared.was_contributory(), "invalid member key");
        let me = self.vault.node();
        let (a, b) = if me < member.node_id.as_str() {
            (me, member.node_id.as_str())
        } else {
            (member.node_id.as_str(), me)
        };
        let digest = sha2::Sha256::digest(
            [
                b"cranibox-roster-peer-v1\0".as_slice(),
                shared.as_bytes(),
                a.as_bytes(),
                b"/",
                b.as_bytes(),
            ]
            .concat(),
        );
        Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
    }
    /// Every member of this identity trusts every other one: create the missing grant
    /// and route for members this device was not paired with directly.
    async fn trust_members(&self) {
        let Some(roster) = self.roster.current() else {
            return;
        };
        let Ok(config) = Config::read(&self.path) else {
            return;
        };
        for member in roster.members {
            if member.node_id == self.vault.node()
                || (config.grants.contains_key(&member.node_id)
                    && config.routes.contains_key(&member.node_id))
            {
                continue;
            }
            let Some(identity) = member.identity.as_ref() else {
                continue;
            };
            let secret = match self.member_secret(&member) {
                Ok(secret) => secret,
                Err(error) => {
                    eprintln!(
                        "roster: no shared secret with {}: {error:#}",
                        member.node_id
                    );
                    continue;
                }
            };
            if let Err(error) =
                crate::peer::transport_iroh::ensure_roster_peer(&self.path, identity, secret).await
            {
                eprintln!("roster: cannot connect to {}: {error:#}", member.node_id);
            }
        }
    }
    /// This device's own roster member (sent when joining another identity).
    pub fn own_member(&self) -> Option<RosterMember> {
        self.roster.current()?.member(self.vault.node()).cloned()
    }
    /// Inviter: what a SAS-verified owner device needs to join this identity: the next
    /// signed roster (with it added) and the owner key sealed to its vault key. Nothing
    /// changes here until [`Self::commit_invite`], once the device has joined.
    pub fn prepare_invite(
        &self,
        member: RosterMember,
        identity: serde_json::Value,
    ) -> Result<serde_json::Value> {
        ensure!(
            agent_workspace_runtime::protocol::valid_node_id(&member.node_id)
                && member.node_id != self.vault.node(),
            "invalid member"
        );
        let mut member = member;
        member.role = Role::Node;
        member.is_claude_node = false;
        member.added_at = cranibox_llm::auth::now();
        member.name = member.name.chars().take(64).collect();
        member.identity = Some(identity);
        let own = Config::identity(&self.path)
            .ok()
            .and_then(|i| serde_json::to_value(i.public()).ok());
        let me = self.vault.node().to_owned();
        let mut members = self.roster.current().context("no roster")?.members;
        members.retain(|m| m.node_id != member.node_id);
        // Older rosters lack this device's identity; members need it to trust us.
        if let Some(m) = members.iter_mut().find(|m| m.node_id == me)
            && m.identity.is_none()
        {
            m.identity = own;
        }
        members.push(member.clone());
        let owner = OwnerKey::load_or_create(&self.owner_path)?;
        let roster = self.roster.prepare(&owner, members)?;
        let sealed = owner.seal_for(&member)?;
        Ok(serde_json::json!({"owner_pub": roster.owner_pub, "roster": roster, "sealed": sealed}))
    }
    /// Inviter: the device joined with `bundle`; adopt the roster that includes it.
    pub fn commit_invite(&self, bundle: &serde_json::Value) -> Result<()> {
        let roster: cranibox_contracts::roster::Roster =
            serde_json::from_value(bundle["roster"].clone())?;
        ensure!(
            roster.owner_pub == self.roster.owner(),
            "the invitation is for another identity"
        );
        self.install_roster(roster)
    }
    /// Inviter, for devices that join without confirming: prepare and commit at once.
    pub fn invite_bundle(
        &self,
        member: RosterMember,
        identity: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let bundle = self.prepare_invite(member, identity)?;
        self.commit_invite(&bundle)?;
        Ok(bundle)
    }
    /// Joiner: take the identity offered by `inviter` over a SAS-verified pairing.
    /// Returns `false` when this device already belongs to an identity with other
    /// devices (merging two identities is not automatic).
    pub fn join_identity(&self, inviter: &str, bundle: &serde_json::Value) -> Result<bool> {
        let owner = bundle["owner_pub"]
            .as_str()
            .context("missing owner")?
            .to_owned();
        let roster: cranibox_contracts::roster::Roster =
            serde_json::from_value(bundle["roster"].clone())?;
        let sealed: agent_workspace_runtime::peers::roster::SealedOwnerKey =
            serde_json::from_value(bundle["sealed"].clone())?;
        agent_workspace_runtime::peers::roster::verify(&roster, &owner)?;
        let me = self.own_member().context("no own roster member")?;
        ensure!(
            roster
                .member(&me.node_id)
                .is_some_and(|m| m.x25519_pub == me.x25519_pub && m.signing_key == me.signing_key),
            "the offered identity does not include this device"
        );
        ensure!(
            roster.member(inviter).is_some(),
            "the inviter is not in its identity"
        );
        if owner == self.roster.owner() {
            if roster.epoch > self.roster.current().map_or(0, |r| r.epoch) {
                self.install_roster(roster)?;
            }
            return Ok(true);
        }
        if self.roster.current().is_some_and(|r| r.members.len() > 1) {
            return Ok(false);
        }
        let dir = self.path.with_file_name("models");
        let private = cranibox_llm::vault::keys::load_or_create(
            self.vault.node(),
            &dir.join("encryption.key"),
            Some(&cranibox_llm::vault::keys::PlatformKeyStore),
        )?;
        let private: [u8; 32] = private.expose().try_into().context("invalid vault key")?;
        // Keep the previous identity's key aside instead of destroying it.
        if self.owner_path.exists() {
            std::fs::rename(
                &self.owner_path,
                self.owner_path.with_extension("key.previous"),
            )?;
        }
        if let Err(error) =
            OwnerKey::install_sealed(&self.owner_path, &owner, &me.node_id, &private, &sealed)
        {
            let _ = std::fs::rename(
                self.owner_path.with_extension("key.previous"),
                &self.owner_path,
            );
            return Err(error);
        }
        self.roster.adopt(&owner, roster)?;
        self.vault.rotate_for_roster()?;
        Ok(true)
    }
    pub fn install_roster(&self, roster: cranibox_contracts::roster::Roster) -> Result<()> {
        let removed_self = roster.member(self.vault.node()).is_none();
        let new_owner = (roster.owner_pub != self.roster.owner()).then(|| roster.owner_pub.clone());
        let sealed = roster.owner_keys.get(self.vault.node()).cloned();
        let removed = self.roster.install(roster)?;
        // After a certified rotation, take the new owner key sealed to this device.
        if let (Some(owner), Some(sealed)) = (new_owner, sealed) {
            let sealed: agent_workspace_runtime::peers::roster::SealedOwnerKey =
                serde_json::from_value(sealed)?;
            let aside = self.owner_path.with_extension("key.rotated");
            if self.owner_path.exists() {
                std::fs::rename(&self.owner_path, &aside)?;
            }
            if let Err(error) = OwnerKey::install_sealed(
                &self.owner_path,
                &owner,
                self.vault.node(),
                &self.vault_private,
                &sealed,
            ) {
                let _ = std::fs::rename(&aside, &self.owner_path);
                eprintln!("roster: could not take the rotated owner key: {error:#}");
            }
        }
        Config::update(&self.path, |c| {
            for node in &removed {
                c.grants.remove(node);
                c.routes.remove(node);
            }
            Ok(())
        })?;
        if removed_self {
            // No longer a member: put the owner key aside so this device cannot sign
            // the identity's roster any more.
            if self.owner_path.exists() {
                std::fs::rename(
                    &self.owner_path,
                    self.owner_path.with_extension("key.removed"),
                )?;
            }
            self.stop.cancel();
        } else {
            self.vault.rotate_for_roster()?;
        }
        Ok(())
    }
    pub fn edit_members(
        &self,
        edit: impl FnOnce(&mut Vec<RosterMember>) -> Result<()>,
    ) -> Result<()> {
        let mut roster = self.roster.current().context("no roster")?;
        edit(&mut roster.members)?;
        let owner = OwnerKey::load_or_create(&self.owner_path)?;
        let removed = self.roster.issue(&owner, roster.members)?;
        Config::update(&self.path, |c| {
            for node in &removed {
                c.grants.remove(node);
                c.routes.remove(node);
            }
            Ok(())
        })?;
        self.vault.rotate_for_roster()?;
        Ok(())
    }
    pub fn directory(&self) -> &Path {
        self.owner_path.parent().expect("owner directory")
    }
}
impl Drop for Models {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn local_service_bootstraps_from_config_identity_and_keeps_vault_key() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("peers.json");
        let config = Config::update(&path, |_| Ok(())).unwrap();
        let stop = CancellationToken::new();
        let service = Models::start_on(
            path.clone(),
            stop.clone(),
            Some(vec!["127.0.0.1:0".parse().unwrap()]),
        )
        .await
        .unwrap();
        let roster = service.roster.current().unwrap();
        let local = roster.member(&config.node_id).unwrap();
        assert_eq!(
            local.signing_key,
            config.identity.as_ref().unwrap().signing_key
        );
        assert_eq!(service.vault.node(), config.node_id);
        let response = reqwest::get(format!("http://{}/v1/models", service.proxy.addresses[0]))
            .await
            .unwrap();
        assert!(response.status().is_success());
        service.proxy.mgmt.status().await.unwrap();
        let key = local.x25519_pub.clone();
        drop(service);
        stop.cancel();
        tokio::task::yield_now().await;
        let restarted = Models::start_on(
            path,
            CancellationToken::new(),
            Some(vec!["127.0.0.1:0".parse().unwrap()]),
        )
        .await
        .unwrap();
        assert_eq!(
            restarted
                .roster
                .current()
                .unwrap()
                .member(&config.node_id)
                .unwrap()
                .x25519_pub,
            key
        );
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    async fn device(root: &Path) -> (Arc<Models>, String) {
        let path = root.join("peers.json");
        let config = Config::update(&path, |_| Ok(())).unwrap();
        let service = Models::start_on(
            path,
            CancellationToken::new(),
            Some(vec!["127.0.0.1:0".parse().unwrap()]),
        )
        .await
        .unwrap();
        (service, config.node_id)
    }
    fn public(models: &Models) -> serde_json::Value {
        serde_json::to_value(Config::identity(&models.path).unwrap().public()).unwrap()
    }

    #[tokio::test]
    async fn an_invitation_changes_the_inviter_only_after_the_device_joined() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let ((inviter, inviter_id), (joiner, joiner_id)) =
            (device(a.path()).await, device(b.path()).await);
        let before = inviter.roster.current().unwrap();
        let bundle = inviter
            .prepare_invite(joiner.own_member().unwrap(), public(&joiner))
            .unwrap();
        assert_eq!(
            inviter.roster.current().unwrap(),
            before,
            "nothing committed yet"
        );
        assert!(joiner.join_identity(&inviter_id, &bundle).unwrap());
        inviter.commit_invite(&bundle).unwrap();
        let (left, right) = (
            inviter.roster.current().unwrap(),
            joiner.roster.current().unwrap(),
        );
        assert_eq!(left, right, "both devices hold the same identity");
        assert_eq!(left.epoch, before.epoch + 1);
        assert!(left.member(&joiner_id).is_some() && left.member(&inviter_id).is_some());
        assert_eq!(joiner.roster.owner(), inviter.roster.owner());
        // A bundle is committed once; a stale one is refused.
        assert!(inviter.commit_invite(&bundle).is_err());

        // A device whose identity already includes others keeps it, and an inviter that
        // never commits stays unchanged.
        let c = tempfile::tempdir().unwrap();
        let (other, _) = device(c.path()).await;
        let unchanged = other.roster.current().unwrap();
        let offer = other
            .prepare_invite(joiner.own_member().unwrap(), public(&joiner))
            .unwrap();
        assert!(!joiner.join_identity(other.vault.node(), &offer).unwrap());
        assert_eq!(other.roster.current().unwrap(), unchanged);
        assert_eq!(joiner.roster.owner(), inviter.roster.owner());
    }

    #[tokio::test]
    async fn a_chosen_passphrase_is_the_recovery_key_and_survives_rotation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("peers.json");
        Config::update(&path, |_| Ok(())).unwrap();
        let service = Models::start_on(
            path,
            CancellationToken::new(),
            Some(vec!["127.0.0.1:0".parse().unwrap()]),
        )
        .await
        .unwrap();
        assert!(service.create_recovery(Some("short")).is_err());
        assert!(service.create_recovery(Some("aaaaaaaaaaaaaaaa")).is_err());
        let passphrase = "correct horse battery";
        assert_eq!(service.create_recovery(Some(passphrase)).unwrap(), None);
        let identity = service.identity();
        assert_eq!(identity["recovery_kind"], "passphrase");
        let code = identity["code"].clone();
        assert!(service.create_recovery(None).is_err(), "set once");
        assert!(service.rotate_owner("correct horse batterY", &[]).is_err());
        let owner = service.roster.owner();
        // Surrounding spaces are ignored, as when pasted.
        service
            .rotate_owner(&format!(" {passphrase} "), &[])
            .unwrap();
        assert_ne!(service.roster.owner(), owner, "the owner key was replaced");
        // The identity is the recovery key, so its code stays; the salt carries over.
        assert_eq!(service.identity()["code"], code);
        service.rotate_owner(passphrase, &[]).unwrap();
    }

    #[test]
    fn recovery_codes_round_trip_and_catch_typos() {
        let seed = [9u8; 32];
        let code = recovery_code(&seed);
        assert_eq!(code.split('-').count(), 14);
        assert_eq!(*parse_recovery_code(&code.to_lowercase()).unwrap(), seed);
        let mut typo: Vec<char> = code.chars().collect();
        typo[0] = if typo[0] == 'A' { 'B' } else { 'A' };
        assert!(parse_recovery_code(&typo.into_iter().collect::<String>()).is_err());
        assert!(parse_recovery_code("not a code").is_err());
    }

    #[test]
    fn identity_code_is_stable_and_short() {
        let code = identity_code(&"ab".repeat(32));
        assert!(code.starts_with("cbo-") && code.len() == 23);
        assert_eq!(code, identity_code(&"ab".repeat(32)));
        assert_ne!(code, identity_code(&"cd".repeat(32)));
    }
}
