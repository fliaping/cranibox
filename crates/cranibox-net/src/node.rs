//! The node's iroh endpoint: identity reuse, relay order and connection pool.
use crate::{ALPN_PAIR, ALPN_PEER_FWD, ALPN_PEER_HTTP, Lane, addr};
use anyhow::{Context, Result, ensure};
use cranibox_contracts::{config::IrohConfig, net::IrohAddr};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayConfig, RelayMap, RelayMode, RelayUrl, SecretKey,
    Watcher,
    address_lookup::MemoryLookup,
    endpoint::{Connection, presets},
};
use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// How long the own relays may stay unreachable before the n0 relays are added.
const FALLBACK_AFTER: Duration = Duration::from_secs(10);
/// How long the own relays must stay healthy before the n0 relays are dropped again.
const RECOVER_AFTER: Duration = Duration::from_secs(60);
/// Pooled connections idle longer than this are closed.
pub const IDLE_CLOSE: Duration = Duration::from_secs(120);

/// Endpoint options. Built from the C5 [`IrohConfig`] plus test / advanced knobs.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Own relays (normally the Hub's `--public-url`), tried first.
    pub relays: Vec<String>,
    /// Add the public n0 relays when the own relays are missing or unreachable.
    pub n0_fallback: bool,
    /// UDP port; 0 = ephemeral.
    pub bind_port: u16,
    /// Disable UDP/IP paths (relay only). Tests and locked-down networks.
    pub relay_only: bool,
    /// Extra trusted TLS roots (DER) for a relay behind a private CA.
    pub extra_roots: Vec<Vec<u8>>,
    /// Bearer presented to relays (one-time pairing access to a Hub relay).
    pub relay_auth: Option<String>,
    /// Disable n0 DNS/pkarr address lookup (tests; addresses come from tickets).
    pub no_address_lookup: bool,
    /// Find trusted devices on the local network (see [`crate::lan`]).
    pub lan: bool,
}
impl Options {
    pub fn from_config(config: &IrohConfig) -> Self {
        Self {
            relays: config.relays.clone(),
            n0_fallback: config.n0_fallback,
            bind_port: config.bind_port,
            lan: config.lan,
            ..Self::default()
        }
    }
}

fn own_relay_map(relays: &[String]) -> Result<RelayMap> {
    // QUIC address discovery is off for own relays: behind a TLS-terminating proxy or
    // Cloudflare Tunnel only HTTPS/WebSocket reaches the relay.
    let configs = relays
        .iter()
        .map(|r| Ok(RelayConfig::new(addr::normalize_relay(r)?, None)))
        .collect::<Result<Vec<_>>>()?;
    Ok(RelayMap::from_iter(configs))
}

pub(crate) struct Pooled {
    pub conn: Connection,
    pub used: Instant,
}

#[derive(Clone)]
pub struct IrohNode {
    endpoint: Endpoint,
    pub(crate) pool: Arc<Mutex<HashMap<(EndpointId, Lane), Pooled>>>,
    stop: CancellationToken,
    own_relays: Arc<std::sync::RwLock<Vec<RelayUrl>>>,
    n0_fallback: bool,
    extra_roots: Arc<Vec<Vec<u8>>>,
    /// Local-network lookup, started once the trusted peers are known.
    lan: Option<(MemoryLookup, SecretKey)>,
}

impl IrohNode {
    /// Bind the endpoint with the node's ed25519 signing seed as the iroh SecretKey.
    pub async fn bind(seed: &[u8; 32], options: Options) -> Result<Self> {
        let secret = SecretKey::from_bytes(seed);
        let lan = (options.lan && !options.relay_only).then(|| {
            (
                MemoryLookup::with_provenance("cranibox_lan"),
                secret.clone(),
            )
        });
        let own = own_relay_map(&options.relays)?;
        let own_relays: Vec<RelayUrl> = own.urls();
        let mode = if own.is_empty() {
            ensure!(
                options.n0_fallback,
                "iroh needs a relay: configure an own relay or enable the n0 fallback"
            );
            RelayMode::Default
        } else {
            RelayMode::Custom(own)
        };
        let mut builder = Endpoint::builder(presets::N0)
            .secret_key(secret)
            .relay_mode(mode)
            .alpns(vec![
                ALPN_PEER_HTTP.to_vec(),
                ALPN_PEER_FWD.to_vec(),
                ALPN_PAIR.to_vec(),
            ]);
        if options.no_address_lookup {
            builder = builder.clear_address_lookup();
        }
        if let Some((memory, _)) = &lan {
            builder = builder.address_lookup(memory.clone());
        }
        if options.relay_only {
            builder = builder.clear_ip_transports();
        } else if options.bind_port != 0 {
            builder = builder
                .bind_addr(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, options.bind_port))
                .context("invalid iroh bind port")?;
        }
        if !options.extra_roots.is_empty() {
            builder = builder.ca_tls_config(
                iroh::tls::CaTlsConfig::embedded().with_extra_roots(
                    options
                        .extra_roots
                        .iter()
                        .map(|der| rustls::pki_types::CertificateDer::from(der.clone())),
                ),
            );
        }
        let endpoint = builder.bind().await.context("bind iroh endpoint")?;
        if let Some(token) = &options.relay_auth {
            for url in &own_relays {
                endpoint
                    .insert_relay(
                        url.clone(),
                        Arc::new(RelayConfig::new(url.clone(), None).with_auth_token(token)),
                    )
                    .await;
            }
        }
        let node = Self {
            endpoint,
            pool: Arc::default(),
            stop: CancellationToken::new(),
            own_relays: Arc::new(std::sync::RwLock::new(own_relays)),
            n0_fallback: options.n0_fallback,
            extra_roots: Arc::new(options.extra_roots.clone()),
            lan,
        };
        if !node.own_relays().is_empty() && options.n0_fallback {
            tokio::spawn(node.clone().relay_fallback());
        }
        tokio::spawn(node.clone().sweep());
        Ok(node)
    }

    /// Start announcing on the local network and learning addresses of `trusted` peers.
    pub fn start_lan(&self, trusted: crate::allow::Allowlist) {
        if let Some((memory, secret)) = self.lan.clone() {
            tokio::spawn(crate::lan::run(
                self.endpoint.clone(),
                secret,
                memory,
                trusted,
                self.stop.clone(),
            ));
        }
    }

    /// Own relays in preference order (configured, hub, learned from pairing).
    pub fn own_relays(&self) -> Vec<RelayUrl> {
        self.own_relays.read().unwrap().clone()
    }

    /// Start using a relay learned at runtime (the hub's relay from a pairing
    /// ticket) as an own relay, QAD off. A node that ran on the n0 relays only
    /// switches to it and keeps n0 as the fallback. Returns whether it was new.
    pub async fn adopt_relay(&self, relay: &str) -> Result<bool> {
        let url = addr::normalize_relay(relay)?;
        let was_empty = {
            let mut own = self.own_relays.write().unwrap();
            if own.contains(&url) {
                return Ok(false);
            }
            own.push(url.clone());
            own.len() == 1
        };
        self.endpoint
            .insert_relay(url.clone(), Arc::new(RelayConfig::new(url.clone(), None)))
            .await;
        if was_empty {
            // We were on the n0 default map: the own relay goes first now.
            for n0 in iroh::defaults::prod::default_relay_map().urls::<Vec<_>>() {
                self.endpoint.remove_relay(&n0).await;
            }
            if self.n0_fallback {
                tokio::spawn(self.clone().relay_fallback());
            }
        }
        tracing::info!("using relay {url}");
        Ok(true)
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
    pub fn id(&self) -> EndpointId {
        self.endpoint.id()
    }
    /// Our current address (EndpointId, home relay, direct addresses).
    pub fn addr(&self) -> IrohAddr {
        let mut out = addr::iroh_addr(&self.endpoint.addr());
        if out.relay_url.is_none() {
            out.relay_url = self.own_relays().first().map(|u| u.to_string());
        }
        out
    }
    /// Wait until a home relay is connected (or time out).
    pub async fn online(&self, timeout: Duration) -> Result<()> {
        tokio::time::timeout(timeout, self.endpoint.online())
            .await
            .context("iroh endpoint did not come online")
    }
    pub fn relays_connected(&self) -> Vec<(String, bool)> {
        self.endpoint
            .home_relay_status()
            .get()
            .iter()
            .map(|s| (s.url().to_string(), s.is_connected()))
            .collect()
    }

    /// Own relays first; the n0 relays join only while all own relays are unreachable.
    async fn relay_fallback(self) {
        let n0 = iroh::defaults::prod::default_relay_map();
        let mut fallback = false;
        let mut since = Instant::now();
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! { _ = self.stop.cancelled() => break, _ = interval.tick() => {} }
            if !fallback {
                let connected = self
                    .endpoint
                    .home_relay_status()
                    .get()
                    .iter()
                    .any(|s| s.is_connected() && self.own_relays().contains(s.url()));
                if connected {
                    since = Instant::now();
                } else if since.elapsed() >= FALLBACK_AFTER {
                    for relay in n0.relays::<Vec<_>>() {
                        self.endpoint
                            .insert_relay(relay.url.clone(), relay.clone())
                            .await;
                    }
                    tracing::warn!("own relay unreachable; using n0 public relays as fallback");
                    fallback = true;
                    since = Instant::now();
                }
                continue;
            }
            // In fallback the own relay may not be a home relay: probe it over HTTPS
            // every 15 s and return once it stayed reachable for RECOVER_AFTER.
            if !since.elapsed().as_secs().is_multiple_of(15) {
                continue;
            }
            let mut reachable = false;
            for url in &self.own_relays() {
                reachable |= crate::relay::ping(url.as_str(), &self.extra_roots).await;
            }
            if !reachable {
                since = Instant::now();
            } else if since.elapsed() >= RECOVER_AFTER {
                for url in n0.urls::<Vec<_>>() {
                    self.endpoint.remove_relay(&url).await;
                }
                tracing::info!("own relay reachable again; n0 relays removed");
                fallback = false;
                since = Instant::now();
            }
        }
    }

    async fn sweep(self) {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        loop {
            tokio::select! { _ = self.stop.cancelled() => break, _ = interval.tick() => {} }
            self.pool.lock().await.retain(|_, pooled| {
                let keep =
                    pooled.conn.close_reason().is_none() && pooled.used.elapsed() < IDLE_CLOSE;
                if !keep {
                    pooled.conn.close(crate::close::DONE.into(), b"idle");
                }
                keep
            });
        }
    }

    /// Dial `addr` and check that the TLS-authenticated id is the expected one.
    pub async fn connect(&self, addr: &EndpointAddr, alpn: &[u8]) -> Result<Connection> {
        let conn = self
            .endpoint
            .connect(addr.clone(), alpn)
            .await
            .context("iroh connect")?;
        ensure!(
            conn.remote_id() == addr.id,
            "remote EndpointId does not match the paired key"
        );
        Ok(conn)
    }

    /// A pooled peer-http connection to `addr` (reused across requests and streams).
    pub async fn peer_connection(&self, addr: &EndpointAddr) -> Result<Connection> {
        self.lane_connection(addr, Lane::Peer).await
    }

    /// A pooled connection to `addr` on `lane`.
    pub async fn lane_connection(&self, addr: &EndpointAddr, lane: Lane) -> Result<Connection> {
        let mut pool = self.pool.lock().await;
        if let Some(pooled) = pool.get_mut(&(addr.id, lane))
            && pooled.conn.close_reason().is_none()
        {
            pooled.used = Instant::now();
            return Ok(pooled.conn.clone());
        }
        drop(pool);
        let conn = self.connect(addr, lane.alpn()).await?;
        self.pool.lock().await.insert(
            (addr.id, lane),
            Pooled {
                conn: conn.clone(),
                used: Instant::now(),
            },
        );
        Ok(conn)
    }

    /// Path of the pooled connection to `id`, if one is open.
    pub async fn pool_path(&self, id: &EndpointId) -> Option<cranibox_contracts::net::PathInfo> {
        let pool = self.pool.lock().await;
        [Lane::Peer, Lane::Forward]
            .iter()
            .filter_map(|lane| pool.get(&(*id, *lane)))
            .find(|p| p.conn.close_reason().is_none())
            .map(|p| crate::path::info(&p.conn))
    }

    /// Close connections of peers that are no longer paired.
    pub async fn drop_peer(&self, id: &EndpointId) {
        let mut pool = self.pool.lock().await;
        for lane in [Lane::Peer, Lane::Forward] {
            if let Some(pooled) = pool.remove(&(*id, lane)) {
                pooled
                    .conn
                    .close(crate::close::NOT_ALLOWED.into(), b"unpaired");
            }
        }
    }

    pub async fn close(&self) {
        self.stop.cancel();
        for (_, pooled) in self.pool.lock().await.drain() {
            pooled.conn.close(crate::close::DONE.into(), b"shutdown");
        }
        self.endpoint.close().await;
    }
}
