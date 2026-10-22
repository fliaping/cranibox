//! Node-owned shells whose lifetime is independent of network attachments.
use crate::{Event, Session, validate_size};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, mpsc, oneshot, watch};

const MAX_SESSIONS: usize = 64;
const REPLAY_BYTES: usize = 256 * 1024;

#[derive(Clone)]
pub struct Engine {
    home: PathBuf,
    sessions: Arc<Mutex<BTreeMap<String, Stored>>>,
}

struct Stored {
    name: String,
    cwd: String,
    created_at: u64,
    last_attached_at: Option<u64>,
    commands: mpsc::Sender<Control>,
    exited: watch::Receiver<Option<u32>>,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Manage {
    List,
    Create {
        name: String,
        #[serde(default)]
        cwd: Option<String>,
    },
    Rename {
        id: String,
        name: String,
    },
    End {
        id: String,
    },
    Remove {
        id: String,
    },
}

enum Control {
    Attach {
        cols: u16,
        rows: u16,
        takeover: bool,
        reply: oneshot::Sender<Result<Attachment>>,
    },
    Input {
        attachment: String,
        data: Vec<u8>,
    },
    Resize {
        attachment: String,
        cols: u16,
        rows: u16,
    },
    Detach {
        attachment: String,
    },
    End {
        reply: oneshot::Sender<()>,
    },
}

pub struct Attachment {
    id: String,
    commands: mpsc::Sender<Control>,
    pub replay: Vec<Vec<u8>>,
    pub output: mpsc::Receiver<Event>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.trim().is_empty()
            && name.chars().count() <= 80
            && !name.chars().any(char::is_control),
        "session name must be 1..80 characters without control characters"
    );
    Ok(())
}

impl Stored {
    fn view(&self, id: &str) -> Value {
        json!({"id":id,"name":self.name,"cwd":self.cwd,"created_at":self.created_at,
            "last_attached_at":self.last_attached_at,
            "status":if self.exited.borrow().is_none() { "running" } else { "ended" }})
    }
    async fn end(self) {
        let (reply, finished) = oneshot::channel();
        if self.commands.send(Control::End { reply }).await.is_ok() {
            let _ = finished.await;
        }
    }
}

impl Engine {
    pub fn new(home: PathBuf) -> Self {
        Self {
            home,
            sessions: Arc::default(),
        }
    }

    pub async fn manage(&self, request: Manage) -> Result<Value> {
        let mut sessions = self.sessions.lock().await;
        match request {
            Manage::List => Ok(json!({"engine":"node-pty","installed":true,"version":null,
                "sessions":sessions.iter().map(|(id, session)| session.view(id)).collect::<Vec<_>>() })),
            Manage::Create { name, cwd } => {
                validate_name(&name)?;
                ensure!(
                    sessions.len() < MAX_SESSIONS,
                    "session limit reached; remove ended sessions"
                );
                let cwd = cwd.map(PathBuf::from).unwrap_or_else(|| {
                    let workspace = self.home.join("Workspace");
                    if workspace.is_dir() {
                        workspace
                    } else {
                        self.home.clone()
                    }
                });
                ensure!(cwd.is_dir(), "working directory does not exist");
                let cwd = std::fs::canonicalize(cwd)?;
                let directory = cwd.to_string_lossy().into_owned();
                let home = self.home.clone();
                let pty = tokio::task::spawn_blocking(move || {
                    Session::open(80, 24, Some(cwd), Some(home))
                })
                .await??;
                let (commands, receiver) = mpsc::channel(32);
                let (status, exited) = watch::channel(None);
                // The actor holds only a weak sender, so dropping the manager
                // and all attachments closes its control queue and ends the shell.
                tokio::spawn(own_session(pty, receiver, commands.downgrade(), status));
                let id = uuid::Uuid::new_v4().to_string();
                let session = Stored {
                    name,
                    cwd: directory,
                    created_at: now(),
                    last_attached_at: None,
                    commands,
                    exited,
                };
                let view = session.view(&id);
                sessions.insert(id, session);
                Ok(json!({"session":view}))
            }
            Manage::Rename { id, name } => {
                validate_name(&name)?;
                sessions.get_mut(&id).context("unknown session")?.name = name;
                Ok(json!({"ok":true}))
            }
            Manage::End { id } | Manage::Remove { id } => {
                let session = sessions.remove(&id).context("unknown session")?;
                session.end().await;
                Ok(json!({"ok":true}))
            }
        }
    }

    pub async fn attach(
        &self,
        id: &str,
        cols: u16,
        rows: u16,
        takeover: bool,
    ) -> Result<Attachment> {
        validate_size(cols, rows)?;
        let mut sessions = self.sessions.lock().await;
        let session = sessions.get_mut(id).context("unknown session")?;
        ensure!(
            session.exited.borrow().is_none(),
            "session ended; create a new session"
        );
        let (reply, attachment) = oneshot::channel();
        session
            .commands
            .send(Control::Attach {
                cols,
                rows,
                takeover,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("session ended; create a new session"))?;
        let attachment = attachment
            .await
            .context("session ended during attachment")??;
        session.last_attached_at = Some(now());
        Ok(attachment)
    }

    pub async fn shutdown(&self) {
        let sessions = std::mem::take(&mut *self.sessions.lock().await);
        for session in sessions.into_values() {
            session.end().await;
        }
    }
}

impl Attachment {
    pub fn write(&self, data: Vec<u8>) -> Result<()> {
        ensure!(data.len() <= 16 * 1024, "terminal input too large");
        self.send(Control::Input {
            attachment: self.id.clone(),
            data,
        })
    }
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        validate_size(cols, rows)?;
        self.send(Control::Resize {
            attachment: self.id.clone(),
            cols,
            rows,
        })
    }
    fn send(&self, command: Control) -> Result<()> {
        self.commands
            .try_send(command)
            .map_err(|_| anyhow::anyhow!("terminal control queue full or closed"))
    }
}

impl Drop for Attachment {
    fn drop(&mut self) {
        let command = Control::Detach {
            attachment: self.id.clone(),
        };
        if let Err(mpsc::error::TrySendError::Full(command)) = self.commands.try_send(command) {
            let commands = self.commands.clone();
            tokio::spawn(async move {
                let _ = commands.send(command).await;
            });
        }
    }
}

struct Attached {
    id: String,
    output: mpsc::Sender<Event>,
    /// Output that did not fit the queue yet, delivered in order once it drains.
    backlog: Vec<u8>,
}
impl Attached {
    /// Queue `data` after any backlog. `false`: the viewer is gone or so far behind
    /// that it must reattach and catch up from the replay history.
    fn deliver(&mut self, data: Vec<u8>) -> bool {
        self.backlog.extend_from_slice(&data);
        while !self.backlog.is_empty() {
            let take = self.backlog.len().min(32 * 1024);
            match self.output.try_reserve() {
                Ok(permit) => {
                    permit.send(Event::Output(self.backlog.drain(..take).collect()));
                }
                Err(mpsc::error::TrySendError::Full(())) => {
                    return self.backlog.len() <= REPLAY_BYTES;
                }
                Err(mpsc::error::TrySendError::Closed(())) => return false,
            }
        }
        true
    }
}

async fn own_session(
    mut pty: Session,
    mut commands: mpsc::Receiver<Control>,
    sender: mpsc::WeakSender<Control>,
    exited: watch::Sender<Option<u32>>,
) {
    let mut attachment: Option<Attached> = None;
    // Bursts beyond the attachment's queue wait here instead of dropping the viewer.
    let mut flush = tokio::time::interval(std::time::Duration::from_millis(25));
    let mut history: VecDeque<Vec<u8>> = VecDeque::new();
    let mut bytes = 0;
    let mut end_reply = None;
    let code = loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Control::Attach { cols, rows, takeover, reply }) => {
                    if reply.is_closed() { continue; }
                    if attachment.as_ref().is_some_and(|a| !a.output.is_closed()) && !takeover {
                        let _ = reply.send(Err(anyhow::anyhow!("session already attached; take over to reconnect")));
                        continue;
                    }
                    if let Err(error) = pty.resize(cols, rows) {
                        let _ = reply.send(Err(error));
                        continue;
                    }
                    let Some(sender) = sender.upgrade() else { break 1; };
                    let id = uuid::Uuid::new_v4().to_string();
                    let (output, receiver) = mpsc::channel(32);
                    let history: Vec<u8> = history.iter().flatten().copied().collect();
                    // Keep replay below the protocol frame and route queue limits.
                    let mut replay: Vec<_> = history.chunks(32 * 1024).map(<[u8]>::to_vec).collect();
                    // Also signal successful attachment when the shell is silent.
                    if replay.is_empty() { replay.push(Vec::new()); }
                    attachment = Some(Attached { id: id.clone(), output, backlog: Vec::new() });
                    let _ = reply.send(Ok(Attachment { id, commands: sender, replay, output: receiver }));
                }
                Some(Control::Input { attachment: id, data }) => {
                    if attachment.as_ref().is_some_and(|a| a.id == id) && pty.write(data).is_err() {
                        attachment = None;
                    }
                }
                Some(Control::Resize { attachment: id, cols, rows }) => {
                    if attachment.as_ref().is_some_and(|a| a.id == id) && pty.resize(cols, rows).is_err() {
                        attachment = None;
                    }
                }
                Some(Control::Detach { attachment: id }) => {
                    if attachment.as_ref().is_some_and(|a| a.id == id) { attachment = None; }
                }
                Some(Control::End { reply }) => { end_reply = Some(reply); break 0; }
                None => break 0,
            },
            event = pty.output.recv() => match event {
                Some(Event::Output(data)) => {
                    bytes += data.len();
                    history.push_back(data.clone());
                    while bytes > REPLAY_BYTES {
                        if let Some(oldest) = history.pop_front() { bytes -= oldest.len(); }
                    }
                    // Slow or disconnected browsers must never stop reading the PTY.
                    if attachment.as_mut().is_some_and(|a| !a.deliver(data)) {
                        attachment = None;
                    }
                }
                Some(Event::Exit(code)) => break code,
                None => break 1,
            },
            _ = flush.tick(), if attachment.as_ref().is_some_and(|a| !a.backlog.is_empty()) => {
                if attachment.as_mut().is_some_and(|a| !a.deliver(Vec::new())) {
                    attachment = None;
                }
            }
        }
    };
    drop(pty);
    exited.send_replace(Some(code));
    if let Some(mut attached) = attachment {
        attached.deliver(Vec::new());
        let _ = attached.output.try_send(Event::Exit(code));
    }
    if let Some(reply) = end_reply {
        let _ = reply.send(());
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A viewer that falls behind during an output burst keeps its attachment: the
    /// burst is delivered in order and later input still reaches the shell.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn output_burst_keeps_the_attachment_and_its_input() {
        let home = tempfile::tempdir().unwrap();
        // SAFETY: tests in this module do not read SHELL concurrently.
        unsafe { std::env::set_var("SHELL", "/bin/sh") };
        let engine = Engine::new(home.path().to_path_buf());
        let created = engine
            .manage(Manage::Create {
                name: "burst".into(),
                cwd: None,
            })
            .await
            .unwrap();
        let id = created["session"]["id"].as_str().unwrap().to_owned();
        let mut attachment = engine.attach(&id, 80, 24, false).await.unwrap();
        attachment.write(b"seq 1 30000\r".to_vec()).unwrap();
        // Do not read while the shell prints far more than the queue holds.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        attachment.write(b"echo DONE_$((40+2))\r".to_vec()).unwrap();
        let mut seen = String::new();
        let found = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            while let Some(event) = attachment.output.recv().await {
                if let Event::Output(data) = event {
                    seen.push_str(&String::from_utf8_lossy(&data));
                    if seen.contains("DONE_42") {
                        return true;
                    }
                }
            }
            false
        })
        .await;
        assert_eq!(found, Ok(true), "input after a burst was lost");
        assert!(
            seen.contains("\n30000"),
            "the burst arrived complete and in order"
        );
        engine.manage(Manage::End { id }).await.unwrap();
    }
}
