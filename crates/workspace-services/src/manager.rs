use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use serde_json::json;
use std::{
    collections::BTreeMap,
    env, fs,
    fs::OpenOptions,
    os::unix::{
        fs::{PermissionsExt, symlink},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const IMPORT_KEYS: &str = "TZ LANG LANGUAGE LC_ALL LC_CTYPE LC_MESSAGES LC_COLLATE LC_NUMERIC LC_TIME LC_MONETARY LC_PAPER LC_NAME LC_ADDRESS LC_TELEPHONE LC_MEASUREMENT LC_IDENTIFICATION HTTP_PROXY HTTPS_PROXY NO_PROXY http_proxy https_proxy no_proxy SSL_CERT_FILE SSL_CERT_DIR REQUESTS_CA_BUNDLE NODE_EXTRA_CA_CERTS";

struct Paths {
    root: PathBuf,
    units: PathBuf,
    wants: PathBuf,
    state: PathBuf,
    logs: PathBuf,
    scan: PathBuf,
}
impl Paths {
    fn new() -> Result<Self> {
        let root = PathBuf::from(env::var_os("HOME").context("HOME is required")?);
        let value = Self::at(root);
        value.initialize()?;
        Ok(value)
    }
    fn at(root: PathBuf) -> Self {
        let units = root.join(".config/systemd/user");
        let state = root.join(".local/run/user-systemd");
        Self {
            wants: units.join("default.target.wants"),
            units,
            logs: root.join(".local/log/user-systemd"),
            scan: root.join(".local/run/workspace-services/services"),
            root,
            state,
        }
    }
    fn initialize(&self) -> Result<()> {
        let value = self;
        for p in [
            &value.units,
            &value.wants,
            &value.state,
            &value.logs,
            &value.scan,
        ] {
            fs::create_dir_all(p)?;
        }
        fs::set_permissions(
            value.scan.parent().unwrap(),
            fs::Permissions::from_mode(0o700),
        )?;
        fs::set_permissions(&value.state, fs::Permissions::from_mode(0o700))?;
        Ok(())
    }
    fn file(&self, name: &str, extension: &str) -> PathBuf {
        self.state.join(format!("{name}.{extension}"))
    }
    fn service(&self, name: &str) -> PathBuf {
        self.scan.join(name)
    }
    fn unit(&self, name: &str) -> PathBuf {
        let p = self.units.join(format!("{name}.service"));
        if p.is_file() {
            p
        } else {
            self.units.join(name)
        }
    }
}

fn write(path: &Path, data: impl AsRef<[u8]>, mode: u32) -> Result<()> {
    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
    let mut options = OpenOptions::new();
    use std::os::unix::fs::OpenOptionsExt;
    options.write(true).create_new(true).mode(mode);
    let mut file = options.open(&temporary)?;
    use std::io::Write;
    file.write_all(data.as_ref())?;
    fs::rename(temporary, path)?;
    Ok(())
}
fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}
fn remove(path: &Path) {
    let _ = fs::remove_file(path);
}
fn name(value: &str) -> Result<String> {
    let n = value.strip_suffix(".service").unwrap_or(value);
    ensure!(
        !n.is_empty()
            && n != "."
            && n != ".."
            && !n.starts_with('-')
            && n.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.@-".contains(&c)),
        "Invalid service name"
    );
    Ok(n.to_owned())
}
fn tool(command: &str, args: &[&str], directory: Option<&Path>) -> Result<()> {
    let mut c = Command::new(command);
    c.args(args);
    if let Some(directory) = directory {
        c.arg(directory);
    }
    ensure!(
        c.status()
            .with_context(|| format!("{command} is required"))?
            .success(),
        "{command} failed"
    );
    Ok(())
}
fn stat(pid: u32) -> Option<(String, String)> {
    let s = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, tail) = s.rsplit_once(") ")?;
    let f: Vec<_> = tail.split_whitespace().collect();
    Some((f.first()?.to_string(), f.get(19)?.to_string()))
}
fn alive(pid: u32) -> bool {
    if pid <= 1 {
        return false;
    }
    stat(pid).is_some_and(|(state, _)| state != "Z" && state != "X")
}
fn pid(paths: &Paths, n: &str) -> Option<u32> {
    let p = read(&paths.file(n, "pid")).trim().parse().ok()?;
    if !alive(p) {
        return None;
    }
    let identity = read(&paths.file(n, "starttime"));
    if identity.is_empty() {
        let supervisor = legacy_supervisor(paths, n)?;
        let s = fs::read_to_string(format!("/proc/{p}/stat")).ok()?;
        let (_, tail) = s.rsplit_once(") ")?;
        if tail.split_whitespace().nth(1)?.parse::<u32>().ok()? != supervisor {
            return None;
        }
    } else if stat(p)?.1 != identity.trim() {
        return None;
    }
    Some(p)
}
fn legacy_supervisor(paths: &Paths, n: &str) -> Option<u32> {
    let p = read(&paths.file(n, "supervisor.pid")).trim().parse().ok()?;
    let command = fs::read(format!("/proc/{p}/cmdline")).ok()?;
    let args: Vec<_> = command.split(|b| *b == 0).collect();
    let service = format!("{n}.service");
    let valid = args.iter().any(|a| {
        matches!(
            a.rsplit(|b| *b == b'/').next(),
            Some(b"systemctl" | b"user-systemctl.sh" | b"user-systemctl-legacy.sh")
        )
    }) && args.contains(&b"--user".as_slice())
        && args
            .iter()
            .any(|a| *a == n.as_bytes() || *a == service.as_bytes());
    valid.then_some(p)
}

#[derive(Default, Debug)]
struct Unit {
    description: String,
    exec: Vec<String>,
    pre: Vec<Vec<String>>,
    success: Vec<i32>,
    environment: BTreeMap<String, String>,
    env_files: Vec<String>,
    working: Option<PathBuf>,
    restart: String,
    delay: f64,
    stop_timeout: f64,
}
fn duration(value: &str) -> Result<f64> {
    let (n, factor) = if let Some(n) = value.strip_suffix("ms") {
        (n, 0.001)
    } else if let Some(n) = value.strip_suffix('s') {
        (n, 1.)
    } else if let Some(n) = value.strip_suffix("min") {
        (n, 60.)
    } else {
        (value, 1.)
    };
    let v: f64 = n.trim().parse()?;
    ensure!(
        v.is_finite() && (0.0..=86400.0).contains(&(v * factor)),
        "Invalid time interval"
    );
    Ok(v * factor)
}
fn unit(path: &Path) -> Result<Unit> {
    let mut u = Unit {
        delay: 1.,
        stop_timeout: 5.,
        success: vec![0],
        ..Unit::default()
    };
    let mut section = "";
    let content = fs::read_to_string(path).context("Unit not found")?;
    for line in content.lines().map(str::trim) {
        if line.starts_with(['#', ';']) || line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            section = line;
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim();
        match (section, k.trim()) {
            ("[Unit]", "Description") => u.description = v.into(),
            ("[Service]", "Type") => ensure!(
                matches!(v, "simple" | "exec"),
                "Unsupported Type={v}; only simple/exec services are supported"
            ),
            ("[Service]", "ExecStart") => {
                ensure!(
                    u.exec.is_empty() || v.is_empty(),
                    "Multiple ExecStart entries are unsupported"
                );
                u.exec = shell_words::split(v)?;
            }
            ("[Service]", "ExecStartPre") => {
                if v.is_empty() {
                    u.pre.clear();
                } else {
                    u.pre.push(shell_words::split(v)?);
                }
            }
            ("[Service]", "SuccessExitStatus") => {
                for code in v.split_whitespace() {
                    u.success.push(code.parse()?);
                }
            }
            ("[Service]", "KillSignal") => ensure!(
                matches!(v, "SIGTERM" | "TERM" | "15"),
                "Only SIGTERM is supported"
            ),
            ("[Service]", "KillMode") => ensure!(
                matches!(v, "control-group" | "mixed"),
                "Unsupported KillMode"
            ),
            ("[Service]", "Environment") => {
                if v.is_empty() {
                    u.environment.clear();
                }
                for assignment in shell_words::split(v)? {
                    if let Some((k, v)) = assignment.split_once('=') {
                        u.environment.insert(k.into(), v.into());
                    } else {
                        bail!("Invalid Environment assignment");
                    }
                }
            }
            ("[Service]", "EnvironmentFile") => u.env_files.extend(shell_words::split(v)?),
            ("[Service]", "WorkingDirectory") => {
                u.working = Some(PathBuf::from(
                    shell_words::split(v)?
                        .first()
                        .context("Empty WorkingDirectory")?,
                ))
            }
            ("[Service]", "Restart") => {
                ensure!(
                    matches!(v, "always" | "on-failure" | "no" | ""),
                    "Unsupported restart policy {v}"
                );
                u.restart = v.into();
            }
            ("[Service]", "RestartSec") => u.delay = duration(v)?,
            ("[Service]", "TimeoutStopSec") => u.stop_timeout = duration(v)?,
            ("[Service]", "ExecStartPost" | "ExecStop" | "User" | "Group") => {
                bail!(
                    "Unsupported unit directive {k}; retain the native service manager for this unit"
                )
            }
            _ => {}
        }
    }
    Ok(u)
}

fn manager_environment(paths: &Paths) -> Result<BTreeMap<String, String>> {
    let mut result: BTreeMap<String, String> = env::vars().collect();
    // New snapshots use NUL-delimited assignments. Old export-p snapshots are
    // parsed as data, never sourced as executable shell input.
    let binary = paths.state.join("environment.nul");
    if binary.is_file() {
        for assignment in fs::read(binary)?.split(|b| *b == 0) {
            let text = String::from_utf8_lossy(assignment);
            if let Some((k, v)) = text.split_once('=') {
                result.insert(k.into(), v.into());
            }
        }
    } else {
        let path = env::var_os("USER_SYSTEMD_ENV_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| paths.state.join("environment"));
        for line in read(&path).lines() {
            let assignment = line
                .strip_prefix("declare -x ")
                .or_else(|| line.strip_prefix("export "));
            if let Some(assignment) = assignment
                && let Ok(parts) = shell_words::split(assignment)
            {
                for p in parts {
                    if let Some((k, v)) = p.split_once('=') {
                        result.insert(k.into(), v.into());
                    }
                }
            }
        }
    }
    Ok(result)
}
fn environment(paths: &Paths, u: &Unit) -> Result<BTreeMap<String, String>> {
    let mut result = manager_environment(paths)?;
    for file in &u.env_files {
        let optional = file.starts_with('-');
        let path = file.trim_start_matches('-');
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(_) if optional => continue,
            Err(error) => return Err(error.into()),
        };
        for line in text
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let (k, v) = line
                .split_once('=')
                .context("Invalid EnvironmentFile assignment")?;
            let tokens = shell_words::split(v)?;
            result.insert(k.trim().into(), tokens.join(" "));
        }
    }
    result.extend(u.environment.clone());
    Ok(result)
}
fn scan(paths: &Paths) -> Result<()> {
    let lock_path = paths.scan.parent().unwrap().join("scanner.lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.lock_exclusive()?;
    let control = paths.scan.join(".s6-svscan/control");
    if !control.exists()
        || !Command::new("s6-svscanctl")
            .arg("-a")
            .arg(&paths.scan)
            .status()?
            .success()
    {
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(paths.logs.join("s6-svscan.log"))?;
        Command::new("s6-svscan")
            .arg(&paths.scan)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .process_group(0)
            .spawn()?;
        wait_for(|| control.exists(), Duration::from_secs(3))?;
    }
    prune(paths);
    Ok(())
}
/// A unit file removed by hand leaves its supervisor behind, still listed as a stopped
/// service. Only a stopped one is removed: a running process keeps its supervisor.
fn prune(paths: &Paths) {
    let Ok(entries) = fs::read_dir(&paths.scan) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.')
            || !entry.path().is_dir()
            || paths.unit(&name).is_file()
            || pid(paths, &name).is_some()
        {
            continue;
        }
        // -d keeps it down, -x makes the supervisor exit once it is down.
        let _ = Command::new("s6-svc").arg("-dx").arg(entry.path()).status();
        let _ = wait_for(
            || !entry.path().join("supervise/control").exists(),
            Duration::from_secs(3),
        );
        let _ = fs::remove_dir_all(entry.path());
        for extension in ["pid", "lock", "exit.json", "stop", "supervisor.pid"] {
            remove(&paths.file(&name, extension));
        }
    }
}
fn wait_for(mut predicate: impl FnMut() -> bool, timeout: Duration) -> Result<()> {
    let end = Instant::now() + timeout;
    while !predicate() {
        ensure!(Instant::now() < end, "Service transition timed out");
        thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}
fn prepare(paths: &Paths, n: &str) -> Result<()> {
    let directory = paths.service(n);
    fs::create_dir_all(&directory)?;
    // down prevents a newly discovered service from starting until start is requested.
    if !directory.join("run").exists() {
        write(&directory.join("down"), "", 0o600)?;
    }
    let binary = shell_words::quote(
        env::current_exe()?
            .to_str()
            .context("Invalid executable path")?,
    )
    .into_owned();
    let binary = if env::current_exe()?
        .file_stem()
        .is_some_and(|n| n == "cranibox")
    {
        format!("{binary} service")
    } else {
        binary
    };
    let quoted = shell_words::quote(n);
    write(
        &directory.join("run"),
        format!("#!/bin/sh\nexec {binary} __run {quoted}\n"),
        0o700,
    )?;
    write(
        &directory.join("finish"),
        format!("#!/bin/sh\nexec {binary} __finish {quoted} \"$@\"\n"),
        0o700,
    )?;
    scan(paths)?;
    tool("s6-svscanctl", &["-a"], Some(&paths.scan))?;
    wait_for(
        || directory.join("supervise/control").exists(),
        Duration::from_secs(3),
    )
}
fn start(paths: &Paths, n: &str) -> Result<()> {
    let u = unit(&paths.unit(n))?;
    ensure!(!u.exec.is_empty(), "No ExecStart in unit");
    if let Some(p) = pid(paths, n) {
        eprintln!("{n}.service is already running (PID {p})");
        return Ok(());
    }
    prepare(paths, n)?;
    remove(&paths.file(n, "stop"));
    remove(&paths.file(n, "exit.json"));
    remove(&paths.service(n).join("down"));
    tool("s6-svc", &["-u"], Some(&paths.service(n)))?;
    // s6 reports the initial exec transition through our PID record. A command
    // that exits immediately may finish before start returns; that is legitimate.
    wait_for(
        || paths.file(n, "pid").exists() || paths.file(n, "exit.json").exists(),
        Duration::from_secs(5),
    )?;
    eprintln!("Started {n}.service (s6)");
    Ok(())
}
fn children(p: u32) -> Vec<u32> {
    let mut result = Vec::new();
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            if let Ok(child) = entry.file_name().to_string_lossy().parse::<u32>() {
                let s = read(&entry.path().join("stat"));
                if let Some((_, tail)) = s.rsplit_once(") ")
                    && tail
                        .split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse::<u32>().ok())
                        == Some(p)
                {
                    result.push(child);
                }
            }
        }
    }
    result
}
fn tree(p: u32) -> Vec<(u32, String)> {
    let mut result = Vec::new();
    for child in children(p) {
        result.extend(tree(child));
    }
    if let Some((_, identity)) = stat(p) {
        result.push((p, identity));
    }
    result
}
fn signal(targets: &[(u32, String)], sig: i32) {
    for (p, identity) in targets {
        if *p > 1 && stat(*p).is_some_and(|(_, s)| s == *identity) {
            unsafe {
                libc::kill(*p as i32, sig);
            }
        }
    }
}
fn stop(paths: &Paths, n: &str) -> Result<()> {
    write(&paths.file(n, "stop"), "", 0o600)?;
    let timeout = unit(&paths.unit(n)).map(|u| u.stop_timeout).unwrap_or(5.);
    let managed = paths.service(n).join("supervise/control").exists();
    let targets = pid(paths, n).map(tree).unwrap_or_default();
    // A failed native migration can have an s6 directory while the service
    // has been restored under the legacy supervisor. Stop both identities.
    let legacy = legacy_supervisor(paths, n).map(|p| (p, tree(p)));
    if managed {
        write(&paths.service(n).join("down"), "", 0o600)?;
        tool("s6-svc", &["-d"], Some(&paths.service(n)))?;
    }
    signal(&targets, libc::SIGTERM);
    let end = Instant::now() + Duration::from_secs_f64(timeout);
    while targets.iter().any(|(p, _)| alive(*p)) && Instant::now() < end {
        thread::sleep(Duration::from_millis(50));
    }
    signal(&targets, libc::SIGKILL);
    if managed {
        tool(
            "s6-svc",
            &["-wD", "-T", "4000", "-d"],
            Some(&paths.service(n)),
        )?;
    }
    if let Some((p, targets)) = legacy {
        // Compatibility with a service started by the old Bash manager. Stop
        // marker is set before stopping its child, so it cannot restart it.
        let end = Instant::now() + Duration::from_secs(3);
        while alive(p) && Instant::now() < end {
            thread::sleep(Duration::from_millis(50));
        }
        if alive(p) {
            signal(&targets, libc::SIGKILL);
        }
    }
    for extension in ["pid", "starttime", "supervisor.pid", "stop"] {
        remove(&paths.file(n, extension));
    }
    eprintln!("Stopped {n}.service");
    Ok(())
}
fn run(paths: &Paths, n: &str) -> Result<()> {
    let u = unit(&paths.unit(n))?;
    ensure!(!u.exec.is_empty(), "No ExecStart");
    let env = environment(paths, &u)?;
    let mut argv = u.exec;
    // s6 already gives the service its own process group. A bare setsid
    // wrapper would fork when this exec is a group leader, leaving s6
    // tracking an exited launcher instead of the actual service.
    if matches!(
        argv.first().map(String::as_str),
        Some("setsid" | "/usr/bin/setsid" | "/bin/setsid")
    ) && argv.get(1).is_some_and(|arg| !arg.starts_with('-'))
    {
        argv.remove(0);
    }
    if matches!(
        argv.first().map(String::as_str),
        Some("env" | "/usr/bin/env")
    ) && argv.get(1).is_some_and(|v| v == "-i")
    {
        let keys = std::env::var("USER_SYSTEMD_IMPORT_ENV").unwrap_or_else(|_| IMPORT_KEYS.into());
        let assignments: Vec<_> = argv
            .iter()
            .skip(2)
            .take_while(|v| v.contains('='))
            .filter_map(|v| v.split_once('=').map(|(k, _)| k.to_owned()))
            .collect();
        let imported: Vec<_> = keys
            .split_whitespace()
            .filter(|k| !assignments.iter().any(|v| v == k))
            .filter_map(|k| env.get(k).map(|v| format!("{k}={v}")))
            .collect();
        argv.splice(2..2, imported);
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.logs.join(format!("{n}.log")))?;
    let mut c = Command::new(&argv[0]);
    c.args(&argv[1..])
        .env_clear()
        .envs(&env)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log.try_clone()?);
    if let Some(p) = &u.working {
        c.current_dir(p);
    } else {
        c.current_dir(&paths.root);
    }
    let p = std::process::id();
    write(&paths.file(n, "pid"), format!("{p}\n"), 0o600)?;
    if let Some((_, identity)) = stat(p) {
        write(&paths.file(n, "starttime"), identity, 0o600)?;
    }
    write(
        &paths.file(n, "supervisor.pid"),
        format!("{}\n", unsafe { libc::getppid() }),
        0o600,
    )?;
    remove(&paths.file(n, "exit.json"));
    for argv in u.pre {
        let mut pre = Command::new(&argv[0]);
        pre.args(&argv[1..])
            .env_clear()
            .envs(&env)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log.try_clone()?)
            .current_dir(u.working.as_ref().unwrap_or(&paths.root));
        ensure!(pre.status()?.success(), "ExecStartPre failed");
    }
    // All shell/Rust launch helpers disappear after this exec.
    Err(c.exec().into())
}
fn finish(paths: &Paths, n: &str, args: &[String]) -> Result<()> {
    let code = args
        .first()
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(256);
    let sig = args.get(1).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
    write(
        &paths.file(n, "exit.json"),
        json!({"code":code,"signal":sig}).to_string(),
        0o600,
    )?;
    remove(&paths.file(n, "pid"));
    remove(&paths.file(n, "starttime"));
    let u = unit(&paths.unit(n));
    let restart = u.as_ref().is_ok_and(|u| {
        let successful = if sig == 0 {
            u.success.contains(&code)
        } else {
            u.success.contains(&(128 + sig))
        };
        u.restart == "always" || (u.restart == "on-failure" && !successful)
    });
    if !restart || paths.file(n, "stop").exists() || paths.service(n).join("down").exists() {
        write(&paths.service(n).join("down"), "", 0o600)?;
        tool("s6-svc", &["-d"], Some(&paths.service(n)))?;
        return Ok(());
    }
    let end = Instant::now() + Duration::from_secs_f64(u?.delay);
    while Instant::now() < end {
        if paths.file(n, "stop").exists() || paths.service(n).join("down").exists() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

fn executable(program: &str) -> bool {
    let check = |path: PathBuf| {
        path.is_file() && fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        return check(PathBuf::from(program));
    }
    env::split_paths(&env::var_os("PATH").unwrap_or_default()).any(|p| check(p.join(program)))
}
fn migration_plan(paths: &Paths, exclusions: &[String]) -> Result<serde_json::Value> {
    let mut services = Vec::new();
    if paths.units.exists() {
        for entry in fs::read_dir(&paths.units)? {
            let entry = entry?;
            if entry.path().extension().is_none_or(|e| e != "service") {
                continue;
            }
            let n = name(&entry.file_name().to_string_lossy())?;
            let old = legacy_supervisor(paths, &n).filter(|p| alive(*p));
            let candidate = old.is_some() && !exclusions.contains(&n);
            let preflight = unit(&entry.path()).and_then(|u| {
                let program = u.exec.first().context("No ExecStart in unit")?;
                ensure!(executable(program), "ExecStart executable unavailable");
                if let Some(dir) = u.working {
                    ensure!(dir.is_dir(), "WorkingDirectory unavailable");
                }
                Ok(())
            });
            services.push(json!({"name":n,"enabled":paths.wants.join(entry.file_name()).is_symlink(),
                "active":pid(paths,&n).is_some(),"legacy_supervisor":old,"supervisor_identity":old.and_then(stat).map(|s|s.1),
                "candidate":candidate,"excluded":exclusions.contains(&n),
                "supported":preflight.is_ok(),"error":preflight.err().map(|e|e.to_string())}));
        }
    }
    services.sort_by_key(|v| v["name"].as_str().unwrap_or_default().to_owned());
    let tools = ["s6-svscan", "s6-svscanctl", "s6-svc", "s6-svstat"];
    let missing: Vec<_> = tools.into_iter().filter(|p| !executable(p)).collect();
    let ready = missing.is_empty()
        && services
            .iter()
            .all(|v| v["candidate"] != true || v["supported"] == true);
    Ok(json!({"root":paths.root,"ready":ready,"missing_tools":missing,"services":services}))
}
fn migration(args: &[String]) -> Result<i32> {
    let apply = args.first().is_some_and(|s| s == "migrate") && args.iter().any(|s| s == "--apply");
    let mut root = PathBuf::from(env::var_os("HOME").context("HOME is required")?);
    let mut exclusions = Vec::new();
    let mut rollback = None;
    for a in args.iter().skip(1) {
        if let Some(value) = a.strip_prefix("--config-root=") {
            root = PathBuf::from(value);
        } else if let Some(value) = a.strip_prefix("--exclude=") {
            exclusions = value.split(',').map(name).collect::<Result<_>>()?;
        } else if let Some(value) = a.strip_prefix("--rollback-manager=") {
            let path = PathBuf::from(value);
            ensure!(
                path.is_absolute() && path.is_file(),
                "rollback manager must be an existing absolute path"
            );
            rollback = Some(path);
        } else {
            ensure!(a == "--apply", "Unsupported migration option: {a}");
        }
    }
    let paths = Paths::at(root);
    let plan = migration_plan(&paths, &exclusions)?;
    if !apply {
        println!("{}", serde_json::to_string_pretty(&plan)?);
        return Ok(if plan["ready"] == true { 0 } else { 1 });
    }
    ensure!(
        paths.root == env::var_os("HOME").unwrap(),
        "apply root must match HOME used by service supervisors"
    );
    ensure!(
        plan["ready"] == true,
        "Migration preflight failed; run migration-plan and resolve unsupported units first"
    );
    paths.initialize()?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(paths.state.join("migration.lock"))?;
    lock.lock_exclusive()?;
    let mut report = json!({"plan":plan,"results":[],"completed":false});
    let report_path = paths.state.join("migration.json");
    write(&report_path, serde_json::to_vec_pretty(&report)?, 0o600)?;
    for service in plan["services"].as_array().unwrap() {
        if service["candidate"] != true {
            continue;
        }
        let n = service["name"].as_str().unwrap();
        let unit_lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(paths.file(n, "lock"))?;
        unit_lock.lock_exclusive()?;
        let result = (|| -> Result<()> {
            let old = legacy_supervisor(&paths, n)
                .context("Legacy supervisor changed after preflight")?;
            ensure!(
                Some(old as u64) == service["legacy_supervisor"].as_u64()
                    && stat(old).is_some_and(
                        |s| Some(s.1.as_str()) == service["supervisor_identity"].as_str()
                    ),
                "Legacy supervisor identity changed after preflight"
            );
            unit(&paths.unit(n))?;
            stop(&paths, n)?;
            start(&paths, n)?;
            let p = pid(&paths, n).context("Service did not remain active")?;
            let supervisor: u32 = read(&paths.file(n, "supervisor.pid")).trim().parse()?;
            ensure!(
                read(Path::new(&format!("/proc/{supervisor}/comm"))).trim() == "s6-supervise",
                "s6 supervisor verification failed"
            );
            thread::sleep(Duration::from_millis(250));
            ensure!(
                pid(&paths, n) == Some(p),
                "Service exited during startup verification"
            );
            Ok(())
        })();
        let mut row = json!({"name":n,"success":result.is_ok(),"error":result.as_ref().err().map(|e|e.to_string())});
        if result.is_err()
            && let Some(manager) = &rollback
        {
            let stopped = stop(&paths, n).is_ok();
            let started = stopped
                && Command::new("bash")
                    .arg(manager)
                    .args(["--user", "start", n])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success());
            row["rollback_started"] = json!(started);
            let restored = pid(&paths, n);
            thread::sleep(Duration::from_millis(250));
            row["rollback_verified"] = json!(
                started
                    && restored.is_some()
                    && pid(&paths, n) == restored
                    && legacy_supervisor(&paths, n).is_some_and(alive)
            );
        }
        report["results"].as_array_mut().unwrap().push(row);
        write(&report_path, serde_json::to_vec_pretty(&report)?, 0o600)?;
        if let Err(error) = result {
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Err(error.context("Migration stopped; remaining services were not restarted"));
        }
    }
    report["completed"] = json!(true);
    write(&report_path, serde_json::to_vec_pretty(&report)?, 0o600)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(0)
}

pub fn entry(original: Vec<String>) -> Result<i32> {
    if original.first().is_some_and(|s| s == "--version") {
        println!("workspace-services {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    if !original.iter().any(|s| s == "--user")
        && !original.first().is_some_and(|s| s.starts_with("__"))
    {
        let real = env::var_os("WORKSPACE_REAL_SYSTEMCTL")
            .unwrap_or_else(|| "/usr/bin/systemctl.py".into());
        return Err(Command::new(real).args(original).exec().into());
    }
    let args: Vec<_> = original
        .iter()
        .filter(|a| a.as_str() != "--user")
        .cloned()
        .collect();
    if args
        .first()
        .is_some_and(|s| matches!(s.as_str(), "migration-plan" | "migrate"))
    {
        return migration(&args);
    }
    let mut positional = Vec::new();
    let mut properties = Vec::new();
    let mut now = false;
    let mut index = 0;
    while index < args.len() {
        let a = &args[index];
        if a == "--now" {
            now = true;
        } else if a == "-p" || a == "--property" {
            index += 1;
            properties.extend(
                args.get(index)
                    .context("Missing property")?
                    .split(',')
                    .map(str::to_owned),
            );
        } else if let Some(p) = a
            .strip_prefix("--property=")
            .or_else(|| a.strip_prefix("-p"))
        {
            properties.extend(p.split(',').map(str::to_owned));
        } else if !a.starts_with('-') {
            positional.push(a.clone());
        }
        index += 1;
    }
    let action = positional
        .first()
        .context("An action is required")?
        .as_str();
    let paths = Paths::new()?;
    if action == "__snapshot-env" {
        let mut bytes = Vec::new();
        for (k, v) in env::vars() {
            bytes.extend(format!("{k}={v}\0").as_bytes());
        }
        write(&paths.state.join("environment.nul"), bytes, 0o600)?;
        return Ok(0);
    }
    if action == "daemon-reload" {
        // Units are read on use; a reload only has to drop supervisors of removed units.
        if paths.scan.join(".s6-svscan/control").exists() {
            prune(&paths);
        }
        return Ok(0);
    }
    if action == "list-unit-files" {
        println!("UNIT FILE                          STATE");
        let mut entries: Vec<_> = fs::read_dir(&paths.units)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "service"))
            .collect();
        entries.sort();
        for p in entries {
            let file = p.file_name().unwrap().to_string_lossy();
            let enabled = paths.wants.join(file.as_ref()).is_symlink();
            println!("{file}    {}", if enabled { "enabled" } else { "disabled" });
        }
        return Ok(0);
    }
    if action == "status" && positional.len() == 1 {
        println!("● user-services - s6 compatibility manager");
        return Ok(0);
    }
    let n = name(positional.get(1).context("A unit is required")?)?;
    if action == "__run" {
        run(&paths, &n)?;
        return Ok(0);
    }
    if action == "__finish" {
        finish(&paths, &n, &positional[2..])?;
        return Ok(0);
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(paths.file(&n, "lock"))?;
    if matches!(action, "start" | "stop" | "restart" | "enable" | "disable") {
        lock.lock_exclusive()?;
    }
    match action {
        "start" => start(&paths, &n)?,
        "stop" => stop(&paths, &n)?,
        "restart" => {
            stop(&paths, &n)?;
            start(&paths, &n)?;
        }
        "enable" => {
            let path = paths.unit(&n);
            ensure!(path.is_file(), "Unit not found");
            let link = paths.wants.join(format!("{n}.service"));
            remove(&link);
            symlink(path, link)?;
            if now {
                start(&paths, &n)?;
            }
            println!("Enabled {n}.service");
        }
        "disable" => {
            remove(&paths.wants.join(format!("{n}.service")));
            if now {
                stop(&paths, &n)?;
            }
            println!("Disabled {n}.service");
        }
        "is-enabled" => {
            let enabled = paths.wants.join(format!("{n}.service")).is_symlink();
            println!("{}", if enabled { "enabled" } else { "disabled" });
            return Ok(if enabled { 0 } else { 1 });
        }
        "is-active" => {
            let active = pid(&paths, &n).is_some();
            println!("{}", if active { "active" } else { "inactive" });
            return Ok(if active { 0 } else { 3 });
        }
        "status" => {
            let active = pid(&paths, &n);
            println!(
                "● {n}.service - {}",
                unit(&paths.unit(&n))
                    .map(|u| u.description)
                    .unwrap_or_default()
            );
            println!(
                "     Active: {}",
                if active.is_some() {
                    "active (running)"
                } else {
                    "inactive (dead)"
                }
            );
            if let Some(p) = active {
                println!("     PID: {p}");
            }
            return Ok(if active.is_some() { 0 } else { 3 });
        }
        "show" => {
            let active = pid(&paths, &n);
            let path = paths.unit(&n);
            let exists = path.is_file();
            let exit: serde_json::Value =
                serde_json::from_str(&read(&paths.file(&n, "exit.json"))).unwrap_or_default();
            if properties.is_empty() {
                properties = [
                    "ActiveState",
                    "SubState",
                    "MainPID",
                    "LoadState",
                    "FragmentPath",
                    "NeedDaemonReload",
                ]
                .iter()
                .map(|v| v.to_string())
                .collect();
            }
            for property in properties {
                let value = match property.as_str() {
                    "ActiveState" => if active.is_some() {
                        "active"
                    } else {
                        "inactive"
                    }
                    .into(),
                    "SubState" => if active.is_some() { "running" } else { "dead" }.into(),
                    "MainPID" => active.unwrap_or(0).to_string(),
                    "LoadState" => if exists { "loaded" } else { "not-found" }.into(),
                    "FragmentPath" => {
                        if exists {
                            path.display().to_string()
                        } else {
                            String::new()
                        }
                    }
                    "NeedDaemonReload" => "no".into(),
                    "ExecMainStatus" => exit["code"].as_i64().unwrap_or(0).to_string(),
                    "ExecMainCode" => if exit["signal"].as_i64().unwrap_or(0) != 0 {
                        "2"
                    } else {
                        "1"
                    }
                    .into(),
                    _ => String::new(),
                };
                println!("{property}={value}");
            }
        }
        _ => bail!("Unsupported action: {action}"),
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn time_units() {
        assert_eq!(duration("100ms").unwrap(), 0.1);
        assert_eq!(duration("2min").unwrap(), 120.);
        assert!(duration("NaN").is_err());
    }
    #[test]
    fn reject_paths() {
        for value in ["../a", "--help", "a/b", ""] {
            assert!(name(value).is_err());
        }
    }
    #[test]
    fn unit_quotes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unit");
        fs::write(&path,"[Service]\nExecStart=/bin/echo 'a b'\nEnvironment=\"A=a b\" B=c\nRestart=on-failure\nRestartSec=200ms\n").unwrap();
        let u = unit(&path).unwrap();
        assert_eq!(u.exec, ["/bin/echo", "a b"]);
        assert_eq!(u.environment["A"], "a b");
        assert_eq!(u.delay, 0.2);
    }
}
