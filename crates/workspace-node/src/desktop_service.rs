//! User-owned VNC process. Its lifetime follows the local node, not a viewer or relay.
mod discovery;
mod provider;
mod setup;
pub use provider::Provider;

use agent_workspace_runtime::{
    config as secrets,
    peers::config::{Config, Desktop, DesktopProtocol},
};
use anyhow::{Context, Result, ensure};
use rand::{Rng, distr::Alphanumeric};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command as Process},
    sync::{mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Status {},
    Discover {},
    Check {
        address: SocketAddr,
        #[serde(default)]
        protocol: DesktopProtocol,
    },
    Plan {},
    Install {
        plan_id: String,
    },
    Start {
        display: String,
        port: u16,
        #[serde(default)]
        provider: Provider,
        #[serde(default)]
        output: String,
    },
    Stop {},
    Credentials {},
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    version: u8,
    #[serde(default)]
    provider: Provider,
    #[serde(default)]
    output: String,
    display: String,
    port: u16,
}
impl Spec {
    fn address(&self) -> SocketAddr {
        (Ipv4Addr::LOCALHOST, self.port).into()
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && self.port >= 1024,
            "Use a VNC port between 1024 and 65535 and a supported configuration version."
        );
        self.provider.validate(&self.display, &self.output)?;
        Ok(())
    }
}
type Request = (Command, oneshot::Sender<Result<Value>>);
pub struct Service {
    requests: mpsc::Sender<Request>,
    task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl Service {
    pub fn start(path: PathBuf, stop: CancellationToken) -> Self {
        let (requests, incoming) = mpsc::channel(8);
        let task = tokio::spawn(Manager::new(path).run(incoming, stop));
        Self {
            requests,
            task: tokio::sync::Mutex::new(Some(task)),
        }
    }
    pub async fn manage(&self, command: Command) -> Result<Value> {
        let (send, receive) = oneshot::channel();
        self.requests
            .try_send((command, send))
            .map_err(|_| anyhow::anyhow!("Desktop service is busy or stopping; retry shortly."))?;
        receive.await.context("Desktop service stopped")?
    }
    pub(crate) async fn shutdown(&self) {
        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        }
    }
}
struct ManagedChild {
    process: Child,
    // wayvnc's control socket lives in an owner-only temporary directory.
    _control: Option<tempfile::TempDir>,
}
struct Manager {
    peers: PathBuf,
    spec_path: PathBuf,
    password_path: PathBuf,
    child: Option<ManagedChild>,
    spec: Option<Spec>,
    error: Option<String>,
    retries: u8,
    next_retry: Instant,
    started: Instant,
    installation: setup::Setup,
}
impl Manager {
    fn new(peers: PathBuf) -> Self {
        Self {
            spec_path: peers.with_file_name("desktop-service.json"),
            password_path: peers.with_file_name("desktop-vnc.password"),
            installation: setup::Setup::new(&peers),
            peers,
            child: None,
            spec: None,
            error: None,
            retries: 0,
            next_retry: Instant::now(),
            started: Instant::now(),
        }
    }
    async fn run(mut self, mut requests: mpsc::Receiver<Request>, stop: CancellationToken) {
        match self.read_spec() {
            Ok(spec) => self.spec = spec,
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                request = requests.recv() => {
                    let Some((command, reply)) = request else { break; };
                    // Cancellation drops a starting child as well as the running child.
                    tokio::select! {
                        _ = stop.cancelled() => break,
                        result = self.manage(command) => { let _ = reply.send(result); }
                    }
                }
                _ = tick.tick() => tokio::select! {
                    _ = stop.cancelled() => break,
                    _ = self.reconcile() => {},
                },
            }
        }
        self.kill().await;
        self.installation.shutdown().await;
    }
    fn read_spec(&self) -> Result<Option<Spec>> {
        if !self.spec_path.try_exists()? {
            return Ok(None);
        }
        let file = std::fs::File::open(&self.spec_path)?;
        ensure!(
            file.metadata()?.len() <= 2048,
            "Desktop service configuration is too large."
        );
        let spec: Spec = serde_json::from_reader(file).context("Invalid desktop service configuration; restore desktop-service.json before enabling managed VNC.")?;
        spec.validate()?;
        Ok(Some(spec))
    }
    fn enabled(&self) -> bool {
        self.spec.as_ref().is_some_and(|spec| {
            Config::read(&self.peers).is_ok_and(|c| {
                c.desktop.is_some_and(|d| {
                    d.address == spec.address() && d.protocol == DesktopProtocol::Vnc
                })
            })
        })
    }
    fn provider(&self) -> Provider {
        if self.enabled() || self.child.is_some() {
            self.spec
                .as_ref()
                .map_or_else(Provider::current, |s| s.provider)
        } else {
            Provider::current()
        }
    }
    fn status(&self) -> Value {
        let provider = self.provider();
        let spec = self.spec.as_ref().filter(|s| s.provider == provider);
        let error = self.error.as_deref().or(self.installation.error());
        json!({"platform":std::env::consts::OS,"supported":provider.problem().is_none(),"provider":provider,"package":provider.package(),"unsupported_reason":provider.problem(),"installed":executable(provider).is_some(),
            "display":spec.map(|s| s.display.clone()).unwrap_or_else(||provider.display()),
            "output":spec.map(|s|s.output.clone()).unwrap_or_default(),
            "username":if provider == Provider::Wayvnc {Some("cranibox")}else{None},
            "wayland":provider::wayland_session(),
            "selkies_wayland":provider::selkies_wayland_session(),
            "can_install":can_install(),"managed":self.spec.is_some(),"enabled":self.enabled(),"running":self.child.is_some(),
            "phase":if self.installation.running(){"installing"}else if self.child.is_some(){"running"}else if self.enabled() && self.retries < 5 {"restarting"}else if error.is_some(){"failed"}else{"stopped"},
            "installation":self.installation.status(),
            "port":spec.map_or(5900,|s|s.port),"error":error,"restart_attempts":self.retries})
    }
    async fn manage(&mut self, command: Command) -> Result<Value> {
        match command {
            Command::Status {} => {}
            Command::Discover {} => {
                let current = Config::read(&self.peers)?.desktop;
                let managed = self.spec.as_ref().map(|spec| Desktop {
                    address: spec.address(),
                    protocol: DesktopProtocol::Vnc,
                });
                return discovery::discover(current, managed).await;
            }
            Command::Check { address, protocol } => {
                Desktop { address, protocol }.validate()?;
                let result = match protocol {
                    DesktopProtocol::Vnc => rfb(address).await,
                    DesktopProtocol::Selkies => {
                        agent_workspace_runtime::peers::desktop::selkies::check(address).await
                    }
                };
                return Ok(
                    json!({"address":address,"ready":result.is_ok(),"error":result.err().map(|e|format!("{e:#}"))}),
                );
            }
            Command::Plan {} => {
                let provider = self.provider();
                return self.installation.review(provider);
            }
            Command::Install { plan_id } => {
                let provider = self.provider();
                ensure!(
                    self.child.is_none(),
                    "Stop managed VNC before installing its package."
                );
                self.installation.start(&plan_id, provider)?;
                self.error = None;
            }
            Command::Start {
                display,
                port,
                provider,
                output,
            } => {
                ensure!(
                    provider.problem().is_none(),
                    "{}",
                    provider.problem().unwrap_or_default()
                );
                ensure!(
                    !self.installation.running(),
                    "Wait for VNC installation to finish, then retry."
                );
                let spec = Spec {
                    version: 1,
                    provider,
                    output: output.trim().into(),
                    display: display.trim().into(),
                    port,
                };
                spec.validate()?;
                ensure!(
                    self.read_spec().is_ok(),
                    "Restore the invalid desktop-service.json before starting managed VNC."
                );
                let current = Config::read(&self.peers)?;
                ensure!(
                    current.desktop.as_ref().is_none_or(|d| self
                        .spec
                        .as_ref()
                        .is_some_and(|s| s.address() == d.address)),
                    "Stop sharing the existing VNC server before starting managed VNC."
                );
                ensure!(
                    self.child.is_none(),
                    "Managed VNC is already running. Stop it before changing its settings."
                );
                let password = self.password()?;
                secrets::write_private(
                    &self.password_path,
                    format!("{password}\n").as_bytes(),
                    true,
                )?;
                let mut child = launch(&spec, &self.password_path).await?;
                let saved = (|| {
                    secrets::write_private(
                        &self.spec_path,
                        &serde_json::to_vec_pretty(&spec)?,
                        true,
                    )?;
                    Config::update(&self.peers, |config| {
                        // Another local owner action may have changed sharing while VNC started.
                        ensure!(
                            config.desktop == current.desktop
                                && config.desktop_epoch == current.desktop_epoch,
                            "Desktop sharing changed during startup. Retry after checking its current settings."
                        );
                        config.desktop = Some(Desktop {
                            address: spec.address(),
                            protocol: DesktopProtocol::Vnc,
                        });
                        config.desktop_epoch = config
                            .desktop_epoch
                            .checked_add(1)
                            .context("desktop generation exhausted")?;
                        Ok(())
                    })?;
                    Ok::<_, anyhow::Error>(())
                })();
                if let Err(error) = saved {
                    let _ = child.process.kill().await;
                    if let Some(previous) = &self.spec {
                        secrets::write_private(
                            &self.spec_path,
                            &serde_json::to_vec_pretty(previous)?,
                            true,
                        )?;
                    } else if self.spec_path.try_exists()? {
                        std::fs::remove_file(&self.spec_path)?;
                    }
                    return Err(error);
                }
                self.child = Some(child);
                self.spec = Some(spec);
                self.error = None;
                self.retries = 0;
                self.started = Instant::now();
            }
            Command::Stop {} => {
                if let Some(spec) = &self.spec {
                    Config::update(&self.peers, |config| {
                        if config
                            .desktop
                            .as_ref()
                            .is_some_and(|d| d.address == spec.address())
                        {
                            config.desktop = None;
                            config.desktop_epoch = config
                                .desktop_epoch
                                .checked_add(1)
                                .context("desktop generation exhausted")?;
                        }
                        Ok(())
                    })?;
                }
                self.kill().await;
                self.error = None;
                self.retries = 0;
            }
            Command::Credentials {} => {
                ensure!(self.spec.is_some(), "Managed VNC has not been configured.");
                ensure!(
                    self.password_path.is_file(),
                    "Managed VNC password file is missing. Stop and start managed VNC to recreate it."
                );
                return Ok(
                    json!({"password":self.password()?,"username":self.spec.as_ref().filter(|s|s.provider==Provider::Wayvnc).map(|_|"cranibox")}),
                );
            }
        }
        Ok(self.status())
    }
    fn password(&self) -> Result<String> {
        if self.password_path.try_exists()? {
            let metadata = std::fs::metadata(&self.password_path)?;
            ensure!(metadata.len() <= 64, "Invalid managed VNC password file.");
            let value = std::fs::read_to_string(&self.password_path)?
                .trim()
                .to_owned();
            ensure!(
                value.len() == 8 && value.bytes().all(|b| b.is_ascii_alphanumeric()),
                "Invalid managed VNC password file."
            );
            Ok(value)
        } else {
            Ok(rand::rng()
                .sample_iter(Alphanumeric)
                .take(8)
                .map(char::from)
                .collect())
        }
    }
    async fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.process.kill().await;
        }
    }
    async fn reconcile(&mut self) {
        self.installation.reconcile().await;
        if !self.enabled() {
            self.kill().await;
            return;
        }
        if let Some(child) = &mut self.child {
            match child.process.try_wait() {
                Ok(None) => {
                    if self.started.elapsed() >= Duration::from_secs(60) {
                        self.retries = 0;
                    }
                    return;
                }
                result => {
                    self.error = Some(format!(
                        "VNC exited ({result:?}). Check the login session, display and its access permissions."
                    ));
                    self.kill().await;
                    self.next_retry = Instant::now() + Duration::from_secs(2);
                }
            }
        }
        if self.retries >= 5 || Instant::now() < self.next_retry || self.installation.running() {
            return;
        }
        let Some(spec) = self.spec.clone() else {
            return;
        };
        self.retries += 1;
        match launch(&spec, &self.password_path).await {
            Ok(child) => {
                self.child = Some(child);
                self.error = None;
                self.started = Instant::now();
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        self.next_retry =
            Instant::now() + Duration::from_secs(2u64.pow(self.retries.min(5) as u32));
    }
}
fn executable(provider: Provider) -> Option<PathBuf> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|root| root.join(provider.package()))
        .find(|path| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                path.metadata()
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            }
            #[cfg(not(unix))]
            {
                path.is_file()
            }
        })
}
async fn launch(spec: &Spec, password: &Path) -> Result<ManagedChild> {
    ensure!(
        spec.provider.problem().is_none(),
        "{}",
        spec.provider.problem().unwrap_or_default()
    );
    spec.validate()?;
    ensure!(
        password.is_absolute() && password.is_file(),
        "Managed VNC password file is missing; start managed sharing again to restore it."
    );
    // Probe occupancy before launch. Never attach an unrelated server to managed sharing.
    drop(tokio::net::TcpListener::bind(spec.address()).await.context("The managed VNC port is occupied. Choose another port or stop its existing server.")?);
    let executable = executable(spec.provider).with_context(|| {
        format!(
            "Install {} before starting managed VNC.",
            spec.provider.package()
        )
    })?;
    let mut process = Process::new(&executable);
    let mut control = None;
    match spec.provider {
        Provider::X11vnc => {
            process
                .args([
                    "-norc",
                    "-noremote",
                    "-localhost",
                    "-listen",
                    "127.0.0.1",
                    "-noipv6",
                    "-forever",
                    "-shared",
                    "-display",
                    &spec.display,
                    "-rfbport",
                    &spec.port.to_string(),
                    "-passwdfile",
                ])
                .arg(password);
        }
        Provider::Wayvnc => {
            let mut version = Process::new(&executable);
            version
                .arg("--version")
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            let version = tokio::time::timeout(Duration::from_secs(2), version.output())
                .await
                .context("wayvnc version check timed out")??;
            ensure!(
                version.status.success()
                    && version.stdout.len() <= 4096
                    && provider::supported_version(&String::from_utf8_lossy(&version.stdout)),
                "Managed sharing requires wayvnc 0.8 or newer. Upgrade it with your system package manager, then retry."
            );
            let runtime = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .context(
                    "Start the node inside your Wayland login session so XDG_RUNTIME_DIR is set.",
                )?;
            ensure!(
                runtime.is_absolute() && runtime.is_dir(),
                "XDG_RUNTIME_DIR must be the existing absolute runtime directory for your Wayland login session."
            );
            let value = std::fs::read_to_string(password)?;
            let value = value.trim();
            ensure!(
                value.len() == 8 && value.bytes().all(|b| b.is_ascii_alphanumeric()),
                "Invalid managed VNC password file."
            );
            let config = password.with_file_name("desktop-wayvnc.conf");
            // ARD authentication is supported by noVNC. Only the local hop may be unencrypted;
            // remote traffic still uses the authorized Cranibox transport.
            secrets::write_private(&config, format!("address=127.0.0.1\nport={}\nenable_auth=true\nrelax_encryption=true\nusername=cranibox\npassword={value}\n", spec.port).as_bytes(), true)?;
            let mut directory = tempfile::Builder::new();
            directory.prefix("cranibox-vnc-");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                directory.permissions(std::fs::Permissions::from_mode(0o700));
            }
            let directory = directory.tempdir()?;
            let socket = directory.path().join("control.sock");
            ensure!(
                socket.as_os_str().len() < 104,
                "The temporary directory path is too long for a private wayvnc control socket."
            );
            process
                .args(["--config"])
                .arg(config)
                .arg("--socket")
                .arg(socket);
            if !spec.output.is_empty() {
                process.arg("--output").arg(&spec.output);
            }
            process
                .args(["127.0.0.1", &spec.port.to_string()])
                .env("WAYLAND_DISPLAY", &spec.display);
            control = Some(directory);
        }
    }
    let mut child = process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("Start {}", spec.provider.package()))?;
    let result=tokio::time::timeout(Duration::from_secs(8),async {
        loop {
            ensure!(child.try_wait()?.is_none(),"VNC could not open this desktop. Check DISPLAY / XAUTHORITY for X11, or WAYLAND_DISPLAY and wlroots compositor compatibility for wayvnc.");
            if rfb(spec.address()).await.is_ok() { return Ok::<_,anyhow::Error>(()); }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }).await;
    match result {
        Ok(Ok(())) => Ok(ManagedChild {
            process: child,
            _control: control,
        }),
        result => {
            let _ = child.kill().await;
            match result {
                Ok(Err(error)) => Err(error),
                _ => anyhow::bail!(
                    "VNC startup timed out. Check the login session, selected display and access permissions, then retry."
                ),
            }
        }
    }
}
pub(crate) async fn rfb(address: SocketAddr) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(2),async {
        let mut stream=tokio::net::TcpStream::connect(address).await.context("Cannot connect to this local VNC port. Start the VNC server and check its loopback address.")?;
        let mut header=[0;12];
        stream.read_exact(&mut header).await.context("The service did not return a VNC greeting.")?;
        ensure!(matches!(&header,b"RFB 003.003\n"|b"RFB 003.007\n"|b"RFB 003.008\n"),"This port did not return a supported VNC greeting.");
        Ok::<_,anyhow::Error>(())
    }).await.context("VNC greeting timed out. Check the server and port.")?
}
fn can_install() -> bool {
    cfg!(target_os = "linux")
        && !Path::new("/.dockerenv").exists()
        && std::fs::read_to_string("/etc/os-release").is_ok_and(|s| {
            s.lines().any(|line| {
                matches!(
                    line,
                    "ID=ubuntu" | "ID=debian" | "ID=\"ubuntu\"" | "ID=\"debian\""
                )
            })
        })
}
fn install_plan(provider: Provider) -> Result<Value> {
    ensure!(
        provider.problem().is_none(),
        "{}",
        provider.problem().unwrap_or_default()
    );
    ensure!(
        can_install(),
        "Automatic VNC installation supports Ubuntu and Debian desktop hosts. Install it using your distribution's package manager, or connect an existing VNC server."
    );
    ensure!(
        executable(provider).is_none(),
        "VNC is already installed. Recheck the environment before starting managed sharing."
    );
    let package = provider.package();
    Ok(
        json!({"commands":format!("pkexec /bin/sh -c 'apt-get update && apt-get install --yes --no-install-recommends {package}'"),"administrator":true,"package":package,"source":if provider==Provider::Wayvnc {"https://github.com/any1/wayvnc"} else {"https://github.com/LibVNC/x11vnc"},"summary":format!("Install {package} from your configured Ubuntu/Debian apt repositories. The system authentication dialog requires your approval. This does not enable sharing.")}),
    )
}
