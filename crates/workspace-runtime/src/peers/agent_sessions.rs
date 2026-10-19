//! Explicit Agent registrations. A recent report is not proof of process liveness.
use super::{Store as Peers, config::Config, messages::now};
use crate::{config, protocol::valid_node_id};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const CAPACITY: usize = 64;
pub const LEASE: u64 = 300;
const RETENTION: u64 = 7 * 24 * 3600;
const MAX_BYTES: u64 = 128 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ready,
    Busy,
    Waiting,
    Ended,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub id: Uuid,
    pub name: String,
    pub harness: String,
    pub status: Status,
    pub updated_at: u64,
    pub expires_at: u64,
    pub revision: u64,
}
impl Session {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.id.is_nil() && self.revision > 0,
            "invalid Agent session identity"
        );
        label(&self.name, 120)?;
        label(&self.harness, 80)?;
        ensure!(
            self.updated_at > 0 && self.expires_at.checked_sub(self.updated_at) == Some(LEASE),
            "invalid Agent session lease"
        );
        Ok(())
    }
    fn active(&self, at: u64) -> bool {
        self.status != Status::Ended
            && self.expires_at > at
            && self.updated_at <= at.saturating_add(30)
    }
}
fn label(value: &str, limit: usize) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control),
        "Agent labels must be plain text within their UTF-8 byte limit"
    );
    Ok(())
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    List {},
    Register {
        id: Uuid,
        name: String,
        harness: String,
    },
    Heartbeat {
        id: Uuid,
        revision: u64,
        status: Status,
    },
    End {
        id: Uuid,
        revision: u64,
    },
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    owner: String,
    sessions: BTreeMap<Uuid, Session>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub owner: String,
    pub observed_at: u64,
    pub lease_secs: u64,
    pub capacity: usize,
    pub sessions: Vec<Session>,
}
impl Catalog {
    pub(super) fn validate(&self, owner: &str) -> Result<()> {
        ensure!(
            self.owner == owner
                && valid_node_id(owner)
                && self.observed_at > 0
                && self.lease_secs == LEASE
                && self.capacity == CAPACITY
                && self.sessions.len() <= CAPACITY,
            "invalid Agent catalog identity or limits"
        );
        let mut ids = std::collections::BTreeSet::new();
        for session in &self.sessions {
            session.validate()?;
            ensure!(
                ids.insert(session.id) && session.updated_at <= self.observed_at.saturating_add(30),
                "invalid Agent catalog report"
            );
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct Registry {
    path: PathBuf,
    owner: String,
}
impl Registry {
    pub fn new(peers: &Path, owner: String) -> Self {
        Self {
            path: peers.with_file_name("agent-sessions.json"),
            owner,
        }
    }
    fn read(&self) -> Result<Ledger> {
        let file = match fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Ledger {
                    owner: self.owner.clone(),
                    sessions: BTreeMap::new(),
                });
            }
            Err(error) => return Err(error.into()),
        };
        ensure!(
            file.metadata()?.len() <= MAX_BYTES,
            "Agent registry is too large"
        );
        let ledger: Ledger = serde_json::from_reader(file)
            .context("read Agent registry; existing data was preserved")?;
        ensure!(
            ledger.owner == self.owner
                && valid_node_id(&ledger.owner)
                && ledger.sessions.len() <= CAPACITY,
            "Agent registry belongs to another node or exceeds capacity"
        );
        for (id, session) in &ledger.sessions {
            session.validate()?;
            ensure!(*id == session.id, "Agent session ID mismatch");
        }
        Ok(ledger)
    }
    pub fn catalog(&self) -> Result<Catalog> {
        Ok(Catalog {
            owner: self.owner.clone(),
            observed_at: now(),
            lease_secs: LEASE,
            capacity: CAPACITY,
            sessions: self.read()?.sessions.into_values().collect(),
        })
    }
    pub(crate) fn require_active(&self, id: Uuid) -> Result<()> {
        ensure!(
            self.read()?
                .sessions
                .get(&id)
                .is_some_and(|session| session.active(now())),
            "target Agent session must register or renew its expired report before claiming"
        );
        Ok(())
    }
    pub fn apply(&self, command: Command) -> Result<Catalog> {
        if matches!(command, Command::List {}) {
            return self.catalog();
        }
        let _lock = config::lock_credentials(&self.path)?;
        let mut ledger = self.read()?;
        let before = serde_json::to_vec(&ledger)?;
        let at = now();
        match command {
            Command::List {} => unreachable!(),
            Command::Register { id, name, harness } => {
                ensure!(!id.is_nil(), "Agent session requires a nonzero UUID");
                label(&name, 120)?;
                label(&harness, 80)?;
                if let Some(session) = ledger.sessions.get(&id) {
                    ensure!(
                        session.name == name && session.harness == harness,
                        "Agent session ID already registered with different labels"
                    );
                } else {
                    ledger
                        .sessions
                        .retain(|_, session| session.expires_at.saturating_add(RETENTION) > at);
                    ensure!(
                        ledger.sessions.len() < CAPACITY,
                        "Agent registry is full; old reports are retained for seven days"
                    );
                    ledger.sessions.insert(
                        id,
                        Session {
                            id,
                            name,
                            harness,
                            status: Status::Ready,
                            updated_at: at,
                            expires_at: at + LEASE,
                            revision: 1,
                        },
                    );
                }
            }
            Command::Heartbeat {
                id,
                revision,
                status,
            } => {
                ensure!(revision > 0, "Agent revision must be positive");
                ensure!(status != Status::Ended, "use end to close an Agent session");
                let session = ledger
                    .sessions
                    .get_mut(&id)
                    .context("register the Agent session first")?;
                ensure!(
                    session.status != Status::Ended,
                    "ended Agent sessions cannot be renewed; create a new session ID"
                );
                if session.revision == revision {
                    session.revision = revision
                        .checked_add(1)
                        .context("Agent revision exhausted")?;
                    session.status = status;
                    session.updated_at = at;
                    session.expires_at = at + LEASE;
                } else {
                    ensure!(
                        revision.checked_add(1) == Some(session.revision)
                            && session.status == status,
                        "stale Agent report; read the current revision"
                    );
                }
            }
            Command::End { id, revision } => {
                ensure!(revision > 0, "Agent revision must be positive");
                let session = ledger
                    .sessions
                    .get_mut(&id)
                    .context("Agent session unavailable")?;
                if session.status == Status::Ended {
                    ensure!(
                        revision.checked_add(1) == Some(session.revision),
                        "stale Agent report"
                    );
                } else {
                    ensure!(
                        session.revision == revision,
                        "stale Agent report; read the current revision"
                    );
                    session.revision = revision
                        .checked_add(1)
                        .context("Agent revision exhausted")?;
                    session.status = Status::Ended;
                    session.updated_at = at;
                    session.expires_at = at + LEASE;
                }
            }
        }
        let bytes = serde_json::to_vec(&ledger)?;
        ensure!(
            bytes.len() as u64 <= MAX_BYTES,
            "Agent registry is too large"
        );
        if before != bytes {
            config::write_private(&self.path, &bytes, true)?;
        }
        self.catalog()
    }
}
impl Peers {
    pub async fn agent_sessions(&self, node: Option<String>, command: Command) -> Result<Value> {
        if let Some(node) = node {
            ensure!(
                matches!(command, Command::List {}),
                "remote Agent registrations are read-only"
            );
            return self.remote_agents(node, None).await;
        }
        let peers = self.clone();
        tokio::task::spawn_blocking(move || {
            let _lock = config::lock_credentials(&peers.path)?;
            let configuration = Config::read(&peers.path)?;
            Ok(serde_json::to_value(
                Registry::new(&peers.path, configuration.node_id).apply(command)?,
            )?)
        })
        .await?
    }
    pub(crate) async fn agent_session_tool(&self, parameters: &Value) -> Result<Value> {
        let mut arguments = parameters["arguments"]
            .as_object()
            .context("Agent arguments must be an object")?
            .clone();
        let node = arguments
            .remove("node")
            .map(serde_json::from_value::<String>)
            .transpose()?;
        self.agent_sessions(node, serde_json::from_value(Value::Object(arguments))?)
            .await
    }
}
pub(crate) fn tools() -> Value {
    json!([{"name":"device_agent_sessions","description":"Register this Agent with a stable UUID, name and harness on this device. Registrations are explicit reports, not process discovery or execution authority. List with optional node reads an authorized connected device and may return an explicitly marked offline snapshot (never a current registration); remote writes are forbidden. Register is idempotent and does not renew an existing lease. Every two minutes call heartbeat with the current revision and ready/busy/waiting status; reports expire after five minutes. After an uncertain heartbeat, reuse its revision and status. Read before changing status. End permanently closes this ID; it does not stop a process or finish tasks. Reports include only public labels and status, never a path, transcript or credential. Receiving a task never starts an Agent; targeted tasks can only be claimed using their registered session UUID.","inputSchema":{"type":"object","oneOf":[
        {"type":"object","properties":{"operation":{"const":"list"},"node":{"type":"string"}},"required":["operation"],"additionalProperties":false},
        {"type":"object","properties":{"operation":{"const":"register"},"id":{"type":"string","format":"uuid"},"name":{"type":"string","maxLength":120},"harness":{"type":"string","maxLength":80}},"required":["operation","id","name","harness"],"additionalProperties":false},
        {"type":"object","properties":{"operation":{"const":"heartbeat"},"id":{"type":"string","format":"uuid"},"revision":{"type":"integer","minimum":1},"status":{"type":"string","enum":["ready","busy","waiting"]}},"required":["operation","id","revision","status"],"additionalProperties":false},
        {"type":"object","properties":{"operation":{"const":"end"},"id":{"type":"string","format":"uuid"},"revision":{"type":"integer","minimum":1}},"required":["operation","id","revision"],"additionalProperties":false}
    ]}}])
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, Registry, Uuid) {
        let root = tempfile::tempdir().unwrap();
        let store = Registry::new(&root.path().join("peers.json"), "device-a".into());
        (root, store, Uuid::new_v4())
    }
    fn register(id: Uuid) -> Command {
        Command::Register {
            id,
            name: "Project reviewer".into(),
            harness: "custom-cli".into(),
        }
    }
    #[test]
    fn registration_is_persistent_and_duplicates_do_not_resurrect_or_renew() {
        let (root, store, id) = setup();
        store.apply(register(id)).unwrap();
        let saved = fs::read(&store.path).unwrap();
        store.apply(register(id)).unwrap();
        assert_eq!(fs::read(&store.path).unwrap(), saved);
        assert!(store.require_active(id).is_ok());
        store
            .apply(Command::Heartbeat {
                id,
                revision: 1,
                status: Status::Busy,
            })
            .unwrap();
        let busy = fs::read(&store.path).unwrap();
        store
            .apply(Command::Heartbeat {
                id,
                revision: 1,
                status: Status::Busy,
            })
            .unwrap();
        assert_eq!(fs::read(&store.path).unwrap(), busy);
        assert!(
            store
                .apply(Command::Heartbeat {
                    id,
                    revision: 1,
                    status: Status::Ready
                })
                .is_err()
        );
        store.apply(Command::End { id, revision: 2 }).unwrap();
        store.apply(Command::End { id, revision: 2 }).unwrap();
        store.apply(register(id)).unwrap();
        assert!(store.require_active(id).is_err());
        assert!(
            store
                .apply(Command::Heartbeat {
                    id,
                    revision: 3,
                    status: Status::Ready
                })
                .is_err()
        );
        assert_eq!(
            Registry::new(&root.path().join("peers.json"), "device-a".into())
                .catalog()
                .unwrap()
                .sessions[0]
                .status,
            Status::Ended
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&store.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn stale_sessions_require_explicit_renewal_and_concurrent_reports_cannot_overwrite() {
        let (_root, store, id) = setup();
        store.apply(register(id)).unwrap();
        let mut ledger = store.read().unwrap();
        let session = ledger.sessions.get_mut(&id).unwrap();
        let at = now();
        session.updated_at = at - LEASE - 1;
        session.expires_at = at - 1;
        config::write_private(&store.path, &serde_json::to_vec(&ledger).unwrap(), true).unwrap();
        assert!(store.require_active(id).is_err());
        store.apply(register(id)).unwrap();
        assert!(store.require_active(id).is_err());
        let copies = [Status::Busy, Status::Waiting]
            .into_iter()
            .map(|status| {
                let copy = store.clone();
                std::thread::spawn(move || {
                    copy.apply(Command::Heartbeat {
                        id,
                        revision: 1,
                        status,
                    })
                    .is_ok()
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            copies
                .into_iter()
                .map(|thread| usize::from(thread.join().unwrap()))
                .sum::<usize>(),
            1
        );
        assert!(store.require_active(id).is_ok());
    }
    #[test]
    fn corrupt_or_wrong_owner_registries_are_preserved() {
        let (_root, store, id) = setup();
        store.apply(register(id)).unwrap();
        assert!(
            Registry {
                path: store.path.clone(),
                owner: "other-device".into()
            }
            .catalog()
            .is_err()
        );
        fs::write(&store.path, b"{broken").unwrap();
        assert!(store.apply(register(Uuid::new_v4())).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), b"{broken");
    }
    #[test]
    fn catalog_and_labels_are_bounded_and_validate_identity() {
        let (_root, store, id) = setup();
        assert!(
            store
                .apply(Command::Register {
                    id,
                    name: "🙂".repeat(31),
                    harness: "cli".into()
                })
                .is_err()
        );
        assert!(
            store
                .apply(Command::Register {
                    id,
                    name: "name\nsecret".into(),
                    harness: "cli".into()
                })
                .is_err()
        );
        for _ in 0..CAPACITY {
            store.apply(register(Uuid::new_v4())).unwrap();
        }
        assert!(store.apply(register(id)).is_err());
        assert!(
            serde_json::from_value::<Command>(json!({"operation":"list","unknown":true})).is_err()
        );
        assert!(
            store
                .apply(Command::Heartbeat {
                    id: store.catalog().unwrap().sessions[0].id,
                    revision: 0,
                    status: Status::Ready
                })
                .is_err()
        );
        let mut catalog = store.catalog().unwrap();
        catalog.validate("device-a").unwrap();
        assert!(catalog.validate("other-device").is_err());
        catalog.sessions.push(catalog.sessions[0].clone());
        assert!(catalog.validate("device-a").is_err());
    }
}
