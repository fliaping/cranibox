use agent_workspace_runtime::{config, protocol::*};
use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{io::AsyncReadExt, process::Command, sync::mpsc, task::JoinSet};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, client::IntoClientRequest, protocol::WebSocketConfig},
};
use tokio_util::sync::CancellationToken;

pub async fn run(
    server: String,
    id: String,
    token_file: PathBuf,
    name: Option<String>,
    allow_insecure: bool,
) -> Result<()> {
    anyhow::ensure!(valid_node_id(&id), "invalid node ID");
    let url = config::check_url(&server, true, allow_insecure)?;
    anyhow::ensure!(
        url.path() == "/",
        "server must be a WebSocket origin without a path"
    );
    let name = name.unwrap_or_else(|| id.clone());
    anyhow::ensure!(name.len() <= 128, "node name too long");
    let stop = CancellationToken::new();
    let stop_signal = stop.clone();
    tokio::spawn(async move {
        agent_workspace_runtime::shutdown_signal().await;
        stop_signal.cancel();
    });
    let endpoint = format!("{}/ws/nodes/{id}", server.trim_end_matches('/'));
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let state = std::fs::canonicalize(&token_file)?
        .parent()
        .context("token file needs a parent directory")?
        .join(format!(".cranibox-herdr-{id}"));
    let engine = agent_workspace_pty::herdr::Engine::new(home, state);
    let mut delay = 1u64;
    loop {
        if stop.is_cancelled() {
            return Ok(());
        }
        let started = Instant::now();
        // Reload token on reconnect so rotation does not require restarting the client.
        let result = session(&endpoint, &token_file, &name, &stop, &engine).await;
        if stop.is_cancelled() {
            return Ok(());
        }
        if let Err(error) = result {
            eprintln!("Node {id}: {error:#}; reconnecting");
        }
        if started.elapsed() >= Duration::from_secs(DEAD_SECS) {
            delay = 1;
        }
        let jitter = u64::from(rand::random::<u16>()) % 1000;
        tokio::select! { _ = stop.cancelled() => return Ok(()), _ = tokio::time::sleep(Duration::from_millis(delay * 1000 + jitter)) => {} }
        delay = (delay * 2).min(30);
    }
}

async fn session(
    endpoint: &str,
    token_file: &std::path::Path,
    name: &str,
    stop: &CancellationToken,
    engine: &agent_workspace_pty::herdr::Engine,
) -> Result<()> {
    let mut request = endpoint.into_client_request()?;
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", config::read_token(token_file)?).parse()?,
    );
    let ws_config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME));
    let (mut socket, _) = tokio::select! {
        _ = stop.cancelled() => return Ok(()),
        r = tokio::time::timeout(Duration::from_secs(10), connect_async_with_config(request, Some(ws_config), false)) => r??,
    };
    socket
        .send(Message::Text(
            serde_json::to_string(&Wire::Hello {
                version: VERSION,
                name: name.into(),
                os: std::env::consts::OS.into(),
                arch: std::env::consts::ARCH.into(),
                terminal: true,
                terminal_sessions: !cfg!(windows),
            })?
            .into(),
        ))
        .await?;
    let reply = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await?
        .context("connection closed before welcome")??;
    anyhow::ensure!(
        matches!(
            serde_json::from_str::<Wire>(reply.to_text()?)?,
            Wire::Welcome { version: VERSION }
        ),
        "unsupported server protocol"
    );
    eprintln!("Node {name} connected");
    let (tx, mut rx) = mpsc::channel::<Wire>(32);
    let mut tasks = JoinSet::new();
    let mut active = BTreeMap::<String, CancellationToken>::new();
    let mut terminals = BTreeMap::<String, tokio::sync::mpsc::Sender<Wire>>::new();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(HEARTBEAT_SECS));
    let mut last_seen = Instant::now();
    let result: Result<()> = async {
        loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                    let id: String = result?;
                    active.remove(&id);
                }
                _ = heartbeat.tick() => {
                    anyhow::ensure!(last_seen.elapsed() <= Duration::from_secs(DEAD_SECS), "server heartbeat expired");
                    tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Text(serde_json::to_string(&Wire::Heartbeat {})?.into()))).await??;
                }
                Some(wire) = rx.recv() => {
                    tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Text(serde_json::to_string(&wire)?.into()))).await??;
                }
                incoming = socket.next() => {
                    let message = incoming.context("server disconnected")??;
                    match message {
                        Message::Text(text) => match serde_json::from_str::<Wire>(&text)? {
                            Wire::Heartbeat {} => last_seen = Instant::now(),
                            Wire::Execute { id, request } => {
                                anyhow::ensure!(uuid::Uuid::parse_str(&id).is_ok() && !active.contains_key(&id), "invalid or duplicate job ID");
                                if active.len() >= MAX_PARALLEL || request.validate().is_err() {
                                    tx.send(Wire::Finished { id, exit_code:None, outcome:Outcome::Failed, error:Some("invalid command or node capacity reached".into()) }).await?;
                                    continue;
                                }
                                let cancel = CancellationToken::new();
                                active.insert(id.clone(), cancel.clone());
                                let tx = tx.clone();
                                tasks.spawn(async move { execute(id.clone(), request, cancel, tx).await; id });
                            }
                            Wire::Cancel { id } => if let Some(cancel) = active.get(&id) { cancel.cancel(); },
                            Wire::TerminalManage { id, request } => {
                                anyhow::ensure!(uuid::Uuid::parse_str(&id).is_ok(), "invalid terminal request ID");
                                if tasks.len() >= 12 { tx.send(Wire::TerminalResult { id, result: serde_json::json!({"error":"node busy"}) }).await?; continue; }
                                let engine = engine.clone(); let tx = tx.clone();
                                tasks.spawn(async move {
                                    let result = match serde_json::from_value(request) {
                                        Ok(request) => engine.manage(request).await,
                                        Err(error) => Err(error.into()),
                                    };
                                    let result = result.unwrap_or_else(|error| serde_json::json!({"error":format!("{error:#}")}));
                                    let _ = tx.send(Wire::TerminalResult { id: id.clone(), result }).await;
                                    id
                                });
                            }
                            Wire::TerminalOpen { id, cols, rows, session, takeover } => {
                                terminals.retain(|_, tx| !tx.is_closed());
                                anyhow::ensure!(uuid::Uuid::parse_str(&id).is_ok() && !terminals.contains_key(&id), "invalid terminal ID");
                                if terminals.len() >= 4 { tx.send(Wire::TerminalExit { id, code: 1 }).await?; continue; }
                                let (controls, receiver) = mpsc::channel(32);
                                terminals.insert(id.clone(), controls);
                                tokio::spawn(terminal(id, (cols, rows), session, takeover, engine.clone(), receiver, tx.clone()));
                            }
                            wire @ (Wire::TerminalInput { .. } | Wire::TerminalResize { .. } | Wire::TerminalClose { .. }) => {
                                let id = match &wire { Wire::TerminalInput { id, .. } | Wire::TerminalResize { id, .. } | Wire::TerminalClose { id } => id.clone(), _ => unreachable!() };
                                if matches!(wire, Wire::TerminalClose { .. }) {
                                    terminals.remove(&id);
                                } else if let Some(control) = terminals.get(&id)
                                    && control.try_send(wire).is_err() {
                                    terminals.remove(&id);
                                }
                            }
                            _ => anyhow::bail!("unexpected server message"),
                        },
                        Message::Ping(_) | Message::Pong(_) => {},
                        Message::Close(_) => anyhow::bail!("server closed connection"),
                        _ => anyhow::bail!("unexpected WebSocket frame"),
                    }
                }
            }
        }
        Ok(())
    }.await;
    terminals.clear();
    for cancel in active.values() {
        cancel.cancel();
    }
    // A dead connection must not strand readers trying to send into a full output queue.
    drop(rx);
    if tokio::time::timeout(Duration::from_secs(5), async {
        while tasks.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.close(None)).await;
    result
}

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
async fn execute(
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
struct ProcessTree {
    pid: i32,
    alive: std::sync::atomic::AtomicBool,
}
#[cfg(unix)]
impl ProcessTree {
    fn new(child: &tokio::process::Child) -> Result<Self> {
        Ok(Self {
            pid: child.id().context("missing child PID")? as i32,
            alive: std::sync::atomic::AtomicBool::new(true),
        })
    }
    fn kill(&self) {
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
struct ProcessTree {
    job: windows_sys::Win32::Foundation::HANDLE,
}
#[cfg(windows)]
unsafe impl Send for ProcessTree {}
#[cfg(windows)]
unsafe impl Sync for ProcessTree {}
#[cfg(windows)]
impl ProcessTree {
    fn new(child: &tokio::process::Child) -> Result<Self> {
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
    fn kill(&self) {
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

async fn terminal(
    id: String,
    size: (u16, u16),
    session: String,
    takeover: bool,
    engine: agent_workspace_pty::herdr::Engine,
    mut controls: mpsc::Receiver<Wire>,
    tx: mpsc::Sender<Wire>,
) {
    let opened = engine.attach(&session, size.0, size.1, takeover).await;
    let Ok(mut pty) = opened else {
        let _ = tx.send(Wire::TerminalExit { id, code: 1 }).await;
        return;
    };
    let mut exit_code = 1;
    loop {
        tokio::select! {
            input = controls.recv() => match input {
                Some(Wire::TerminalInput { data, .. }) => {
                    let Ok(data) = STANDARD.decode(data) else { break; };
                    if pty.write(data).is_err() { break; }
                }
                Some(Wire::TerminalResize { cols, rows, .. }) => if pty.resize(cols, rows).is_err() { break; },
                _ => break,
            },
            output = pty.output.recv() => {
                let wire = match output {
                    Some(agent_workspace_pty::Event::Output(data)) => Wire::TerminalOutput { id: id.clone(), data: STANDARD.encode(data) },
                    Some(agent_workspace_pty::Event::Exit(code)) => { exit_code = code; break; },
                    None => break,
                };
                if !matches!(tokio::time::timeout(Duration::from_secs(5), tx.send(wire)).await, Ok(Ok(()))) { break; }
            }
        }
    }
    drop(pty);
    let _ = tokio::time::timeout(
        Duration::from_secs(5),
        tx.send(Wire::TerminalExit {
            id,
            code: exit_code,
        }),
    )
    .await;
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
