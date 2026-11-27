//! `cranibox serve --daemon`, `cranibox serve status` and `cranibox serve stop`:
//! run the device service in the background where there is no service manager
//! (containers, CI machines, servers without systemd or root).
//!
//! The background process is the same program started again as `cranibox serve …`
//! in its own session, with output appended to `<device dir>/logs/serve.log` and its
//! process id in `<device dir>/serve.pid`. It goes through the normal start path, so a
//! managed runtime still updates itself; stopping signals the whole session the way a
//! service manager would.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What the command line asked for.
#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    /// Start in the background with these arguments (program name first, `--daemon` removed).
    Start(Vec<String>),
    Status,
    Stop,
}

/// Where the background service keeps its process id and log.
#[derive(Clone, Debug)]
pub struct Paths {
    pub pid: PathBuf,
    pub log: PathBuf,
}
impl Paths {
    /// Next to the device settings (`--config`, default [`crate::settings::default_path`]).
    pub fn for_config(config: &Path) -> Self {
        let dir = config
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            pid: dir.join("serve.pid"),
            log: dir.join("logs").join("serve.log"),
        }
    }
}

/// Contents of the pid file.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Record {
    pub pid: u32,
    pub log: PathBuf,
    pub started_at: u64,
    pub args: Vec<String>,
}

/// Recognise `serve --daemon`, `serve status` and `serve stop` (also after `node`), and
/// the `--config` they apply to. Anything else is left to the normal command line.
pub fn parse(args: &[String]) -> Option<(Request, PathBuf)> {
    let mut config = None;
    let mut serve_at = None;
    let mut node_at = None;
    let mut i = 1;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--config" => {
                config = args.get(i + 1).map(PathBuf::from);
                i += 2;
                continue;
            }
            _ if arg.starts_with("--config=") => config = Some(PathBuf::from(&arg[9..])),
            "-h" | "--help" | "-V" | "--version" => return None,
            "node" if serve_at.is_none() && node_at.is_none() => node_at = Some(i),
            "serve" if serve_at.is_none() => serve_at = Some(i),
            _ if serve_at.is_none() && !arg.starts_with('-') => return None,
            _ => {}
        }
        i += 1;
    }
    let serve_at = serve_at?;
    let config = config.unwrap_or_else(crate::settings::default_path);
    match args.get(serve_at + 1).map(String::as_str) {
        Some("status") => return Some((Request::Status, config)),
        Some("stop") => return Some((Request::Stop, config)),
        _ => {}
    }
    if !args[serve_at + 1..].iter().any(|a| a == "--daemon") {
        return None;
    }
    // `node serve` and `serve` are the same command.
    let rest: Vec<String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| *i == 0 || (a.as_str() != "--daemon" && Some(*i) != node_at))
        .map(|(_, a)| a.clone())
        .collect();
    Some((Request::Start(rest), config))
}

/// Handle a daemon request if `args` is one; returns the exit code.
pub fn intercept(args: &[String]) -> Option<i32> {
    let (request, config) = parse(args)?;
    let paths = Paths::for_config(&config);
    let result = match request {
        Request::Start(rest) => std::env::current_exe()
            .context("find this program")
            .and_then(|exe| start(&paths, &exe, &rest[1..]))
            .map(|outcome| {
                println!("{}", outcome.message(&paths));
                0
            }),
        Request::Status => Ok(match status(&paths) {
            State::Running(record) => {
                println!(
                    "Cranibox is running in the background (pid {}).\n  log: {}\n  stop: cranibox serve stop",
                    record.pid,
                    record.log.display()
                );
                0
            }
            State::Stale(record) => {
                println!(
                    "Cranibox is not running (pid {} has exited; removed {}).\n  last log: {}",
                    record.pid,
                    paths.pid.display(),
                    record.log.display()
                );
                3
            }
            State::Stopped => {
                println!("Cranibox is not running in the background.");
                3
            }
        }),
        Request::Stop => stop(&paths, Duration::from_secs(20)).map(|stopped| {
            match stopped {
                Some(pid) => println!("Stopped Cranibox (pid {pid})."),
                None => println!("Cranibox is not running in the background."),
            }
            0
        }),
    };
    Some(result.unwrap_or_else(|error| {
        eprintln!("Error: {error:#}");
        1
    }))
}

/// The background service's state according to its pid file.
#[derive(Debug, PartialEq, Eq)]
pub enum State {
    Running(Record),
    /// The pid file named a process that is gone; the file was removed.
    Stale(Record),
    Stopped,
}

pub fn read_record(paths: &Paths) -> Option<Record> {
    let text = std::fs::read_to_string(&paths.pid).ok()?;
    serde_json::from_str(&text).ok().or_else(|| {
        // A bare number written by hand or an older tool.
        text.trim().parse().ok().map(|pid| Record {
            pid,
            log: paths.log.clone(),
            started_at: 0,
            args: vec![],
        })
    })
}

pub fn status(paths: &Paths) -> State {
    let Some(record) = read_record(paths) else {
        if paths.pid.exists() {
            let _ = std::fs::remove_file(&paths.pid);
        }
        return State::Stopped;
    };
    if alive(record.pid) {
        State::Running(record)
    } else {
        let _ = std::fs::remove_file(&paths.pid);
        State::Stale(record)
    }
}

/// Result of [`start`].
#[derive(Debug, PartialEq, Eq)]
pub enum Started {
    New(Record),
    AlreadyRunning(Record),
}
impl Started {
    pub fn message(&self, paths: &Paths) -> String {
        match self {
            Started::New(record) => format!(
                "Cranibox is running in the background (pid {}).\n  log: {}\n  pid file: {}\n  status: cranibox serve status\n  stop: cranibox serve stop",
                record.pid,
                record.log.display(),
                paths.pid.display()
            ),
            Started::AlreadyRunning(record) => format!(
                "Cranibox is already running in the background (pid {}).\n  log: {}",
                record.pid,
                record.log.display()
            ),
        }
    }
}

#[cfg(unix)]
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(unix)]
/// Last `lines` lines of the log, for a service that exited right away.
fn tail(path: &Path, lines: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

#[cfg(unix)]
/// How long a new service must stay up before it counts as started.
const SETTLE: Duration = Duration::from_millis(1500);

/// Start `program args…` detached, logging to [`Paths::log`].
#[cfg(unix)]
pub fn start(paths: &Paths, program: &Path, args: &[String]) -> Result<Started> {
    use std::os::unix::process::CommandExt;
    if let State::Running(record) = status(paths) {
        return Ok(Started::AlreadyRunning(record));
    }
    let log_dir = paths.log.parent().context("log directory")?;
    std::fs::create_dir_all(log_dir).with_context(|| format!("create {}", log_dir.display()))?;
    if let Some(dir) = paths.pid.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.log)
        .with_context(|| format!("open {}", paths.log.display()))?;
    {
        use std::io::Write;
        let _ = writeln!(
            log,
            "--- cranibox serve started in the background at {} (unix time) ---",
            now()
        );
    }
    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .env("CRANIBOX_DAEMON", "1");
    // SAFETY: setsid is async-signal-safe; it detaches from the terminal so closing
    // the shell does not hang up the service, and makes the pid a group to signal.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("start {}", program.display()))?;
    let record = Record {
        pid: child.id(),
        log: paths.log.clone(),
        started_at: now(),
        args: args.to_vec(),
    };
    write_record(paths, &record)?;
    let deadline = std::time::Instant::now() + SETTLE;
    while std::time::Instant::now() < deadline {
        if let Some(exit) = child.try_wait()? {
            let _ = std::fs::remove_file(&paths.pid);
            bail!(
                "the background service exited right away ({exit}). Last lines of {}:\n{}",
                paths.log.display(),
                tail(&paths.log, 20)
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(Started::New(record))
}

#[cfg(not(unix))]
pub fn start(_paths: &Paths, _program: &Path, _args: &[String]) -> Result<Started> {
    bail!(
        "`cranibox serve --daemon` is for Linux and macOS; on Windows use the desktop app, or run `cranibox serve` in a terminal"
    )
}

#[cfg(unix)]
fn write_record(paths: &Paths, record: &Record) -> Result<()> {
    let temporary = paths.pid.with_extension("pid.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(record)?)?;
    std::fs::rename(&temporary, &paths.pid)
        .with_context(|| format!("write {}", paths.pid.display()))
}

/// Stop the background service: SIGTERM to its session, SIGKILL after `grace`.
/// Returns the pid it stopped, or None when nothing was running.
#[cfg(unix)]
pub fn stop(paths: &Paths, grace: Duration) -> Result<Option<u32>> {
    let record = match status(paths) {
        State::Running(record) => record,
        State::Stale(_) | State::Stopped => return Ok(None),
    };
    let pid = record.pid as libc::pid_t;
    signal(pid, libc::SIGTERM);
    if !wait_gone(record.pid, grace) {
        signal(pid, libc::SIGKILL);
        if !wait_gone(record.pid, Duration::from_secs(5)) {
            bail!("pid {} did not stop", record.pid);
        }
    }
    let _ = std::fs::remove_file(&paths.pid);
    Ok(Some(record.pid))
}

#[cfg(not(unix))]
pub fn stop(paths: &Paths, _grace: Duration) -> Result<Option<u32>> {
    match status(paths) {
        State::Running(_) => bail!("`cranibox serve stop` is for Linux and macOS"),
        State::Stale(_) | State::Stopped => Ok(None),
    }
}

/// Signal the session the service leads (it called setsid), else just the process.
#[cfg(unix)]
fn signal(pid: libc::pid_t, sig: libc::c_int) {
    // SAFETY: plain kill(2) calls.
    unsafe {
        if libc::getpgid(pid) == pid {
            libc::kill(-pid, sig);
        } else {
            libc::kill(pid, sig);
        }
    }
}

#[cfg(unix)]
fn wait_gone(pid: u32, within: Duration) -> bool {
    let deadline = std::time::Instant::now() + within;
    loop {
        if !alive(pid) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The process exists, is not a zombie and (on Linux) is still a `serve` process,
/// so a recycled pid is not mistaken for the service.
#[cfg(unix)]
pub fn alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    // SAFETY: signal 0 only checks that the process exists.
    let exists = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0
        || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    if !exists {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        let proc = PathBuf::from(format!("/proc/{pid}"));
        if let Ok(stat) = std::fs::read_to_string(proc.join("stat")) {
            // `pid (comm) S …`: the state follows the last ')'.
            if let Some(state) = stat.rsplit_once(')').map(|(_, rest)| rest.trim_start())
                && (state.starts_with('Z') || state.starts_with('X'))
            {
                return false;
            }
        }
        if let Ok(cmdline) = std::fs::read(proc.join("cmdline")) {
            return cmdline.split(|b| *b == 0).any(|arg| arg == b"serve");
        }
    }
    true
}

#[cfg(not(unix))]
pub fn alive(_pid: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn recognises_daemon_status_and_stop_only() {
        let config = PathBuf::from("/tmp/x/settings.json");
        assert_eq!(
            parse(&args(&[
                "cranibox",
                "serve",
                "--daemon",
                "--listen",
                "127.0.0.1:9"
            ])),
            Some((
                Request::Start(args(&["cranibox", "serve", "--listen", "127.0.0.1:9"])),
                crate::settings::default_path()
            ))
        );
        assert_eq!(
            parse(&args(&[
                "cranibox",
                "--config",
                "/tmp/x/settings.json",
                "node",
                "serve",
                "--daemon"
            ])),
            Some((
                Request::Start(args(&[
                    "cranibox",
                    "--config",
                    "/tmp/x/settings.json",
                    "serve"
                ])),
                config.clone()
            ))
        );
        assert_eq!(
            parse(&args(&[
                "cranibox",
                "serve",
                "status",
                "--config=/tmp/x/settings.json"
            ])),
            Some((Request::Status, config.clone()))
        );
        assert_eq!(
            parse(&args(&["cranibox", "serve", "stop"])).map(|(r, _)| r),
            Some(Request::Stop)
        );
        // Plain `serve`, other commands and help stay with the normal command line.
        for other in [
            &["cranibox", "serve"][..],
            &["cranibox", "serve", "--listen", "127.0.0.1:9"],
            &["cranibox", "serve", "--daemon", "--help"],
            &["cranibox", "peer", "serve"],
            &["cranibox", "nodes", "--daemon"],
            &["cranibox"],
        ] {
            assert_eq!(parse(&args(other)), None, "{other:?}");
        }
    }

    #[test]
    fn paths_sit_next_to_the_settings() {
        let paths = Paths::for_config(Path::new("/data/cranibox/settings.json"));
        assert_eq!(paths.pid, Path::new("/data/cranibox/serve.pid"));
        assert_eq!(paths.log, Path::new("/data/cranibox/logs/serve.log"));
    }

    #[test]
    fn stale_and_unreadable_pid_files_are_cleaned_up() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::for_config(&dir.path().join("settings.json"));
        assert_eq!(status(&paths), State::Stopped);
        std::fs::write(&paths.pid, "not a pid").unwrap();
        assert_eq!(status(&paths), State::Stopped);
        assert!(!paths.pid.exists());
        // A pid that cannot exist.
        std::fs::write(&paths.pid, format!("{}", i32::MAX as u32 - 1)).unwrap();
        assert!(matches!(status(&paths), State::Stale(_)));
        assert!(!paths.pid.exists());
        assert_eq!(stop(&paths, Duration::from_secs(1)).unwrap(), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn start_status_stop_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::for_config(&dir.path().join("settings.json"));
        let sh = Path::new("/bin/sh");
        // `$0` is `serve`, so the process looks like the service.
        let script = args(&["-c", "echo service-up; while :; do sleep 1; done", "serve"]);
        let started = start(&paths, sh, &script).unwrap();
        let Started::New(record) = started else {
            panic!("expected a new service: {started:?}")
        };
        assert!(paths.pid.exists());
        assert_eq!(read_record(&paths).unwrap(), record);
        assert!(alive(record.pid));
        assert_eq!(status(&paths), State::Running(record.clone()));
        // Starting again reports the running one instead of a second copy.
        assert!(
            matches!(start(&paths, sh, &script).unwrap(), Started::AlreadyRunning(r) if r.pid == record.pid)
        );
        let log = std::fs::read_to_string(&paths.log).unwrap();
        assert!(log.contains("started in the background"), "{log}");
        assert!(log.contains("service-up"), "{log}");
        assert_eq!(
            stop(&paths, Duration::from_secs(5)).unwrap(),
            Some(record.pid)
        );
        assert!(!paths.pid.exists());
        assert!(!alive(record.pid));
        assert_eq!(status(&paths), State::Stopped);
        // The log stays for later reading.
        assert!(paths.log.exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_service_that_exits_at_once_is_reported_with_its_log() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::for_config(&dir.path().join("settings.json"));
        let error = start(
            &paths,
            Path::new("/bin/sh"),
            &args(&["-c", "echo port already in use >&2; exit 7", "serve"]),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("exited right away"), "{error}");
        assert!(error.contains("port already in use"), "{error}");
        assert!(!paths.pid.exists());
    }
}
