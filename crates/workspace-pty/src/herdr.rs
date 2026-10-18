//! External herdr adapter: public JSON CLI for management, PTY client for attachment.
use crate::Session;
use anyhow::{Context, Result, ensure};
use portable_pty::CommandBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{process::Command, sync::Mutex};

#[derive(Clone)]
pub struct Engine {
    root: PathBuf,
    state: PathBuf,
    gate: Arc<Mutex<()>>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub name: String,
    pub cwd: String,
    pub created_at: u64,
    pub last_attached_at: Option<u64>,
    workspace: String,
    terminal: String,
}
pub use crate::persistent::Manage;
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
impl Engine {
    pub fn new(root: PathBuf, state: PathBuf) -> Self {
        Self {
            root,
            state,
            gate: Arc::default(),
        }
    }
    fn binary(&self) -> PathBuf {
        if let Some(path) = std::env::var_os("CRANIBOX_HERDR_BIN") {
            let path = PathBuf::from(path);
            return if path.is_relative() && path.components().count() > 1 {
                std::env::current_dir().unwrap_or_default().join(path)
            } else {
                path
            };
        }
        let name = if cfg!(windows) { "herdr.exe" } else { "herdr" };
        let local = self.root.join("bin").join(name);
        if local.is_file() { local } else { name.into() }
    }
    fn env(&self) -> [(String, String); 4] {
        // Private named session keeps recovery data separate from personal herdr.
        let hash = self
            .state
            .to_string_lossy()
            .bytes()
            .fold(0xcbf29ce484222325u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
            });
        [
            ("HERDR_SESSION".into(), format!("cranibox-{hash:016x}")),
            (
                "HERDR_CONFIG_PATH".into(),
                self.state
                    .join("config.toml")
                    .to_string_lossy()
                    .into_owned(),
            ),
            (
                "HERDR_SOCKET_PATH".into(),
                self.state.join("h.sock").to_string_lossy().into_owned(),
            ),
            ("HOME".into(), self.root.to_string_lossy().into_owned()),
        ]
    }
    fn command(&self) -> Command {
        let mut command = Command::new(self.binary());
        command.envs(self.env()).env_remove("HERDR");
        command.stdin(Stdio::null()).kill_on_drop(true);
        command
    }
    async fn cli(&self, args: &[&str]) -> Result<Value> {
        let output =
            tokio::time::timeout(Duration::from_secs(5), self.command().args(args).output())
                .await
                .context("herdr command timed out")?
                .context("herdr is unavailable; install herdr 0.9.3 or set CRANIBOX_HERDR_BIN")?;
        ensure!(
            output.stdout.len() <= 2 * 1024 * 1024,
            "herdr response too large"
        );
        let decoded: Option<Value> = serde_json::from_slice(&output.stdout).ok();
        if let Some(error) = decoded.as_ref().and_then(|v| v.get("error")) {
            anyhow::bail!(
                "{}: {}",
                error["code"].as_str().unwrap_or("herdr_error"),
                error["message"].as_str().unwrap_or("herdr request failed")
            );
        }
        ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(1000)
                .collect::<String>()
        );
        let value: Value =
            serde_json::from_slice(&output.stdout).context("unsupported herdr JSON response")?;
        ensure!(
            value.get("error").is_none(),
            "{}",
            value["error"]["message"]
                .as_str()
                .unwrap_or("herdr request failed")
        );
        value
            .get("result")
            .cloned()
            .context("herdr response has no result")
    }
    async fn version(&self) -> Result<String> {
        let out = tokio::time::timeout(
            Duration::from_secs(3),
            self.command().arg("--version").output(),
        )
        .await??;
        ensure!(out.status.success(), "herdr --version failed");
        Ok(String::from_utf8_lossy(&out.stdout).trim().into())
    }
    fn prepare(&self) -> Result<()> {
        std::fs::create_dir_all(&self.state)?;
        let config = self.state.join("config.toml");
        if !config.exists() {
            std::fs::write(
                config,
                "[update]\nversion_check = false\nmanifest_check = false\n",
            )?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.state, std::fs::Permissions::from_mode(0o700))?;
            ensure!(
                self.state.join("h-client.sock").as_os_str().len() < 104,
                "herdr state path is too long for local sockets"
            );
        }
        Ok(())
    }
    async fn start(&self) -> Result<()> {
        if self.cli(&["workspace", "list"]).await.is_ok() {
            return Ok(());
        }
        self.version()
            .await
            .context("install herdr 0.9.3 or set CRANIBOX_HERDR_BIN")?;
        self.prepare()?;
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.state.join("engine.log"))?;
        let mut command = self.command();
        command
            .arg("server")
            .current_dir(&self.root)
            .stdout(log.try_clone()?)
            .stderr(log)
            .kill_on_drop(false);
        #[cfg(unix)]
        {
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        #[cfg(windows)]
        command.creation_flags(0x00000200 | 0x00000008);
        let mut child = command.spawn().context("start herdr engine")?;
        std::fs::write(
            self.state.join("engine.pid"),
            child.id().unwrap_or_default().to_string(),
        )?;
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
        for _ in 0..50 {
            if self.cli(&["workspace", "list"]).await.is_ok() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        anyhow::bail!("herdr engine did not become ready; check engine.log")
    }
    fn records(&self) -> Result<Vec<Record>> {
        match std::fs::read(self.state.join("sessions.json")) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(e.into()),
        }
    }
    fn save(&self, records: &[Record]) -> Result<()> {
        self.prepare()?;
        let path = self.state.join("sessions.json");
        let tmp = self.state.join("sessions.tmp");
        std::fs::write(&tmp, serde_json::to_vec(records)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(tmp, path)?;
        Ok(())
    }
    async fn snapshot(&self) -> Result<Value> {
        match self.cli(&["api", "snapshot"]).await {
            Ok(value) => Ok(value["snapshot"].clone()),
            Err(error) if error.to_string().starts_with("server_not_running:") => {
                Ok(json!({"panes":[],"workspaces":[]}))
            }
            Err(error) => Err(error),
        }
    }
    fn running(record: &Record, snapshot: &Option<Value>) -> bool {
        snapshot
            .as_ref()
            .and_then(|s| s["panes"].as_array())
            .is_some_and(|panes| {
                panes.iter().any(|p| {
                    p["workspace_id"].as_str() == Some(&record.workspace)
                        && p["terminal_id"].as_str() == Some(&record.terminal)
                })
            })
    }
    fn view(record: &Record, snapshot: &Option<Value>) -> Value {
        json!({"id":record.id,"name":record.name,"cwd":record.cwd,"created_at":record.created_at,
            "last_attached_at":record.last_attached_at,"status":if snapshot.is_none() { "unknown" } else if Self::running(record, snapshot) { "running" } else { "ended" }})
    }
    pub async fn manage(&self, request: Manage) -> Result<Value> {
        let _guard = self.gate.lock().await;
        let mut records = self.records()?;
        if matches!(request, Manage::List) {
            let version = self.version().await.ok();
            let snapshot = if version.is_some() {
                self.snapshot().await.ok()
            } else {
                None
            };
            return Ok(
                json!({"engine":"herdr","installed":version.is_some(),"version":version,
                "sessions":records.iter().map(|r| Self::view(r, &snapshot)).collect::<Vec<_>>() }),
            );
        }
        match request {
            Manage::Create { name, cwd } => {
                validate_name(&name)?;
                ensure!(
                    records.len() < 64,
                    "session limit reached; remove ended sessions"
                );
                let cwd = cwd.map(PathBuf::from).unwrap_or_else(|| {
                    if self.root.join("Workspace").is_dir() {
                        self.root.join("Workspace")
                    } else {
                        self.root.clone()
                    }
                });
                ensure!(cwd.is_dir(), "working directory does not exist");
                let cwd = std::fs::canonicalize(cwd)?.to_string_lossy().into_owned();
                self.start().await?;
                let result = self
                    .cli(&["workspace", "create", "--label", &name, "--cwd", &cwd])
                    .await?;
                let workspace = result["workspace"]["workspace_id"]
                    .as_str()
                    .context("herdr missing workspace ID")?
                    .to_owned();
                let terminal = result["root_pane"]["terminal_id"]
                    .as_str()
                    .context("herdr missing terminal ID")?
                    .to_owned();
                let record = Record {
                    id: uuid::Uuid::new_v4().to_string(),
                    name,
                    cwd,
                    created_at: now(),
                    last_attached_at: None,
                    workspace,
                    terminal,
                };
                records.push(record.clone());
                if let Err(error) = self.save(&records) {
                    let _ = self.cli(&["workspace", "close", &record.workspace]).await;
                    return Err(error);
                }
                Ok(json!({"session":Self::view(&record, &self.snapshot().await.ok())}))
            }
            Manage::Rename { id, name } => {
                validate_name(&name)?;
                let record = records
                    .iter_mut()
                    .find(|r| r.id == id)
                    .context("unknown session")?;
                if Self::running(record, &Some(self.snapshot().await?)) {
                    self.cli(&["workspace", "rename", &record.workspace, &name])
                        .await?;
                }
                record.name = name;
                self.save(&records)?;
                Ok(json!({"ok":true}))
            }
            Manage::End { id } | Manage::Remove { id } => {
                let record = records
                    .iter()
                    .find(|r| r.id == id)
                    .context("unknown session")?;
                if Self::running(record, &Some(self.snapshot().await?)) {
                    self.cli(&["workspace", "close", &record.workspace]).await?;
                }
                records.retain(|r| r.id != id);
                self.save(&records)?;
                if self
                    .snapshot()
                    .await
                    .is_ok_and(|s| s["workspaces"].as_array().is_some_and(|w| w.is_empty()))
                {
                    let _ = self.cli(&["server", "stop"]).await;
                }
                Ok(json!({"ok":true}))
            }
            Manage::List => unreachable!(),
        }
    }
    pub async fn attach(&self, id: &str, cols: u16, rows: u16, takeover: bool) -> Result<Session> {
        ensure!(
            !cfg!(windows),
            "herdr 0.9.3 direct terminal attachment is not supported on Windows"
        );
        crate::validate_size(cols, rows)?;
        let _guard = self.gate.lock().await;
        let mut records = self.records()?;
        let record = records
            .iter_mut()
            .find(|r| r.id == id)
            .context("unknown session")?;
        ensure!(
            Self::running(record, &Some(self.snapshot().await?)),
            "session ended; create a new session"
        );
        let mut command = CommandBuilder::new(self.binary());
        command.args(["terminal", "attach", &record.terminal]);
        if takeover {
            command.arg("--takeover");
        }
        for (key, value) in self.env() {
            command.env(key, value);
        }
        command.env_remove("HERDR");
        command.env("TERM", "xterm-256color");
        command.cwd(&self.root);
        let mut session =
            tokio::task::spawn_blocking(move || Session::command(cols, rows, command)).await??;
        session.ready().await?;
        record.last_attached_at = Some(now());
        self.save(&records)?;
        Ok(session)
    }
}
fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.trim().is_empty()
            && name.chars().count() <= 80
            && !name.chars().any(char::is_control),
        "session name must be 1..80 characters without control characters"
    );
    Ok(())
}
