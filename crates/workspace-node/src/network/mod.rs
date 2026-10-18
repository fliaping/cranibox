//! Optional provider processes belong to the node, independently of viewers and relays.
mod config;
mod discovery;
mod forward;
mod install;
mod invitations;
mod process;
mod tailscale;
mod tailscale_discovery;
mod tailscale_nodes;
#[cfg(all(test, unix))]
mod tests;

use crate::peer::{NetworkEndpoint, NetworkListener};
use agent_workspace_runtime::{
    config as secrets,
    peers::{Store, config::Config, local_forward},
};
use anyhow::{Context, Result, ensure};
use config::{EasyTier, Spec};
use discovery::Device;
use forward::{Destination, Forward};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    net::TcpListener,
    process::{Child, Command as Process},
    sync::{Semaphore, mpsc, oneshot},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Status {},
    Plan {},
    Install {},
    EasyTierJoin {
        network: String,
        token: String,
        #[serde(default)]
        peer: String,
        #[serde(default)]
        peers: Vec<String>,
        listen_port: Option<u16>,
    },
    EasyTierStop {},
    EasyTierInvite {
        peers: Vec<String>,
    },
    EasyTierRevokeInvitation {
        id: String,
    },
    EasyTierEnroll {
        url: String,
        token: String,
        listen_port: Option<u16>,
    },
    EasyTierConnect {
        node: String,
        token: String,
    },
    TailscaleLogin {},
    TailscaleCancelLogin {},
    TailscaleDiscovery {
        port: Option<u16>,
        ports: Option<Vec<u16>>,
    },
    TailscaleDiscover {
        address: std::net::IpAddr,
        port: u16,
    },
    TailscaleConnect {
        address: std::net::IpAddr,
        port: u16,
        node: String,
        token: String,
    },
}
enum Action {
    Manage(Command),
    Redeem(String, Option<String>),
}
type Request = (Action, oneshot::Sender<Result<Value>>);
pub struct Service {
    requests: mpsc::Sender<Request>,
    task: tokio::sync::Mutex<Option<JoinHandle<()>>>,
}
impl Service {
    pub(crate) fn start(
        peers: PathBuf,
        endpoint: NetworkEndpoint,
        stop: CancellationToken,
    ) -> Self {
        let (requests, incoming) = mpsc::channel(8);
        let task = tokio::spawn(Manager::new(peers, endpoint).run(incoming, stop));
        Self {
            requests,
            task: tokio::sync::Mutex::new(Some(task)),
        }
    }
    pub async fn manage(&self, command: Command) -> Result<Value> {
        self.request(Action::Manage(command)).await
    }
    pub(crate) async fn redeem(&self, token: String, recovery: Option<String>) -> Result<Value> {
        self.request(Action::Redeem(token, recovery)).await
    }
    async fn request(&self, action: Action) -> Result<Value> {
        let (send, receive) = oneshot::channel();
        self.requests
            .try_send((action, send))
            .map_err(|_| anyhow::anyhow!("Network service is busy or stopping; retry shortly."))?;
        receive.await.context("Network service stopped")?
    }
    pub(crate) async fn shutdown(&self) {
        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        }
    }
}
struct Manager {
    peers: PathBuf,
    spec_path: PathBuf,
    spec: Spec,
    own_id: String,
    endpoint: NetworkEndpoint,
    child: Option<Child>,
    listener: Option<NetworkListener>,
    rpc: Option<SocketAddr>,
    socks: Option<SocketAddr>,
    devices: Vec<Device>,
    forwards: BTreeMap<String, Forward>,
    forward_errors: Vec<String>,
    slots: Arc<Semaphore>,
    error: Option<String>,
    state_valid: bool,
    retries: u8,
    next_retry: Instant,
    started: Instant,
    next_poll: Instant,
    installation: Option<JoinHandle<Result<()>>>,
    discovery: Option<JoinHandle<Result<Vec<Device>>>>,
    tailscale: Value,
    tailscale_poll: Option<JoinHandle<Result<Value>>>,
    login: Option<Child>,
    tailscale_requested: Option<Instant>,
    tailscale_discovery: tailscale_discovery::Discovery,
}
impl Manager {
    fn new(peers: PathBuf, endpoint: NetworkEndpoint) -> Self {
        Self {
            spec_path: peers.with_file_name("networks.json"),
            own_id: String::new(),
            peers,
            endpoint,
            spec: Spec::default(),
            child: None,
            listener: None,
            rpc: None,
            socks: None,
            devices: Vec::new(),
            forwards: BTreeMap::new(),
            forward_errors: Vec::new(),
            slots: Arc::new(Semaphore::new(32)),
            error: None,
            state_valid: true,
            retries: 0,
            next_retry: Instant::now(),
            started: Instant::now(),
            next_poll: Instant::now(),
            installation: None,
            discovery: None,
            tailscale: json!({"state":"Checking","devices":[]}),
            tailscale_poll: None,
            login: None,
            tailscale_requested: None,
            tailscale_discovery: tailscale_discovery::Discovery::default(),
        }
    }
    async fn run(mut self, mut requests: mpsc::Receiver<Request>, stop: CancellationToken) {
        let loaded = (|| {
            self.spec = Spec::read(&self.spec_path)?;
            self.own_id = Config::read(&self.peers)?.node_id;
            Ok::<_, anyhow::Error>(())
        })();
        if let Err(e) = loaded {
            self.state_valid = false;
            self.error = Some(format!("{e:#}"));
        }
        for (node, port) in self.spec.forwards.clone() {
            let result = async {
                let lease = local_forward::path(&self.peers, &node)?;
                // Upgrade saved routes before binding, including when another program owns the port.
                let url = format!("http://127.0.0.1:{port}");
                let config = Config::read(&self.peers)?;
                if config.routes.get(&node).is_some_and(|route| {
                    route.url == url && route.local_forward_lock.as_ref() != Some(&lease)
                }) {
                    Config::update(&self.peers, |config| {
                        if let Some(route) = config
                            .routes
                            .get_mut(&node)
                            .filter(|route| route.url == url)
                        {
                            route.local_forward_lock = Some(lease.clone());
                        }
                        Ok(())
                    })?;
                }
                Forward::bind(port, self.slots.clone(), &lease).await
            }
            .await;
            match result {
                Ok(forward) => {
                    self.forwards.insert(node, forward);
                }
                Err(_) => {
                    self.forward_errors.push(format!(
                        "Saved network port {port} is unavailable. Free it and restart this node."
                    ));
                }
            }
        }
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                request = requests.recv() => {
                    let Some((action, reply)) = request else { break; };
                    tokio::select! {
                        _ = stop.cancelled() => break,
                        result = self.dispatch(action) => { let _ = reply.send(result); }
                    }
                },
                _ = tick.tick() => tokio::select! { _ = stop.cancelled() => break, _ = self.reconcile() => {} },
            }
        }
        self.kill().await;
        if let Some(mut login) = self.login.take() {
            let _ = login.kill().await;
        }
        if let Some(task) = self.installation.take() {
            task.abort();
        }
        if let Some(task) = self.tailscale_poll.take() {
            task.abort();
        }
        self.tailscale_discovery.reset();
        for forward in std::mem::take(&mut self.forwards).into_values() {
            forward.shutdown().await;
        }
    }
    fn status(&self) -> Value {
        let phase = if self.installation.is_some() {
            "installing"
        } else if self.child.is_some() {
            "running"
        } else if self.spec.easytier.is_some() && self.retries < 5 && self.state_valid {
            "restarting"
        } else if self.error.is_some() || !self.forward_errors.is_empty() {
            "failed"
        } else {
            "stopped"
        };
        let configured: Vec<_> = self.spec.forwards.keys().collect();
        let error = if self.forward_errors.is_empty() {
            self.error.clone()
        } else {
            Some(
                self.forward_errors
                    .iter()
                    .cloned()
                    .chain(self.error.iter().cloned())
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        };
        let mut tailscale = self.tailscale.clone();
        tailscale["login_pending"] = json!(self.login.is_some());
        tailscale["discovery"] = self
            .tailscale_discovery
            .status(&self.spec.discovery_ports());
        let invitations =
            invitations::Store::new(&self.spec_path).list(self.spec.easytier.as_ref());
        let invitation_error = invitations.as_ref().err().map(|e| format!("{e:#}"));
        json!({"node_id":self.own_id,"easytier":{"version":install::VERSION,"installed":install::binaries(&self.peers).is_some(),
            "enabled":self.spec.easytier.is_some(),"phase":phase,"error":error,"restart_attempts":self.retries,
            "network":self.spec.easytier.as_ref().map(|s|&s.network),"peer":self.spec.easytier.as_ref().and_then(|s|s.entries().first()),
            "peers":self.spec.easytier.as_ref().map(|s|s.entries()).unwrap_or_default(),
            "invitations":invitations.unwrap_or_default(),"invitation_error":invitation_error,
            "listen_port":self.spec.easytier.as_ref().and_then(|s|s.listen_port),"devices":self.devices,"connections":configured},"tailscale":tailscale})
    }
    async fn dispatch(&mut self, action: Action) -> Result<Value> {
        match action {
            Action::Manage(command) => self.manage(command).await,
            Action::Redeem(token, recovery) => {
                ensure!(self.state_valid, "Network configuration needs attention.");
                let spec = self
                    .spec
                    .easytier
                    .as_ref()
                    .context("This node is not connected to EasyTier.")?;
                Ok(serde_json::to_value(
                    invitations::Store::new(&self.spec_path).redeem(
                        spec,
                        &token,
                        recovery.as_deref(),
                    )?,
                )?)
            }
        }
    }
    fn can_join(&self) -> Result<()> {
        ensure!(
            self.state_valid,
            "Restore networks.json before changing network settings."
        );
        ensure!(
            self.installation.is_none(),
            "Wait for EasyTier installation to finish."
        );
        ensure!(
            install::binaries(&self.peers).is_some(),
            "Install EasyTier first."
        );
        ensure!(
            self.spec.easytier.is_none(),
            "Disconnect the existing EasyTier network before changing its settings."
        );
        // Check local prerequisites before consuming a single-use invitation.
        Spec::read(&self.spec_path)?;
        invitations::Store::new(&self.spec_path).list(None)?;
        Ok(())
    }
    fn save_network(&mut self, spec: EasyTier) -> Result<()> {
        spec.validate()?;
        let mut saved = self.spec.clone();
        saved.easytier = Some(spec);
        // A disconnect followed by rejoining the same network must not reactivate old invitations.
        invitations::Store::new(&self.spec_path).clear()?;
        saved.write(&self.spec_path)?;
        self.spec = saved;
        self.retries = 0;
        self.error = None;
        self.next_retry = Instant::now();
        Ok(())
    }
    async fn manage(&mut self, command: Command) -> Result<Value> {
        match command {
            Command::Status {} => {
                self.tailscale_requested = Some(Instant::now());
            }
            Command::Plan {} => return install::plan(&self.peers),
            Command::Install {} => {
                ensure!(
                    self.installation.is_none() && self.child.is_none(),
                    "Wait for installation to finish or stop EasyTier first."
                );
                ensure!(
                    install::binaries(&self.peers).is_none(),
                    "EasyTier is already installed."
                );
                install::plan(&self.peers)?;
                self.error = None;
                self.installation = Some(tokio::spawn(install::install(self.peers.clone())));
            }
            Command::EasyTierJoin {
                network,
                token,
                peer,
                peers,
                listen_port,
            } => {
                self.can_join()?;
                let spec = EasyTier {
                    network,
                    token,
                    peer,
                    peers,
                    listen_port,
                };
                self.save_network(spec)?;
                // Reconciliation owns startup; leaving this page never cancels the connection.
            }
            Command::EasyTierEnroll {
                url,
                token,
                listen_port,
            } => {
                self.can_join()?;
                let spec = invitations::enroll(&self.spec_path, &url, &token, listen_port).await?;
                self.save_network(spec)?;
                invitations::clear_enrollment(&self.spec_path);
            }
            Command::EasyTierInvite { peers } => {
                ensure!(self.state_valid, "Network configuration needs attention.");
                let spec = self
                    .spec
                    .easytier
                    .as_ref()
                    .context("Connect EasyTier before inviting another device.")?;
                let invitation = invitations::Store::new(&self.spec_path).create(spec, peers)?;
                return Ok(json!({"network":self.status(),"invitation":invitation}));
            }
            Command::EasyTierRevokeInvitation { id } => {
                invitations::Store::new(&self.spec_path).revoke(&id)?;
            }
            Command::EasyTierStop {} => {
                ensure!(
                    self.state_valid,
                    "Restore networks.json before changing network settings."
                );
                let mut saved = self.spec.clone();
                saved.easytier = None;
                saved.write(&self.spec_path)?;
                self.spec = saved;
                self.kill().await;
                self.error = None;
                self.retries = 0;
            }
            Command::EasyTierConnect { node, token } => {
                ensure!(
                    self.state_valid && self.child.is_some(),
                    "Connect EasyTier first."
                );
                let device = self
                    .devices
                    .iter()
                    .find(|d| d.node == node)
                    .context("Device is no longer uniquely advertised; refresh and retry.")?
                    .clone();
                // Validate before allocating a forwarding entry or writing configuration.
                ensure!(
                    token.len() == 64 && token.bytes().all(|c| c.is_ascii_hexdigit()),
                    "Supply the target's 64-character access token."
                );
                ensure!(
                    self.spec.forwards.contains_key(&node) || self.spec.forwards.len() < 128,
                    "Network device limit reached."
                );
                if !self.forwards.contains_key(&node) {
                    let port = self.spec.forwards.get(&node).copied().unwrap_or(0);
                    let lease = local_forward::path(&self.peers, &node)?;
                    let forward = Forward::bind(port, self.slots.clone(), &lease)
                        .await
                        .context("Bind the saved network route")?;
                    let mut saved = self.spec.clone();
                    saved.forwards.insert(node.clone(), forward.port);
                    saved.write(&self.spec_path)?;
                    self.spec = saved;
                    self.forwards.insert(node.clone(), forward);
                }
                let forward = &self.forwards[&node];
                forward.update(self.socks.map(|socks| Destination {
                    socks,
                    target: device.address,
                }));
                let result = Store::new(self.peers.clone()).connect_forward(
                    node.clone(),
                    format!("http://127.0.0.1:{}", forward.port),
                    token,
                    local_forward::path(&self.peers, &node)?,
                )?;
                return Ok(json!({"network":self.status(),"catalog":result}));
            }
            Command::TailscaleLogin {} => {
                ensure!(self.login.is_none(), "Tailscale login is already pending.");
                self.login = Some(tailscale::login().await?);
                self.next_poll = Instant::now();
            }
            Command::TailscaleCancelLogin {} => {
                if let Some(mut login) = self.login.take() {
                    let _ = login.kill().await;
                }
                self.next_poll = Instant::now();
            }
            Command::TailscaleDiscovery { port, ports } => {
                ensure!(
                    self.state_valid,
                    "Restore networks.json before changing network settings."
                );
                ensure!(
                    port.is_none() || ports.is_none(),
                    "Supply discovery ports or the legacy port field, not both."
                );
                let mut saved = self.spec.clone();
                saved.tailscale_discovery_port = port;
                saved.tailscale_discovery_ports = ports.unwrap_or_default();
                saved.write(&self.spec_path)?;
                self.spec = saved;
                self.tailscale_discovery.reset();
                self.tailscale_requested = Some(Instant::now());
                self.next_poll = Instant::now();
            }
            Command::TailscaleDiscover { address, port } => {
                return tailscale_nodes::discover(address, port, &self.own_id).await;
            }
            Command::TailscaleConnect {
                address,
                port,
                node,
                token,
            } => {
                ensure!(
                    token.len() == 64 && token.bytes().all(|c| c.is_ascii_hexdigit()),
                    "Supply the target's 64-character access token."
                );
                ensure!(
                    agent_workspace_runtime::protocol::valid_node_id(&node) && node != self.own_id,
                    "Choose a valid remote node."
                );
                let found = tailscale_nodes::discover(address, port, &self.own_id).await?;
                ensure!(
                    found["node_id"] == node,
                    "The target node identity changed. Find the node again before saving."
                );
                let catalog = Store::new(self.peers.clone()).connect_discovered(
                    node,
                    tailscale_nodes::origin(address, port),
                    token,
                )?;
                return Ok(json!({"network":self.status(),"catalog":catalog}));
            }
        }
        Ok(self.status())
    }
    async fn launch(&mut self) -> Result<()> {
        let spec = self.spec.easytier.as_ref().context("Network disabled")?;
        let (core, _) = install::binaries(&self.peers)
            .context("Install EasyTier to restore the saved network.")?;
        let listener = self
            .endpoint
            .listen((Ipv4Addr::LOCALHOST, 0).into())
            .await?;
        let port = listener.address().port();
        let rpc_reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let socks_reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let rpc = rpc_reservation.local_addr()?;
        let socks = socks_reservation.local_addr()?;
        let quoted = |s: &str| serde_json::to_string(s).unwrap();
        let listeners = spec
            .listen_port
            .map(|p| vec![format!("tcp://0.0.0.0:{p}"), format!("udp://0.0.0.0:{p}")])
            .unwrap_or_default();
        let mut text = format!(
            "hostname={}\ninstance_name=\"cranibox\"\ndhcp=true\nlisteners={}\nsocks5_proxy={}\ntcp_whitelist=[\"{port}\"]\nudp_whitelist=[\"0\"]\n\n[network_identity]\nnetwork_name={}\nnetwork_secret={}\n\n[flags]\nno_tun=true\nuse_smoltcp=true\ndisable_upnp=true\naccept_dns=false\nprivate_mode=true\nrelay_network_whitelist={}\nbind_device=false\n\n",
            quoted(&config::hostname(&self.own_id, port)),
            serde_json::to_string(&listeners)?,
            quoted(&format!("socks5://{socks}")),
            quoted(&spec.network),
            quoted(&spec.token),
            quoted(&spec.network)
        );
        for peer in spec.entries() {
            text.push_str(&format!("[[peer]]\nuri={}\n", quoted(peer)));
        }
        let path = self.peers.with_file_name("easytier.toml");
        secrets::write_private(&path, text.as_bytes(), true)?;
        drop(rpc_reservation);
        drop(socks_reservation);
        // Credentials only enter the private file, never arguments, status or captured logs.
        let child = Process::new(core)
            .arg("--disable-env-parsing")
            .arg("--config-file")
            .arg(path)
            .arg("--rpc-portal")
            .arg(rpc.to_string())
            .args(["--console-log-level", "error"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        self.child = Some(child);
        self.listener = Some(listener);
        self.rpc = Some(rpc);
        self.socks = Some(socks);
        self.started = Instant::now();
        self.next_poll = Instant::now();
        self.error = None;
        Ok(())
    }
    async fn kill(&mut self) {
        for forward in self.forwards.values() {
            forward.update(None);
        }
        self.devices.clear();
        if let Some(task) = self.discovery.take() {
            task.abort();
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
        }
        if let Some(listener) = self.listener.take() {
            listener.shutdown().await;
        }
        self.rpc = None;
        self.socks = None;
        // Remove the generated secret-bearing config when disabled or stopped.
        let _ = std::fs::remove_file(self.peers.with_file_name("easytier.toml"));
    }
    async fn reconcile(&mut self) {
        if self.installation.as_ref().is_some_and(|t| t.is_finished()) {
            let result = self.installation.take().unwrap().await;
            self.error = match result {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(format!("{e:#}")),
                Err(_) => Some("EasyTier installation stopped.".into()),
            };
            self.retries = 0;
            self.next_retry = Instant::now();
        }
        let exited = self
            .child
            .as_mut()
            .and_then(|c| c.try_wait().transpose())
            .is_some();
        if exited {
            if self.started.elapsed() > Duration::from_secs(60) {
                self.retries = 0;
            }
            self.kill().await;
            self.failed("EasyTier exited. The node will retry its saved network.".into());
        }
        if self.child.is_none()
            && self.spec.easytier.is_some()
            && self.state_valid
            && self.installation.is_none()
            && self.retries < 5
            && Instant::now() >= self.next_retry
            && let Err(e) = self.launch().await
        {
            self.failed(format!("{e:#}"));
        }
        if self.discovery.as_ref().is_some_and(|t| t.is_finished()) {
            match self.discovery.take().unwrap().await {
                Ok(Ok(devices)) => {
                    self.devices = devices;
                    self.error = None;
                }
                _ => {
                    self.devices.clear();
                    self.error = Some("EasyTier discovery is unavailable; saved routes are paused until it recovers.".into());
                }
            }
            for (node, forward) in &self.forwards {
                forward.update(self.devices.iter().find(|d| &d.node == node).and_then(|d| {
                    self.socks.map(|socks| Destination {
                        socks,
                        target: d.address,
                    })
                }));
            }
        }
        if self
            .tailscale_poll
            .as_ref()
            .is_some_and(|t| t.is_finished())
        {
            self.tailscale = match self.tailscale_poll.take().unwrap().await {
                Ok(Ok(value)) => value,
                Ok(Err(e)) => {
                    json!({"installed":tailscale::executable().is_some(),"state":"Unavailable","devices":[],"error":format!("{e:#}")})
                }
                Err(_) => {
                    json!({"state":"Unavailable","devices":[],"error":"Tailscale status check stopped."})
                }
            };
        }
        if self
            .login
            .as_mut()
            .and_then(|c| c.try_wait().transpose())
            .is_some()
        {
            self.login = None;
        }
        self.tailscale_discovery
            .reconcile(&self.tailscale, &self.spec.discovery_ports(), &self.own_id)
            .await;
        if Instant::now() >= self.next_poll {
            self.next_poll = Instant::now() + Duration::from_secs(3);
            if self.discovery.is_none()
                && let Some(rpc) = self.rpc
                && let Some(socks) = self.socks
                && let Some((_, cli)) = install::binaries(&self.peers)
            {
                let own = self.own_id.clone();
                self.discovery = Some(tokio::spawn(async move {
                    let bytes =
                        process::output(&cli, &["-p", &rpc.to_string(), "-o", "json", "peer"])
                            .await?;
                    discovery::discover(&bytes, &own, socks).await
                }));
            }
            if (self.login.is_some()
                || !self.spec.discovery_ports().is_empty()
                || self
                    .tailscale_requested
                    .is_some_and(|seen| seen.elapsed() < Duration::from_secs(15)))
                && self.tailscale_poll.is_none()
            {
                self.tailscale_poll = Some(tokio::spawn(tailscale::status()));
            }
        }
    }
    fn failed(&mut self, message: String) {
        self.retries += 1;
        self.error = Some(message);
        self.next_retry = Instant::now() + Duration::from_secs(1u64 << self.retries);
    }
}
