use anyhow::{Context as _, Result, bail};
use serde_json::Value;
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command};

#[derive(Clone)]
pub struct Context {
    pub root: PathBuf,
    pub workspace: PathBuf,
    pub source: PathBuf,
    pub http: reqwest::Client,
}

impl Context {
    pub fn new() -> Result<Self> {
        let root = env::var_os("AGENT_WORKSPACE_CONFIG_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/config"));
        let workspace = env::var_os("AGENT_WORKSPACE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("Workspace"));
        let source = env::var_os("AGENT_WORKSPACE_SOURCE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("agent-workspace-manager/source"));
        Ok(Self {
            root,
            workspace,
            source,
            // Local control-plane probes must not go through the terminal's HTTP proxy.
            http: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()?,
        })
    }
    pub fn state(&self) -> PathBuf {
        self.root.join(".local/state/agent-workspace")
    }
    pub fn units(&self) -> PathBuf {
        self.root.join(".config/systemd/user")
    }
    pub fn logs(&self) -> PathBuf {
        self.root.join(".local/log/user-systemd")
    }
    pub fn implementation_script(&self, name: &str) -> Result<PathBuf> {
        if let Some(path) = env::var_os("AGENT_WORKSPACE_CTL_SOURCE_DIR") {
            let script = PathBuf::from(path).join("scripts").join(name);
            if script.is_file() {
                return Ok(script);
            }
            bail!("Workspace script is unavailable: {}", script.display());
        }
        let cached = self.source.join("scripts").join(name);
        if cached.is_file() {
            return Ok(cached);
        }
        let installed = PathBuf::from("/usr/local/bin").join(name);
        if installed.is_file() {
            return Ok(installed);
        }
        let development = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts")
            .join(name);
        if development.is_file() {
            return Ok(development);
        }
        bail!("Workspace script {name} is unavailable; set AGENT_WORKSPACE_CTL_SOURCE_DIR")
    }
    pub fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        if env::var_os("HOME").is_none() {
            command.env("HOME", &self.root);
        }
        command
    }
    pub async fn probe(&self, args: &[&str], timeout: f64) -> Probe {
        let Some(program) = args.first() else {
            return Probe::failed("empty command");
        };
        let mut command = self.command(program);
        command.args(&args[1..]);
        capture(command, Duration::from_secs_f64(timeout)).await
    }
    pub async fn execute(&self, args: &[&str]) -> Result<i32> {
        let Some(program) = args.first() else {
            bail!("empty command");
        };
        let mut command = self.command(program);
        command.args(&args[1..]);
        Ok(command
            .status()
            .await
            .with_context(|| format!("cannot execute {program}"))?
            .code()
            .unwrap_or(1))
    }
    pub async fn sudo(&self) -> bool {
        static AVAILABLE: tokio::sync::OnceCell<bool> = tokio::sync::OnceCell::const_new();
        *AVAILABLE
            .get_or_init(|| async { self.probe(&["sudo", "-n", "true"], 2.).await.code == 0 })
            .await
    }
    pub async fn get_json(&self, url: &str, timeout: f64) -> Result<Value> {
        Ok(self
            .http
            .get(url)
            .timeout(Duration::from_secs_f64(timeout))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
}

#[derive(Debug)]
pub struct Probe {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}
impl Probe {
    pub fn failed(message: impl ToString) -> Self {
        Self {
            code: 127,
            stdout: String::new(),
            stderr: message.to_string(),
        }
    }
    pub fn output(&self) -> &str {
        if self.stdout.trim().is_empty() {
            self.stderr.trim()
        } else {
            self.stdout.trim()
        }
    }
    pub fn checked(self, program: &str) -> Result<String> {
        if self.code != 0 {
            bail!("{program} failed (exit {})", self.code);
        }
        Ok(self.stdout.trim().to_owned())
    }
}

// Drain both pipes concurrently, but retain bounded output even from a broken CLI.
async fn drain(mut reader: impl tokio::io::AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
    let mut retained = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let keep = count.min((2 * 1024 * 1024usize).saturating_sub(retained.len()));
        retained.extend_from_slice(&buffer[..keep]);
    }
    Ok(retained)
}

pub async fn capture(mut command: Command, timeout: Duration) -> Probe {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return Probe::failed(error),
    };
    let pid = child.id();
    let stdout = tokio::spawn(drain(child.stdout.take().expect("piped stdout")));
    let stderr = tokio::spawn(drain(child.stderr.take().expect("piped stderr")));
    let status = tokio::time::timeout(timeout, child.wait()).await;
    let code = match status {
        Ok(Ok(status)) => status.code().unwrap_or(1),
        _ => {
            #[cfg(unix)]
            if let Some(pid) = pid {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            #[cfg(not(unix))]
            let _ = pid;
            let _ = child.kill().await;
            let _ = child.wait().await;
            124
        }
    };
    async fn collect(mut task: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>) -> String {
        match tokio::time::timeout(Duration::from_secs(1), &mut task).await {
            Ok(Ok(Ok(bytes))) => String::from_utf8_lossy(&bytes).into_owned(),
            _ => {
                task.abort();
                String::new()
            }
        }
    }
    let (stdout, mut stderr) = tokio::join!(collect(stdout), collect(stderr));
    if code == 124 {
        stderr = "command timed out".into();
    }
    Probe {
        code,
        stdout,
        stderr,
    }
}

pub fn command_path(name: &str) -> String {
    let candidates = if Path::new(name).components().count() > 1 {
        vec![PathBuf::from(name)]
    } else {
        env::split_paths(&env::var_os("PATH").unwrap_or_default())
            .map(|dir| dir.join(name))
            .collect()
    };
    for path in candidates {
        if executable(&path) {
            return path.to_string_lossy().into_owned();
        }
        #[cfg(windows)]
        for suffix in env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
        {
            let candidate = path.with_extension(suffix.trim_start_matches('.'));
            if executable(&candidate) {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    String::new()
}
pub fn executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}
pub fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path).unwrap_or_default()
}
pub fn json_file(path: impl AsRef<Path>) -> Value {
    serde_json::from_str(&read(path)).unwrap_or(Value::Null)
}
pub fn directories(path: impl AsRef<Path>) -> Vec<PathBuf> {
    let mut paths = fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}
pub fn mode(path: &Path, default: u32) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(default)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        default
    }
}
pub fn atomic_write(path: &Path, content: impl AsRef<[u8]>, permissions: u32) -> Result<()> {
    let parent = path.parent().context("missing parent directory")?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(content.as_ref())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(permissions))?;
    }
    #[cfg(not(unix))]
    let _ = permissions;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}
pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    atomic_write(
        path,
        format!("{}\n", serde_json::to_string_pretty(value)?),
        mode(path, 0o600),
    )
}
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.@-".contains(&c))
        && name != "."
        && name != ".."
}
pub fn validate_name(name: &str) -> Result<()> {
    if !valid_name(name) {
        bail!("Invalid or missing name: {name}");
    }
    Ok(())
}
pub fn env_file(path: &Path) -> std::collections::BTreeMap<String, String> {
    read(path)
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let values = shell_words::split(value).ok()?;
            Some((
                key.trim().to_owned(),
                values.first().cloned().unwrap_or_default(),
            ))
        })
        .collect()
}
pub fn print_json(value: &Value) {
    println!("{value}");
}
pub fn detach(command: &mut Command, log: &Path) -> Result<tokio::process::Child> {
    fs::create_dir_all(log.parent().context("missing log parent")?)?;
    let output = fs::OpenOptions::new().create(true).append(true).open(log)?;
    command
        .stdin(Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(output);
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(command.spawn()?)
}
