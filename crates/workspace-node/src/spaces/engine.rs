use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Docker,
    Colima,
    Orbstack,
    Wsl,
}
impl Provider {
    pub fn validate_platform(self) -> Result<()> {
        ensure!(
            match self {
                Self::Docker => true,
                Self::Colima | Self::Orbstack => cfg!(target_os = "macos"),
                Self::Wsl => cfg!(target_os = "windows"),
            },
            "container provider is unavailable on this platform"
        );
        Ok(())
    }
    pub(super) fn runner(self) -> Result<Runner> {
        self.validate_platform()?;
        Ok(match self {
            Self::Docker => Runner {
                program: binary("docker"),
                prefix: vec![],
            },
            Self::Colima => Runner {
                program: binary("docker"),
                prefix: vec!["--context".into(), "colima".into()],
            },
            Self::Orbstack => Runner {
                program: binary("docker"),
                prefix: vec!["--context".into(), "orbstack".into()],
            },
            Self::Wsl => Runner {
                program: "wsl.exe".into(),
                prefix: ["--distribution", "Ubuntu", "--", "docker"]
                    .map(str::to_owned)
                    .into(),
            },
        })
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum SetupAction {
    ColimaInstall,
    ColimaStart,
    OrbstackInstall,
    OrbstackStart,
    WslInstall,
    WslDocker,
    LinuxDocker,
}

pub(super) struct Runner {
    program: String,
    prefix: Vec<String>,
}
pub(super) struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}
/// A container manager reaches children on its existing private Docker network.
/// A native manager reaches the engine's published loopback ports.
pub(super) struct Access {
    pub network: Option<String>,
}
impl Runner {
    pub async fn access(&self, provider: Provider) -> Result<Access> {
        if provider == Provider::Wsl {
            return Ok(Access { network: None });
        }
        let endpoint = match std::env::var("DOCKER_HOST") {
            Ok(value) if provider == Provider::Docker && !value.is_empty() => value,
            _ => {
                let output = self
                    .run(
                        &[
                            "context".into(),
                            "inspect".into(),
                            "--format".into(),
                            "{{json .Endpoints.docker.Host}}".into(),
                        ],
                        None,
                        10,
                    )
                    .await?;
                serde_json::from_str::<String>(output.trim())
                    .context("invalid Docker context endpoint")?
            }
        };
        ensure!(
            endpoint.starts_with("unix://") || endpoint.starts_with("npipe://"),
            "Select a local Docker engine. Remote Docker contexts are not supported for virtual spaces."
        );
        if !PathBuf::from("/.dockerenv").exists() {
            return Ok(Access { network: None });
        }
        let hostname = std::env::var("HOSTNAME")
            .context("container hostname unavailable; use the desktop client on the engine host")?;
        ensure!(
            !hostname.is_empty()
                && hostname
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
            "invalid container hostname"
        );
        let output = self.run(&["container".into(),"inspect".into(),"--format".into(),"{{json .}}".into(),hostname.clone()],None,10).await
            .context("Cannot identify this manager on the selected Docker engine. Use the desktop client on that engine's host.")?;
        let container: Value = serde_json::from_str(output.trim())?;
        ensure!(
            container["Config"]["Hostname"] == hostname && container["State"]["Running"] == true,
            "Docker engine does not contain this running manager"
        );
        let network = container["NetworkSettings"]["Networks"]
            .as_object()
            .context("manager networks unavailable")?
            .keys()
            .find(|name| name.as_str() != "host" && name.as_str() != "none")
            .cloned()
            .context("Manager needs a Docker bridge network to reach virtual spaces")?;
        Ok(Access {
            network: Some(network),
        })
    }
    pub async fn output(
        &self,
        args: &[String],
        input: Option<&[u8]>,
        seconds: u64,
    ) -> Result<Output> {
        let mut command = Command::new(&self.program);
        command
            .args(&self.prefix)
            .args(args)
            .env("LC_ALL", "C")
            .kill_on_drop(true)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("start {}", self.program))?;
        let stdout = child.stdout.take().context("command stdout unavailable")?;
        let stderr = child.stderr.take().context("command stderr unavailable")?;
        let stdin = child.stdin.take();
        tokio::time::timeout(Duration::from_secs(seconds), async {
            let write = async {
                if let Some(mut stdin) = stdin {
                    stdin.write_all(input.unwrap_or_default()).await?;
                    stdin.shutdown().await?;
                }
                Ok::<_, std::io::Error>(())
            };
            let (stdout, stderr, status, write) =
                tokio::join!(capture(stdout), capture(stderr), child.wait(), write);
            let status = status?;
            let stdout = stdout?;
            let stderr = stderr?;
            if status.success() {
                write?;
            }
            Ok(Output {
                success: status.success(),
                stdout,
                stderr,
            })
        })
        .await
        .context("container command timed out; retry to reconcile its current state")?
    }
    pub async fn run(&self, args: &[String], input: Option<&[u8]>, seconds: u64) -> Result<String> {
        let output = self.output(args, input, seconds).await?;
        ensure!(
            output.success,
            "{}: {}",
            args.first().map_or("command", String::as_str),
            output.stderr.trim()
        );
        Ok(output.stdout)
    }
}
async fn capture(mut reader: impl AsyncRead + Unpin) -> std::io::Result<String> {
    let mut tail = std::collections::VecDeque::with_capacity(65536);
    let mut buffer = [0; 4096];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        for byte in &buffer[..count] {
            if tail.len() == 65536 {
                tail.pop_front();
            }
            tail.push_back(*byte);
        }
    }
    Ok(String::from_utf8_lossy(&tail.into_iter().collect::<Vec<_>>()).into_owned())
}
fn binary(name: &str) -> String {
    // Finder-launched Tauri apps do not inherit a shell's Homebrew PATH.
    if cfg!(target_os = "macos") {
        for root in ["/opt/homebrew/bin", "/usr/local/bin"] {
            let path = PathBuf::from(root).join(name);
            if path.is_file() {
                return path.to_string_lossy().into();
            }
        }
    }
    name.into()
}
pub async fn preflight(provider: Provider) -> Result<Value> {
    let runner = provider.runner()?;
    let result = runner
        .run(
            &["info".into(), "--format".into(), "{{json .}}".into()],
            None,
            10,
        )
        .await;
    let (ready, detail, engine) = match result {
        Ok(output) => {
            let info: Value =
                serde_json::from_str(output.trim()).context("invalid Docker info response")?;
            let engine = json!({"name":info["Name"],"version":info["ServerVersion"],"os":info["OperatingSystem"],"architecture":info["Architecture"]});
            if info["OSType"] != "linux" {
                (false, "Switch Docker to Linux containers.".into(), engine)
            } else {
                match runner.access(provider).await {
                    Ok(access) => (
                        true,
                        access.network.map_or(String::new(), |network| {
                            format!("Spaces will join this manager's Docker network: {network}")
                        }),
                        engine,
                    ),
                    Err(error) => (false, format!("{error:#}"), engine),
                }
            }
        }
        Err(error) => (false, format!("{error:#}"), Value::Null),
    };
    let actions: Vec<SetupAction> = match provider {
        Provider::Docker if cfg!(target_os = "linux") && !PathBuf::from("/.dockerenv").exists() => {
            vec![SetupAction::LinuxDocker]
        }
        Provider::Docker => vec![],
        Provider::Colima => vec![SetupAction::ColimaInstall, SetupAction::ColimaStart],
        Provider::Orbstack => vec![SetupAction::OrbstackInstall, SetupAction::OrbstackStart],
        Provider::Wsl => vec![SetupAction::WslInstall, SetupAction::WslDocker],
    };
    Ok(
        json!({"provider":provider,"ready":ready,"detail":detail,"engine":engine,"actions":actions,
        "platform":std::env::consts::OS,"containerized":PathBuf::from("/.dockerenv").exists(),
        "notice":"Containers run on the selected local Docker engine. A container manager joins spaces to its existing Docker network. Published desktop/editor ports use the engine host's loopback address."}),
    )
}
const DOCKER_INSTALL: &str = include_str!("install-docker.sh");
pub fn plan(action: SetupAction) -> Result<Value> {
    ensure!(
        !matches!(action, SetupAction::LinuxDocker) || !PathBuf::from("/.dockerenv").exists(),
        "Install Docker on the host through the desktop client. A container cannot install its host's dependencies."
    );
    let (platform, summary, commands, administrator, restart) = match action {
        SetupAction::ColimaInstall => (
            "macos",
            "Install the open-source Colima runtime and Docker CLI with Homebrew.",
            "brew install colima docker",
            false,
            false,
        ),
        SetupAction::ColimaStart => (
            "macos",
            "Start the Colima Linux virtual machine.",
            "colima start --cpu 2 --memory 4 --disk 60",
            false,
            false,
        ),
        SetupAction::OrbstackInstall => (
            "macos",
            "Install OrbStack and the Docker CLI with Homebrew. OrbStack has its own license terms.",
            "brew install --cask orbstack\nbrew install docker",
            false,
            false,
        ),
        SetupAction::OrbstackStart => (
            "macos",
            "Open OrbStack; finish its first-run setup if requested.",
            "open -a OrbStack",
            false,
            false,
        ),
        SetupAction::WslInstall => (
            "windows",
            "Install WSL2 and Ubuntu using Windows administrator approval. Windows may require a restart. Existing distributions are preserved.",
            "wsl.exe --install --distribution Ubuntu --no-launch",
            true,
            true,
        ),
        SetupAction::WslDocker => (
            "windows",
            "Install Docker Engine inside the Ubuntu WSL distribution. Finish Ubuntu's initial user setup first.",
            DOCKER_INSTALL,
            true,
            false,
        ),
        SetupAction::LinuxDocker => (
            "linux",
            "Install Docker Engine from Docker's official apt repository on Ubuntu or Debian. Uses the system authentication dialog and adds your account to the docker group; sign out and back in afterwards.",
            DOCKER_INSTALL,
            true,
            false,
        ),
    };
    ensure!(
        std::env::consts::OS == platform,
        "setup action is unavailable on this platform"
    );
    Ok(
        json!({"action":action,"summary":summary,"commands":commands,"administrator":administrator,"restart_may_be_required":restart,
        "requires_homebrew":matches!(action, SetupAction::ColimaInstall|SetupAction::OrbstackInstall),
        "source":match action { SetupAction::WslInstall=>"https://learn.microsoft.com/windows/wsl/install", SetupAction::ColimaInstall|SetupAction::ColimaStart=>"https://colima.run/docs/installation/", SetupAction::OrbstackInstall|SetupAction::OrbstackStart=>"https://docs.orbstack.dev/install", _=>"https://docs.docker.com/engine/install/ubuntu/" }}),
    )
}
async fn run_program(program: &str, args: &[&str], seconds: u64) -> Result<()> {
    let mut command = Command::new(binary(program));
    command.args(args).env("LC_ALL", "C");
    monitor_setup(command, Duration::from_secs(seconds)).await
}
async fn monitor_setup(mut command: Command, timeout: Duration) -> Result<()> {
    // System transactions must survive cancellation of their monitor. Null streams
    // also avoid a broken pipe when the node goes away while an installer writes.
    command
        .kill_on_drop(false)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn().context("Start system setup. Check that the runtime installer and system authentication agent are available.")?;
    let status = tokio::time::timeout(timeout, child.wait()).await
        .context("Setup monitoring timed out. The system installer may still be running. Check the system package manager and recheck the environment before retrying.")??;
    ensure!(
        status.success(),
        "Setup was cancelled or failed ({status}). Check the system installer and recheck the environment before preparing another setup action."
    );
    Ok(())
}
pub async fn install(action: SetupAction) -> Result<()> {
    plan(action)?;
    match action {
        SetupAction::ColimaInstall => {
            run_program("brew", &["install", "colima", "docker"], 1800).await?;
        }
        SetupAction::ColimaStart => {
            run_program(
                "colima",
                &["start", "--cpu", "2", "--memory", "4", "--disk", "60"],
                1800,
            )
            .await?;
        }
        SetupAction::OrbstackInstall => {
            run_program("brew", &["install", "--cask", "orbstack"], 1800).await?;
            run_program("brew", &["install", "docker"], 1800).await?;
        }
        SetupAction::OrbstackStart => {
            run_program("open", &["-a", "OrbStack"], 30).await?;
        }
        SetupAction::WslInstall => {
            run_program("powershell.exe", &["-NoProfile","-NonInteractive","-Command", "$p = Start-Process -FilePath wsl.exe -ArgumentList '--install','--distribution','Ubuntu','--no-launch' -Verb RunAs -Wait -PassThru; exit $p.ExitCode"], 1800).await?;
        }
        SetupAction::WslDocker => {
            run_program(
                "wsl.exe",
                &[
                    "--distribution",
                    "Ubuntu",
                    "--user",
                    "root",
                    "--",
                    "sh",
                    "-c",
                    DOCKER_INSTALL,
                    "cranibox-setup",
                ],
                1800,
            )
            .await?;
        }
        SetupAction::LinuxDocker => {
            let user = Runner {
                program: "id".into(),
                prefix: vec![],
            }
            .run(&["-un".into()], None, 5)
            .await?;
            run_program(
                "pkexec",
                &[
                    "/bin/sh",
                    "-c",
                    DOCKER_INSTALL,
                    "cranibox-setup",
                    user.trim(),
                ],
                1800,
            )
            .await?;
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn setup_monitor_does_not_kill_system_transaction_on_timeout_or_shutdown() {
        let root = tempfile::tempdir().unwrap();
        for timeout in [false, true] {
            let ready = root.path().join(format!("ready-{timeout}"));
            let release = root.path().join(format!("release-{timeout}"));
            let done = root.path().join(format!("done-{timeout}"));
            let mut command = Command::new("/bin/sh");
            command.args(["-c", "printf ready > \"$1\"; attempt=0; while [ ! -f \"$2\" ] && [ \"$attempt\" -lt 1000 ]; do sleep 0.01; attempt=$((attempt + 1)); done; [ -f \"$2\" ] || exit 1; printf done > \"$3\"", "fixture"]).arg(&ready).arg(&release).arg(&done);
            let monitor = tokio::spawn(monitor_setup(
                command,
                if timeout {
                    Duration::from_millis(500)
                } else {
                    Duration::from_secs(10)
                },
            ));
            tokio::time::timeout(Duration::from_secs(5), async {
                while !ready.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            if timeout {
                assert!(
                    monitor
                        .await
                        .unwrap()
                        .unwrap_err()
                        .to_string()
                        .contains("may still be running")
                );
            } else {
                monitor.abort();
                assert!(monitor.await.unwrap_err().is_cancelled());
            }
            std::fs::write(&release, b"continue").unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while !done.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exit 7"]);
        let error = monitor_setup(command, Duration::from_secs(5))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("cancelled or failed") && error.contains('7'));
    }
}
