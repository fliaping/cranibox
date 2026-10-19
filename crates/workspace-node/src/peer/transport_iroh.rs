//! iroh default transport of a node (WS1, `docs/design/tasks/01-iroh-network.md`).
//!
//! Started next to the other network services. It binds one iroh endpoint with
//! the node's signing key, accepts `cranibox/peer-http/1` from paired keys and
//! hands every stream to this node's mesh listener, keeps one loopback forward
//! per paired peer (the first path of its route), and runs link pairing.
//!
//! Until the Phase 0 `mesh_router()` export and the `Authenticated` check in
//! `auth` exist (CONTRACT-REQUEST in the PR), streams are spliced to a
//! loopback `NetworkEndpoint` listener, so the existing bearer / Noise
//! credentials still run inside the QUIC channel.
use super::config::{Config, Endpoint, Grant, Inbound, Permissions, Pinned, Route};
use agent_workspace_runtime::{
    config as secrets,
    peers::{
        identity::{self, PublicIdentity},
        local_forward::Lease,
    },
};
use anyhow::{Context, Result, ensure};
use cranibox_contracts::{
    app::{PairAcceptReq, PairAccepted, PairConfirmReq, PairInvite, PairInviteReq},
    config::{IROH_SECTION, IrohConfig, NETWORK_FILE},
    net::{IrohAddr, PairPurpose},
};
use cranibox_net::{
    Allowlist, IrohNode, Options, addr,
    forward::Forward,
    mesh::{self, MeshTarget},
    pair::{FlowState, FlowView, Outcome, PairHost, Pairing, Side},
    relay::Relay,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

/// Peers learned from pairing (addresses and stable forward ports).
pub const LEARNED_FILE: &str = "iroh-peers.json";
/// Local control socket (0600) for the CLI and the App API back end.
pub const CONTROL_SOCKET: &str = "cranibox-net.sock";
const MAINTAIN_EVERY: Duration = Duration::from_secs(2);
const MIGRATE_RETRY: Duration = Duration::from_secs(600);

/// Hub-only settings handed over by `cranibox serve` before the local node starts.
#[derive(Clone)]
pub struct HubOptions {
    pub allow: Allowlist,
    pub relay: Option<Relay>,
    /// `--public-url`: own relay URL, ticket web links and the relay self-check.
    pub public_url: Option<String>,
}
static HUB: OnceLock<Mutex<Option<HubOptions>>> = OnceLock::new();
/// Called by `cranibox serve` before `LocalNode::start_with`.
pub fn configure_hub(options: HubOptions) {
    *HUB.get_or_init(Default::default).lock().unwrap() = Some(options);
}
fn hub_options() -> Option<HubOptions> {
    HUB.get().and_then(|h| h.lock().unwrap().clone())
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Learned {
    #[serde(default)]
    version: u32,
    /// Hub relays learned from tickets; used when `networks.json` sets none.
    #[serde(default)]
    relays: Vec<String>,
    #[serde(default)]
    peers: BTreeMap<String, LearnedPeer>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct LearnedPeer {
    addr: IrohAddr,
    #[serde(default)]
    port: u16,
}
impl Learned {
    fn path(peers: &Path) -> PathBuf {
        peers.with_file_name(LEARNED_FILE)
    }
    fn read(peers: &Path) -> Self {
        std::fs::read(Self::path(peers))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    fn write(&self, peers: &Path) -> Result<()> {
        let mut value = self.clone();
        value.version = 1;
        secrets::write_private(
            &Self::path(peers),
            &serde_json::to_vec_pretty(&value)?,
            true,
        )
    }
}

/// iroh settings: `networks.json` `"iroh"` section (C5), plus advanced env overrides.
fn iroh_config(peers: &Path) -> Result<IrohConfig> {
    let path = peers.with_file_name(NETWORK_FILE);
    let Ok(bytes) = std::fs::read(&path) else {
        return Ok(IrohConfig::default());
    };
    let value: Value = serde_json::from_slice(&bytes).context("invalid networks.json")?;
    match value.get(IROH_SECTION) {
        Some(section) => serde_json::from_value(section.clone()).context("invalid iroh settings"),
        None => Ok(IrohConfig::default()),
    }
}

fn lock_path(peers: &Path, node: &str) -> Result<PathBuf> {
    ensure!(
        agent_workspace_runtime::protocol::valid_node_id(node),
        "invalid node ID"
    );
    let parent = peers
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(std::fs::canonicalize(parent)?.join(format!(".iroh-{node}.lock")))
}
fn token_path(peers: &Path, node: &str) -> Result<PathBuf> {
    let parent = peers
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(std::fs::canonicalize(parent)?.join(format!("iroh-{node}.token")))
}
fn endpoint_of(identity: &PublicIdentity) -> Option<iroh::EndpointId> {
    addr::parse_id(&identity.signing_key).ok()
}

struct Shared {
    path: PathBuf,
    node: IrohNode,
    /// Learned hub relays are adopted at runtime unless relays were configured.
    adopt_learned: bool,
    forwards: tokio::sync::Mutex<HashMap<String, (Forward, Lease)>>,
    migrate_tried: Mutex<HashMap<String, Instant>>,
}

impl Shared {
    /// Bind (or re-bind) the loopback forward of `node` and make it the route's first path.
    async fn ensure_forward(&self, node: &str, entry: &mut LearnedPeer) -> Result<bool> {
        let target = addr::endpoint_addr(&entry.addr)?;
        let mut forwards = self.forwards.lock().await;
        let mut changed = false;
        if let Some((forward, _)) = forwards.get(node) {
            forward.update(target);
        } else {
            let forward = match Forward::bind(self.node.clone(), entry.port, target.clone()).await {
                Ok(forward) => forward,
                Err(_) if entry.port != 0 => Forward::bind(self.node.clone(), 0, target).await?,
                Err(error) => return Err(error),
            };
            let lease = Lease::acquire(&lock_path(&self.path, node)?)?;
            if entry.port != forward.address.port() {
                entry.port = forward.address.port();
                changed = true;
            }
            forwards.insert(node.to_owned(), (forward, lease));
        }
        drop(forwards);
        let url = format!("http://127.0.0.1:{}", entry.port);
        let iroh_addr = entry.addr.clone();
        let lock = lock_path(&self.path, node)?;
        let present = Config::read(&self.path)?
            .routes
            .get(node)
            .is_some_and(|r| r.url == url);
        if !present {
            let path = self.path.clone();
            let node = node.to_owned();
            tokio::task::spawn_blocking(move || {
                Config::update(&path, |config| {
                    let Some(route) = config.routes.get_mut(&node) else {
                        return Ok(());
                    };
                    let mut endpoints: Vec<Endpoint> = route
                        .endpoints()
                        .into_iter()
                        .filter(|e| e.local_forward_lock.as_ref() != Some(&lock))
                        .collect();
                    endpoints.insert(
                        0,
                        Endpoint {
                            url,
                            allow_insecure: false,
                            local_forward_lock: Some(lock),
                            iroh: Some(iroh_addr),
                        },
                    );
                    endpoints.truncate(super::config::MAX_PATHS);
                    let first = endpoints.remove(0);
                    route.url = first.url;
                    route.allow_insecure = first.allow_insecure;
                    route.local_forward_lock = first.local_forward_lock;
                    route.alternatives = endpoints;
                    Ok(())
                })
            })
            .await??;
        }
        Ok(changed)
    }

    async fn drop_forward(&self, node: &str) {
        if let Some((forward, lease)) = self.forwards.lock().await.remove(node) {
            self.node.drop_peer(&forward.target().id).await;
            drop(forward);
            drop(lease);
        }
    }

    /// One maintenance pass: allowlist, forwards, migration of existing routes.
    async fn maintain(&self, allow: &Allowlist) -> Result<()> {
        let config = Config::read(&self.path)?;
        let mut ids = vec![self.node.id()];
        for grant in config.grants.values() {
            if let Some(id) = grant.peer.as_ref().and_then(|p| endpoint_of(&p.identity)) {
                ids.push(id);
            }
        }
        for route in config.routes.values() {
            if let Some(id) = route
                .target_identity
                .as_ref()
                .and_then(|p| endpoint_of(&p.identity))
            {
                ids.push(id);
            }
        }
        for removed in allow.set(ids) {
            self.node.drop_peer(&removed).await;
        }
        let mut learned = Learned::read(&self.path);
        let mut dirty = false;
        let nodes: Vec<String> = learned.peers.keys().cloned().collect();
        for node in nodes {
            let paired = config.routes.contains_key(&node) || config.grants.contains_key(&node);
            if !paired {
                self.drop_forward(&node).await;
                learned.peers.remove(&node);
                dirty = true;
                continue;
            }
            if !config.routes.contains_key(&node) {
                continue;
            }
            let entry = learned.peers.get_mut(&node).expect("listed");
            match self.ensure_forward(&node, entry).await {
                Ok(changed) => dirty |= changed,
                Err(error) => eprintln!("iroh forward for {node}: {error:#}"),
            }
        }
        // Migration (opt-in, CRANIBOX_IROH_MIGRATE=1): a route with a pinned key whose
        // peer answers on iroh gets iroh first. Off by default because changing a
        // route changes its binding, which cancels queued messages and tasks to it.
        let migrate = std::env::var_os("CRANIBOX_IROH_MIGRATE").is_some();
        for (node, route) in config.routes.iter().filter(|_| migrate) {
            if learned.peers.contains_key(node) {
                continue;
            }
            let Some(id) = route
                .target_identity
                .as_ref()
                .and_then(|p| endpoint_of(&p.identity))
            else {
                continue;
            };
            {
                let mut tried = self.migrate_tried.lock().unwrap();
                if tried
                    .get(node)
                    .is_some_and(|at| at.elapsed() < MIGRATE_RETRY)
                {
                    continue;
                }
                tried.insert(node.clone(), Instant::now());
            }
            let mut target = iroh::EndpointAddr::new(id);
            for url in self.node.own_relays() {
                target = target.with_relay_url(url);
            }
            let probe =
                tokio::time::timeout(Duration::from_secs(8), self.node.peer_connection(&target))
                    .await;
            if matches!(probe, Ok(Ok(_))) {
                eprintln!("iroh: {node} answers on iroh; using it as the first path");
                learned.peers.insert(
                    node.clone(),
                    LearnedPeer {
                        addr: addr::iroh_addr(&target),
                        port: 0,
                    },
                );
                dirty = true;
            }
        }
        if dirty {
            learned.write(&self.path)?;
        }
        Ok(())
    }
}

/// Persists confirmed pairings into `peers.json` (grant and/or route).
struct Host {
    shared: Arc<Shared>,
    identity: PublicIdentity,
    allow: Allowlist,
}
fn owner_permissions() -> Permissions {
    Permissions {
        exec: true,
        terminals: true,
        projects: true,
        desktop: true,
        computer: true,
        browser: true,
        messages: true,
        activity: true,
        spaces: true,
        // Own devices share the model proxy and the account vault (WS6 still
        // checks roster membership).
        llm: true,
        vault: true,
    }
}
#[async_trait::async_trait]
impl PairHost for Host {
    fn identity(&self) -> Value {
        serde_json::to_value(&self.identity).expect("identity serializes")
    }
    fn verify(&self, peer: &Value, endpoint: &iroh::EndpointId) -> Result<(String, String)> {
        let peer: PublicIdentity =
            serde_json::from_value(peer.clone()).context("invalid device identity")?;
        peer.verify()?;
        ensure!(
            peer.signing_key == addr::hex_id(endpoint),
            "the device identity does not belong to the connected key"
        );
        ensure!(
            peer.node_id != self.identity.node_id,
            "cannot pair with itself"
        );
        Ok((
            peer.node_id.clone(),
            identity::pairing_code(&self.identity, &peer),
        ))
    }
    async fn apply(&self, outcome: Outcome) -> Result<()> {
        let peer: PublicIdentity = serde_json::from_value(outcome.peer_identity.clone())?;
        let node = outcome.peer_node_id.clone();
        let path = self.shared.path.clone();
        let mut learned_peer = LearnedPeer {
            addr: outcome.peer_addr.clone(),
            port: 0,
        };
        // Route side: bind the forward first; the lease must be live before it is saved.
        let forward = if outcome.received.is_some() {
            let target = addr::endpoint_addr(&learned_peer.addr)?;
            let forward = Forward::bind(self.shared.node.clone(), 0, target).await?;
            let lease = Lease::acquire(&lock_path(&path, &node)?)?;
            learned_peer.port = forward.address.port();
            Some((forward, lease))
        } else {
            None
        };
        let token = match &outcome.received {
            Some(secret) => {
                let file = token_path(&path, &node)?;
                secrets::write_private(&file, secret.as_bytes(), true)?;
                Some(file)
            }
            None => None,
        };
        let mut pinned = Pinned::new(peer.clone());
        // The owner compared the pairing code during this flow.
        pinned.verified = true;
        let issued = outcome.issued.clone();
        let purpose = outcome.purpose;
        let lock = lock_path(&path, &node)?;
        let port = learned_peer.port;
        let update_path = path.clone();
        let update_node = node.clone();
        tokio::task::spawn_blocking(move || {
            Config::update(&update_path, |config| {
                if let Some(secret) = &issued {
                    let permissions = match purpose {
                        PairPurpose::OwnerDevice => owner_permissions(),
                        PairPurpose::Guest => Permissions {
                            messages: true,
                            ..Permissions::default()
                        },
                    };
                    let mut grant = Grant::issue(secret, permissions);
                    grant.peer = Some(pinned.clone());
                    grant.inbound = match purpose {
                        PairPurpose::OwnerDevice => Inbound::Accept,
                        PairPurpose::Guest => Inbound::Hold,
                    };
                    config.grants.insert(update_node.clone(), grant);
                }
                if let Some(token_file) = token {
                    let mut alternatives = Vec::new();
                    if let Some(old) = config.routes.remove(&update_node) {
                        alternatives = old
                            .endpoints()
                            .into_iter()
                            .filter(|e| e.local_forward_lock.as_ref() != Some(&lock))
                            .take(super::config::MAX_PATHS - 1)
                            .collect();
                    }
                    config.routes.insert(
                        update_node.clone(),
                        Route {
                            url: format!("http://127.0.0.1:{port}"),
                            token_file,
                            allow_insecure: false,
                            local_forward_lock: Some(lock),
                            alternatives,
                            target_identity: Some(pinned),
                            config_path: None,
                        },
                    );
                }
                Ok(())
            })
        })
        .await??;
        if let Some(id) = endpoint_of(&peer) {
            self.allow.insert(id);
        }
        if let Some(forward) = forward {
            self.shared
                .forwards
                .lock()
                .await
                .insert(node.clone(), forward);
        }
        let mut learned = Learned::read(&path);
        learned.peers.insert(node, learned_peer);
        if outcome.side == Side::Joiner
            && let Some(relay) = &outcome.peer_addr.relay_url
            && !relay.contains(".iroh.link")
            && !learned.relays.contains(relay)
        {
            learned.relays.insert(0, relay.clone());
            learned.relays.truncate(4);
            if self.shared.adopt_learned
                && let Err(error) = self.shared.node.adopt_relay(relay).await
            {
                eprintln!("iroh: cannot use relay {relay}: {error:#}");
            }
        }
        learned.write(&path)?;
        Ok(())
    }
}

/// Running iroh transport; [`Handle`] is what the App API (WS7) calls.
pub struct Service {
    handle: Option<Handle>,
    stop: CancellationToken,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    _listener: Option<super::server::NetworkListener>,
}

#[derive(Clone)]
pub struct Handle {
    shared: Arc<Shared>,
    pairing: Pairing,
    public_url: Option<String>,
    allow: Allowlist,
}

impl Service {
    /// `router` is the node's `Running::mesh_router()`; it serves ALPN
    /// `cranibox/peer-http/1` with `Authenticated`. `endpoint` serves forwards
    /// (`cranibox/peer-fwd/1`), which keep the route's bearer/Noise credential.
    pub(crate) async fn start(
        path: PathBuf,
        endpoint: super::server::NetworkEndpoint,
        router: axum::Router,
        stop: CancellationToken,
    ) -> Self {
        let stop = stop.child_token();
        match Self::try_start(path, endpoint, router, stop.clone()).await {
            Ok(service) => service,
            Err(error) => {
                eprintln!("iroh network disabled: {error:#}");
                Self {
                    handle: None,
                    stop,
                    tasks: Vec::new(),
                    _listener: None,
                }
            }
        }
    }

    async fn try_start(
        path: PathBuf,
        endpoint: super::server::NetworkEndpoint,
        router: axum::Router,
        stop: CancellationToken,
    ) -> Result<Self> {
        let config = iroh_config(&path)?;
        ensure!(config.enabled, "turned off in networks.json");
        let hub = hub_options();
        let learned = Learned::read(&path);
        let mut options = Options::from_config(&config);
        let mut adopt_learned = options.relays.is_empty();
        if options.relays.is_empty() {
            options.relays = learned.relays.clone();
        }
        if let Some(url) = hub.as_ref().and_then(|h| h.public_url.clone()).filter(|u| {
            u.starts_with("https://") && hub.as_ref().is_some_and(|h| h.relay.is_some())
        }) {
            options.relays.retain(|r| r != &url);
            options.relays.insert(0, url);
        }
        // Advanced / test overrides (never shown in the UI).
        options.relay_only = std::env::var_os("CRANIBOX_IROH_RELAY_ONLY").is_some();
        options.no_address_lookup = std::env::var_os("CRANIBOX_IROH_NO_LOOKUP").is_some();
        if let Some(file) = std::env::var_os("CRANIBOX_IROH_EXTRA_ROOT") {
            options
                .extra_roots
                .push(std::fs::read(file).context("read extra relay root")?);
        }
        if let Some(extra) = std::env::var_os("CRANIBOX_IROH_RELAYS") {
            adopt_learned = false;
            options.relays = extra
                .to_string_lossy()
                .split(',')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
        }
        let peers = Config::read(&path)?;
        let identity = Config::identity(&path)?;
        ensure!(
            identity.node_id() == peers.node_id,
            "node identity mismatch"
        );
        let node = IrohNode::bind(&identity.signing_seed(), options.clone()).await?;
        let listener = endpoint.listen("127.0.0.1:0".parse()?).await?;
        let forwarded = MeshTarget::Tcp(listener.address());
        let resolve_path = path.clone();
        let resolve: mesh::Resolve = Arc::new(move |id: &iroh::EndpointId| {
            let key = addr::hex_id(id);
            let config = Config::read(&resolve_path).ok()?;
            config
                .grants
                .iter()
                .find(|(_, grant)| {
                    grant
                        .peer
                        .as_ref()
                        .is_some_and(|p| p.identity.signing_key.eq_ignore_ascii_case(&key))
                })
                .map(|(node, _)| node.clone())
        });
        let target = MeshTarget::Router { router, resolve };
        let allow = hub
            .as_ref()
            .map_or_else(Allowlist::default, |h| h.allow.clone());
        allow.insert(node.id());
        let shared = Arc::new(Shared {
            path: path.clone(),
            node: node.clone(),
            adopt_learned,
            forwards: Default::default(),
            migrate_tried: Default::default(),
        });
        let host = Arc::new(Host {
            shared: shared.clone(),
            identity: identity.public().clone(),
            allow: allow.clone(),
        });
        let hub_relay = hub.as_ref().and_then(|h| h.relay.clone());
        let is_hub = hub_relay.is_some();
        let pairing = Pairing::new(
            node.clone(),
            peers.node_id.clone(),
            host,
            hub_relay,
            is_hub.then(|| node.addr()),
        );
        let handle = Handle {
            shared: shared.clone(),
            pairing: pairing.clone(),
            public_url: hub.as_ref().and_then(|h| h.public_url.clone()),
            allow: allow.clone(),
        };
        eprintln!(
            "iroh endpoint {} (node {}) relays: {}",
            node.id(),
            peers.node_id,
            if options.relays.is_empty() {
                "n0 public".to_owned()
            } else {
                format!(
                    "{:?}{}",
                    options.relays,
                    if options.n0_fallback {
                        " + n0 fallback"
                    } else {
                        ""
                    }
                )
            }
        );
        let mut tasks = Vec::new();
        tasks.push(tokio::spawn(accept(
            node.clone(),
            allow.clone(),
            target,
            forwarded,
            pairing,
            stop.clone(),
        )));
        let maintain_shared = shared.clone();
        let maintain_allow = allow.clone();
        let maintain_stop = stop.clone();
        tasks.push(tokio::spawn(async move {
            let mut interval = tokio::time::interval(MAINTAIN_EVERY);
            loop {
                tokio::select! { _ = maintain_stop.cancelled() => break, _ = interval.tick() => {} }
                if let Err(error) = maintain_shared.maintain(&maintain_allow).await {
                    eprintln!("iroh maintenance: {error:#}");
                }
            }
        }));
        #[cfg(unix)]
        match control::serve(
            path.with_file_name(CONTROL_SOCKET),
            handle.clone(),
            stop.clone(),
        ) {
            Ok(task) => tasks.push(task),
            Err(error) => eprintln!("iroh control socket: {error:#}"),
        }
        Ok(Self {
            handle: Some(handle),
            stop,
            tasks,
            _listener: Some(listener),
        })
    }

    pub fn handle(&self) -> Option<Handle> {
        self.handle.clone()
    }

    pub async fn shutdown(self) {
        self.stop.cancel();
        if let Some(handle) = &self.handle {
            handle.shared.forwards.lock().await.clear();
            handle.shared.node.close().await;
        }
        for task in self.tasks {
            let _ = task.await;
        }
        if let Some(listener) = self._listener {
            listener.shutdown().await;
        }
    }
}

async fn accept(
    node: IrohNode,
    allow: Allowlist,
    target: MeshTarget,
    forwarded: MeshTarget,
    pairing: Pairing,
    stop: CancellationToken,
) {
    let endpoint = node.endpoint().clone();
    loop {
        let incoming = tokio::select! {
            _ = stop.cancelled() => break,
            incoming = endpoint.accept() => incoming,
        };
        let Some(incoming) = incoming else { break };
        let allow = allow.clone();
        let target = target.clone();
        let forwarded = forwarded.clone();
        let pairing = pairing.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            let Ok(conn) = incoming.await else { return };
            let result = if conn.alpn() == cranibox_net::ALPN_PAIR {
                pairing.handle(conn).await
            } else if conn.alpn() == cranibox_net::ALPN_PEER_HTTP {
                mesh::serve_connection(conn, &allow, target, stop).await
            } else if conn.alpn() == cranibox_net::ALPN_PEER_FWD {
                mesh::serve_connection(conn, &allow, forwarded, stop).await
            } else {
                Ok(())
            };
            if let Err(error) = result {
                eprintln!("iroh: {error:#}");
            }
        });
    }
}

impl Handle {
    pub fn node_id(&self) -> String {
        self.shared.node.id().to_string()
    }
    /// `POST /api/v1/app/pair/invite` back end (C6).
    pub fn pair_invite(&self, req: PairInviteReq) -> Result<PairInvite> {
        let invitation = self.pairing.invite(
            req.purpose,
            self.public_url
                .as_deref()
                .filter(|u| u.starts_with("https://")),
        )?;
        Ok(PairInvite {
            link: invitation.link,
            web_link: invitation.web_link,
            qr_svg: invitation.qr_svg,
            expires_at: invitation.ticket.expires_at,
        })
    }
    /// `POST /api/v1/app/pair/accept` back end (C6).
    pub async fn pair_accept(&self, req: PairAcceptReq) -> Result<PairAccepted> {
        let flow = self.pairing.accept(&req.link).await?;
        Ok(PairAccepted {
            flow_id: flow.flow_id,
            sas: flow.sas,
        })
    }
    /// `POST /api/v1/app/pair/{flow}/confirm` back end (C6).
    pub async fn pair_confirm(&self, flow: &str, req: PairConfirmReq) -> Result<FlowState> {
        self.pairing.confirm(flow, req.matches).await
    }
    /// Pending and recent flows (pairing approval cards on the inviter).
    pub fn pair_flows(&self) -> Vec<FlowView> {
        self.pairing.flows()
    }
    pub async fn status(&self) -> Value {
        let targets: Vec<_> = self
            .shared
            .forwards
            .lock()
            .await
            .iter()
            .map(|(node, (forward, _))| (node.clone(), forward.address, forward.target().id))
            .collect();
        let mut peers = vec![];
        for (node, address, id) in targets {
            let path = self.shared.node.pool_path(&id).await;
            peers.push(json!({"node": node, "forward": address.to_string(), "path": path}));
        }
        json!({
            "endpoint_id": self.shared.node.id().to_string(),
            "addr": self.shared.node.addr(),
            "relays": self.shared.node.relays_connected(),
            "allowlist": self.allow.len(),
            "open_invitations": self.pairing.open_invitations(),
            "peers": peers,
        })
    }
}

#[cfg(unix)]
pub(crate) mod control {
    //! `cranibox-net.sock`: HTTP/1.1 + JSON, owner-only (0600), for the CLI.
    use super::*;
    use axum::{
        Json, Router,
        extract::{Path as UrlPath, State},
        http::StatusCode,
        routing::{get, post},
    };

    fn fail(error: anyhow::Error) -> (StatusCode, Json<Value>) {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("{error:#}")})),
        )
    }

    pub(crate) fn serve(
        socket: PathBuf,
        handle: Handle,
        stop: CancellationToken,
    ) -> Result<tokio::task::JoinHandle<()>> {
        let _ = std::fs::remove_file(&socket);
        let listener = tokio::net::UnixListener::bind(&socket)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        let router = Router::new()
            .route(
                "/status",
                get(|State(h): State<Handle>| async move { Json(h.status().await) }),
            )
            .route(
                "/pair/invite",
                post(
                    |State(h): State<Handle>, Json(req): Json<PairInviteReq>| async move {
                        h.pair_invite(req).map(Json).map_err(fail)
                    },
                ),
            )
            .route(
                "/pair/accept",
                post(
                    |State(h): State<Handle>, Json(req): Json<PairAcceptReq>| async move {
                        h.pair_accept(req).await.map(Json).map_err(fail)
                    },
                ),
            )
            .route(
                "/pair/flows",
                get(|State(h): State<Handle>| async move { Json(h.pair_flows()) }),
            )
            .route(
                "/pair/{flow}/confirm",
                post(
                    |State(h): State<Handle>,
                     UrlPath(flow): UrlPath<String>,
                     Json(req): Json<PairConfirmReq>| async move {
                        h.pair_confirm(&flow, req)
                            .await
                            .map(|state| Json(json!({"state": state})))
                            .map_err(fail)
                    },
                ),
            )
            .with_state(handle);
        Ok(tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(stop.cancelled_owned())
                .await;
            let _ = std::fs::remove_file(&socket);
        }))
    }

    /// Minimal client used by `cranibox-node peer pair …`.
    pub async fn call(
        peers: &Path,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let socket = peers.with_file_name(CONTROL_SOCKET);
        let mut stream = tokio::net::UnixStream::connect(&socket).await.context(
            "the Cranibox node service is not running (start it with `cranibox-node peer serve`)",
        )?;
        let body = body.map(|b| b.to_string()).unwrap_or_default();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nhost: localhost\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).await?;
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(120), stream.read_to_end(&mut response))
            .await
            .context("the node service did not answer")??;
        let text = String::from_utf8_lossy(&response);
        let (head, payload) = text
            .split_once("\r\n\r\n")
            .context("invalid control response")?;
        let ok = head.starts_with("HTTP/1.1 2");
        let payload = if head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
        {
            dechunk(payload)
        } else {
            payload.to_owned()
        };
        let value: Value = serde_json::from_str(&payload).unwrap_or(Value::String(payload));
        ensure!(
            ok,
            "{}",
            value["error"].as_str().unwrap_or("request failed")
        );
        Ok(value)
    }
    fn dechunk(body: &str) -> String {
        let mut out = String::new();
        let mut rest = body;
        while let Some((size, tail)) = rest.split_once("\r\n") {
            let Ok(n) = usize::from_str_radix(size.trim(), 16) else {
                break;
            };
            if n == 0 || tail.len() < n {
                break;
            }
            out.push_str(&tail[..n]);
            rest = tail[n..].trim_start_matches("\r\n");
        }
        out
    }
}

/// `cranibox-node peer pair …`
#[derive(Debug, clap::Subcommand)]
pub enum PairCommand {
    /// Create a one-time pairing link and QR code (15 minutes).
    Invite {
        /// Pair a colleague's device: messages only, never joins your devices.
        #[arg(long)]
        guest: bool,
        /// Write the QR code SVG to this file.
        #[arg(long)]
        qr: Option<PathBuf>,
    },
    /// Open a pairing link from another device and show the pairing code.
    Accept { link: String },
    /// Confirm that the pairing code is the same on both devices (or --reject).
    Confirm {
        flow: String,
        #[arg(long)]
        reject: bool,
    },
    /// Pairings waiting for a decision on this device.
    Flows,
    /// iroh endpoint, relays and paths.
    Status,
}

#[cfg(unix)]
pub async fn run(peers: PathBuf, command: PairCommand) -> Result<()> {
    let value = match command {
        PairCommand::Invite { guest, qr } => {
            let purpose = if guest {
                PairPurpose::Guest
            } else {
                PairPurpose::OwnerDevice
            };
            let mut value = control::call(
                &peers,
                "POST",
                "/pair/invite",
                Some(json!({"purpose": purpose})),
            )
            .await?;
            if let Some(file) = qr {
                std::fs::write(&file, value["qr_svg"].as_str().unwrap_or_default())?;
            }
            value["qr_svg"] = json!("(omitted; use --qr FILE)");
            value
        }
        PairCommand::Accept { link } => {
            control::call(&peers, "POST", "/pair/accept", Some(json!({"link": link}))).await?
        }
        PairCommand::Confirm { flow, reject } => {
            control::call(
                &peers,
                "POST",
                &format!("/pair/{flow}/confirm"),
                Some(json!({"match": !reject})),
            )
            .await?
        }
        PairCommand::Flows => control::call(&peers, "GET", "/pair/flows", None).await?,
        PairCommand::Status => control::call(&peers, "GET", "/status", None).await?,
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
#[cfg(not(unix))]
pub async fn run(_peers: PathBuf, _command: PairCommand) -> Result<()> {
    anyhow::bail!(
        "pairing from the command line is not available on this platform yet; use the app"
    )
}
