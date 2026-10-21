//! Local capability providers for the App API: detection, OS permissions and the
//! sharing switch, for terminal, desktop (VNC), Computer Use (cua-driver) and the
//! managed browser. Peer grants still decide which device may use each one.
use super::{error, unavailable};
use crate::desktop_service;
use agent_workspace_runtime::{
    config as secrets,
    peers::{
        self,
        browser::manager,
        computer::{self, ProviderKind, cua},
        config::{Config, Desktop, DesktopProtocol},
    },
};
use anyhow::{Context, Result, ensure};
use cranibox_contracts::app::{
    AppError, Capabilities, CapabilityAction, CapabilityId, CapabilityItem, CapabilityPermission,
    CapabilityProvider, CapabilitySharing, InstallPlan, InstallReq, PermissionId, PermissionState,
    Platform,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::Read,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{process::Command, sync::Mutex, task::JoinHandle};

const PLAN_TTL: Duration = Duration::from_secs(600);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(900);
/// macOS Screen Sharing, the system VNC server.
const SCREEN_SHARING: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 5900);
const SHARING_SETTINGS: &str = "x-apple.systempreferences:com.apple.Sharing-Settings.extension";
const PRIVACY_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

/// What the capability layer needs from the running node.
#[async_trait::async_trait]
pub trait Host: Send + Sync {
    fn config_path(&self) -> &Path;
    async fn desktop(&self, command: desktop_service::Command) -> Result<Value>;
    async fn browser(&self, command: manager::Command) -> Result<Value>;
    async fn peers(&self, command: peers::Command) -> Result<Value>;
}
#[async_trait::async_trait]
impl Host for crate::local::LocalNode {
    fn config_path(&self) -> &Path {
        crate::local::LocalNode::config_path(self)
    }
    async fn desktop(&self, command: desktop_service::Command) -> Result<Value> {
        self.manage_desktop_service(command).await
    }
    async fn browser(&self, command: manager::Command) -> Result<Value> {
        self.manage_browser_service(command).await
    }
    async fn peers(&self, command: peers::Command) -> Result<Value> {
        self.manage_peers(command).await
    }
}

pub fn platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::Macos
    } else if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Linux
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
fn conflict(message: impl Into<String>) -> AppError {
    error(409, "conflict", message)
}
fn failed(e: anyhow::Error) -> AppError {
    error(409, "failed", format!("{e:#}"))
}

/// Reviewed-plan → single-execution installation of cua-driver.
struct Plan {
    id: String,
    created: Instant,
    command: String,
}
#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
enum Phase {
    #[default]
    Idle,
    Running,
    Succeeded,
    Failed,
    Interrupted,
}
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    phase: Phase,
    error: Option<String>,
}
struct Setup {
    path: PathBuf,
    record: Record,
    plan: Option<Plan>,
    task: Option<JoinHandle<Result<()>>>,
}
/// The exact command `install` runs, as shown for review.
pub fn install_command() -> Result<String> {
    ensure!(
        !cfg!(windows),
        "Install cua-driver on Windows with its PowerShell installer (https://cua.ai/docs/cua-driver/quickstart)."
    );
    Ok(format!(
        "/bin/bash -c \"$(curl -fsSL {})\"",
        cua::INSTALL_URL
    ))
}
impl Setup {
    fn new(peers: &Path) -> Self {
        let path = peers.with_file_name("cua-setup.json");
        let mut record = match std::fs::File::open(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Record::default(),
            result => result
                .ok()
                .and_then(|file| {
                    let mut bytes = Vec::new();
                    file.take(8193).read_to_end(&mut bytes).ok()?;
                    (bytes.len() <= 8192).then_some(bytes)
                })
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_else(|| Record {
                    phase: Phase::Interrupted,
                    error: Some("The previous cua-driver installation record is unreadable. Check ~/.cua-driver before reviewing a new plan.".into()),
                }),
        };
        if record.phase == Phase::Running {
            record.phase = Phase::Interrupted;
            record.error = Some("The node restarted during cua-driver installation. Check ~/.cua-driver before reviewing a new plan.".into());
        }
        Self {
            path,
            record,
            plan: None,
            task: None,
        }
    }
    fn running(&self) -> bool {
        self.task.is_some()
    }
    fn review(&mut self) -> Result<InstallPlan> {
        ensure!(
            !self.running(),
            "cua-driver installation is already running."
        );
        self.plan = None;
        let command = install_command()?;
        let id = uuid::Uuid::new_v4().to_string();
        let expires_at = now() + PLAN_TTL.as_secs();
        self.plan = Some(Plan {
            id: id.clone(),
            created: Instant::now(),
            command: command.clone(),
        });
        Ok(InstallPlan {
            plan_id: id,
            commands: vec![command],
            url: Some(cua::INSTALL_URL.into()),
            expires_at,
        })
    }
    /// Consume the reviewed plan, whether or not the work can start.
    fn begin(
        &mut self,
        id: &str,
        work: impl FnOnce(String) -> JoinHandle<Result<()>>,
    ) -> Result<()> {
        ensure!(
            !self.running(),
            "cua-driver installation is already running."
        );
        let plan = self
            .plan
            .take()
            .context("Review a new cua-driver installation plan first.")?;
        ensure!(
            plan.id == id
                && plan.created.elapsed() < PLAN_TTL
                && Some(&plan.command) == install_command().ok().as_ref(),
            "The installation plan changed, expired or was already used. Review a new plan."
        );
        let record = Record {
            phase: Phase::Running,
            error: None,
        };
        // Without a journal no installer may start.
        secrets::write_private(&self.path, &serde_json::to_vec(&record)?, true)?;
        self.record = record;
        self.task = Some(work(plan.command));
        Ok(())
    }
    async fn reconcile(&mut self) {
        if !self.task.as_ref().is_some_and(|t| t.is_finished()) {
            return;
        }
        let result = self
            .task
            .take()
            .unwrap()
            .await
            .context("installation monitor stopped")
            .and_then(|r| r);
        self.record.phase = if result.is_ok() {
            Phase::Succeeded
        } else {
            Phase::Failed
        };
        self.record.error = result.err().map(|e| format!("{e:#}"));
        if serde_json::to_vec(&self.record)
            .map_err(anyhow::Error::from)
            .and_then(|b| secrets::write_private(&self.path, &b, true))
            .is_err()
        {
            self.record.error =
                Some("cua-driver installation finished, but its result could not be saved.".into());
        }
    }
}
impl Drop for Setup {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
async fn install(command: String) -> Result<()> {
    // A user-level installer: it may outlive a stopped node, like other system installers.
    let mut child = Command::new("/bin/bash")
        .args(["-c", &command])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(false)
        .spawn()
        .context("start the cua-driver installer")?;
    let status = tokio::time::timeout(INSTALL_TIMEOUT, child.wait())
        .await
        .context("The cua-driver installer is still running; check it before retrying.")??;
    ensure!(
        status.success(),
        "The cua-driver installer failed (exit {}).",
        status.code().map_or("unknown".into(), |c| c.to_string())
    );
    ensure!(
        cua::locate().is_some(),
        "The installer finished but cua-driver was not found on PATH, in ~/.local/bin or /Applications."
    );
    Ok(())
}

pub struct Service {
    setup: Mutex<Setup>,
    /// One owner action at a time; status reads never wait for it.
    actions: Mutex<()>,
}
impl Service {
    pub fn new(peers: &Path) -> Self {
        Self {
            setup: Mutex::new(Setup::new(peers)),
            actions: Mutex::new(()),
        }
    }
    pub async fn list(&self, host: &dyn Host) -> Result<Capabilities, AppError> {
        let config = Config::read(host.config_path()).map_err(unavailable)?;
        let (desktop, computer, browser) = tokio::join!(
            self.desktop(host, &config),
            self.computer(&config),
            self.browser(host, &config)
        );
        Ok(Capabilities {
            platform: platform(),
            items: vec![terminal(&config), desktop, computer, browser],
        })
    }
    async fn item(&self, host: &dyn Host, id: CapabilityId) -> Result<CapabilityItem, AppError> {
        let config = Config::read(host.config_path()).map_err(unavailable)?;
        Ok(match id {
            CapabilityId::Terminal => terminal(&config),
            CapabilityId::Desktop => self.desktop(host, &config).await,
            CapabilityId::Computer => self.computer(&config).await,
            CapabilityId::Browser => self.browser(host, &config).await,
        })
    }
    /// `POST /capabilities/{id}/{action}`: the updated item, or an [`InstallPlan`].
    pub async fn act(
        &self,
        host: &dyn Host,
        id: CapabilityId,
        action: CapabilityAction,
        body: Value,
    ) -> Result<Value, AppError> {
        let _serial = self.actions.lock().await;
        let item = self.item(host, id).await?;
        if action != CapabilityAction::Refresh && !item.actions.contains(&action) {
            return Err(conflict(
                "This action does not apply in the current state; refresh and retry.",
            ));
        }
        let path = host.config_path().to_path_buf();
        use CapabilityAction::*;
        match (id, action) {
            (_, Refresh) => {}
            (CapabilityId::Computer, InstallPlan) => {
                let plan = self.setup.lock().await.review().map_err(failed)?;
                return serde_json::to_value(plan).map_err(unavailable);
            }
            (CapabilityId::Computer, Install) => {
                let req: InstallReq = serde_json::from_value(body)
                    .map_err(|_| error(400, "invalid", "Install requires a reviewed plan_id"))?;
                self.setup
                    .lock()
                    .await
                    .begin(&req.plan_id, |command| tokio::spawn(install(command)))
                    .map_err(failed)?;
            }
            (CapabilityId::Computer, Start) => {
                cua::start_daemon(&driver()?).await.map_err(failed)?;
            }
            (CapabilityId::Computer, Stop) => {
                cua::driver().stop().await;
                cua::stop_daemon(&driver()?).await.map_err(failed)?;
            }
            (CapabilityId::Computer, Enable) => {
                let ready = computer::bridge(
                    &computer::Provider::cua(),
                    json!({"action":"status"}),
                    "local-owner",
                )
                .await
                .map_err(failed)?;
                if ready["backend_available"] != true {
                    return Err(conflict(
                        "cua-driver is not answering; start it in your desktop session, then retry.",
                    ));
                }
                host.peers(peers::Command::ComputerSharing {
                    computer: Some(computer::Provider::cua()),
                })
                .await
                .map_err(failed)?;
            }
            (CapabilityId::Computer, Disable) => {
                host.peers(peers::Command::ComputerSharing { computer: None })
                    .await
                    .map_err(failed)?;
                cua::driver().stop().await;
            }
            (CapabilityId::Computer, RequestPermissions) => {
                cua::request_permissions(&driver()?).await.map_err(failed)?;
            }
            (CapabilityId::Computer, OpenSettings) => {
                open(PRIVACY_SETTINGS).await.map_err(failed)?
            }
            (CapabilityId::Desktop, OpenSettings) => {
                open(SHARING_SETTINGS).await.map_err(failed)?
            }
            (CapabilityId::Desktop, InstallPlan) => {
                let review = host
                    .desktop(desktop_service::Command::Plan {})
                    .await
                    .map_err(failed)?;
                let plan = cranibox_contracts::app::InstallPlan {
                    plan_id: review["id"].as_str().unwrap_or_default().into(),
                    commands: review["commands"]
                        .as_str()
                        .map(|c| vec![c.to_owned()])
                        .unwrap_or_default(),
                    url: review["source"].as_str().map(str::to_owned),
                    expires_at: now() + review["expires_in_seconds"].as_u64().unwrap_or(0),
                };
                return serde_json::to_value(plan).map_err(unavailable);
            }
            (CapabilityId::Desktop, Install) => {
                let req: InstallReq = serde_json::from_value(body)
                    .map_err(|_| error(400, "invalid", "Install requires a reviewed plan_id"))?;
                host.desktop(desktop_service::Command::Install {
                    plan_id: req.plan_id,
                })
                .await
                .map_err(failed)?;
            }
            (CapabilityId::Desktop, Start) => {
                let status = host
                    .desktop(desktop_service::Command::Status {})
                    .await
                    .map_err(failed)?;
                host.desktop(desktop_service::Command::Start {
                    display: status["display"].as_str().unwrap_or_default().into(),
                    port: status["port"]
                        .as_u64()
                        .and_then(|p| p.try_into().ok())
                        .unwrap_or(5900),
                    provider: serde_json::from_value(status["provider"].clone())
                        .unwrap_or_default(),
                    output: status["output"].as_str().unwrap_or_default().into(),
                })
                .await
                .map_err(failed)?;
            }
            (CapabilityId::Desktop, Stop) => {
                host.desktop(desktop_service::Command::Stop {})
                    .await
                    .map_err(failed)?;
            }
            (CapabilityId::Desktop, Enable) => {
                let desktop = self.detected_desktop(host).await.ok_or_else(|| {
                    conflict("No desktop server answers on this device; turn on screen sharing, then retry.")
                })?;
                share_desktop(&path, Some(desktop)).map_err(failed)?;
            }
            (CapabilityId::Desktop, Disable) => {
                let status = host.desktop(desktop_service::Command::Status {}).await.ok();
                if status.as_ref().is_some_and(|s| s["enabled"] == true) {
                    host.desktop(desktop_service::Command::Stop {})
                        .await
                        .map_err(failed)?;
                } else {
                    share_desktop(&path, None).map_err(failed)?;
                }
            }
            (CapabilityId::Browser, Start) => {
                host.browser(manager::Command::Start { browser: None })
                    .await
                    .map_err(failed)?;
            }
            (CapabilityId::Browser, Stop) => {
                host.browser(manager::Command::Stop {})
                    .await
                    .map_err(failed)?;
            }
            (CapabilityId::Browser, Disable) => {
                host.peers(peers::Command::BrowserSharing { browser: None })
                    .await
                    .map_err(failed)?;
            }
            _ => return Err(conflict("This action does not apply to this capability.")),
        }
        serde_json::to_value(self.item(host, id).await?).map_err(unavailable)
    }

    /// The desktop server to share: macOS Screen Sharing, else a detected VNC/Selkies.
    async fn detected_desktop(&self, host: &dyn Host) -> Option<Desktop> {
        if cfg!(target_os = "macos") {
            return desktop_service::rfb(SCREEN_SHARING)
                .await
                .ok()
                .map(|_| Desktop {
                    address: SCREEN_SHARING,
                    protocol: DesktopProtocol::Vnc,
                });
        }
        let found = host
            .desktop(desktop_service::Command::Discover {})
            .await
            .ok()?;
        let services: Vec<Desktop> = found["services"]
            .as_array()?
            .iter()
            .filter_map(|s| {
                Some(Desktop {
                    address: s["address"].as_str()?.parse().ok()?,
                    protocol: serde_json::from_value(s["protocol"].clone()).ok()?,
                })
            })
            .collect();
        services
            .iter()
            .find(|d| d.protocol == DesktopProtocol::Vnc)
            .or(services.first())
            .cloned()
    }
    async fn desktop(&self, host: &dyn Host, config: &Config) -> CapabilityItem {
        use CapabilityAction::*;
        let shared = config.desktop.as_ref();
        let sharing = CapabilitySharing {
            enabled: shared.is_some(),
            address: shared.map(|d| d.address.to_string()),
        };
        if cfg!(target_os = "macos") {
            let on = desktop_service::rfb(SCREEN_SHARING).await.is_ok();
            let mut actions = vec![];
            if sharing.enabled {
                actions.push(Disable);
            } else if on {
                actions.push(Enable);
            }
            actions.extend([OpenSettings, Refresh]);
            return CapabilityItem {
                id: CapabilityId::Desktop,
                provider: CapabilityProvider {
                    kind: "screen_sharing".into(),
                    installed: true,
                    running: on,
                    version: None,
                    detail: Some(if on {
                        format!("macOS Screen Sharing on {SCREEN_SHARING}")
                    } else {
                        "Turn on Screen Sharing in System Settings → General → Sharing.".into()
                    }),
                },
                permissions: vec![],
                sharing,
                actions,
                error: None,
            };
        }
        let status = host
            .desktop(desktop_service::Command::Status {})
            .await
            .unwrap_or_else(|e| json!({"error":format!("{e:#}")}));
        let external = if status["running"] == true {
            None
        } else {
            self.detected_desktop(host).await
        };
        let managed_running = status["running"] == true;
        let installed = status["installed"] == true || external.is_some();
        let mut actions = vec![];
        if status["installed"] != true
            && external.is_none()
            && status["can_install"] == true
            && status["supported"] == true
        {
            actions.extend([InstallPlan, Install]);
        }
        if managed_running {
            actions.push(Stop);
        } else if status["installed"] == true && status["supported"] == true && !sharing.enabled {
            actions.push(Start);
        }
        if sharing.enabled {
            actions.push(Disable);
        } else if external.is_some() {
            actions.push(Enable);
        }
        actions.push(Refresh);
        CapabilityItem {
            id: CapabilityId::Desktop,
            provider: CapabilityProvider {
                kind: if external.is_some() && !managed_running {
                    match external.as_ref().map(|d| d.protocol) {
                        Some(DesktopProtocol::Selkies) => "selkies".into(),
                        _ => "vnc".into(),
                    }
                } else {
                    status["provider"].as_str().unwrap_or("vnc").into()
                },
                installed,
                running: managed_running || external.is_some(),
                version: None,
                detail: external
                    .map(|d| format!("Desktop server on {}", d.address))
                    .or_else(|| status["unsupported_reason"].as_str().map(str::to_owned)),
            },
            permissions: vec![],
            sharing,
            actions,
            error: status["error"].as_str().map(str::to_owned),
        }
    }
    async fn computer(&self, config: &Config) -> CapabilityItem {
        use CapabilityAction::*;
        let mut setup = self.setup.lock().await;
        setup.reconcile().await;
        let installing = setup.running();
        let setup_error = setup.record.error.clone();
        drop(setup);
        if let Some(provider) = config
            .computer
            .as_ref()
            .filter(|p| p.kind == ProviderKind::Http)
        {
            let status = computer::bridge(provider, json!({"action":"status"}), "local-owner")
                .await
                .ok();
            return CapabilityItem {
                id: CapabilityId::Computer,
                provider: CapabilityProvider {
                    kind: "http".into(),
                    installed: true,
                    running: status
                        .as_ref()
                        .is_some_and(|s| s["backend_available"] == true),
                    version: None,
                    detail: Some("agent-desktop-bridge".into()),
                },
                permissions: not_required(),
                sharing: CapabilitySharing {
                    enabled: true,
                    address: provider.address.map(|a| a.to_string()),
                },
                actions: vec![Disable, Refresh],
                error: status
                    .is_none()
                    .then(|| "The Computer Use bridge is not answering.".into()),
            };
        }
        let program = cua::locate();
        let enabled = config.computer.is_some();
        let (version, running, permissions) = match &program {
            Some(program) => {
                let (version, running, permissions) = tokio::join!(
                    cua::version(program),
                    cua::daemon_running(program),
                    cua::permissions(program)
                );
                (version, running, permissions)
            }
            None => (None, false, None),
        };
        let permissions = if cfg!(target_os = "macos") {
            let state = |granted: Option<bool>| match granted {
                Some(true) => PermissionState::Granted,
                Some(false) => PermissionState::Denied,
                None => PermissionState::Unknown,
            };
            let (screen, accessibility) = permissions.unwrap_or((None, None));
            vec![
                CapabilityPermission {
                    id: PermissionId::ScreenRecording,
                    state: state(screen),
                },
                CapabilityPermission {
                    id: PermissionId::Accessibility,
                    state: state(accessibility),
                },
            ]
        } else {
            not_required()
        };
        let mut actions = vec![];
        if program.is_none() {
            if !installing && install_command().is_ok() {
                actions.extend([InstallPlan, Install]);
            }
        } else {
            actions.push(if running { Stop } else { Start });
            actions.push(if enabled { Disable } else { Enable });
            if cfg!(target_os = "macos") {
                actions.extend([RequestPermissions, OpenSettings]);
            }
        }
        actions.push(Refresh);
        let missing = permissions
            .iter()
            .any(|p| p.state == PermissionState::Denied);
        CapabilityItem {
            id: CapabilityId::Computer,
            provider: CapabilityProvider {
                kind: "cua".into(),
                installed: program.is_some(),
                running,
                version,
                detail: if installing {
                    Some("Installing cua-driver…".into())
                } else {
                    program.map(|p| p.display().to_string())
                },
            },
            permissions,
            sharing: CapabilitySharing {
                enabled,
                address: None,
            },
            actions,
            error: if missing {
                Some("Grant Screen Recording and Accessibility to CuaDriver in System Settings, then reopen CuaDriver.".into())
            } else if installing {
                None
            } else {
                setup_error
            },
        }
    }
    async fn browser(&self, host: &dyn Host, config: &Config) -> CapabilityItem {
        use CapabilityAction::*;
        let status = host
            .browser(manager::Command::Status {})
            .await
            .unwrap_or_else(|e| json!({"error":format!("{e:#}")}));
        let installed = status["candidates"]
            .as_array()
            .is_some_and(|c| !c.is_empty());
        let managed = status["enabled"] == true;
        let mut actions = vec![];
        if managed {
            actions.push(Stop);
        } else if installed {
            actions.push(Start);
        }
        if config.browser.is_some() && !managed {
            actions.push(Disable);
        }
        actions.push(Refresh);
        CapabilityItem {
            id: CapabilityId::Browser,
            provider: CapabilityProvider {
                kind: status["browser"].as_str().unwrap_or("chrome").into(),
                installed,
                running: status["ready"] == true,
                version: status["version"].as_str().map(str::to_owned),
                detail: status["executable"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| {
                        (!installed).then(|| {
                            "Install Google Chrome, Chromium, Microsoft Edge or Brave.".into()
                        })
                    }),
            },
            permissions: vec![],
            sharing: CapabilitySharing {
                enabled: config.browser.is_some(),
                address: config.browser.as_ref().map(|b| b.address.to_string()),
            },
            actions,
            error: status["error"].as_str().map(str::to_owned),
        }
    }
}
fn terminal(config: &Config) -> CapabilityItem {
    CapabilityItem {
        id: CapabilityId::Terminal,
        provider: CapabilityProvider {
            kind: "pty".into(),
            installed: true,
            running: true,
            version: None,
            detail: None,
        },
        permissions: vec![],
        sharing: CapabilitySharing {
            enabled: config.grants.values().any(|g| g.permissions.terminals),
            address: None,
        },
        actions: vec![CapabilityAction::Refresh],
        error: None,
    }
}
fn not_required() -> Vec<CapabilityPermission> {
    [PermissionId::ScreenRecording, PermissionId::Accessibility]
        .map(|id| CapabilityPermission {
            id,
            state: PermissionState::NotRequired,
        })
        .into()
}
fn driver() -> Result<PathBuf, AppError> {
    cua::locate().ok_or_else(|| conflict("Install cua-driver first."))
}
/// Share (or stop sharing) a desktop server; a probe must succeed before sharing.
fn share_desktop(path: &Path, desktop: Option<Desktop>) -> Result<()> {
    if let Some(desktop) = &desktop {
        desktop.validate()?;
    }
    Config::update(path, |c| {
        if c.desktop != desktop {
            c.desktop = desktop;
            c.desktop_epoch = c
                .desktop_epoch
                .checked_add(1)
                .context("desktop generation exhausted")?;
        }
        Ok(())
    })?;
    Ok(())
}
/// Open a System Settings deep link (macOS only).
async fn open(url: &str) -> Result<()> {
    ensure!(
        cfg!(target_os = "macos"),
        "Open your system settings to change this."
    );
    let status = Command::new("/usr/bin/open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await?;
    ensure!(status.success(), "System Settings could not be opened");
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct Fake {
        path: PathBuf,
        browser: manager::Service,
    }
    #[async_trait::async_trait]
    impl Host for Fake {
        fn config_path(&self) -> &Path {
            &self.path
        }
        async fn desktop(&self, command: desktop_service::Command) -> Result<Value> {
            Ok(match command {
                desktop_service::Command::Discover {} => json!({"services":[]}),
                _ => json!({"provider":"x11vnc","installed":false,"running":false,"enabled":false,
                    "supported":true,"can_install":false,"error":null}),
            })
        }
        async fn browser(&self, command: manager::Command) -> Result<Value> {
            self.browser.manage(command).await
        }
        async fn peers(&self, command: peers::Command) -> Result<Value> {
            peers::Store::new(self.path.clone()).manage(command).await
        }
    }

    #[tokio::test]
    async fn capabilities_follow_the_app_contract() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("peers.json");
        Config::update(&path, |_| Ok(())).unwrap();
        let (_stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let host = Fake {
            browser: manager::Service::start_with(
                path.clone(),
                async move {
                    let _ = stopped.await;
                },
                Arc::new(Vec::new),
            ),
            path: path.clone(),
        };
        let service = Service::new(&path);
        let list = service.list(&host).await.unwrap();
        let value = serde_json::to_value(&list).unwrap();
        // The exact JSON shape the UI builds against.
        assert!(matches!(
            value["platform"].as_str(),
            Some("linux" | "macos")
        ));
        let ids: Vec<_> = value["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["terminal", "desktop", "computer", "browser"]);
        for item in value["items"].as_array().unwrap() {
            let keys: Vec<_> = item.as_object().unwrap().keys().cloned().collect();
            assert_eq!(
                keys,
                [
                    "actions",
                    "error",
                    "id",
                    "permissions",
                    "provider",
                    "sharing"
                ]
            );
            let provider: Vec<_> = item["provider"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            assert_eq!(
                provider,
                ["detail", "installed", "kind", "running", "version"]
            );
            assert!(item["sharing"]["enabled"].is_boolean());
            assert!(
                item["actions"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("refresh"))
            );
        }
        let browser = &value["items"][3];
        assert_eq!(browser["provider"]["installed"], false);
        assert_eq!(browser["actions"], json!(["refresh"]));
        // Actions that do not apply now are refused before anything runs.
        let refused = service
            .act(
                &host,
                CapabilityId::Browser,
                CapabilityAction::Start,
                Value::Null,
            )
            .await
            .unwrap_err();
        assert_eq!(refused.status, 409);
        let item = service
            .act(
                &host,
                CapabilityId::Terminal,
                CapabilityAction::Refresh,
                Value::Null,
            )
            .await
            .unwrap();
        assert_eq!(item["id"], "terminal");
        // An externally configured browser can be unshared from here.
        host.peers(peers::Command::BrowserSharing {
            browser: Some(peers::browser::Provider {
                address: "127.0.0.1:9222".parse().unwrap(),
            }),
        })
        .await
        .unwrap();
        let item = service
            .act(
                &host,
                CapabilityId::Browser,
                CapabilityAction::Disable,
                Value::Null,
            )
            .await
            .unwrap();
        assert_eq!(item["sharing"], json!({"enabled":false,"address":null}));
        assert!(Config::read(&path).unwrap().browser.is_none());
    }

    #[tokio::test]
    async fn install_plans_run_once_and_expire() {
        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        let mut setup = Setup::new(&peers);
        let plan = setup.review().unwrap();
        assert_eq!(plan.url.as_deref(), Some(cua::INSTALL_URL));
        assert_eq!(
            plan.commands,
            [format!(
                "/bin/bash -c \"$(curl -fsSL {})\"",
                cua::INSTALL_URL
            )]
        );
        assert!(plan.expires_at > now() && plan.expires_at <= now() + 600);
        // A wrong id consumes the plan; nothing starts.
        assert!(setup.begin("wrong", |_| unreachable!()).is_err());
        assert!(setup.begin(&plan.plan_id, |_| unreachable!()).is_err());
        let plan = setup.review().unwrap();
        let ran = Arc::new(std::sync::Mutex::new(vec![]));
        let seen = ran.clone();
        setup
            .begin(&plan.plan_id, move |command| {
                seen.lock().unwrap().push(command);
                tokio::spawn(async { anyhow::bail!("fixture failure") })
            })
            .unwrap();
        assert_eq!(*ran.lock().unwrap(), plan.commands);
        assert_eq!(
            std::fs::read_to_string(peers.with_file_name("cua-setup.json")).unwrap(),
            r#"{"phase":"running","error":null}"#
        );
        assert!(setup.begin(&plan.plan_id, |_| unreachable!()).is_err());
        while setup.running() {
            tokio::time::sleep(Duration::from_millis(10)).await;
            setup.reconcile().await;
        }
        assert_eq!(setup.record.phase, Phase::Failed);
        // A restart during installation is reported, never replayed.
        std::fs::write(
            peers.with_file_name("cua-setup.json"),
            r#"{"phase":"running","error":null}"#,
        )
        .unwrap();
        let restarted = Setup::new(&peers);
        assert_eq!(restarted.record.phase, Phase::Interrupted);
        assert!(!restarted.running() && restarted.plan.is_none());
        // An expired plan is refused.
        let mut setup = Setup::new(&peers);
        let plan = setup.review().unwrap();
        setup.plan.as_mut().unwrap().created = Instant::now() - PLAN_TTL;
        assert!(setup.begin(&plan.plan_id, |_| unreachable!()).is_err());
    }
}
