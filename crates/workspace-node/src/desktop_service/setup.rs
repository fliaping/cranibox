//! Explicit package installation, separate from the managed desktop process.
use super::{Provider, executable, install_plan};
use agent_workspace_runtime::config as secrets;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    future::Future,
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{process::Command, task::JoinHandle};

const PLAN_TTL: Duration = Duration::from_secs(600);

struct Plan {
    id: String,
    provider: Provider,
    created: Instant,
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
    provider: Option<Provider>,
    error: Option<String>,
}
pub(super) struct Setup {
    path: PathBuf,
    record: Record,
    plan: Option<Plan>,
    task: Option<JoinHandle<Result<()>>>,
}
impl Setup {
    pub fn new(peers: &Path) -> Self {
        let path = peers.with_file_name("desktop-setup.json");
        let mut record = match std::fs::File::open(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Record::default(),
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
                    provider: None,
                    error: Some("The previous VNC installation record is unreadable. Check your system package manager before reviewing a new plan.".into()),
                }),
        };
        if record.phase == Phase::Running {
            record.phase = Phase::Interrupted;
            record.error = Some("The node restarted during VNC installation. The system installer may still be running. Check your package manager before reviewing a new plan.".into());
        }
        Self {
            path,
            record,
            plan: None,
            task: None,
        }
    }
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    pub fn error(&self) -> Option<&str> {
        self.record.error.as_deref()
    }
    pub fn status(&self) -> Value {
        json!({"phase":self.record.phase,"provider":self.record.provider,"error":self.record.error})
    }
    pub fn review(&mut self, provider: Provider) -> Result<Value> {
        ensure!(!self.running(), "VNC installation is already running.");
        self.plan = None;
        let review = install_plan(provider)?;
        Ok(self.prepare(provider, review))
    }
    fn prepare(&mut self, provider: Provider, mut review: Value) -> Value {
        let id = uuid::Uuid::new_v4().to_string();
        review["id"] = json!(id);
        review["expires_in_seconds"] = json!(PLAN_TTL.as_secs());
        self.plan = Some(Plan {
            id,
            provider,
            created: Instant::now(),
        });
        review
    }
    pub fn start(&mut self, id: &str, provider: Provider) -> Result<()> {
        // Recheck the session, platform and package state before using the reviewed plan.
        install_plan(provider)?;
        self.begin(id, provider, install(provider))
    }
    fn begin(
        &mut self,
        id: &str,
        provider: Provider,
        work: impl Future<Output = Result<()>> + Send + 'static,
    ) -> Result<()> {
        ensure!(!self.running(), "VNC installation is already running.");
        let plan = self
            .plan
            .as_ref()
            .context("Review a new VNC installation plan first.")?;
        ensure!(
            plan.id == id && plan.created.elapsed() < PLAN_TTL && plan.provider == provider,
            "The VNC installation plan changed or expired. Review a new plan."
        );
        let record = Record {
            phase: Phase::Running,
            provider: Some(provider),
            error: None,
        };
        // If the journal cannot be saved, no system command may start.
        secrets::write_private(&self.path, &serde_json::to_vec(&record)?, true)?;
        self.record = record;
        self.plan = None;
        self.task = Some(tokio::spawn(work));
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
            .context("VNC installation monitor stopped")
            .and_then(|result| result);
        self.record.phase = if result.is_ok() {
            Phase::Succeeded
        } else {
            Phase::Failed
        };
        self.record.error = result.err().map(|error| format!("{error:#}"));
        let saved = serde_json::to_vec(&self.record)
            .context("Serialize VNC installation result")
            .and_then(|bytes| secrets::write_private(&self.path, &bytes, true));
        if saved.is_err() {
            self.record.error = Some("VNC installation finished, but its result could not be saved. Recheck the environment and your package manager before retrying.".into());
        }
        true
    }
    pub async fn shutdown(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
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

async fn install(provider: Provider) -> Result<()> {
    let script = format!(
        "apt-get update && apt-get install --yes --no-install-recommends {}",
        provider.package()
    );
    let mut command = Command::new("pkexec");
    command.args(["/bin/sh", "-c", &script]);
    monitor(command, Duration::from_secs(1800)).await?;
    ensure!(
        executable(provider).is_some(),
        "The package command finished but {} is not available on PATH. Restart the client and recheck.",
        provider.package()
    );
    Ok(())
}
async fn monitor(mut command: Command, timeout: Duration) -> Result<()> {
    // An elevated package transaction belongs to the system installer. Closing the
    // node or timing out its monitor must not deliberately terminate that transaction.
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(false)
        .spawn()
        .context("Open the system authentication dialog (pkexec)")?;
    let status = tokio::time::timeout(timeout, child.wait()).await.context(
        "VNC installation monitoring timed out. The system installer may still be running; check your package manager before retrying."
    )??;
    ensure!(
        status.success(),
        "VNC installation was cancelled or failed (exit {}). Check the system authentication dialog and package manager before reviewing a new plan.",
        status
            .code()
            .map_or("unknown".into(), |code| code.to_string())
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_and_bad_journals_never_replay_installation() {
        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        let path = peers.with_file_name("desktop-setup.json");
        assert_eq!(Setup::new(&peers).status()["phase"], "idle");
        for bytes in [
            br#"{"phase":"running","provider":"x11vnc","error":null}"#.to_vec(),
            vec![b'x'; 8193],
            br#"{"phase":"other"}"#.to_vec(),
        ] {
            std::fs::write(&path, bytes).unwrap();
            let setup = Setup::new(&peers);
            assert_eq!(setup.status()["phase"], "interrupted");
            assert!(!setup.running());
            assert!(setup.plan.is_none());
            assert!(setup.error().is_some());
        }
    }

    #[tokio::test]
    async fn only_a_current_single_use_provider_plan_can_start() {
        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        let mut setup = Setup::new(&peers);
        let review = setup.prepare(Provider::X11vnc, json!({}));
        let id = review["id"].as_str().unwrap();
        assert!(!setup.path.exists());
        assert!(
            setup
                .begin("wrong", Provider::X11vnc, async { Ok(()) })
                .is_err()
        );
        assert!(setup.begin(id, Provider::Wayvnc, async { Ok(()) }).is_err());
        setup.plan.as_mut().unwrap().created = Instant::now() - PLAN_TTL;
        assert!(setup.begin(id, Provider::X11vnc, async { Ok(()) }).is_err());
        let replaced = setup.prepare(Provider::X11vnc, json!({}));
        assert!(setup.begin(id, Provider::X11vnc, async { Ok(()) }).is_err());
        let id = replaced["id"].as_str().unwrap();
        setup.begin(id, Provider::X11vnc, async { Ok(()) }).unwrap();
        assert_eq!(setup.status()["phase"], "running");
        assert!(setup.begin(id, Provider::X11vnc, async { Ok(()) }).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                setup.path.metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        finish(&mut setup).await;
        assert_eq!(setup.status()["phase"], "succeeded");
        assert_eq!(Setup::new(&peers).status()["phase"], "succeeded");
        assert!(setup.begin(id, Provider::X11vnc, async { Ok(()) }).is_err());
    }

    async fn finish(setup: &mut Setup) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !setup.reconcile().await {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_failed_journal_prevents_work_and_failed_work_is_persisted() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        let mut setup = Setup::new(&peers);
        let review = setup.prepare(Provider::Wayvnc, json!({}));
        let id = review["id"].as_str().unwrap();
        let ran = Arc::new(AtomicBool::new(false));
        let marker = ran.clone();
        std::fs::create_dir(&setup.path).unwrap();
        assert!(
            setup
                .begin(id, Provider::Wayvnc, async move {
                    marker.store(true, Ordering::SeqCst);
                    Ok(())
                })
                .is_err()
        );
        assert!(!setup.running());
        assert!(!ran.load(Ordering::SeqCst));
        std::fs::remove_dir(&setup.path).unwrap();
        setup
            .begin(id, Provider::Wayvnc, async {
                anyhow::bail!("Installer declined")
            })
            .unwrap();
        finish(&mut setup).await;
        assert_eq!(setup.status()["phase"], "failed");
        assert_eq!(Setup::new(&peers).error(), Some("Installer declined"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_and_timeout_leave_the_system_transaction_running() {
        let root = tempfile::tempdir().unwrap();
        for timeout in [false, true] {
            let mut setup = Setup::new(&root.path().join(if timeout {
                "timeout/peers.json"
            } else {
                "shutdown/peers.json"
            }));
            std::fs::create_dir_all(setup.path.parent().unwrap()).unwrap();
            let ready = root.path().join(if timeout {
                "timeout-ready"
            } else {
                "shutdown-ready"
            });
            let release = root.path().join(if timeout {
                "timeout-release"
            } else {
                "shutdown-release"
            });
            let done = root.path().join(if timeout {
                "timeout-done"
            } else {
                "shutdown-done"
            });
            let mut command = Command::new("/bin/sh");
            command.args(["-c", "printf ready > \"$1\"; attempt=0; while [ ! -f \"$2\" ] && [ \"$attempt\" -lt 1000 ]; do sleep 0.01; attempt=$((attempt + 1)); done; [ -f \"$2\" ] || exit 1; printf done > \"$3\"", "fixture"]).arg(&ready).arg(&release).arg(&done);
            let review = setup.prepare(Provider::X11vnc, json!({}));
            setup
                .begin(
                    review["id"].as_str().unwrap(),
                    Provider::X11vnc,
                    monitor(
                        command,
                        if timeout {
                            Duration::from_millis(150)
                        } else {
                            Duration::from_secs(5)
                        },
                    ),
                )
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while !ready.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            if timeout {
                finish(&mut setup).await;
                assert!(setup.error().unwrap().contains("may still be running"));
            } else {
                setup.shutdown().await;
                let restored = Setup::new(&setup.path.with_file_name("peers.json"));
                assert_eq!(restored.status()["phase"], "interrupted");
                assert!(!restored.running());
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
    }
}
