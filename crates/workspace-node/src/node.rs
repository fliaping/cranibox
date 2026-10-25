//! Process execution owned by this device: bounded output and contained process trees.
use agent_workspace_runtime::protocol::*;
use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command, sync::mpsc};
use tokio_util::sync::CancellationToken;

async fn read_output<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    id: String,
    stream: Stream,
    tx: mpsc::Sender<Wire>,
    budget: Arc<AtomicUsize>,
    stop: CancellationToken,
) -> Result<()> {
    let mut buffer = [0u8; 4096];
    loop {
        let count = tokio::select! {
            _ = stop.cancelled() => return Ok(()),
            read = reader.read(&mut buffer) => read?,
        };
        if count == 0 {
            return Ok(());
        }
        // Keep the shared output limit atomic without requiring Rust's newer try_update API.
        let mut prior = budget.load(Ordering::Relaxed);
        loop {
            match budget.compare_exchange_weak(
                prior,
                prior.saturating_sub(count),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(left) => prior = left,
            }
        }
        let send = count.min(prior);
        if send > 0 {
            let output = Wire::Output {
                id: id.clone(),
                stream,
                data: STANDARD.encode(&buffer[..send]),
            };
            tokio::select! {
                _ = stop.cancelled() => return Ok(()),
                sent = tx.send(output) => sent.context("output channel closed")?,
            }
        }
    }
}
pub(crate) async fn execute(
    id: String,
    request: ExecRequest,
    cancel: CancellationToken,
    tx: mpsc::Sender<Wire>,
) {
    let result = execute_process(&id, request, cancel, tx.clone()).await;
    let (exit_code, outcome, error) = match result {
        Ok((code, outcome)) => (code, outcome, None),
        Err(error) => (
            None,
            Outcome::Failed,
            Some(error.to_string().chars().take(1000).collect()),
        ),
    };
    let _ = tx
        .send(Wire::Finished {
            id,
            exit_code,
            outcome,
            error,
        })
        .await;
}
async fn execute_process(
    id: &str,
    request: ExecRequest,
    cancel: CancellationToken,
    tx: mpsc::Sender<Wire>,
) -> Result<(Option<i32>, Outcome)> {
    let mut command = Command::new(&request.program);
    command
        .args(&request.args)
        .envs(&request.env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = &request.cwd {
        command.current_dir(cwd);
    }
    #[cfg(unix)]
    command.process_group(0);
    // Assign the Windows process to its Job Object before any application code runs.
    #[cfg(windows)]
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
    let mut child = command.spawn().context("start command")?;
    let tree = ProcessTree::new(&child)?;
    let budget = Arc::new(AtomicUsize::new(MAX_OUTPUT + 1)); // Extra byte tells the server output was truncated.
    let stop_reading = CancellationToken::new();
    let readers_done = CancellationToken::new();
    let stdout = read_output(
        child.stdout.take().context("stdout unavailable")?,
        id.to_owned(),
        Stream::Stdout,
        tx.clone(),
        budget.clone(),
        stop_reading.clone(),
    );
    let stderr = read_output(
        child.stderr.take().context("stderr unavailable")?,
        id.to_owned(),
        Stream::Stderr,
        tx,
        budget,
        stop_reading.clone(),
    );
    let readers = async {
        tokio::try_join!(stdout, stderr)?;
        readers_done.cancel();
        Ok::<_, anyhow::Error>(())
    };
    // Readers stay in this task so aborting it cannot leave detached pipe-reading tasks.
    let status = async {
        let outcome = tokio::select! {
            status = child.wait() => (status?.code(), Outcome::Exited),
            _ = cancel.cancelled() => (None, Outcome::Cancelled),
            _ = tokio::time::sleep(Duration::from_secs(request.timeout_secs)) => (None, Outcome::TimedOut),
        };
        // Reap the complete process tree even when the main command exits normally.
        tree.kill();
        let _ = child.kill().await;
        let _ = child.wait().await;
        // A process that escapes the group may hold an inherited pipe open. Never
        // let that prevent reporting command completion or timeout indefinitely.
        tokio::select! {
            _ = readers_done.cancelled() => {},
            _ = tokio::time::sleep(Duration::from_secs(2)) => stop_reading.cancel(),
        }
        Ok::<_, anyhow::Error>(outcome)
    };
    let (_, outcome) = tokio::try_join!(readers, status)?;
    Ok(outcome)
}

#[cfg(unix)]
pub(crate) struct ProcessTree {
    pid: i32,
    alive: std::sync::atomic::AtomicBool,
}
#[cfg(unix)]
impl ProcessTree {
    pub(crate) fn new(child: &tokio::process::Child) -> Result<Self> {
        Ok(Self {
            pid: child.id().context("missing child PID")? as i32,
            alive: std::sync::atomic::AtomicBool::new(true),
        })
    }
    pub(crate) fn kill(&self) {
        if self.alive.swap(false, Ordering::Relaxed) {
            unsafe {
                libc::kill(-self.pid, libc::SIGKILL);
            }
        }
    }
}
#[cfg(unix)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(windows)]
pub(crate) struct ProcessTree {
    job: windows_sys::Win32::Foundation::HANDLE,
}
#[cfg(windows)]
unsafe impl Send for ProcessTree {}
#[cfg(windows)]
unsafe impl Sync for ProcessTree {}
#[cfg(windows)]
impl ProcessTree {
    pub(crate) fn new(child: &tokio::process::Child) -> Result<Self> {
        Self::with_breakaway(child, false)
    }
    /// Only the runtime may launch a terminal owner outside its shutdown job.
    pub(crate) fn new_runtime(child: &tokio::process::Child) -> Result<Self> {
        Self::with_breakaway(child, true)
    }
    fn with_breakaway(child: &tokio::process::Child, breakaway: bool) -> Result<Self> {
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            anyhow::ensure!(
                !job.is_null(),
                "create Windows Job Object: {}",
                std::io::Error::last_os_error()
            );
            let tree = Self { job };
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if breakaway {
                info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_BREAKAWAY_OK;
            }
            anyhow::ensure!(
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of_val(&info) as u32
                ) != 0,
                "set Windows Job Object: {}",
                std::io::Error::last_os_error()
            );
            anyhow::ensure!(
                AssignProcessToJobObject(
                    job,
                    child.raw_handle().context("missing process handle")? as _
                ) != 0,
                "assign Windows Job Object: {}",
                std::io::Error::last_os_error()
            );
            resume_primary_thread(child.id().context("missing child PID")?)?;
            Ok(tree)
        }
    }
    pub(crate) fn kill(&self) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
        }
    }
}

#[cfg(windows)]
fn resume_primary_thread(pid: u32) -> Result<()> {
    use windows_sys::Win32::{
        Foundation::*,
        System::{Diagnostics::ToolHelp::*, Threading::*},
    };
    // Tokio retains the process handle but not CreateProcess's primary thread
    // handle. A CREATE_SUSPENDED process has one thread, found through ToolHelp.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        anyhow::ensure!(
            snapshot != INVALID_HANDLE_VALUE,
            "snapshot Windows threads: {}",
            std::io::Error::last_os_error()
        );
        let result = (|| {
            let mut entry: THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = std::mem::size_of_val(&entry) as u32;
            let mut available = Thread32First(snapshot, &mut entry);
            while available != 0 {
                if entry.th32OwnerProcessID == pid {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    anyhow::ensure!(
                        !thread.is_null(),
                        "open Windows primary thread: {}",
                        std::io::Error::last_os_error()
                    );
                    let resumed = ResumeThread(thread);
                    let error = std::io::Error::last_os_error();
                    CloseHandle(thread);
                    anyhow::ensure!(
                        resumed != u32::MAX,
                        "resume Windows primary thread: {error}"
                    );
                    return Ok(());
                }
                available = Thread32Next(snapshot, &mut entry);
            }
            anyhow::bail!("Windows primary thread not found");
        })();
        CloseHandle(snapshot);
        result
    }
}
#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        self.kill();
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job);
        }
    }
}

#[cfg(test)]
mod terminal_tests {
    use super::*;
    use agent_workspace_pty::engine::Engine as TerminalEngine;
    use agent_workspace_pty::{
        Event,
        engine::{Attachment, Manage},
    };

    async fn expect(attachment: &mut Attachment, text: &str) -> usize {
        let mut output: Vec<u8> = attachment.take_replay().into_iter().flatten().collect();
        let replay_bytes = output.len();
        let finished = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if output
                    .windows(text.len())
                    .any(|window| window == text.as_bytes())
                {
                    return;
                }
                match attachment.recv().await {
                    Some(Event::Output(data)) => output.extend(data),
                    _ => panic!(
                        "terminal closed before {text}: {}",
                        String::from_utf8_lossy(&output)
                    ),
                }
            }
        })
        .await;
        assert!(
            finished.is_ok(),
            "terminal timed out before {text}: {}",
            String::from_utf8_lossy(&output[output.len().saturating_sub(2048)..])
        );
        replay_bytes
    }

    fn marker(first: &str, second: &str) -> Vec<u8> {
        #[cfg(windows)]
        let command = format!("echo {first}^{second}\r");
        #[cfg(not(windows))]
        let command = format!("printf '%s%s\\n' {first} {second}\r");
        command.into_bytes()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn node_owns_shell_across_detach_and_takeover() {
        let directory = tempfile::tempdir().unwrap();
        let engine = TerminalEngine::new(directory.path().into());
        let created = engine
            .manage(Manage::Create {
                name: "Native shell".into(),
                cwd: None,
            })
            .await
            .unwrap();
        let id = created["session"]["id"].as_str().unwrap().to_owned();
        let mut first = engine.attach(&id, 80, 24, false).await.unwrap();
        #[cfg(windows)]
        first
            .write(b"set CRANIBOX_TEST=retained\r".to_vec())
            .unwrap();
        #[cfg(not(windows))]
        first
            .write(b"export CRANIBOX_TEST=retained\r".to_vec())
            .unwrap();
        first.write(marker("FIRST", "_READY")).unwrap();
        expect(&mut first, "FIRST_READY").await;
        drop(first);
        let mut second = engine.attach(&id, 101, 31, false).await.unwrap();
        expect(&mut second, "FIRST_READY").await;
        #[cfg(windows)]
        second
            .write(b"echo %CRANIBOX_TEST%^_AFTER_DETACH\r".to_vec())
            .unwrap();
        #[cfg(not(windows))]
        second
            .write(b"printf '%s%s\\n' \"$CRANIBOX_TEST\" _AFTER_DETACH\r".to_vec())
            .unwrap();
        expect(&mut second, "retained_AFTER_DETACH").await;
        assert!(engine.attach(&id, 80, 24, false).await.is_err());
        let mut replacement = engine.attach(&id, 80, 24, true).await.unwrap();
        drop(second);
        replacement.take_replay();
        replacement.write(marker("TAKEOVER", "_READY")).unwrap();
        expect(&mut replacement, "TAKEOVER_READY").await;
        engine
            .manage(Manage::Rename {
                id: id.clone(),
                name: "Renamed shell".into(),
            })
            .await
            .unwrap();
        let catalog = engine.manage(Manage::List).await.unwrap();
        assert_eq!(catalog["engine"], "node-pty");
        assert_eq!(catalog["sessions"][0]["name"], "Renamed shell");
        assert_eq!(catalog["sessions"][0]["status"], "running");
        engine.manage(Manage::End { id }).await.unwrap();
        assert_eq!(
            engine.manage(Manage::List).await.unwrap()["sessions"],
            serde_json::json!([])
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = replacement.recv().await {
                if matches!(event, Event::Exit(_)) {
                    return;
                }
            }
        })
        .await
        .unwrap();
        engine.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shell_exit_marks_session_ended() {
        let directory = tempfile::tempdir().unwrap();
        let engine = TerminalEngine::new(directory.path().into());
        let created = engine
            .manage(Manage::Create {
                name: "Short lived".into(),
                cwd: None,
            })
            .await
            .unwrap();
        let id = created["session"]["id"].as_str().unwrap().to_owned();
        let mut attachment = engine.attach(&id, 80, 24, false).await.unwrap();
        attachment.write(b"exit 7\r".to_vec()).unwrap();
        let mut output = Vec::new();
        let code = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match attachment.recv().await {
                    Some(Event::Exit(code)) => return code,
                    Some(Event::Output(data)) => output.extend(data),
                    None => panic!("shell closed without an exit status"),
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("shell exit timed out: {}", String::from_utf8_lossy(&output)));
        assert_eq!(code, 7);
        assert_eq!(
            engine.manage(Manage::List).await.unwrap()["sessions"][0]["status"],
            "ended"
        );
        assert!(engine.attach(&id, 80, 24, false).await.is_err());
        engine.manage(Manage::Remove { id }).await.unwrap();
        engine.shutdown().await;
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn disconnected_output_is_drained_and_replay_is_bounded() {
        let directory = tempfile::tempdir().unwrap();
        let engine = TerminalEngine::new(directory.path().into());
        let created = engine
            .manage(Manage::Create {
                name: "Offline output".into(),
                cwd: None,
            })
            .await
            .unwrap();
        let id = created["session"]["id"].as_str().unwrap();
        let first = engine.attach(id, 80, 24, false).await.unwrap();
        first.write(b"yes x | head -c 2097152; printf '%s%s\\n' REPLAY _DONE; printf finished > complete\r".to_vec()).unwrap();
        drop(first);
        tokio::time::timeout(Duration::from_secs(10), async {
            while !directory.path().join("complete").exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("offline output blocked the shell");
        let mut reconnected = engine.attach(id, 80, 24, false).await.unwrap();
        assert!(expect(&mut reconnected, "REPLAY_DONE").await <= 256 * 1024);
        reconnected.write(marker("AFTER", "_OFFLINE")).unwrap();
        expect(&mut reconnected, "AFTER_OFFLINE").await;
        engine.shutdown().await;
    }
}

#[cfg(test)]
mod output_tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_streams_share_one_output_limit() {
        let budget = Arc::new(AtomicUsize::new(10_001));
        let (tx, mut rx) = mpsc::channel(32);
        let tasks = [Stream::Stdout, Stream::Stderr].map(|stream| {
            tokio::spawn(read_output(
                std::io::Cursor::new(vec![b'a'; 8192]),
                "test".into(),
                stream,
                tx.clone(),
                budget.clone(),
                CancellationToken::new(),
            ))
        });
        drop(tx);
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        let mut total = 0;
        while let Some(wire) = rx.recv().await {
            if let Wire::Output { data, .. } = wire {
                total += STANDARD.decode(data).unwrap().len();
            }
        }
        assert_eq!(total, 10_001);
        assert_eq!(budget.load(Ordering::Relaxed), 0);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    use std::collections::BTreeMap;
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn escaped_descendant_cannot_hold_completion_open() {
        let temp = tempfile::tempdir().unwrap();
        let pid_file = temp.path().join("pid");
        let (tx, _rx) = mpsc::channel(32);
        let request = ExecRequest {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                r#"setsid sh -c 'echo $$ > "$PID_FILE"; sleep 30' & while [ ! -s "$PID_FILE" ]; do sleep 0.01; done; exit 0"#.into(),
            ],
            cwd: None,
            env: BTreeMap::from([("PID_FILE".into(), pid_file.to_string_lossy().into_owned())]),
            timeout_secs: 5,
        };
        let (result, pid) = tokio::join!(
            tokio::time::timeout(
                Duration::from_secs(4),
                execute_process("test", request, CancellationToken::new(), tx),
            ),
            async {
                tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        if let Ok(text) = std::fs::read_to_string(&pid_file)
                            && let Ok(pid) = text.trim().parse::<i32>()
                        {
                            break pid;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
            }
        );
        // Escaped groups are outside the node's containment; clean the test fixture.
        let escaped = ProcessTree {
            pid: pid.unwrap(),
            alive: std::sync::atomic::AtomicBool::new(true),
        };
        escaped.kill();
        let (exit_code, outcome) = result.unwrap().unwrap();
        assert_eq!(exit_code, Some(0));
        assert_eq!(outcome, Outcome::Exited);
    }

    #[tokio::test]
    async fn captures_both_streams_and_cleans_up_descendants_on_timeout() {
        let (tx, mut rx) = mpsc::channel(32);
        let request = ExecRequest {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                "printf out; printf err >&2; sleep 30 & wait".into(),
            ],
            cwd: None,
            env: Default::default(),
            timeout_secs: 1,
        };
        let outcome = tokio::time::timeout(
            Duration::from_secs(4),
            execute_process("test", request, CancellationToken::new(), tx),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(outcome.1, Outcome::TimedOut);
        let mut streams = vec![];
        while let Some(wire) = rx.recv().await {
            if let Wire::Output { data, .. } = wire {
                streams.push(STANDARD.decode(data).unwrap());
            }
        }
        assert!(streams.contains(&b"out".to_vec()));
        assert!(streams.contains(&b"err".to_vec()));
    }
}
