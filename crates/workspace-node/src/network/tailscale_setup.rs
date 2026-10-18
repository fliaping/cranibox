//! Explicit system setup. Never log in, change tailnet settings, or own the daemon.
use super::tailscale;
use agent_workspace_runtime::config as secrets;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{process::Command, task::JoinHandle};

const SCRIPT: &str = include_str!("install-tailscale.sh");
const MAC_SCRIPT: &str = include_str!("install-tailscale-macos.sh");
const PLAN_TTL: Duration = Duration::from_secs(600);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Invocation {
    program: PathBuf,
    args: Vec<String>,
}
struct Plan {
    id: String,
    created: Instant,
    installed: bool,
    commands: Vec<Invocation>,
    review: Value,
}
#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
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
    action: Option<String>,
}
pub(super) struct Setup {
    path: PathBuf,
    record: Record,
    plan: Option<Plan>,
    task: Option<JoinHandle<Result<()>>>,
}

impl Setup {
    pub fn new(peers: &Path) -> Self {
        let path = peers.with_file_name("tailscale-setup.json");
        let mut record = if path.exists() {
            std::fs::File::open(&path).ok().and_then(|file| { let mut bytes = Vec::new(); file.take(8193).read_to_end(&mut bytes).ok()?; Some(bytes) }).filter(|bytes| bytes.len() <= 8192)
                .and_then(|bytes| serde_json::from_slice::<Record>(&bytes).ok())
                .unwrap_or_else(|| Record { phase: Phase::Interrupted, error: Some("The previous setup record is unreadable. Check the system installer before retrying.".into()), action: None })
        } else {
            Record::default()
        };
        if record.phase == Phase::Running {
            record.phase = Phase::Interrupted;
            record.error = Some("The node restarted while setup was running. The system installer may still be working. Check it before reviewing a new plan.".into());
        }
        Self {
            path,
            record,
            plan: None,
            task: None,
        }
    }
    pub fn status(&self) -> Value {
        json!({"phase":self.record.phase,"error":self.record.error,"action":self.record.action})
    }
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    pub fn review(&mut self, unavailable: bool) -> Result<Value> {
        ensure!(!self.running(), "Tailscale setup is already running.");
        self.plan = None;
        let installed = tailscale::executable().is_some();
        let (commands, action, script) = match commands(installed, unavailable) {
            Ok(value) => value,
            Err(error) => {
                return Ok(
                    json!({"available":false,"platform":std::env::consts::OS,"reason":format!("{error:#}")}),
                );
            }
        };
        Ok(self.prepare(installed, commands, action, script))
    }
    fn prepare(
        &mut self,
        installed: bool,
        commands: Vec<Invocation>,
        action: &str,
        script: Option<&str>,
    ) -> Value {
        let id = uuid::Uuid::new_v4().to_string();
        let command_lines: Vec<_> = commands.iter().map(|c| json!({"program":c.program,"args":c.args.iter().map(|arg| if Some(arg.as_str()) == script { "<bundled installation script>" } else { arg }).collect::<Vec<_>>()})).collect();
        let review = json!({"available":true,"id":id,"platform":std::env::consts::OS,"action":action,"requires_admin":action != "open_app","expires_in_seconds":PLAN_TTL.as_secs(),"commands":command_lines,"script":script,"script_sha256":script.map(|s|format!("{:x}",Sha256::digest(s.as_bytes())))});
        self.plan = Some(Plan {
            id,
            created: Instant::now(),
            installed,
            commands,
            review: review.clone(),
        });
        review
    }
    pub fn start(&mut self, id: &str, unavailable: bool) -> Result<()> {
        ensure!(!self.running(), "Tailscale setup is already running.");
        let plan = self
            .plan
            .as_ref()
            .context("Review a new Tailscale setup plan first.")?;
        ensure!(
            plan.id == id && plan.created.elapsed() < PLAN_TTL,
            "The setup plan is invalid or expired. Review a new plan."
        );
        let installed = tailscale::executable().is_some();
        ensure!(
            plan.installed == installed,
            "Tailscale installation changed. Review a new plan."
        );
        let (current, _, _) = commands(installed, unavailable)?;
        self.begin(id, installed, &current)
    }
    fn begin(&mut self, id: &str, installed: bool, current: &[Invocation]) -> Result<()> {
        ensure!(!self.running(), "Tailscale setup is already running.");
        let plan = self
            .plan
            .as_ref()
            .context("Review a new Tailscale setup plan first.")?;
        ensure!(
            plan.id == id && plan.created.elapsed() < PLAN_TTL,
            "The setup plan is invalid or expired. Review a new plan."
        );
        ensure!(
            plan.installed == installed && current == plan.commands,
            "System setup changed. Review a new plan."
        );
        let record = Record {
            phase: Phase::Running,
            error: None,
            action: plan.review["action"].as_str().map(str::to_owned),
        };
        secrets::write_private(&self.path, &serde_json::to_vec(&record)?, true)?;
        self.record = record;
        let plan = self.plan.take().unwrap();
        self.task = Some(tokio::spawn(run(plan.commands)));
        Ok(())
    }
    pub async fn reconcile(&mut self) -> bool {
        if !self.task.as_ref().is_some_and(|task| task.is_finished()) {
            return false;
        }
        let result = self
            .task
            .take()
            .unwrap()
            .await
            .context("Setup monitor stopped")
            .and_then(|result| result);
        self.record.phase = if result.is_ok() {
            Phase::Succeeded
        } else {
            Phase::Failed
        };
        self.record.error = result.err().map(|error| format!("{error:#}"));
        if self.save().is_err() {
            self.record.error = Some("Setup finished, but its result could not be saved. Refresh the system service status.".into());
        }
        true
    }
    fn save(&self) -> Result<()> {
        secrets::write_private(&self.path, &serde_json::to_vec(&self.record)?, true)
    }
}

fn locate(name: &str, extra: &[PathBuf]) -> Option<PathBuf> {
    extra
        .iter()
        .cloned()
        .chain(std::env::var_os("PATH").into_iter().flat_map(|paths| {
            std::env::split_paths(&paths)
                .map(|p| p.join(name))
                .collect::<Vec<_>>()
        }))
        .find(|p| p.is_file())
}
fn invocation(program: impl Into<PathBuf>, args: &[&str]) -> Invocation {
    Invocation {
        program: program.into(),
        args: args.iter().map(|s| s.to_string()).collect(),
    }
}
fn commands(
    installed: bool,
    unavailable: bool,
) -> Result<(Vec<Invocation>, &'static str, Option<&'static str>)> {
    ensure!(
        !installed || unavailable,
        "The Tailscale service is already available. Sign in or use its system settings."
    );
    match std::env::consts::OS {
        "linux" => {
            ensure!(
                !Path::new("/.dockerenv").exists() && !Path::new("/run/.containerenv").exists(),
                "Install Tailscale on the container host, or configure the Workspace Tailscale service. Container setup does not change the host network."
            );
            ensure!(
                Path::new("/run/systemd/system").is_dir(),
                "Automatic setup requires a Linux host using systemd. Use the official installation guide on this device."
            );
            ensure!(
                std::env::var_os("DISPLAY").is_some()
                    || std::env::var_os("WAYLAND_DISPLAY").is_some(),
                "Open the desktop client on this device to approve administrator access, or install Tailscale from its terminal."
            );
            let pkexec = PathBuf::from("/usr/bin/pkexec");
            ensure!(
                pkexec.is_file(),
                "Install PolicyKit (pkexec) on this device before using automatic setup."
            );
            if installed {
                return Ok((
                    vec![invocation(
                        pkexec,
                        &["/usr/bin/systemctl", "enable", "--now", "tailscaled"],
                    )],
                    "start_service",
                    None,
                ));
            }
            let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
            ensure!(
                release.lines().any(|line| matches!(
                    line.trim(),
                    "ID=ubuntu" | "ID=debian" | "ID=\"ubuntu\"" | "ID=\"debian\""
                )),
                "Automatic Linux installation currently supports Debian and Ubuntu. Use Tailscale's official guide for this distribution."
            );
            Ok((
                vec![invocation(pkexec, &["/bin/sh", "-c", SCRIPT])],
                "install",
                Some(SCRIPT),
            ))
        }
        "macos" => {
            let open = invocation("/usr/bin/open", &["-a", "Tailscale"]);
            if installed {
                ensure!(
                    Path::new("/Applications/Tailscale.app").is_dir(),
                    "This device uses the Tailscale command line service. Start it from its existing system configuration."
                );
                return Ok((vec![open], "open_app", None));
            }
            let brew = locate("brew", &["/opt/homebrew/bin/brew".into(),"/usr/local/bin/brew".into()]).context("Install Tailscale from the official downloads page, or install Homebrew to use guided setup.")?;
            Ok((
                vec![invocation(
                    "/bin/sh",
                    &[
                        "-c",
                        MAC_SCRIPT,
                        "tailscale-setup",
                        brew.to_str().context("Homebrew path is not valid UTF-8.")?,
                    ],
                )],
                "install",
                Some(MAC_SCRIPT),
            ))
        }
        "windows" => {
            if installed {
                let root = std::env::var_os("SystemRoot")
                    .context("Windows system directory is unavailable.")?;
                let program =
                    PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
                return Ok((
                    vec![invocation(
                        program,
                        &[
                            "-NoProfile",
                            "-NonInteractive",
                            "-Command",
                            r#"$p = Start-Process -FilePath "$env:SystemRoot\System32\sc.exe" -ArgumentList @('start','Tailscale') -Verb RunAs -Wait -PassThru; exit $p.ExitCode"#,
                        ],
                    )],
                    "start_service",
                    None,
                ));
            }
            let extra = std::env::var_os("LOCALAPPDATA")
                .map(|p| PathBuf::from(p).join("Microsoft/WindowsApps/winget.exe"))
                .into_iter()
                .collect::<Vec<_>>();
            let winget = locate("winget.exe", &extra).context("Windows Package Manager (winget) is unavailable. Install Tailscale from the official downloads page.")?;
            Ok((
                vec![invocation(
                    winget,
                    &[
                        "install",
                        "--id",
                        "Tailscale.Tailscale",
                        "--exact",
                        "--source",
                        "winget",
                        "--accept-package-agreements",
                        "--accept-source-agreements",
                        "--disable-interactivity",
                    ],
                )],
                "install",
                None,
            ))
        }
        _ => anyhow::bail!("Use the official Tailscale installation guide for this platform."),
    }
}

async fn run(commands: Vec<Invocation>) -> Result<()> {
    for invocation in commands {
        let mut command = Command::new(invocation.program);
        command
            .args(invocation.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Package managers may own an elevated installer. Closing the node must not
        // deliberately terminate a system package transaction. The journal becomes
        // interrupted on restart; it never automatically repeats the installation.
        command.kill_on_drop(false);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command.spawn().context(
            "Start Tailscale setup. Check the system package manager and administrator prompt.",
        )?;
        let status = tokio::time::timeout(Duration::from_secs(1800),child.wait()).await
            .context("Setup monitoring timed out. The system installer may still be running; check it before retrying.")??;
        ensure!(
            status.success(),
            "Tailscale setup did not finish successfully (exit {}). Check the administrator prompt and system package manager before retrying.",
            status
                .code()
                .map_or("unknown".into(), |code| code.to_string())
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_never_replays_setup_and_bounds_unreadable_records() {
        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        let mut setup = Setup::new(&peers);
        assert_eq!(setup.status()["phase"], "idle");
        setup.record.phase = Phase::Running;
        setup.save().unwrap();
        let restored = Setup::new(&peers);
        assert_eq!(restored.status()["phase"], "interrupted");
        assert!(!restored.running());
        assert!(restored.plan.is_none());
        for invalid in [vec![b'x'; 9000], br#"{"phase":"unknown"}"#.to_vec()] {
            std::fs::write(&setup.path, invalid).unwrap();
            let restored = Setup::new(&peers);
            assert_eq!(restored.status()["phase"], "interrupted");
            assert!(
                restored.status()["error"]
                    .as_str()
                    .unwrap()
                    .contains("unreadable")
            );
            assert!(!restored.running());
        }
    }

    #[cfg(unix)]
    fn fixture(marker: &Path) -> Vec<Invocation> {
        vec![invocation(
            "/bin/sh",
            &[
                "-c",
                "printf installed > \"$1\"",
                "setup",
                marker.to_str().unwrap(),
            ],
        )]
    }

    #[cfg(unix)]
    async fn finish(setup: &mut Setup) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !setup.reconcile().await {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn review_is_inert_and_only_a_current_single_use_plan_can_start() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("installed");
        let peers = root.path().join("peers.json");
        let mut setup = Setup::new(&peers);
        let commands = fixture(&marker);
        let review = setup.prepare(false, commands.clone(), "install", None);
        let id = review["id"].as_str().unwrap();
        assert!(!marker.exists());
        assert!(!setup.path.exists());
        assert!(setup.begin("wrong", false, &commands).is_err());
        assert!(setup.begin(id, true, &commands).is_err());
        assert!(setup.begin(id, false, &[]).is_err());
        setup.plan.as_mut().unwrap().created = Instant::now() - PLAN_TTL;
        assert!(setup.begin(id, false, &commands).is_err());
        assert!(!marker.exists());
        let review = setup.prepare(false, commands.clone(), "install", None);
        let id = review["id"].as_str().unwrap();
        setup.begin(id, false, &commands).unwrap();
        assert_eq!(setup.status()["phase"], "running");
        assert_eq!(
            setup.path.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(setup.begin(id, false, &commands).is_err());
        finish(&mut setup).await;
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "installed");
        assert_eq!(setup.status()["phase"], "succeeded");
        assert!(setup.begin(id, false, &commands).is_err());
        assert_eq!(Setup::new(&peers).status()["phase"], "succeeded");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_installer_is_reported_and_later_commands_do_not_run() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("must-not-exist");
        let peers = root.path().join("peers.json");
        let mut setup = Setup::new(&peers);
        let commands = [
            vec![invocation("/bin/sh", &["-c", "exit 7"])],
            fixture(&marker),
        ]
        .concat();
        let review = setup.prepare(false, commands.clone(), "install", None);
        setup
            .begin(review["id"].as_str().unwrap(), false, &commands)
            .unwrap();
        finish(&mut setup).await;
        assert_eq!(setup.status()["phase"], "failed");
        assert!(setup.status()["error"].as_str().unwrap().contains("exit 7"));
        assert!(!marker.exists());
        assert_eq!(Setup::new(&peers).status()["phase"], "failed");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn journal_failure_prevents_any_system_command() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("must-not-exist");
        let mut setup = Setup::new(&root.path().join("peers.json"));
        let commands = fixture(&marker);
        let review = setup.prepare(false, commands.clone(), "install", None);
        std::fs::create_dir(&setup.path).unwrap();
        assert!(
            setup
                .begin(review["id"].as_str().unwrap(), false, &commands)
                .is_err()
        );
        assert!(!setup.running());
        assert!(!marker.exists());
        assert_eq!(setup.status()["phase"], "idle");
    }
}
