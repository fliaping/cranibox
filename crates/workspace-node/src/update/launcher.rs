use std::{ffi::OsString, path::PathBuf, process::Stdio, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use tokio::{
    process::{Child, Command},
    time::{Instant, sleep, timeout},
};
use tokio_util::sync::CancellationToken;

use super::{
    layout::{LaunchDecision, Layout},
    npm::executable_relative_path,
    version::Version,
};
use crate::node::ProcessTree;

pub const RESTART_EXIT_CODE: i32 = 75;

#[derive(Debug)]
pub struct LauncherConfig {
    pub root: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    pub max_attempts: u32,
    pub confirm_window: Duration,
    pub backoff_base: Duration,
    pub backoff_max: Duration,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::new(),
            args: Vec::new(),
            env: Vec::new(),
            max_attempts: 3,
            confirm_window: Duration::from_secs(120),
            backoff_base: Duration::from_millis(500),
            backoff_max: Duration::from_secs(30),
        }
    }
}

impl LauncherConfig {
    /// Supply the required runtime root and arguments with the default restart policy.
    pub fn new(root: impl Into<PathBuf>, args: Vec<OsString>) -> Self {
        Self {
            root: root.into(),
            args,
            ..Self::default()
        }
    }
}

/// Run until the runtime exits normally, recovery is unavailable, or `stop` fires.
/// The host should forward its shutdown signals to `stop`.
pub async fn run(config: LauncherConfig, stop: CancellationToken) -> Result<i32> {
    ensure!(
        !config.root.as_os_str().is_empty(),
        "runtime root is required"
    );
    let layout = Arc::new(Layout::new(&config.root));
    let base = config.backoff_base.min(config.backoff_max);
    let mut backoff = base;
    loop {
        if stop.is_cancelled() {
            return Ok(0);
        }
        let pending_layout = Arc::clone(&layout);
        let max_attempts = config.max_attempts;
        let confirm_window = config.confirm_window;
        // Layout takes a filesystem lock and fsyncs the pointer before each start.
        let decision = tokio::task::spawn_blocking(move || {
            pending_layout.begin_launch(max_attempts, confirm_window)
        })
        .await
        .context("join runtime launch decision")??;
        if stop.is_cancelled() {
            return Ok(0);
        }
        let version = match decision {
            LaunchDecision::Run { version } => version,
            LaunchDecision::RolledBack { from, to, reason } => {
                eprintln!("runtime rolled back from {from} to {to}: {reason}");
                continue;
            }
            LaunchDecision::Broken { reason } => {
                eprintln!("runtime cannot start: {reason}");
                return Ok(1);
            }
        };
        let started = Instant::now();
        match spawn_runtime(&config, &version).await {
            Ok((mut child, tree)) => {
                let status = tokio::select! {
                    biased;
                    _ = stop.cancelled() => {
                        terminate(&mut child, &tree).await?;
                        return Ok(0);
                    }
                    status = child.wait() => status.context("wait for runtime"),
                };
                // A runtime may leave descendants behind even after a normal exit.
                tree.kill();
                match status?.code() {
                    Some(0) => return Ok(0),
                    Some(RESTART_EXIT_CODE) => {
                        backoff = base;
                        continue;
                    }
                    _ => {}
                }
            }
            Err(error) => eprintln!("runtime {version} failed to start: {error:#}"),
        }
        if started.elapsed() >= Duration::from_secs(60) {
            backoff = base;
        }
        tokio::select! {
            biased;
            _ = stop.cancelled() => return Ok(0),
            _ = sleep(backoff) => {}
        }
        backoff = backoff.saturating_mul(2).min(config.backoff_max);
    }
}

async fn spawn_runtime(config: &LauncherConfig, version: &Version) -> Result<(Child, ProcessTree)> {
    let executable = Layout::new(&config.root)
        .version_dir(version)
        .join(executable_relative_path());
    let mut command = Command::new(executable);
    command
        .args(&config.args)
        .envs(config.env.iter().cloned())
        .env("CRANIBOX_RUNTIME_ROOT", &config.root)
        .env("CRANIBOX_RUNTIME_VERSION", version.to_string())
        .env("CRANIBOX_RUNTIME_MANAGED", "1")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    // ProcessTree assigns the suspended child to its kill-on-close Job, then resumes it.
    #[cfg(windows)]
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
    let mut child = command.spawn().context("spawn runtime")?;
    match ProcessTree::new(&child) {
        Ok(tree) => Ok((child, tree)),
        Err(error) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(error).context("contain runtime process tree")
        }
    }
}

async fn terminate(child: &mut Child, tree: &ProcessTree) -> Result<()> {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // The runtime and its descendants share the group created before exec.
        unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
    }
    #[cfg(windows)]
    tree.kill();
    let waited = timeout(Duration::from_secs(5), child.wait()).await;
    tree.kill();
    match waited {
        Ok(status) => {
            status.context("wait for stopped runtime")?;
        }
        Err(_) => {
            let _ = child.start_kill();
            child.wait().await.context("reap killed runtime")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use super::super::layout::PointerState;

    const PREVIOUS: [&str; 2] = [
        "echo restored > restored\nexit 0",
        "echo restored>restored\r\nexit /b 0",
    ];
    const CRASH: [&str; 2] = [
        "echo crash >> crashes\nexit 9",
        "echo crash>>crashes\r\nexit /b 9",
    ];
    const LONG_RUNNING: [&str; 2] = [
        "echo $$ > started\nwhile :; do sleep 1; done",
        "echo started>started\r\npowershell.exe -NoProfile -NonInteractive -Command \"Start-Sleep -Seconds 60\"\r\nexit /b 0",
    ];

    fn fixture() -> (tempfile::TempDir, Layout, LauncherConfig) {
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
        let scratch = target.join("update-launcher-tests");
        fs::create_dir_all(&scratch).unwrap();
        let root = tempfile::Builder::new()
            .prefix("runtime with spaces-")
            .tempdir_in(scratch)
            .unwrap();
        let layout = Layout::new(root.path());
        #[cfg(unix)]
        let args = Vec::new();
        #[cfg(windows)]
        let args = [
            "--ignored",
            "--exact",
            "update::launcher::tests::windows_runtime",
            "--nocapture",
        ]
        .map(OsString::from)
        .to_vec();
        let mut config = LauncherConfig::new(root.path(), args);
        config.backoff_base = Duration::from_millis(5);
        config.backoff_max = Duration::from_millis(10);
        (root, layout, config)
    }

    fn install(layout: &Layout, value: &str, body: [&str; 2]) -> Version {
        let version = Version::parse(value).unwrap();
        let stage = layout.begin_stage(&version).unwrap();
        let executable = stage.join(executable_relative_path());
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::write(
                &executable,
                format!(
                    "#!/bin/sh\ncd \"$CRANIBOX_RUNTIME_ROOT\" || exit 99\n{}\n",
                    body[0]
                ),
            )
            .unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        }
        #[cfg(windows)]
        {
            // A native test-harness shim runs the .cmd fixture inside the runtime's Job.
            fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
            fs::write(
                executable.with_extension("cmd"),
                format!(
                    "@echo off\r\ncd /d \"%CRANIBOX_RUNTIME_ROOT%\" || exit /b 99\r\n{}\r\n",
                    body[1]
                ),
            )
            .unwrap();
        }
        layout.commit_stage(&version).unwrap();
        version
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "runtime fixture invoked only in a child process"]
    fn windows_runtime() {
        let script = std::env::current_exe().unwrap().with_extension("cmd");
        let status = std::process::Command::new("cmd.exe")
            .args(["/D", "/C"])
            .arg(script)
            .status()
            .unwrap();
        std::process::exit(status.code().unwrap_or(1));
    }

    fn activate(layout: &Layout, value: &str, body: [&str; 2]) -> Version {
        let version = install(layout, value, body);
        layout.activate(&version).unwrap();
        version
    }

    fn pending_pair(layout: &Layout, body: [&str; 2]) -> Version {
        let previous = activate(layout, "1.0.0", PREVIOUS);
        layout.confirm(&previous).unwrap();
        activate(layout, "2.0.0", body)
    }

    async fn run_to_exit(config: LauncherConfig) -> i32 {
        timeout(
            Duration::from_secs(4),
            run(config, CancellationToken::new()),
        )
        .await
        .expect("launcher did not exit")
        .unwrap()
    }

    async fn wait_for_file(path: PathBuf) {
        timeout(Duration::from_secs(2), async {
            while !fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 0) {
                sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("runtime did not write its marker");
    }

    #[test]
    fn default_policy() {
        let config = LauncherConfig::new("runtime", vec![OsString::from("node")]);
        assert_eq!(config.root, PathBuf::from("runtime"));
        assert_eq!(config.args, [OsString::from("node")]);
        assert!(config.env.is_empty());
        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.confirm_window, Duration::from_secs(120));
        assert_eq!(config.backoff_base, Duration::from_millis(500));
        assert_eq!(config.backoff_max, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn extra_environment_reaches_child() {
        let (root, layout, mut config) = fixture();
        activate(
            &layout,
            "1.0.0",
            [
                "printf '%s' \"$CRANIBOX_LAUNCHER_TEST\" > environment\nexit 0",
                "<nul set /p =%CRANIBOX_LAUNCHER_TEST%>environment\r\nexit /b 0",
            ],
        );
        config
            .env
            .push(("CRANIBOX_LAUNCHER_TEST".into(), "value with spaces".into()));
        assert_eq!(run_to_exit(config).await, 0);
        assert_eq!(
            fs::read_to_string(root.path().join("environment")).unwrap(),
            "value with spaces"
        );
    }

    #[tokio::test]
    async fn normal_exit_does_not_restart() {
        let (root, layout, config) = fixture();
        activate(
            &layout,
            "1.0.0",
            ["echo run >> runs\nexit 0", "echo run>>runs\r\nexit /b 0"],
        );
        assert_eq!(run_to_exit(config).await, 0);
        assert_eq!(
            fs::read_to_string(root.path().join("runs"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        assert_eq!(layout.pointer().unwrap().unwrap().attempts, 1);
    }

    #[tokio::test]
    async fn requested_restart_skips_backoff() {
        let (root, layout, mut config) = fixture();
        activate(
            &layout,
            "1.0.0",
            [
                "if [ ! -f once ]; then echo once > once; exit 75; fi\necho done > done\nexit 0",
                "if exist once goto done\r\necho once>once\r\nexit /b 75\r\n:done\r\necho done>done\r\nexit /b 0",
            ],
        );
        config.backoff_base = Duration::from_secs(30);
        config.backoff_max = config.backoff_base;
        assert_eq!(run_to_exit(config).await, 0);
        assert!(root.path().join("done").exists());
        assert_eq!(layout.pointer().unwrap().unwrap().attempts, 2);
    }

    #[tokio::test]
    async fn crash_then_success_waits_for_backoff() {
        let (_root, layout, mut config) = fixture();
        activate(
            &layout,
            "1.0.0",
            [
                "if [ ! -f once ]; then echo once > once; exit 9; fi\nexit 0",
                "if exist once exit /b 0\r\necho once>once\r\nexit /b 9",
            ],
        );
        config.backoff_base = Duration::from_millis(60);
        config.backoff_max = config.backoff_base;
        let started = Instant::now();
        assert_eq!(run_to_exit(config).await, 0);
        assert!(started.elapsed() >= Duration::from_millis(60));
        assert_eq!(layout.pointer().unwrap().unwrap().attempts, 2);
    }

    #[tokio::test]
    async fn crash_loop_restores_previous_runtime() {
        let (root, layout, mut config) = fixture();
        let failed = pending_pair(&layout, CRASH);
        config.max_attempts = 2;
        assert_eq!(run_to_exit(config).await, 0);
        assert_eq!(
            fs::read_to_string(root.path().join("crashes"))
                .unwrap()
                .lines()
                .count(),
            2
        );
        assert!(root.path().join("restored").exists());
        let pointer = layout.pointer().unwrap().unwrap();
        assert_eq!(pointer.current, "1.0.0");
        assert_eq!(pointer.state, PointerState::Confirmed);
        assert_eq!(pointer.failed, [failed.to_string()]);
    }

    #[tokio::test]
    async fn unconfirmed_restart_expires_before_attempt_limit() {
        let (root, layout, mut config) = fixture();
        let failed = pending_pair(&layout, CRASH);
        // Layout timestamps have one-second resolution; allow the first start,
        // then cross the confirmation deadline during the restart backoff.
        config.confirm_window = Duration::from_secs(1);
        config.max_attempts = 10;
        config.backoff_base = Duration::from_millis(1100);
        config.backoff_max = config.backoff_base;
        assert_eq!(run_to_exit(config).await, 0);
        let crashes = fs::read_to_string(root.path().join("crashes"))
            .unwrap()
            .lines()
            .count();
        assert!((1..=2).contains(&crashes));
        assert!(root.path().join("restored").exists());
        assert_eq!(
            layout.pointer().unwrap().unwrap().failed,
            [failed.to_string()]
        );
    }

    #[tokio::test]
    async fn broken_pointer_returns_failure() {
        let (_root, layout, config) = fixture();
        assert_eq!(run_to_exit(config).await, 1);
        assert!(layout.pointer().unwrap().is_none());
    }

    #[tokio::test]
    async fn crash_loop_without_previous_becomes_broken() {
        let (root, layout, mut config) = fixture();
        activate(&layout, "1.0.0", CRASH);
        config.max_attempts = 2;
        assert_eq!(run_to_exit(config).await, 1);
        assert_eq!(layout.pointer().unwrap().unwrap().attempts, 3);
        assert_eq!(
            fs::read_to_string(root.path().join("crashes"))
                .unwrap()
                .lines()
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn missing_executable_counts_as_failed_starts() {
        let (root, layout, mut config) = fixture();
        let failed = pending_pair(&layout, CRASH);
        fs::remove_file(layout.version_dir(&failed).join(executable_relative_path())).unwrap();
        config.max_attempts = 2;
        assert_eq!(run_to_exit(config).await, 0);
        assert!(!root.path().join("crashes").exists());
        assert!(root.path().join("restored").exists());
        assert_eq!(
            layout.pointer().unwrap().unwrap().failed,
            [failed.to_string()]
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unexecutable_runtime_counts_as_failed_starts() {
        use std::os::unix::fs::PermissionsExt;

        let (root, layout, mut config) = fixture();
        let failed = pending_pair(&layout, CRASH);
        fs::set_permissions(
            layout.version_dir(&failed).join(executable_relative_path()),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        config.max_attempts = 1;
        assert_eq!(run_to_exit(config).await, 0);
        assert!(root.path().join("restored").exists());
        assert_eq!(
            layout.pointer().unwrap().unwrap().failed,
            [failed.to_string()]
        );
    }

    #[tokio::test]
    async fn stop_terminates_running_child() {
        let (root, layout, config) = fixture();
        activate(&layout, "1.0.0", LONG_RUNNING);
        let stop = CancellationToken::new();
        let controller = async {
            wait_for_file(root.path().join("started")).await;
            stop.cancel();
        };
        let (result, ()) = tokio::join!(
            timeout(Duration::from_secs(2), run(config, stop.clone())),
            controller,
        );
        assert_eq!(result.unwrap().unwrap(), 0);
        #[cfg(unix)]
        {
            let pid: i32 = fs::read_to_string(root.path().join("started"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
        }
    }

    #[tokio::test]
    async fn stop_interrupts_backoff() {
        let (root, layout, mut config) = fixture();
        activate(&layout, "1.0.0", CRASH);
        config.backoff_base = Duration::from_secs(30);
        config.backoff_max = config.backoff_base;
        let stop = CancellationToken::new();
        let controller = async {
            wait_for_file(root.path().join("crashes")).await;
            sleep(Duration::from_millis(20)).await;
            stop.cancel();
        };
        let (result, ()) = tokio::join!(
            timeout(Duration::from_secs(2), run(config, stop.clone())),
            controller,
        );
        assert_eq!(result.unwrap().unwrap(), 0);
        assert_eq!(layout.pointer().unwrap().unwrap().attempts, 1);
    }

    #[tokio::test]
    async fn already_stopped_does_not_start_runtime() {
        let (root, layout, config) = fixture();
        activate(&layout, "1.0.0", CRASH);
        let stop = CancellationToken::new();
        stop.cancel();
        assert_eq!(run(config, stop).await.unwrap(), 0);
        assert_eq!(layout.pointer().unwrap().unwrap().attempts, 0);
        assert!(!root.path().join("crashes").exists());
    }

    #[tokio::test]
    async fn runtime_receives_managed_environment() {
        let (root, layout, config) = fixture();
        activate(
            &layout,
            "1.2.3",
            [
                "printf '%s\\n' \"$CRANIBOX_RUNTIME_ROOT\" \"$CRANIBOX_RUNTIME_VERSION\" \"$CRANIBOX_RUNTIME_MANAGED\" > environment\nprintf '%s\\n' \"$@\" > arguments\nexit 0",
                "(echo %CRANIBOX_RUNTIME_ROOT%&echo %CRANIBOX_RUNTIME_VERSION%&echo %CRANIBOX_RUNTIME_MANAGED%)>environment\r\nexit /b 0",
            ],
        );
        #[cfg(unix)]
        let config = {
            let mut config = config;
            config.args.extend([
                OsString::from("node"),
                OsString::from("argument with spaces"),
            ]);
            config
        };
        assert_eq!(run_to_exit(config).await, 0);
        let environment = fs::read_to_string(root.path().join("environment")).unwrap();
        assert_eq!(
            environment.lines().collect::<Vec<_>>(),
            [root.path().to_str().unwrap(), "1.2.3", "1"]
        );
        #[cfg(unix)]
        assert_eq!(
            fs::read_to_string(root.path().join("arguments")).unwrap(),
            "node\nargument with spaces\n"
        );
    }
}
