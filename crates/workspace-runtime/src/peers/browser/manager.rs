//! A dedicated Chromium-family browser owned by the node. It uses its own profile,
//! never the user's everyday one, and listens for CDP on loopback only. When it is
//! ready the node shares it as the `browser` provider; stopping it clears sharing.
//!
//! It starts without a window (agents open their own tabs), and an instance left
//! running on the profile — e.g. after the desktop app quit — is adopted rather than
//! launched again: a second launch would only hand Chrome another blank tab.
use super::{super::config::Config, Provider};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    process::{Child, Command as Process},
    sync::{mpsc, oneshot, watch},
};

const READY_TIMEOUT: Duration = Duration::from_secs(20);
/// How often an adopted instance, which is not our child, is checked for liveness.
const ADOPTED_CHECK: Duration = Duration::from_secs(5);
const MAX_RETRIES: u8 = 5;

/// An installed browser that can serve CDP.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Candidate {
    /// `chrome`, `chromium`, `edge` or `brave`.
    pub id: String,
    pub name: String,
    pub path: PathBuf,
}
const KNOWN: &[(&str, &str, &str, &[&str], &str)] = &[
    // id, name, macOS app, Linux names, Windows path below Program Files
    (
        "chrome",
        "Google Chrome",
        "Google Chrome.app/Contents/MacOS/Google Chrome",
        &["google-chrome", "google-chrome-stable"],
        "Google/Chrome/Application/chrome.exe",
    ),
    (
        "chromium",
        "Chromium",
        "Chromium.app/Contents/MacOS/Chromium",
        &["chromium", "chromium-browser"],
        "Chromium/Application/chrome.exe",
    ),
    (
        "edge",
        "Microsoft Edge",
        "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        &["microsoft-edge", "microsoft-edge-stable"],
        "Microsoft/Edge/Application/msedge.exe",
    ),
    (
        "brave",
        "Brave",
        "Brave Browser.app/Contents/MacOS/Brave Browser",
        &["brave-browser", "brave"],
        "BraveSoftware/Brave-Browser/Application/brave.exe",
    ),
];
/// Installed browsers, in preference order.
pub fn detect() -> Vec<Candidate> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let path = std::env::var_os("PATH").unwrap_or_default();
    KNOWN
        .iter()
        .filter_map(|(id, name, app, linux, windows)| {
            let mut paths: Vec<PathBuf> = vec![];
            if cfg!(target_os = "macos") {
                paths.push(Path::new("/Applications").join(app));
                paths.extend(home.iter().map(|h| h.join("Applications").join(app)));
            } else if cfg!(windows) {
                for root in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
                    paths.extend(std::env::var_os(root).map(|r| PathBuf::from(r).join(windows)));
                }
            } else {
                for name in *linux {
                    paths.extend(std::env::split_paths(&path).map(|root| root.join(name)));
                }
            }
            let path = paths.into_iter().find(|p| p.is_file())?;
            Some(Candidate {
                id: (*id).into(),
                name: (*name).into(),
                path,
            })
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Status {},
    /// Start (and keep running) the detected browser, or the one with this id.
    Start {
        #[serde(default)]
        browser: Option<String>,
    },
    Stop {},
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    version: u8,
    browser: String,
    executable: PathBuf,
    /// Last CDP port, reused when free so a restart keeps the shared address.
    #[serde(default)]
    port: u16,
}
type Detect = Arc<dyn Fn() -> Vec<Candidate> + Send + Sync>;
/// The running browser: our child, or an instance already running on the profile.
enum Instance {
    Child(Child),
    Adopted {
        address: SocketAddr,
        checked: Instant,
    },
}
impl Instance {
    /// `false` once the browser is gone.
    async fn alive(&mut self) -> bool {
        match self {
            Self::Child(child) => matches!(child.try_wait(), Ok(None)),
            Self::Adopted { address, checked } => {
                if checked.elapsed() < ADOPTED_CHECK {
                    return true;
                }
                *checked = Instant::now();
                Provider::at(*address).discover().await.is_ok()
            }
        }
    }
    async fn stop(self) {
        match self {
            Self::Child(mut child) => {
                let _ = child.kill().await;
            }
            Self::Adopted { address, .. } => close(address).await,
        }
    }
}
type Request = (Command, oneshot::Sender<Result<Value>>);
pub struct Service {
    requests: mpsc::Sender<Request>,
    task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl Service {
    /// The browser runs until `stop` completes (the node's lifetime).
    pub fn start(peers: PathBuf, stop: impl Future<Output = ()> + Send + 'static) -> Self {
        Self::start_with(peers, stop, Arc::new(detect))
    }
    pub fn start_with(
        peers: PathBuf,
        stop: impl Future<Output = ()> + Send + 'static,
        detect: Detect,
    ) -> Self {
        let (requests, incoming) = mpsc::channel(8);
        let (stopped, watch) = watch::channel(false);
        tokio::spawn(async move {
            tokio::select! {
                _ = stop => { let _ = stopped.send(true); }
                _ = stopped.closed() => {}
            }
        });
        let task = tokio::spawn(Manager::new(peers, detect).run(incoming, watch));
        Self {
            requests,
            task: tokio::sync::Mutex::new(Some(task)),
        }
    }
    pub async fn manage(&self, command: Command) -> Result<Value> {
        let (send, receive) = oneshot::channel();
        self.requests
            .try_send((command, send))
            .map_err(|_| anyhow::anyhow!("Browser service is busy or stopping; retry shortly."))?;
        receive.await.context("Browser service stopped")?
    }
    pub async fn shutdown(&self) {
        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        }
    }
}

struct Manager {
    peers: PathBuf,
    spec_path: PathBuf,
    profile: PathBuf,
    detect: Detect,
    spec: Option<Spec>,
    child: Option<Instance>,
    /// The provider this manager wrote into the peer configuration.
    shared: Option<Provider>,
    version: Option<String>,
    error: Option<String>,
    retries: u8,
    next_retry: Instant,
    started: Instant,
}
impl Manager {
    fn new(peers: PathBuf, detect: Detect) -> Self {
        Self {
            spec_path: peers.with_file_name("browser-service.json"),
            profile: peers.with_file_name("browser-profile"),
            peers,
            detect,
            spec: None,
            child: None,
            shared: None,
            version: None,
            error: None,
            retries: 0,
            next_retry: Instant::now(),
            started: Instant::now(),
        }
    }
    async fn run(mut self, mut requests: mpsc::Receiver<Request>, stop: watch::Receiver<bool>) {
        let cancelled = || {
            let mut stop = stop.clone();
            async move {
                let _ = stop.wait_for(|stopped| *stopped).await;
            }
        };
        match self.read_spec() {
            Ok(spec) => self.spec = spec,
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        // A configured address from the previous run is ours until the restart moves it.
        if let Some(spec) = &self.spec {
            let previous = Provider::at((Ipv4Addr::LOCALHOST, spec.port).into());
            if Config::read(&self.peers).is_ok_and(|c| c.browser.as_ref() == Some(&previous)) {
                self.shared = Some(previous);
            }
        }
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                biased;
                _ = cancelled() => break,
                request = requests.recv() => {
                    let Some((command, reply)) = request else { break; };
                    tokio::select! {
                        _ = cancelled() => break,
                        result = self.manage(command) => { let _ = reply.send(result); }
                    }
                }
                _ = tick.tick() => tokio::select! {
                    _ = cancelled() => break,
                    _ = self.reconcile() => {},
                },
            }
        }
        // The browser follows the node; sharing stays configured for the next start.
        self.kill().await;
    }
    fn read_spec(&self) -> Result<Option<Spec>> {
        if !self.spec_path.try_exists()? {
            return Ok(None);
        }
        let file = std::fs::File::open(&self.spec_path)?;
        ensure!(
            file.metadata()?.len() <= 4096,
            "Managed browser configuration is too large."
        );
        let spec: Spec = serde_json::from_reader(file)
            .context("Invalid browser-service.json; remove it to reset the managed browser.")?;
        ensure!(
            spec.version == 1,
            "Unsupported managed browser configuration."
        );
        Ok(Some(spec))
    }
    fn write_spec(&self, spec: Option<&Spec>) -> Result<()> {
        match spec {
            Some(spec) => crate::config::write_private(
                &self.spec_path,
                &serde_json::to_vec_pretty(spec)?,
                true,
            ),
            None if self.spec_path.try_exists()? => Ok(std::fs::remove_file(&self.spec_path)?),
            None => Ok(()),
        }
    }
    fn status(&self) -> Value {
        let candidates = (self.detect)();
        let ready = self.child.is_some() && self.shared.is_some();
        json!({"candidates":candidates,"browser":self.spec.as_ref().map(|s| &s.browser).or(candidates.first().map(|c| &c.id)),
            "executable":self.spec.as_ref().map(|s| &s.executable).or(candidates.first().map(|c| &c.path)),
            "enabled":self.spec.is_some(),"running":self.child.is_some(),"ready":ready,
            "address":self.shared.as_ref().and_then(Provider::address),"version":self.version,"profile":self.profile,
            "phase":if ready {"running"} else if self.spec.is_some() && self.retries < MAX_RETRIES {"starting"} else if self.error.is_some() {"failed"} else {"stopped"},
            "error":self.error,"restart_attempts":self.retries})
    }
    async fn manage(&mut self, command: Command) -> Result<Value> {
        match command {
            Command::Status {} => {}
            Command::Start { browser } => {
                if self.child.is_some() {
                    return Ok(self.status());
                }
                let candidates = (self.detect)();
                let candidate = match &browser {
                    Some(id) => candidates.iter().find(|c| &c.id == id),
                    None => candidates.first(),
                }
                .context("Install Google Chrome, Chromium, Microsoft Edge or Brave first.")?;
                let current = Config::read(&self.peers)?;
                ensure!(
                    current.browser.is_none() || current.browser == self.shared,
                    "Stop sharing the existing browser before starting the managed browser."
                );
                let mut spec = Spec {
                    version: 1,
                    browser: candidate.id.clone(),
                    executable: candidate.path.clone(),
                    port: self.spec.as_ref().map_or(0, |s| s.port),
                };
                let (child, provider, version) = launch(&spec, &self.profile).await?;
                spec.port = provider.address().map_or(spec.port, |a| a.port());
                let saved = self
                    .write_spec(Some(&spec))
                    .and_then(|_| self.share(&provider));
                if let Err(error) = saved {
                    child.stop().await;
                    return Err(error);
                }
                self.child = Some(child);
                self.spec = Some(spec);
                self.version = version;
                self.error = None;
                self.retries = 0;
                self.started = Instant::now();
            }
            Command::Stop {} => {
                self.unshare()?;
                self.write_spec(None)?;
                self.spec = None;
                self.kill().await;
                self.error = None;
                self.retries = 0;
            }
        }
        Ok(self.status())
    }
    /// Point the `browser` provider at the managed instance (a new epoch on change).
    fn share(&mut self, provider: &Provider) -> Result<()> {
        let previous = self.shared.clone();
        Config::update(&self.peers, |c| {
            ensure!(
                c.browser.is_none() || c.browser == previous,
                "Browser sharing changed while the managed browser started."
            );
            if c.browser.as_ref() != Some(provider) {
                c.browser = Some(provider.clone());
                c.browser_epoch = c
                    .browser_epoch
                    .checked_add(1)
                    .context("browser generation exhausted")?;
            }
            Ok(())
        })?;
        self.shared = Some(provider.clone());
        Ok(())
    }
    fn unshare(&mut self) -> Result<()> {
        if let Some(shared) = self.shared.take() {
            Config::update(&self.peers, |c| {
                if c.browser.as_ref() == Some(&shared) {
                    c.browser = None;
                    c.browser_epoch = c
                        .browser_epoch
                        .checked_add(1)
                        .context("browser generation exhausted")?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }
    async fn kill(&mut self) {
        if let Some(child) = self.child.take() {
            child.stop().await;
        }
    }
    async fn reconcile(&mut self) {
        let Some(spec) = self.spec.clone() else {
            self.kill().await;
            return;
        };
        // The owner chose another browser or disabled sharing: give up management.
        if let Some(shared) = &self.shared
            && Config::read(&self.peers).is_ok_and(|c| c.browser.as_ref() != Some(shared))
        {
            self.shared = None;
            self.spec = None;
            let _ = self.write_spec(None);
            self.kill().await;
            self.error =
                Some("Browser sharing was changed elsewhere; the managed browser stopped.".into());
            return;
        }
        if let Some(child) = &mut self.child {
            if child.alive().await {
                if self.started.elapsed() >= Duration::from_secs(60) {
                    self.retries = 0;
                }
                return;
            }
            self.error = Some("The managed browser exited; restarting it.".into());
            self.child = None;
            self.next_retry = Instant::now() + Duration::from_secs(1);
        }
        if self.retries >= MAX_RETRIES || Instant::now() < self.next_retry {
            return;
        }
        self.retries += 1;
        self.next_retry =
            Instant::now() + Duration::from_secs(2u64.pow(u32::from(self.retries.min(5))));
        match launch(&spec, &self.profile).await {
            Ok((child, provider, version)) => {
                let mut spec = spec;
                spec.port = provider.address().map_or(spec.port, |a| a.port());
                match self
                    .write_spec(Some(&spec))
                    .and_then(|_| self.share(&provider))
                {
                    Ok(()) => {
                        self.child = Some(child);
                        self.spec = Some(spec);
                        self.version = version;
                        self.error = None;
                        self.started = Instant::now();
                    }
                    Err(error) => self.error = Some(format!("{error:#}")),
                }
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
    }
}

fn free_port(preferred: u16) -> Result<u16> {
    if preferred >= 1024 && std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, preferred)).is_ok() {
        return Ok(preferred);
    }
    Ok(std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?
        .local_addr()?
        .port())
}
/// The instance already serving CDP on `profile`, if one is running.
async fn running_on(profile: &Path) -> Option<(SocketAddr, Option<String>)> {
    let (port, _) = super::active_port(profile).ok()?;
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let (metadata, _) = Provider::at(address).discover().await.ok()?;
    Some((address, metadata["version"].as_str().map(str::to_owned)))
}
/// Ask an adopted browser to quit over CDP (it is not our child to kill).
async fn close(address: SocketAddr) {
    use futures_util::SinkExt;
    let Ok((_, id)) = Provider::at(address).discover().await else {
        return;
    };
    let url = format!("ws://{address}/devtools/browser/{id}");
    let closing = async {
        let (mut socket, _) = tokio_tungstenite::connect_async(url.as_str()).await.ok()?;
        let message = json!({"id": 1, "method": "Browser.close"}).to_string();
        socket.send(message.into()).await.ok()
    };
    let _ = tokio::time::timeout(Duration::from_secs(3), closing).await;
}
async fn launch(spec: &Spec, profile: &Path) -> Result<(Instance, Provider, Option<String>)> {
    ensure!(
        spec.executable.is_absolute() && spec.executable.is_file(),
        "The configured browser is no longer installed; start the managed browser again."
    );
    if let Some((address, version)) = running_on(profile).await {
        let instance = Instance::Adopted {
            address,
            checked: Instant::now(),
        };
        return Ok((instance, Provider::at(address), version));
    }
    std::fs::create_dir_all(profile)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(profile, std::fs::Permissions::from_mode(0o700))?;
    }
    let port = free_port(spec.port)?;
    let provider = Provider::at(SocketAddr::from((Ipv4Addr::LOCALHOST, port)));
    let mut child = Process::new(&spec.executable)
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("--remote-debugging-address=127.0.0.1")
        .arg(format!("--remote-debugging-port={port}"))
        .args([
            "--no-first-run",
            "--no-default-browser-check",
            // Agents open the tabs they need; no blank window at every start.
            "--no-startup-window",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("Start {}", spec.executable.display()))?;
    let ready = tokio::time::timeout(READY_TIMEOUT, async {
        loop {
            ensure!(
                child.try_wait()?.is_none(),
                "The browser exited during startup. Close other windows using its managed profile and retry."
            );
            if let Ok((metadata, _)) = provider.discover().await {
                return Ok(metadata["version"].as_str().map(str::to_owned));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await;
    match ready {
        Ok(Ok(version)) => Ok((Instance::Child(child), provider, version)),
        result => {
            let _ = child.kill().await;
            match result {
                Ok(Err(error)) => Err(error),
                _ => anyhow::bail!("The browser did not open its debugging port in time."),
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// Not a test: the fake browser process, re-executed from this test binary.
    #[test]
    #[ignore]
    fn fake_browser() {
        let Ok(args) = std::env::var("CRANIBOX_FAKE_BROWSER_ARGS") else {
            return;
        };
        let log = std::env::var("CRANIBOX_FAKE_BROWSER_LOG").unwrap();
        let launches = std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .count();
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .unwrap()
            .write_all(format!("{args}\n").as_bytes())
            .unwrap();
        let port: u16 = args
            .split(' ')
            .find_map(|a| a.strip_prefix("--remote-debugging-port="))
            .unwrap()
            .parse()
            .unwrap();
        if launches == 0 {
            // The first instance crashes, so the supervisor must restart it.
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(1500));
                std::process::exit(9);
            });
        }
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).unwrap();
        // Like Chrome, announce the live endpoint inside the profile.
        if let Some(profile) = args
            .split(' ')
            .find_map(|a| a.strip_prefix("--user-data-dir="))
        {
            let _ = std::fs::create_dir_all(profile);
            std::fs::write(
                Path::new(profile).join("DevToolsActivePort"),
                format!("{port}\n/devtools/browser/fake-{launches}\n"),
            )
            .unwrap();
        }
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut request = [0; 2048];
            let _ = stream.read(&mut request);
            let body = format!(
                r#"{{"Browser":"FakeChrome/1.{launches}","webSocketDebuggerUrl":"ws://127.0.0.1:{port}/devtools/browser/fake-{launches}"}}"#
            );
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    }

    async fn wait(service: &Service, check: impl Fn(&Value) -> bool) -> Value {
        for _ in 0..200 {
            let status = service.manage(Command::Status {}).await.unwrap();
            if check(&status) {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("browser supervisor did not reach the expected state");
    }

    #[tokio::test]
    async fn supervisor_starts_restarts_shares_and_stops_a_dedicated_browser() {
        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        Config::update(&peers, |_| Ok(())).unwrap();
        let log = root.path().join("launches.log");
        let wrapper = root.path().join("fake-chrome");
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nCRANIBOX_FAKE_BROWSER_ARGS=\"$*\" CRANIBOX_FAKE_BROWSER_LOG='{}' exec '{}' --exact peers::browser::manager::tests::fake_browser --ignored --nocapture --test-threads=1 >/dev/null 2>&1\n",
                log.display(),
                std::env::current_exe().unwrap().display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let candidate = Candidate {
            id: "chrome".into(),
            name: "Google Chrome".into(),
            path: wrapper,
        };
        let (stop, stopped) = oneshot::channel::<()>();
        let service = Service::start_with(
            peers.clone(),
            async move {
                let _ = stopped.await;
            },
            Arc::new(move || vec![candidate.clone()]),
        );
        let idle = service.manage(Command::Status {}).await.unwrap();
        assert_eq!(
            (idle["phase"].as_str(), idle["browser"].as_str()),
            (Some("stopped"), Some("chrome"))
        );
        assert!(
            service
                .manage(Command::Start {
                    browser: Some("edge".into())
                })
                .await
                .is_err()
        );

        let epoch = Config::read(&peers).unwrap().browser_epoch;
        let started = service
            .manage(Command::Start { browser: None })
            .await
            .unwrap();
        assert_eq!(started["ready"], true);
        assert_eq!(started["version"], "FakeChrome/1.0");
        let address: SocketAddr = serde_json::from_value(started["address"].clone()).unwrap();
        let config = Config::read(&peers).unwrap();
        assert_eq!(config.browser, Some(Provider::at(address)));
        assert_eq!(config.browser_epoch, epoch + 1);
        let args = std::fs::read_to_string(&log).unwrap();
        for expected in [
            format!(
                "--user-data-dir={}",
                root.path().join("browser-profile").display()
            ),
            "--remote-debugging-address=127.0.0.1".into(),
            format!("--remote-debugging-port={}", address.port()),
            "--no-first-run".into(),
            "--no-startup-window".into(),
        ] {
            assert!(args.contains(&expected), "{args} lacks {expected}");
        }

        // The first instance exits; the supervisor restarts it on the same port.
        let restarted = wait(&service, |s| {
            s["ready"] == true && s["version"] == "FakeChrome/1.1"
        })
        .await;
        assert_eq!(restarted["address"], started["address"]);
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 2);
        assert_eq!(
            Config::read(&peers).unwrap().browser,
            Some(Provider::at(address))
        );

        let stopped = service.manage(Command::Stop {}).await.unwrap();
        assert_eq!(
            (stopped["running"].as_bool(), stopped["enabled"].as_bool()),
            (Some(false), Some(false))
        );
        assert!(Config::read(&peers).unwrap().browser.is_none());
        assert!(Provider::at(address).discover().await.is_err());
        assert!(!root.path().join("browser-service.json").exists());
        let _ = stop.send(());
        service.shutdown().await;
    }

    #[tokio::test]
    async fn an_instance_left_running_on_the_profile_is_adopted_not_launched_again() {
        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        Config::update(&peers, |_| Ok(())).unwrap();
        let log = root.path().join("launches.log");
        // One earlier launch on record, so the fake does not crash on purpose.
        std::fs::write(&log, "earlier\n").unwrap();
        let wrapper = root.path().join("fake-chrome");
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nCRANIBOX_FAKE_BROWSER_ARGS=\"$*\" CRANIBOX_FAKE_BROWSER_LOG='{}' exec '{}' --exact peers::browser::manager::tests::fake_browser --ignored --nocapture --test-threads=1 >/dev/null 2>&1\n",
                log.display(),
                std::env::current_exe().unwrap().display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // The previous node run left its browser behind (e.g. the app quit).
        let port = free_port(0).unwrap();
        let profile = root.path().join("browser-profile");
        let mut orphan = Process::new(&wrapper)
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg(format!("--remote-debugging-port={port}"))
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        for _ in 0..100 {
            if running_on(&profile).await.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let spec = json!({"version": 1, "browser": "chrome", "executable": wrapper, "port": port});
        std::fs::write(root.path().join("browser-service.json"), spec.to_string()).unwrap();

        let (stop, stopped) = oneshot::channel::<()>();
        let service = Service::start_with(
            peers.clone(),
            async move {
                let _ = stopped.await;
            },
            Arc::new(Vec::new),
        );
        let adopted = wait(&service, |s| s["ready"] == true).await;
        assert_eq!(adopted["address"], json!(address));
        assert_eq!(
            Config::read(&peers).unwrap().browser,
            Some(Provider::at(address))
        );
        // Only the orphan ever launched: no second instance, so no extra blank tab.
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 2);

        // Once it is gone, the supervisor launches a fresh one.
        orphan.kill().await.unwrap();
        let relaunched = wait(&service, |s| {
            s["ready"] == true && std::fs::read_to_string(&log).unwrap().lines().count() == 3
        })
        .await;
        assert_eq!(relaunched["phase"], "running");
        let _ = stop.send(());
        service.shutdown().await;
    }
}
