//! Local mutual supervision. If both roles exit, the OS entry must restart base.
//! Callers authorize and stage updates; confirmation means the role's mesh/handshake is ready.

use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::time::{Instant, sleep};
use tokio_util::sync::CancellationToken;

use super::{
    layout::{LaunchDecision, Layout, PointerState},
    npm::executable_relative_path,
    version::Version,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Base,
    Upper,
}

impl Role {
    pub fn other(self) -> Self {
        match self {
            Self::Base => Self::Upper,
            Self::Upper => Self::Base,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Upper => "upper",
        }
    }
}

pub trait Spawner: Send + Sync + 'static {
    fn spawn(&self, role: Role, version: &Version, exe: &Path) -> Result<()>;
    fn kill(&self, pid: u32) -> Result<()>;
}

#[derive(Default)]
pub struct ProcessSpawner {
    args: Vec<OsString>,
}

impl ProcessSpawner {
    pub fn new(args: Vec<OsString>) -> Self {
        Self { args }
    }
}

impl Spawner for ProcessSpawner {
    fn spawn(&self, role: Role, version: &Version, exe: &Path) -> Result<()> {
        // The packaged executable is <runtime>/<role>/versions/<version>/bin/cranibox.
        let runtime = exe
            .ancestors()
            .nth(5)
            .context("executable has no runtime root")?;
        let mut command = Command::new(exe);
        command
            .args(["--role", role.as_str()])
            .args(&self.args)
            .env("CRANIBOX_RUNTIME_ROOT", runtime)
            .env("CRANIBOX_RUNTIME_VERSION", version.to_string())
            .env("CRANIBOX_RUNTIME_ROLE", role.as_str())
            .env("CRANIBOX_RUNTIME_MANAGED", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: setsid is async-signal-safe and touches no Rust state after fork.
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        #[cfg(windows)]
        let child = {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::{
                CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
            };
            let flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
            command.creation_flags(flags | CREATE_BREAKAWAY_FROM_JOB);
            command.spawn().or_else(|_| {
                command.creation_flags(flags);
                command.spawn()
            })
        };
        #[cfg(not(windows))]
        let child = command.spawn();
        let mut child = child.context("spawn detached peer")?;
        // No ProcessTree/kill-on-close job: the peer must survive this process.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }

    fn kill(&self, pid: u32) -> Result<()> {
        ensure!(pid > 1, "invalid peer pid");
        #[cfg(unix)]
        {
            ensure!(pid <= i32::MAX as u32, "invalid peer pid");
            // SAFETY: a positive PID addresses just the peer, never our process group.
            if unsafe { libc::kill(pid as i32, libc::SIGKILL) } == -1 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(error).context("kill peer");
                }
            }
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::{
                Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0},
                System::Threading::{
                    OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess,
                    WaitForSingleObject,
                },
            };
            // SAFETY: the handle is checked, used only for termination, and closed once.
            unsafe {
                let handle = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid);
                if handle.is_null() {
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
                        return Ok(());
                    }
                    return Err(error).context("open peer for termination");
                }
                let terminated = TerminateProcess(handle, 1);
                let error = std::io::Error::last_os_error();
                // TerminateProcess can refuse an already exited process still held by its reaper.
                let exited = WaitForSingleObject(handle, if terminated != 0 { 5000 } else { 0 })
                    == WAIT_OBJECT_0;
                CloseHandle(handle);
                ensure!(terminated != 0 || exited, "terminate peer: {error}");
                ensure!(exited, "peer did not exit after termination");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct DuoConfig {
    pub runtime: PathBuf,
    pub me: Role,
    pub heartbeat_every: Duration,
    pub stale_after: Duration,
    pub respawn_grace: Duration,
    pub backoff_base: Duration,
    pub max_attempts: u32,
    pub confirm_window: Duration,
}

impl Default for DuoConfig {
    fn default() -> Self {
        Self {
            runtime: PathBuf::new(),
            me: Role::Base,
            heartbeat_every: Duration::from_millis(300),
            stale_after: Duration::from_millis(1500),
            respawn_grace: Duration::from_secs(2),
            backoff_base: Duration::from_millis(500),
            max_attempts: 3,
            confirm_window: Duration::from_secs(8),
        }
    }
}

impl DuoConfig {
    pub fn new(runtime: impl Into<PathBuf>, me: Role) -> Self {
        Self {
            runtime: runtime.into(),
            me,
            ..Self::default()
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum UpdateOutcome {
    Updated {
        #[serde(with = "version_string")]
        version: Version,
    },
    RolledBack {
        #[serde(with = "version_string")]
        restored: Version,
        reason: String,
    },
    Refused {
        reason: String,
    },
}

mod version_string {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(version: &Version, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(version)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Version, D::Error> {
        Version::parse(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Serialize, Deserialize)]
struct Heartbeat {
    pid: u32,
    version: String,
    t_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct Request {
    actor: Role,
    version: String,
    t_ms: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Reply {
    Outcome(UpdateOutcome),
    Error { error: String },
}

#[derive(Default, Serialize, Deserialize)]
struct SpawnState {
    failures: u32,
    next_ms: u64,
    grace_until_ms: u64,
}

struct Lock(File);

impl Lock {
    fn try_acquire(path: &Path) -> Result<Option<Self>> {
        fs::create_dir_all(path.parent().context("lock has no parent")?)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => Ok(Some(Self(file))),
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                Ok(None)
            }
            Err(error) => Err(error).context("lock duo operation"),
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

pub struct Duo {
    config: DuoConfig,
    spawner: Arc<dyn Spawner>,
    version: OnceLock<Version>,
}

impl Duo {
    pub fn new(config: DuoConfig, spawner: Arc<dyn Spawner>) -> Self {
        let duo = Self {
            config,
            spawner,
            version: OnceLock::new(),
        };
        // Capture the running version before an actor can change this role's pointer.
        let _ = duo.running_version();
        duo
    }

    pub fn layout(&self, role: Role) -> Layout {
        Layout::new(self.config.runtime.join(role.as_str()))
    }

    /// Heartbeat and control are separate futures so an update cannot starve heartbeats.
    pub async fn run(&self, stop: CancellationToken) -> Result<()> {
        ensure!(
            !self.config.runtime.as_os_str().is_empty(),
            "runtime root is required"
        );
        ensure!(
            !self.config.heartbeat_every.is_zero(),
            "heartbeat interval must be positive"
        );
        let _instance = Lock::try_acquire(
            &self
                .shared()
                .join(format!("run-{}.lock", self.config.me.as_str())),
        )?
        .context("role is already running")?;
        let version = self.running_version()?.to_string();
        tokio::select! {
            biased;
            _ = stop.cancelled() => Ok(()),
            result = async {
                tokio::try_join!(self.heartbeats(&version, std::process::id()), self.control())?;
                Ok(())
            } => result,
        }
    }

    /// Call only after this role's network/mesh or peer handshake is ready.
    pub fn confirm_self(&self) -> Result<()> {
        self.layout(self.config.me)
            .confirm(self.running_version()?)?;
        Ok(())
    }

    /// The manager must authorize the caller and install the peer's version first.
    /// Timing out the caller does not cancel an update already accepted by the actor.
    pub async fn request_peer_update(
        &self,
        version: &Version,
        timeout: Duration,
    ) -> Result<UpdateOutcome> {
        let requests = self.shared().join("req");
        fs::create_dir_all(&requests)?;
        let request = requests.join(format!("{}.json", uuid::Uuid::new_v4()));
        atomic_write(
            &request,
            &Request {
                actor: self.config.me,
                version: version.to_string(),
                t_ms: now_ms()?,
            },
        )?;
        let result = request.with_extension("res");
        tokio::time::timeout(timeout, async {
            loop {
                if let Some(reply) = read_json(&result)? {
                    return match reply {
                        Reply::Outcome(outcome) => Ok(outcome),
                        Reply::Error { error } => Err(anyhow::anyhow!(error)),
                    };
                }
                sleep(self.config.heartbeat_every.max(Duration::from_millis(1))).await;
            }
        })
        .await
        .context("timed out waiting for peer update result")?
    }

    fn shared(&self) -> PathBuf {
        self.config.runtime.join("duo")
    }

    fn running_version(&self) -> Result<&Version> {
        if let Some(version) = self.version.get() {
            return Ok(version);
        }
        let value = match std::env::var("CRANIBOX_RUNTIME_VERSION") {
            Ok(value) => value,
            Err(_) => {
                self.layout(self.config.me)
                    .pointer()?
                    .context("no runtime has been activated")?
                    .current
            }
        };
        let version = Version::parse(&value)?;
        let _ = self.version.set(version);
        Ok(self.version.get().expect("running version set"))
    }

    fn heartbeat(&self, role: Role) -> Result<Option<Heartbeat>> {
        match read_json(&self.shared().join(format!("hb-{}.json", role.as_str()))) {
            // An incomplete/corrupt heartbeat provides no evidence of liveness.
            Err(error) if error.downcast_ref::<serde_json::Error>().is_some() => Ok(None),
            result => result,
        }
    }

    fn fresh(&self, heartbeat: &Heartbeat) -> bool {
        heartbeat.pid > 1
            && now_ms()
                .ok()
                .and_then(|now| now.checked_sub(heartbeat.t_ms))
                .is_some_and(|age| Duration::from_millis(age) < self.config.stale_after)
    }

    async fn heartbeats(&self, version: &str, pid: u32) -> Result<()> {
        loop {
            atomic_write(
                &self
                    .shared()
                    .join(format!("hb-{}.json", self.config.me.as_str())),
                &Heartbeat {
                    pid,
                    version: version.to_owned(),
                    t_ms: now_ms()?,
                },
            )?;
            sleep(self.config.heartbeat_every).await;
        }
    }

    async fn control(&self) -> Result<()> {
        fs::create_dir_all(self.shared().join("req"))?;
        loop {
            self.requests().await?;
            if let Err(error) = self.supervise() {
                eprintln!(
                    "duo: supervise {}: {error:#}",
                    self.config.me.other().as_str()
                );
            }
            sleep(self.config.heartbeat_every).await;
        }
    }

    async fn requests(&self) -> Result<()> {
        for entry in fs::read_dir(self.shared().join("req"))? {
            let path = entry?.path();
            if path.extension().is_none_or(|ext| ext != "json")
                || path.with_extension("res").exists()
            {
                continue;
            }
            let Ok(Some(request)) = read_json::<Request>(&path) else {
                continue;
            };
            if request.actor != self.config.me
                || now_ms()?
                    .checked_sub(request.t_ms)
                    .is_none_or(|age| age >= 120_000)
            {
                continue;
            }
            let outcome = match Version::parse(&request.version) {
                Ok(version) => self.update_peer(&version).await,
                Err(error) => Ok(UpdateOutcome::Refused {
                    reason: error.to_string(),
                }),
            };
            // A broken target must not stop its guard, including when recovery itself fails.
            let reply = match outcome {
                Ok(outcome) => Reply::Outcome(outcome),
                Err(error) => Reply::Error {
                    error: format!("{error:#}"),
                },
            };
            atomic_write(&path.with_extension("res"), &reply)?;
        }
        Ok(())
    }

    fn spawn_state_path(&self) -> PathBuf {
        self.shared()
            .join(format!("spawn-{}.json", self.config.me.other().as_str()))
    }

    fn supervise(&self) -> Result<()> {
        // Also serializes opposite-direction updates. Never block the heartbeat task on a lock.
        let Some(_operation) = Lock::try_acquire(&self.shared().join("operation.lock"))? else {
            return Ok(());
        };
        let mut state: SpawnState = read_json(&self.spawn_state_path())?.unwrap_or_default();
        let now = now_ms()?;
        if let Some(heartbeat) = self.heartbeat(self.config.me.other())?
            && self.fresh(&heartbeat)
        {
            if heartbeat.t_ms >= state.grace_until_ms
                && state.failures > 0
                && self
                    .layout(self.config.me.other())
                    .pointer()?
                    .is_some_and(|p| {
                        p.state == PointerState::Confirmed && p.current == heartbeat.version
                    })
            {
                state = SpawnState::default();
                atomic_write(&self.spawn_state_path(), &state)?;
            }
            return Ok(());
        }
        if now < state.grace_until_ms || now < state.next_ms {
            return Ok(());
        }
        self.launch_peer(&mut state)?;
        Ok(())
    }

    // The operation lock covers the persisted lease, Layout's own locked decision, and spawn.
    fn launch_peer(&self, state: &mut SpawnState) -> Result<Version> {
        let now = now_ms()?;
        let backoff = self
            .config
            .backoff_base
            .saturating_mul(1u32 << state.failures.min(31))
            .min(Duration::from_secs(4));
        state.failures = state.failures.saturating_add(1);
        state.next_ms = now.saturating_add(millis(backoff));
        state.grace_until_ms = now.saturating_add(millis(self.config.respawn_grace));
        // Reserve even failed spawns: another observer must respect the same backoff.
        atomic_write(&self.spawn_state_path(), state)?;
        let layout = self.layout(self.config.me.other());
        let version =
            match layout.begin_launch(self.config.max_attempts, self.config.confirm_window)? {
                LaunchDecision::Run { version } => version,
                LaunchDecision::RolledBack { to, .. } => to,
                LaunchDecision::Broken { reason } => bail!("{reason}"),
            };
        ensure!(
            !layout
                .pointer()?
                .context("no peer pointer")?
                .failed
                .contains(&version.to_string()),
            "peer version previously failed"
        );
        let spawned = self.spawner.spawn(
            self.config.me.other(),
            &version,
            &layout
                .version_dir(&version)
                .join(executable_relative_path()),
        );
        // Disk/process creation latency must not shorten the grace or the next retry gap.
        let finished = now_ms()?;
        state.next_ms = finished.saturating_add(millis(backoff));
        state.grace_until_ms = finished.saturating_add(millis(self.config.respawn_grace));
        atomic_write(&self.spawn_state_path(), state)?;
        spawned?;
        Ok(version)
    }

    async fn update_peer(&self, version: &Version) -> Result<UpdateOutcome> {
        let refused = |reason: &str| {
            Ok(UpdateOutcome::Refused {
                reason: reason.to_owned(),
            })
        };
        let Some(_operation) = Lock::try_acquire(&self.shared().join("operation.lock"))? else {
            return refused("another duo operation is in progress");
        };
        let me = self.layout(self.config.me).pointer()?;
        if !me.is_some_and(|p| {
            p.state == PointerState::Confirmed
                && p.current
                    == self
                        .running_version()
                        .map(ToString::to_string)
                        .unwrap_or_default()
        }) || !self
            .heartbeat(self.config.me)?
            .is_some_and(|h| self.fresh(&h))
        {
            return refused("actor must be online and confirmed");
        }
        let peer = self.config.me.other();
        let layout = self.layout(peer);
        let Some(old_pointer) = layout.pointer()? else {
            return refused("peer has no pointer");
        };
        let Some(old_heartbeat) = self
            .heartbeat(peer)?
            .filter(|h| self.fresh(h) && h.version == old_pointer.current)
        else {
            return refused("peer must be online");
        };
        if old_pointer.state != PointerState::Confirmed {
            return refused("peer pointer must be confirmed");
        }
        let mut state: SpawnState = read_json(&self.spawn_state_path())?.unwrap_or_default();
        if let Err(error) = layout.activate(version) {
            return Ok(UpdateOutcome::Refused {
                reason: format!("{error:#}"),
            });
        }
        let attempt = self
            .spawner
            .kill(old_heartbeat.pid)
            .and_then(|()| self.launch_peer(&mut state));
        let reason = match attempt {
            Ok(launched) if launched == *version => {
                if self.wait_healthy(version, old_heartbeat.pid).await? {
                    return Ok(UpdateOutcome::Updated {
                        version: version.clone(),
                    });
                }
                "peer did not confirm with a fresh heartbeat and new pid before its deadline"
                    .to_owned()
            }
            Ok(restored) => {
                let reason = "peer launch budget required rollback".to_owned();
                // begin_launch already selected and started the fallback. Do not start it twice.
                ensure!(
                    self.wait_healthy(&restored, old_heartbeat.pid).await?,
                    "rollback to {restored} did not recover: {reason}"
                );
                return Ok(UpdateOutcome::RolledBack { restored, reason });
            }
            Err(error) => format!("peer failed to start: {error:#}"),
        };
        let pointer = layout.pointer()?.context("peer pointer disappeared")?;
        if pointer.current == version.to_string() {
            layout.rollback(&reason)?;
        } else {
            ensure!(
                pointer.current == old_pointer.current,
                "peer pointer changed during update"
            );
        }
        if let Some(heartbeat) = self.heartbeat(peer)? {
            self.spawner.kill(heartbeat.pid)?;
        }
        let restored = self.launch_peer(&mut state)?;
        ensure!(
            self.wait_healthy(&restored, old_heartbeat.pid).await?,
            "rollback to {restored} did not recover: {reason}"
        );
        Ok(UpdateOutcome::RolledBack { restored, reason })
    }

    async fn wait_healthy(&self, version: &Version, old_pid: u32) -> Result<bool> {
        let deadline = Instant::now() + self.config.confirm_window;
        let name = version.to_string();
        loop {
            if self
                .layout(self.config.me.other())
                .pointer()?
                .is_some_and(|p| p.state == PointerState::Confirmed && p.current == name)
                && self
                    .heartbeat(self.config.me.other())?
                    .is_some_and(|h| self.fresh(&h) && h.version == name && h.pid != old_pid)
            {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            sleep(self.config.heartbeat_every.max(Duration::from_millis(1))).await;
        }
    }
}

fn millis(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

fn now_ms() -> Result<u64> {
    Ok(millis(SystemTime::now().duration_since(UNIX_EPOCH)?))
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).with_context(|| format!("read {}", path.display()))?,
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

fn atomic_write(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("JSON file has no parent")?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(&mut temporary, value)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicU32, Ordering},
    };
    use tokio::{task::JoinHandle, time::timeout};

    const ROLES: [Role; 2] = [Role::Base, Role::Upper];

    #[derive(Clone, Copy, Default)]
    enum Behavior {
        #[default]
        Good,
        NeverConfirm,
        CrashImmediately,
        CrashAfterConfirm,
        SpawnError,
    }

    struct Process {
        role: Role,
        pid: u32,
        task: JoinHandle<()>,
    }

    struct Spawn {
        role: Role,
        pid: u32,
        version: String,
        at: Instant,
    }

    #[derive(Default)]
    struct FakeState {
        behaviors: Vec<(Role, String, Behavior)>,
        processes: Vec<Process>,
        spawns: Vec<Spawn>,
        kills: Vec<u32>,
        failures: Vec<String>,
    }

    #[derive(Clone)]
    struct FakeSpawner {
        config: DuoConfig,
        state: Arc<Mutex<FakeState>>,
        next_pid: Arc<AtomicU32>,
    }

    impl FakeSpawner {
        fn config(&self, role: Role) -> DuoConfig {
            DuoConfig {
                me: role,
                ..self.config.clone()
            }
        }

        fn duo(&self, role: Role) -> Arc<Duo> {
            Arc::new(Duo::new(self.config(role), Arc::new(self.clone())))
        }

        fn behavior(&self, role: Role, version: &str, behavior: Behavior) {
            self.state
                .lock()
                .unwrap()
                .behaviors
                .push((role, version.to_owned(), behavior));
        }

        fn start(&self, role: Role) {
            let duo = self.duo(role);
            let version =
                Version::parse(&duo.layout(role).pointer().unwrap().unwrap().current).unwrap();
            self.spawn(
                role,
                &version,
                &duo.layout(role)
                    .version_dir(&version)
                    .join(executable_relative_path()),
            )
            .unwrap();
        }

        fn pid(&self, role: Role) -> u32 {
            let state = self.state.lock().unwrap();
            state
                .processes
                .iter()
                .rev()
                .find(|p| p.role == role)
                .map(|p| p.pid)
                .or_else(|| {
                    state
                        .spawns
                        .iter()
                        .rev()
                        .find(|p| p.role == role)
                        .map(|p| p.pid)
                })
                .expect("role was spawned")
        }

        fn stop_all(&self) {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            for process in state.processes.drain(..) {
                process.task.abort();
            }
        }
    }

    impl Spawner for FakeSpawner {
        fn spawn(&self, role: Role, version: &Version, exe: &Path) -> Result<()> {
            assert_eq!(
                exe,
                self.duo(role)
                    .layout(role)
                    .version_dir(version)
                    .join(executable_relative_path())
            );
            let pid = self.next_pid.fetch_add(1, Ordering::Relaxed);
            let version = version.to_string();
            let mut state = self.state.lock().unwrap();
            state.spawns.push(Spawn {
                role,
                pid,
                version: version.clone(),
                at: Instant::now(),
            });
            let behavior = state
                .behaviors
                .iter()
                .rev()
                .find(|(r, v, _)| *r == role && *v == version)
                .map(|(_, _, b)| *b)
                .unwrap_or_default();
            if matches!(behavior, Behavior::CrashImmediately) {
                return Ok(());
            }
            if matches!(behavior, Behavior::SpawnError) {
                bail!("fake spawn failed");
            }
            let fake = self.clone();
            let duo = self.duo(role);
            let task = tokio::spawn(async move {
                let lifecycle = async {
                    sleep(Duration::from_millis(25)).await;
                    if !matches!(behavior, Behavior::NeverConfirm) {
                        duo.confirm_self()?;
                    }
                    if matches!(behavior, Behavior::CrashAfterConfirm) {
                        sleep(Duration::from_millis(25)).await;
                        return Ok(());
                    }
                    std::future::pending::<Result<()>>().await
                };
                let result = tokio::select! {
                    result = duo.heartbeats(&version, pid) => result,
                    result = duo.control() => result,
                    result = lifecycle => result,
                };
                if let Err(error) = result {
                    fake.state
                        .lock()
                        .unwrap()
                        .failures
                        .push(format!("{role:?}: {error:#}"));
                }
            });
            state.processes.push(Process { role, pid, task });
            Ok(())
        }

        fn kill(&self, pid: u32) -> Result<()> {
            let mut state = self.state.lock().unwrap();
            state.kills.push(pid);
            if let Some(index) = state.processes.iter().position(|p| p.pid == pid) {
                state.processes.remove(index).task.abort();
            }
            Ok(())
        }
    }

    struct Fixture {
        fake: FakeSpawner,
        _root: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let target = std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
            let scratch = target.join("update-duo-tests");
            fs::create_dir_all(&scratch).unwrap();
            let root = tempfile::tempdir_in(scratch).unwrap();
            let fake = FakeSpawner {
                config: DuoConfig {
                    heartbeat_every: Duration::from_millis(10),
                    // Leave room for parallel tests to fsync on the shared Cargo target disk.
                    stale_after: Duration::from_millis(500),
                    respawn_grace: Duration::from_millis(80),
                    backoff_base: Duration::from_millis(30),
                    confirm_window: Duration::from_secs(1),
                    ..DuoConfig::new(root.path(), Role::Base)
                },
                state: Arc::default(),
                next_pid: Arc::new(AtomicU32::new(100_000)),
            };
            let fixture = Self { fake, _root: root };
            for role in ROLES {
                let layout = fixture.fake.duo(role).layout(role);
                fixture.install(role, "1.0.0");
                let version = Version::parse("1.0.0").unwrap();
                layout.activate(&version).unwrap();
                layout.confirm(&version).unwrap();
            }
            fixture
        }

        fn install(&self, role: Role, version: &str) -> Version {
            let version = Version::parse(version).unwrap();
            let layout = self.fake.duo(role).layout(role);
            let stage = layout.begin_stage(&version).unwrap();
            let exe = stage.join(executable_relative_path());
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            fs::write(exe, b"fake executable").unwrap();
            layout.commit_stage(&version).unwrap();
            version
        }

        async fn start(&self) {
            // Starting base models the OS entry. Base alone must bring up upper.
            self.fake.start(Role::Base);
            self.healthy(Role::Base, "1.0.0").await;
            self.healthy(Role::Upper, "1.0.0").await;
        }

        async fn healthy(&self, role: Role, version: &str) {
            let duo = self.fake.duo(role);
            until(|| {
                duo.heartbeat(role)
                    .unwrap()
                    .is_some_and(|h| duo.fresh(&h) && h.version == version)
                    && duo
                        .layout(role)
                        .pointer()
                        .unwrap()
                        .is_some_and(|p| p.current == version && p.state == PointerState::Confirmed)
            })
            .await;
        }

        async fn update(&self, actor: Role, version: &Version) -> UpdateOutcome {
            self.fake
                .duo(actor)
                .request_peer_update(version, Duration::from_secs(3))
                .await
                .unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.fake.stop_all();
            if !std::thread::panicking() {
                let state = self.fake.state.lock().unwrap();
                assert!(
                    state.failures.is_empty(),
                    "fake process failed: {:?}",
                    state.failures
                );
            }
        }
    }

    async fn until(mut condition: impl FnMut() -> bool) {
        timeout(Duration::from_secs(2), async {
            while !condition() {
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("condition did not become true");
    }

    fn seed_heartbeat(duo: &Duo, role: Role, version: &str, pid: u32) {
        atomic_write(
            &duo.shared().join(format!("hb-{}.json", role.as_str())),
            &Heartbeat {
                pid,
                version: version.to_owned(),
                t_ms: now_ms().unwrap(),
            },
        )
        .unwrap();
    }

    #[test]
    fn roles_defaults_and_wire_format() {
        assert_eq!(Role::Base.other(), Role::Upper);
        assert_eq!(Role::Upper.other(), Role::Base);
        assert_eq!(Role::Base.as_str(), "base");
        assert_eq!(Role::Upper.as_str(), "upper");
        let config = DuoConfig::default();
        assert_eq!(config.heartbeat_every, Duration::from_millis(300));
        assert_eq!(config.stale_after, Duration::from_millis(1500));
        assert_eq!(config.respawn_grace, Duration::from_secs(2));
        assert_eq!(config.backoff_base, Duration::from_millis(500));
        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.confirm_window, Duration::from_secs(8));
        for outcome in [
            UpdateOutcome::Updated {
                version: Version::parse("2.0.0").unwrap(),
            },
            UpdateOutcome::RolledBack {
                restored: Version::parse("1.0.0").unwrap(),
                reason: "deadline".into(),
            },
            UpdateOutcome::Refused {
                reason: "offline".into(),
            },
        ] {
            assert_eq!(
                serde_json::from_slice::<UpdateOutcome>(&serde_json::to_vec(&outcome).unwrap())
                    .unwrap(),
                outcome
            );
        }
    }

    #[tokio::test]
    async fn killed_peer_is_respawned_in_both_directions() {
        let fixture = Fixture::new();
        fixture.start().await;
        for target in ROLES {
            let actor = target.other();
            let actor_pid = fixture.fake.pid(actor);
            let old_pid = fixture.fake.pid(target);
            fixture.fake.kill(old_pid).unwrap();
            let started = Instant::now();
            until(|| fixture.fake.pid(target) != old_pid).await;
            fixture.healthy(target, "1.0.0").await;
            assert!(started.elapsed() < Duration::from_secs(1));
            assert_eq!(fixture.fake.pid(actor), actor_pid);
        }
    }

    #[tokio::test]
    async fn good_updates_in_both_directions_keep_independent_pointers() {
        for actor in ROLES {
            let fixture = Fixture::new();
            fixture.start().await;
            let target = actor.other();
            let version = fixture.install(target, "2.0.0");
            let actor_pid = fixture.fake.pid(actor);
            assert_eq!(
                fixture.update(actor, &version).await,
                UpdateOutcome::Updated { version }
            );
            fixture.healthy(target, "2.0.0").await;
            fixture.healthy(actor, "1.0.0").await;
            assert_eq!(fixture.fake.pid(actor), actor_pid);
            assert_eq!(
                fixture
                    .fake
                    .duo(actor)
                    .layout(actor)
                    .pointer()
                    .unwrap()
                    .unwrap()
                    .current,
                "1.0.0"
            );
        }
    }

    #[tokio::test]
    async fn never_confirming_updates_roll_back_in_both_directions() {
        rollback_updates(Behavior::NeverConfirm).await;
    }

    #[tokio::test]
    async fn immediately_crashing_updates_roll_back_in_both_directions() {
        rollback_updates(Behavior::CrashImmediately).await;
    }

    async fn rollback_updates(behavior: Behavior) {
        for actor in ROLES {
            let fixture = Fixture::new();
            fixture.start().await;
            let target = actor.other();
            let version = fixture.install(target, "2.0.0");
            fixture.fake.behavior(target, "2.0.0", behavior);
            let actor_pid = fixture.fake.pid(actor);
            let outcome = fixture.update(actor, &version).await;
            assert!(
                matches!(outcome, UpdateOutcome::RolledBack { restored, reason } if restored.to_string() == "1.0.0" && !reason.is_empty())
            );
            fixture.healthy(target, "1.0.0").await;
            assert_eq!(fixture.fake.pid(actor), actor_pid);
            let pointer = fixture
                .fake
                .duo(actor)
                .layout(target)
                .pointer()
                .unwrap()
                .unwrap();
            assert_eq!(pointer.failed, ["2.0.0"]);
            assert_eq!(pointer.previous, None);
            assert!(matches!(
                fixture.update(actor, &version).await,
                UpdateOutcome::Refused { .. }
            ));
            let old_pid = fixture.fake.pid(target);
            fixture.fake.kill(old_pid).unwrap();
            until(|| fixture.fake.pid(target) != old_pid).await;
            fixture.healthy(target, "1.0.0").await;
            assert_eq!(
                fixture
                    .fake
                    .state
                    .lock()
                    .unwrap()
                    .spawns
                    .iter()
                    .filter(|s| s.role == target && s.version == "2.0.0")
                    .count(),
                1
            );
        }
    }

    #[tokio::test]
    async fn pending_crash_loop_counts_attempts_and_persists_growing_backoff() {
        for actor in ROLES {
            let mut fixture = Fixture::new();
            fixture.fake.config.respawn_grace = Duration::from_millis(5);
            fixture.fake.config.backoff_base = Duration::from_millis(40);
            fixture.fake.config.confirm_window = Duration::from_secs(5);
            fixture.start().await;
            let target = actor.other();
            let version = fixture.install(target, "2.0.0");
            fixture
                .fake
                .behavior(target, "2.0.0", Behavior::CrashImmediately);
            let duo = fixture.fake.duo(actor);
            duo.layout(target).activate(&version).unwrap();
            fixture.fake.kill(fixture.fake.pid(target)).unwrap();
            until(|| {
                duo.layout(target)
                    .pointer()
                    .unwrap()
                    .unwrap()
                    .failed
                    .contains(&"2.0.0".to_owned())
            })
            .await;
            fixture.healthy(target, "1.0.0").await;
            let state = fixture.fake.state.lock().unwrap();
            let starts: Vec<_> = state
                .spawns
                .iter()
                .filter(|s| s.role == target && s.version == "2.0.0")
                .map(|s| s.at)
                .collect();
            assert_eq!(starts.len(), 3);
            // The persisted wall clock lease has millisecond resolution.
            let resolution = Duration::from_millis(1);
            assert!(starts[1] - starts[0] + resolution >= Duration::from_millis(40));
            assert!(starts[2] - starts[1] + resolution >= Duration::from_millis(80));
            let recovery = state
                .spawns
                .iter()
                .rev()
                .find(|s| s.role == target && s.version == "1.0.0")
                .unwrap();
            assert!(recovery.at - starts[2] + resolution >= Duration::from_millis(160));
        }
    }

    #[tokio::test]
    async fn confirmed_crash_is_restarted_from_its_own_current_pointer() {
        let fixture = Fixture::new();
        fixture.start().await;
        let actor = Role::Base;
        let target = actor.other();
        let version = fixture.install(target, "2.0.0");
        fixture
            .fake
            .behavior(target, "2.0.0", Behavior::CrashAfterConfirm);
        assert!(matches!(
            fixture.update(actor, &version).await,
            UpdateOutcome::Updated { .. }
        ));
        let pid = fixture.fake.pid(target);
        fixture.fake.behavior(target, "2.0.0", Behavior::Good);
        until(|| fixture.fake.pid(target) != pid).await;
        fixture.healthy(target, "2.0.0").await;
        assert_eq!(
            fixture
                .fake
                .duo(actor)
                .layout(target)
                .pointer()
                .unwrap()
                .unwrap()
                .current,
            "2.0.0"
        );
    }

    #[tokio::test]
    async fn refuses_offline_pending_failed_and_uninstalled_targets_and_unconfirmed_actor() {
        for actor in ROLES {
            let fixture = Fixture::new();
            let duo = fixture.fake.duo(actor);
            let target = actor.other();
            let version = fixture.install(target, "2.0.0");
            let me = duo.layout(actor);
            let peer = duo.layout(target);
            seed_heartbeat(&duo, actor, "1.0.0", 101);
            assert!(
                matches!(duo.update_peer(&version).await.unwrap(), UpdateOutcome::Refused { reason } if reason.contains("online"))
            );
            seed_heartbeat(&duo, target, "1.0.0", 102);
            assert!(matches!(
                duo.update_peer(&Version::parse("9.0.0").unwrap())
                    .await
                    .unwrap(),
                UpdateOutcome::Refused { .. }
            ));
            peer.activate(&version).unwrap();
            seed_heartbeat(&duo, target, "2.0.0", 102);
            assert!(
                matches!(duo.update_peer(&Version::parse("9.0.0").unwrap()).await.unwrap(), UpdateOutcome::Refused { reason } if reason.contains("confirmed"))
            );
            peer.rollback("test failure").unwrap();
            seed_heartbeat(&duo, target, "1.0.0", 102);
            assert!(
                matches!(duo.update_peer(&version).await.unwrap(), UpdateOutcome::Refused { reason } if reason.contains("failed"))
            );
            let own_version = fixture.install(actor, "2.0.0");
            me.activate(&own_version).unwrap();
            assert!(
                matches!(duo.update_peer(&version).await.unwrap(), UpdateOutcome::Refused { reason } if reason.contains("actor"))
            );
            // A process from the old version cannot confirm a newly activated pointer.
            assert!(duo.confirm_self().is_err());
            assert_eq!(me.pointer().unwrap().unwrap().state, PointerState::Pending);
            assert!(fixture.fake.state.lock().unwrap().spawns.is_empty());
        }
    }

    #[tokio::test]
    async fn actor_keeps_heartbeating_and_excludes_crossed_updates() {
        for actor in ROLES {
            let fixture = Fixture::new();
            fixture.start().await;
            let target = actor.other();
            let version = fixture.install(target, "2.0.0");
            fixture
                .fake
                .behavior(target, "2.0.0", Behavior::NeverConfirm);
            let duo = fixture.fake.duo(actor);
            let actor_pid = fixture.fake.pid(actor);
            let initial = duo.heartbeat(actor).unwrap().unwrap().t_ms;
            let request_duo = duo.clone();
            let request = tokio::spawn(async move {
                request_duo
                    .request_peer_update(&version, Duration::from_secs(3))
                    .await
            });
            until(|| duo.layout(target).pointer().unwrap().unwrap().state == PointerState::Pending)
                .await;
            let opposite = fixture.fake.duo(target);
            let own_update = fixture.install(actor, "3.0.0");
            assert!(
                matches!(opposite.update_peer(&own_update).await.unwrap(), UpdateOutcome::Refused { reason } if reason.contains("in progress"))
            );
            for _ in 0..4 {
                sleep(Duration::from_millis(60)).await;
                let beat = duo.heartbeat(actor).unwrap().unwrap();
                assert!(duo.fresh(&beat));
                assert!(beat.t_ms > initial);
                assert_eq!(beat.pid, actor_pid);
            }
            assert!(matches!(
                request.await.unwrap().unwrap(),
                UpdateOutcome::RolledBack { .. }
            ));
            assert!(
                !fixture
                    .fake
                    .state
                    .lock()
                    .unwrap()
                    .kills
                    .contains(&actor_pid)
            );
            assert_eq!(
                fixture
                    .fake
                    .state
                    .lock()
                    .unwrap()
                    .spawns
                    .iter()
                    .filter(|s| s.role == actor)
                    .count(),
                1
            );
            assert_eq!(
                duo.layout(actor).pointer().unwrap().unwrap().current,
                "1.0.0"
            );
        }
    }

    #[tokio::test]
    async fn both_down_require_os_entry_and_stopping_actor_does_not_kill_peer() {
        let fixture = Fixture::new();
        fixture.start().await;
        fixture.fake.kill(fixture.fake.pid(Role::Base)).unwrap();
        assert!(
            fixture
                .fake
                .duo(Role::Upper)
                .heartbeat(Role::Upper)
                .unwrap()
                .is_some()
        );
        fixture.fake.kill(fixture.fake.pid(Role::Upper)).unwrap();
        let count = fixture.fake.state.lock().unwrap().spawns.len();
        sleep(Duration::from_millis(250)).await;
        assert_eq!(fixture.fake.state.lock().unwrap().spawns.len(), count);
        fixture.fake.start(Role::Base);
        until(|| fixture.fake.state.lock().unwrap().spawns.len() >= count + 2).await;
        fixture.healthy(Role::Upper, "1.0.0").await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_supervisors_respect_persisted_spawn_grace() {
        for actor in ROLES {
            let fixture = Fixture::new();
            let version = fixture.install(actor.other(), "2.0.0");
            fixture
                .fake
                .behavior(actor.other(), "2.0.0", Behavior::CrashImmediately);
            let first = fixture.fake.duo(actor);
            first.layout(actor.other()).activate(&version).unwrap();
            let second = fixture.fake.duo(actor);
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let tasks: Vec<_> = [first.clone(), second]
                .into_iter()
                .map(|duo| {
                    let barrier = barrier.clone();
                    tokio::task::spawn_blocking(move || {
                        barrier.wait();
                        duo.supervise().unwrap();
                    })
                })
                .collect();
            for task in tasks {
                task.await.unwrap();
            }
            first.supervise().unwrap();
            assert_eq!(
                first
                    .layout(actor.other())
                    .pointer()
                    .unwrap()
                    .unwrap()
                    .attempts,
                1
            );
            assert_eq!(
                fixture
                    .fake
                    .state
                    .lock()
                    .unwrap()
                    .spawns
                    .iter()
                    .filter(|s| s.role == actor.other())
                    .count(),
                1
            );
        }
    }

    #[tokio::test]
    async fn stale_and_wrong_actor_requests_are_ignored() {
        for actor in ROLES {
            let fixture = Fixture::new();
            let duo = fixture.fake.duo(actor);
            let requests = duo.shared().join("req");
            let stale = requests.join("stale.json");
            atomic_write(
                &stale,
                &Request {
                    actor,
                    version: "2.0.0".into(),
                    t_ms: now_ms().unwrap() - 120_001,
                },
            )
            .unwrap();
            let wrong = requests.join("wrong.json");
            atomic_write(
                &wrong,
                &Request {
                    actor: actor.other(),
                    version: "2.0.0".into(),
                    t_ms: now_ms().unwrap(),
                },
            )
            .unwrap();
            fs::write(requests.join("partial.json"), b"{").unwrap();
            duo.requests().await.unwrap();
            assert!(!stale.with_extension("res").exists());
            assert!(!wrong.with_extension("res").exists());
            assert!(fixture.fake.state.lock().unwrap().spawns.is_empty());
            assert_eq!(
                duo.layout(actor.other())
                    .pointer()
                    .unwrap()
                    .unwrap()
                    .current,
                "1.0.0"
            );
        }
    }

    #[tokio::test]
    async fn confirmation_requires_new_pid_matching_version_and_fresh_heartbeat() {
        let mut fixture = Fixture::new();
        fixture.fake.config.confirm_window = Duration::from_millis(25);
        let duo = fixture.fake.duo(Role::Base);
        let version = Version::parse("1.0.0").unwrap();
        seed_heartbeat(&duo, Role::Upper, "1.0.0", 102);
        assert!(!duo.wait_healthy(&version, 102).await.unwrap());
        seed_heartbeat(&duo, Role::Upper, "9.0.0", 103);
        assert!(!duo.wait_healthy(&version, 102).await.unwrap());
        let path = duo.shared().join("hb-upper.json");
        atomic_write(
            &path,
            &Heartbeat {
                pid: 103,
                version: "1.0.0".into(),
                t_ms: now_ms().unwrap() - 1_000,
            },
        )
        .unwrap();
        assert!(!duo.wait_healthy(&version, 102).await.unwrap());
        seed_heartbeat(&duo, Role::Upper, "1.0.0", 103);
        assert!(duo.wait_healthy(&version, 102).await.unwrap());
    }

    #[tokio::test]
    async fn run_cancellation_leaves_peer_alive() {
        let fixture = Fixture::new();
        fixture.fake.start(Role::Upper);
        // Reserve upper's spawn lease for base so the fake peer cannot also start base.
        let base = fixture.fake.duo(Role::Base);
        atomic_write(
            &fixture.fake.duo(Role::Upper).spawn_state_path(),
            &SpawnState {
                failures: 1,
                next_ms: now_ms().unwrap() + 5_000,
                grace_until_ms: now_ms().unwrap() + 5_000,
            },
        )
        .unwrap();
        let stop = CancellationToken::new();
        let runner = base.clone();
        let token = stop.clone();
        let task = tokio::spawn(async move { runner.run(token).await });
        until(|| base.heartbeat(Role::Base).unwrap().is_some()).await;
        let peer_pid = fixture.fake.pid(Role::Upper);
        stop.cancel();
        task.await.unwrap().unwrap();
        sleep(Duration::from_millis(30)).await;
        assert_eq!(fixture.fake.pid(Role::Upper), peer_pid);
        assert!(!fixture.fake.state.lock().unwrap().kills.contains(&peer_pid));
    }

    #[tokio::test]
    async fn expired_pending_launch_rolls_back_without_starting_failed_version() {
        for actor in ROLES {
            let fixture = Fixture::new();
            let duo = fixture.fake.duo(actor);
            let target = actor.other();
            let version = fixture.install(target, "2.0.0");
            let mut pointer = duo.layout(target).activate(&version).unwrap();
            pointer.activated_at = now_ms().unwrap() / 1000 - 20;
            atomic_write(
                &fixture
                    .fake
                    .config
                    .runtime
                    .join(target.as_str())
                    .join("current.json"),
                &pointer,
            )
            .unwrap();
            duo.supervise().unwrap();
            let pointer = duo.layout(target).pointer().unwrap().unwrap();
            assert_eq!(pointer.current, "1.0.0");
            assert_eq!(pointer.failed, ["2.0.0"]);
            let state = fixture.fake.state.lock().unwrap();
            assert_eq!(state.spawns.len(), 1);
            assert_eq!(state.spawns[0].version, "1.0.0");
        }
    }

    #[tokio::test]
    async fn update_launch_budget_rollback_does_not_double_spawn_fallback() {
        for actor in ROLES {
            let mut fixture = Fixture::new();
            fixture.fake.config.max_attempts = 0;
            fixture.start().await;
            let target = actor.other();
            let version = fixture.install(target, "2.0.0");
            assert!(
                matches!(fixture.update(actor, &version).await, UpdateOutcome::RolledBack { restored, .. } if restored.to_string() == "1.0.0")
            );
            let state = fixture.fake.state.lock().unwrap();
            let spawns: Vec<_> = state
                .spawns
                .iter()
                .filter(|s| s.role == target)
                .map(|s| s.version.as_str())
                .collect();
            assert_eq!(spawns, ["1.0.0", "1.0.0"]);
        }
    }

    #[tokio::test]
    async fn failed_recovery_reports_error_and_guard_keeps_running() {
        let fixture = Fixture::new();
        fixture.start().await;
        let duo = fixture.fake.duo(Role::Base);
        let version = fixture.install(Role::Upper, "2.0.0");
        fixture
            .fake
            .behavior(Role::Upper, "2.0.0", Behavior::SpawnError);
        fixture
            .fake
            .behavior(Role::Upper, "1.0.0", Behavior::SpawnError);
        let actor_pid = fixture.fake.pid(Role::Base);
        let error = duo
            .request_peer_update(&version, Duration::from_secs(2))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("fake spawn failed"));
        sleep(Duration::from_millis(40)).await;
        let heartbeat = duo.heartbeat(Role::Base).unwrap().unwrap();
        assert!(duo.fresh(&heartbeat));
        assert_eq!(heartbeat.pid, actor_pid);
        let pointer = duo.layout(Role::Upper).pointer().unwrap().unwrap();
        assert_eq!(pointer.current, "1.0.0");
        assert_eq!(pointer.failed, ["2.0.0"]);
    }

    #[tokio::test]
    async fn backoff_caps_at_four_seconds_and_bad_heartbeat_is_not_live() {
        let fixture = Fixture::new();
        let duo = fixture.fake.duo(Role::Base);
        fs::create_dir_all(duo.shared()).unwrap();
        fs::write(duo.shared().join("hb-upper.json"), b"{").unwrap();
        assert!(duo.heartbeat(Role::Upper).unwrap().is_none());
        let mut state = SpawnState {
            failures: u32::MAX,
            ..SpawnState::default()
        };
        let before = now_ms().unwrap();
        duo.launch_peer(&mut state).unwrap();
        let after = now_ms().unwrap();
        assert!(state.next_ms >= before + 4000 && state.next_ms <= after + 4000);
        assert_eq!(state.failures, u32::MAX);
        duo.supervise().unwrap();
        assert_eq!(fixture.fake.state.lock().unwrap().spawns.len(), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn process_spawner_detaches_and_passes_role_arguments_and_environment() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = Fixture::new();
        let version = Version::parse("1.0.0").unwrap();
        let exe = fixture
            .fake
            .duo(Role::Base)
            .layout(Role::Base)
            .version_dir(&version)
            .join(executable_relative_path());
        fs::write(&exe, b"#!/bin/sh\nprintf '%s\\n' \"$$\" \"$CRANIBOX_RUNTIME_ROOT\" \"$CRANIBOX_RUNTIME_VERSION\" \"$CRANIBOX_RUNTIME_ROLE\" \"$CRANIBOX_RUNTIME_MANAGED\" \"$@\" > \"$CRANIBOX_RUNTIME_ROOT/spawned\"\nexec sleep 5\n").unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        let marker = fixture.fake.config.runtime.join("spawned");
        ProcessSpawner::new(vec![OsString::from("argument with spaces")])
            .spawn(Role::Base, &version, &exe)
            .unwrap();
        until(|| fs::read_to_string(&marker).is_ok_and(|text| text.lines().count() == 8)).await;
        let marker = fs::read_to_string(marker).unwrap();
        let lines: Vec<_> = marker.lines().collect();
        let pid = lines[0].parse::<u32>().unwrap();
        // Kill on assertion failure too, so this test never leaves a runtime behind.
        struct Cleanup(u32);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = ProcessSpawner::default().kill(self.0);
            }
        }
        let cleanup = Cleanup(pid);
        assert_eq!(lines[1], fixture.fake.config.runtime.to_str().unwrap());
        assert_eq!(
            &lines[2..],
            [
                "1.0.0",
                "base",
                "1",
                "--role",
                "base",
                "argument with spaces"
            ]
        );
        // SAFETY: these calls only query the live fixture process's session and group.
        unsafe {
            assert_eq!(libc::getsid(pid as i32), pid as i32);
            assert_eq!(libc::getpgid(pid as i32), pid as i32);
        }
        drop(cleanup);
        until(|| {
            // SAFETY: signal zero only checks whether the child has been reaped.
            unsafe { libc::kill(pid as i32, 0) == -1 }
        })
        .await;
        ProcessSpawner::default().kill(pid).unwrap();
    }
}
