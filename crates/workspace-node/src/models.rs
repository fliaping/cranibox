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
                    name: std::env::var("HOSTNAME").unwrap_or_else(|_| "My device".into()),
                    added_at: cranibox_llm::auth::now(),
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
        });
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
        // Verify even an equal epoch; a peer may never introduce a different owner.
        agent_workspace_runtime::peers::roster::verify(&remote, &local.owner_pub)?;
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
    pub fn install_roster(&self, roster: cranibox_contracts::roster::Roster) -> Result<()> {
        let removed_self = roster.member(self.vault.node()).is_none();
        let removed = self.roster.install(roster)?;
        Config::update(&self.path, |c| {
            for node in &removed {
                c.grants.remove(node);
                c.routes.remove(node);
            }
            Ok(())
        })?;
        if removed_self {
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
