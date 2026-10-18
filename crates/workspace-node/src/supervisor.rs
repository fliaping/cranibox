//! Node-owned noninteractive processes. A browser is an observer, never their owner.
use crate::node::ProcessTree;
use agent_workspace_runtime::peers::{
    agent_sessions::{Command as Report, Registry, Status},
    executions::{LOG_LIMIT, Record, State, Store},
};
use anyhow::{Context, Result};
use std::{collections::HashSet, io::Write, path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::Mutex,
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub(crate) struct Service {
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl Service {
    pub fn start(path: PathBuf, owner: String, stop: CancellationToken) -> Result<Self> {
        let store = Store::new(&path, owner.clone());
        let registry = Registry::new(&path, owner.clone());
        for session in store.recover()? {
            end_registration(&registry, session);
        }
        let task = tokio::spawn(async move {
            let mut children = JoinSet::new();
            let mut active = HashSet::new();
            let mut tick = tokio::time::interval(Duration::from_millis(300));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => break,
                    Some(result) = children.join_next(), if !children.is_empty() => { if let Ok(id)=result { active.remove(&id); } },
                    _ = tick.tick() => {
                        let Ok(records)=store.records() else { continue; };
                        for record in records.into_iter().filter(|record| record.state == State::Queued) {
                            if !active.insert(record.id) { continue; }
                            let (store,registry,stop,path,owner)=(store.clone(),registry.clone(),stop.clone(),path.clone(),owner.clone());
                            children.spawn(async move {
                                let id=record.id;
                                if let Err(error)=run(&store,&registry,record.clone(),stop,&path,&owner).await {
                                    // Detailed OS errors are not emitted to shared logs or directory discovery.
                                    let message=if error.chain().any(|cause| cause.to_string()=="Agent registry is full; old reports are retained for seven days") { "Agent registry is full (64 reports retained for seven days); no program was launched." } else if error.downcast_ref::<std::io::Error>().is_some() { "Unable to start or record the program. Check the executable, directory and disk access." } else { "Execution could not be completed. Check the local launch and output." };
                                    let _=store.change(id, |record| { if record.state.active() { record.state=State::Failed;record.pid=None;record.error=Some(message.into()); } Ok(()) });
                                }
                                end_registration(&registry,record.session_id);
                                id
                            });
                        }
                    }
                }
            }
            // Each child observes stop, reaps its process tree and saves a final interrupted record.
            while children.join_next().await.is_some() {}
        });
        Ok(Self {
            task: Mutex::new(Some(task)),
        })
    }
    pub async fn shutdown(&self) {
        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        }
    }
}
fn end_registration(registry: &Registry, id: Uuid) {
    if let Ok(catalog) = registry.catalog()
        && let Some(session) = catalog
            .sessions
            .iter()
            .find(|session| session.id == id && session.status != Status::Ended)
    {
        let _ = registry.apply(Report::End {
            id,
            revision: session.revision,
        });
    }
}
fn heartbeat(registry: &Registry, id: Uuid) -> Result<()> {
    let catalog = registry.catalog()?;
    let session = catalog
        .sessions
        .iter()
        .find(|session| session.id == id)
        .context("managed Agent registration unavailable")?;
    registry.apply(Report::Heartbeat {
        id,
        revision: session.revision,
        status: Status::Busy,
    })?;
    Ok(())
}
async fn read_log(
    mut input: impl AsyncRead + Unpin,
    mut output: std::fs::File,
    done: CancellationToken,
) -> Result<bool> {
    let mut total = 0;
    let mut truncated = false;
    let mut bytes = [0; 16384];
    loop {
        let count =
            tokio::select! { value=input.read(&mut bytes) => value?, _=done.cancelled() => break };
        if count == 0 {
            break;
        }
        let keep = count.min(LOG_LIMIT - total);
        if keep > 0 {
            output.write_all(&bytes[..keep])?;
            total += keep;
        }
        truncated |= keep < count;
    }
    output.sync_all()?;
    Ok(truncated)
}
async fn run(
    store: &Store,
    registry: &Registry,
    record: Record,
    stop: CancellationToken,
    path: &std::path::Path,
    owner: &str,
) -> Result<()> {
    let mut claimed = false;
    store.change(record.id, |record| {
        if record.state == State::Queued && !record.permitted(path, owner) {
            record.state = State::Cancelled;
            record.error = Some("Execution access was revoked; no program was launched.".into());
        } else if record.state == State::Queued {
            record.state = State::Starting;
            claimed = true;
        }
        Ok(())
    })?;
    if !claimed {
        return Ok(());
    }
    registry.apply(Report::Register {
        id: record.session_id,
        name: record.launch.name.clone(),
        harness: record.launch.harness.clone(),
    })?;
    heartbeat(registry, record.session_id)?;
    let stdout_file = store.create_log(record.id, false)?;
    let stderr_file = store.create_log(record.id, true)?;
    let mut command = Command::new(&record.launch.program);
    command
        .args(&record.launch.args)
        .current_dir(std::fs::canonicalize(&record.launch.cwd)?)
        .env("CRANIBOX_AGENT_SESSION_ID", record.session_id.to_string())
        .env("CRANIBOX_EXECUTION_ID", record.id.to_string())
        .env("CRANIBOX_NODE_ID", owner)
        .env("CRANIBOX_PEER_CONFIG", path)
        .stdin(if record.launch.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
    if stop.is_cancelled()
        || !record.permitted(path, owner)
        || store.record(record.id)?.state != State::Starting
    {
        store.change(record.id, |record| {
            record.state = if stop.is_cancelled() {
                State::Interrupted
            } else {
                State::Cancelled
            };
            if !record.permitted(path, owner) {
                record.error =
                    Some("Execution access was revoked; no program was launched.".into());
            }
            Ok(())
        })?;
        return Ok(());
    }
    let mut child = command.spawn()?;
    let tree = ProcessTree::new(&child)?;
    store.change(record.id, |record| {
        if record.state == State::Starting {
            record.state = State::Running;
        }
        record.pid = child.id();
        Ok(())
    })?;
    let stdout = child.stdout.take().context("stdout unavailable")?;
    let stderr = child.stderr.take().context("stderr unavailable")?;
    let input = child.stdin.take();
    let readers_done = CancellationToken::new();
    let stop_reading = CancellationToken::new();
    let stop_writing = CancellationToken::new();
    let io_failed = CancellationToken::new();
    // Drain output while writing input: a CLI can fill stdout before reading its prompt.
    let writer = async {
        let write = async {
            if let Some(mut input) = input {
                input
                    .write_all(
                        record
                            .launch
                            .stdin
                            .as_deref()
                            .unwrap_or_default()
                            .as_bytes(),
                    )
                    .await?;
                input.shutdown().await?;
            }
            Ok::<_, std::io::Error>(())
        };
        let result = tokio::select! {
            biased;
            result = write => result,
            _ = stop_writing.cancelled() => Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "program exited before task delivery")),
        };
        if result.is_err() {
            io_failed.cancel();
        }
        result
    };
    let readers = async {
        let result = tokio::try_join!(
            read_log(stdout, stdout_file, stop_reading.clone()),
            read_log(stderr, stderr_file, stop_reading.clone())
        );
        readers_done.cancel();
        if result.is_err() {
            io_failed.cancel();
        }
        result
    };
    let status = async {
        let mut poll = tokio::time::interval(Duration::from_millis(300));
        let mut report = tokio::time::interval(Duration::from_secs(60));
        let result: Result<(State, Option<i32>)> = loop {
            tokio::select! {
                status=child.wait() => break status.map(|status| (if status.success(){State::Succeeded}else{State::Failed},status.code())).map_err(Into::into),
                _=stop.cancelled() => break Ok((State::Interrupted,None)),
                _=io_failed.cancelled() => break Ok((State::Failed,None)),
                _=poll.tick() => { match store.record(record.id) {
                    Ok(record) if !record.permitted(path, owner) => {
                        store.change(record.id, |record| { record.error = Some("Execution access was revoked; the process was stopped.".into()); Ok(()) })?;
                        break Ok((State::Cancelled,None));
                    },
                    Ok(record) if record.state == State::Stopping => break Ok((State::Cancelled,None)),
                    Err(error) => break Err(error),
                    _ => {},
                } },
                _=report.tick() => { let _=heartbeat(registry,record.session_id); },
            }
        };
        tree.kill();
        let _ = child.kill().await;
        let _ = child.wait().await;
        stop_writing.cancel();
        tokio::select! { _=readers_done.cancelled()=>{}, _=tokio::time::sleep(Duration::from_secs(2))=>stop_reading.cancel() }
        result
    };
    let (logs, result, input) = tokio::join!(readers, status, writer);
    let (stdout_truncated, stderr_truncated) = logs?;
    let (mut state, exit_code) = result?;
    if input.is_err() && state == State::Succeeded {
        state = State::Failed;
    }
    store.change(record.id, |record| {
        record.state = state;
        record.exit_code = exit_code;
        record.pid = None;
        record.stdout_truncated = stdout_truncated;
        record.stderr_truncated = stderr_truncated;
        if state == State::Interrupted {
            record.error = Some(
                "Node service stopped; review the previous execution before starting again.".into(),
            );
        } else if state == State::Failed && input.is_err() {
            record.error = Some("Unable to deliver the task to the program's standard input. Review output before starting again.".into());
        }
        Ok(())
    })?;
    Ok(())
}
