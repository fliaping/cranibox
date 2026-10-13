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

async fn own_session(
    mut pty: Session,
    mut commands: mpsc::Receiver<Control>,
    sender: mpsc::WeakSender<Control>,
    exited: watch::Sender<Option<u32>>,
) {
    let mut attachment: Option<(String, mpsc::Sender<Event>)> = None;
    let mut history: VecDeque<Vec<u8>> = VecDeque::new();
    let mut bytes = 0;
    let mut end_reply = None;
    let code = loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Control::Attach { cols, rows, takeover, reply }) => {
                    if reply.is_closed() { continue; }
                    if attachment.as_ref().is_some_and(|(_, output)| !output.is_closed()) && !takeover {
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
                    attachment = Some((id.clone(), output));
                    let _ = reply.send(Ok(Attachment { id, commands: sender, replay, output: receiver }));
                }
                Some(Control::Input { attachment: id, data }) => {
                    if attachment.as_ref().is_some_and(|(current, _)| current == &id) && pty.write(data).is_err() {
                        attachment = None;
                    }
                }
                Some(Control::Resize { attachment: id, cols, rows }) => {
                    if attachment.as_ref().is_some_and(|(current, _)| current == &id) && pty.resize(cols, rows).is_err() {
                        attachment = None;
                    }
                }
                Some(Control::Detach { attachment: id }) => {
                    if attachment.as_ref().is_some_and(|(current, _)| current == &id) { attachment = None; }
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
                    if attachment.as_ref().is_some_and(|(_, output)| output.try_send(Event::Output(data)).is_err()) {
                        attachment = None;
                    }
                }
                Some(Event::Exit(code)) => break code,
                None => break 1,
            },
        }
    };
    drop(pty);
    exited.send_replace(Some(code));
    if let Some((_, output)) = attachment {
        let _ = output.try_send(Event::Exit(code));
    }
    if let Some(reply) = end_reply {
        let _ = reply.send(());
    }
}
