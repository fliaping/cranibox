//! Explicit, read-only views of a node's active collaboration records.
//! The source remains authoritative. A digest identifies content, not a global clock or signature.
use super::{Store, config::Config, messages, tasks};
use crate::{config, protocol::valid_node_id};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path};

// The source ledgers are independently bounded to 16 MiB and 8 MiB.
pub const MAX_WIRE: usize = 24 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Incoming,
    Outgoing,
}
impl Direction {
    fn validate(self, owner: &str, sender: &str, recipient: &str) -> Result<()> {
        ensure!(
            match self {
                Self::Incoming => recipient == owner,
                Self::Outgoing => sender == owner,
            },
            "activity participant does not match its source"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub direction: Direction,
    pub task: tasks::Task,
    pub current: tasks::Receipt,
    pub cancel_requested: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub direction: Direction,
    pub message: messages::Envelope,
    pub status: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub scope: tasks::collaborations::Scope,
    pub members: Vec<String>,
    pub state: String,
    pub revision: u64,
    pub created_at: u64,
    pub result: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub version: u32,
    pub owner: String,
    pub observed_at: u64,
    pub digest: String,
    pub tasks: Vec<Task>,
    pub messages: Vec<Message>,
    pub runs: Vec<Run>,
}
impl Snapshot {
    fn digest(&self) -> Result<String> {
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                self.version,
                &self.owner,
                &self.tasks,
                &self.messages,
                &self.runs,
            ))?)
        ))
    }
    pub fn validate(&self, owner: &str) -> Result<()> {
        ensure!(
            self.version == 1
                && valid_node_id(owner)
                && self.owner == owner
                && self.observed_at > 0
                && self.observed_at <= 253402300799,
            "unexpected activity source or protocol"
        );
        ensure!(
            self.tasks.len() <= 128 && self.messages.len() <= 256 && self.runs.len() <= 32,
            "activity capacity exceeded"
        );
        let mut ids = BTreeSet::new();
        for item in &self.tasks {
            item.task.validate()?;
            item.current.validate(&item.task)?;
            item.direction
                .validate(owner, &item.task.sender, &item.task.recipient)?;
            ensure!(ids.insert(item.task.id), "duplicate activity task");
        }
        ids.clear();
        for item in &self.messages {
            item.message.validate()?;
            item.direction
                .validate(owner, &item.message.sender, &item.message.recipient)?;
            ensure!(ids.insert(item.message.id), "duplicate activity message");
            ensure!(
                match item.direction {
                    Direction::Incoming =>
                        matches!(item.status.as_str(), "received" | "acknowledged"),
                    Direction::Outgoing => matches!(
                        item.status.as_str(),
                        "queued" | "received" | "acknowledged" | "blocked" | "expired"
                    ),
                },
                "invalid activity message status"
            );
        }
        ids.clear();
        for run in &self.runs {
            run.scope.validate(owner)?;
            ensure!(
                ids.insert(run.scope.id)
                    && run.revision > 0
                    && run.created_at > 0
                    && run.created_at <= 253402300799
                    && matches!(run.state.as_str(), "open" | "stopping" | "closed")
                    && (run.state == "closed") == run.result.is_some(),
                "invalid activity collaboration"
            );
            let members: BTreeSet<_> = run.members.iter().collect();
            ensure!(
                !members.is_empty()
                    && members.len() <= 16
                    && members.len() == run.members.len()
                    && members
                        .iter()
                        .all(|node| valid_node_id(node) && *node != owner),
                "invalid activity collaboration members"
            );
            if let Some(result) = &run.result {
                ensure!(
                    !result.trim().is_empty()
                        && result.len() <= 8192
                        && result
                            .chars()
                            .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t')),
                    "invalid activity collaboration result"
                );
            }
        }
        ensure!(
            self.digest == self.digest()?,
            "activity content digest mismatch"
        );
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_WIRE,
            "activity response is too large"
        );
        Ok(())
    }
}

/// Caller holds the peer configuration lock. Each atomic ledger read has its own observation;
/// the combined response is deliberately not represented as an atomic transaction across ledgers.
pub fn local(path: &Path, configuration: &Config) -> Result<Snapshot> {
    let owner = &configuration.node_id;
    let (tasks, runs) = tasks::Store::new(path, owner.clone()).activity()?;
    let messages = messages::Store::new(path, owner.clone()).activity()?;
    let mut snapshot = Snapshot {
        version: 1,
        owner: owner.clone(),
        observed_at: messages::now(),
        digest: String::new(),
        tasks,
        messages,
        runs,
    };
    snapshot.digest = snapshot.digest()?;
    snapshot.validate(owner)?;
    Ok(snapshot)
}

impl Store {
    pub async fn activity(&self, node: Option<String>) -> Result<Value> {
        enum Read {
            Local(Value),
            Remote {
                target: String,
                route: super::config::Route,
                binding: String,
            },
        }
        let path = self.path.clone();
        let read = tokio::task::spawn_blocking(move || -> Result<Read> {
            let _lock = config::lock_credentials(&path)?;
            let configuration = Config::read(&path)?;
            let target = node.unwrap_or_else(|| configuration.node_id.clone());
            ensure!(valid_node_id(&target), "invalid activity target");
            if target == configuration.node_id {
                return Ok(Read::Local(serde_json::to_value(local(
                    &path,
                    &configuration,
                )?)?));
            }
            let binding = messages::binding(&configuration, &target)?;
            let route = configuration.routes[&target].clone();
            Ok(Read::Remote {
                target,
                route,
                binding,
            })
        })
        .await??;
        let (target, route, binding) = match read {
            Read::Remote {
                target,
                route,
                binding,
            } => (target, route, binding),
            Read::Local(local) => return Ok(local),
        };
        let response = route
            .api(&target)?
            .activity(&format!("/api/v1/nodes/{target}/activity"))
            .await?;
        let snapshot: Snapshot = serde_json::from_value(response)?;
        snapshot.validate(&target)?;
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            let _lock = config::lock_credentials(&path)?;
            let current = Config::read(&path)?;
            ensure!(
                messages::binding(&current, &target)? == binding,
                "connection or credentials changed; read activity again"
            );
            Ok(serde_json::to_value(snapshot)?)
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn snapshot() -> Snapshot {
        let id = Uuid::new_v4();
        let mut snapshot = Snapshot {
            version: 1,
            owner: "source-node".into(),
            observed_at: 100,
            digest: String::new(),
            tasks: vec![],
            messages: vec![Message {
                direction: Direction::Incoming,
                message: messages::Envelope {
                    id,
                    thread: id,
                    in_reply_to: None,
                    sender: "another-node".into(),
                    recipient: "source-node".into(),
                    sender_session: None,
                    target_session: None,
                    text: "A report, not a command".into(),
                    created_at: 100,
                    expires_at: 100 + messages::TTL,
                },
                status: "received".into(),
            }],
            runs: vec![],
        };
        snapshot.digest = snapshot.digest().unwrap();
        snapshot
    }

    #[test]
    fn repeated_observations_keep_content_identity_without_renewing_record_time() {
        let mut snapshot = snapshot();
        snapshot.validate("source-node").unwrap();
        snapshot.observed_at += 60;
        snapshot.validate("source-node").unwrap();
        assert_eq!(snapshot.messages[0].message.created_at, 100);
        snapshot.messages[0].message.text = "A revised report".into();
        assert!(snapshot.validate("source-node").is_err());
        snapshot.digest = snapshot.digest().unwrap();
        snapshot.validate("source-node").unwrap();
    }

    #[test]
    fn even_self_consistent_content_cannot_change_source_direction_or_duplicate_ids() {
        let snapshot = snapshot();
        let mut invalid = snapshot.clone();
        invalid.messages[0].direction = Direction::Outgoing;
        invalid.digest = invalid.digest().unwrap();
        assert!(invalid.validate("source-node").is_err());
        let mut duplicate = snapshot.clone();
        duplicate.messages.push(duplicate.messages[0].clone());
        duplicate.digest = duplicate.digest().unwrap();
        assert!(duplicate.validate("source-node").is_err());
        assert!(snapshot.validate("another-node").is_err());
        let mut unknown = serde_json::to_value(snapshot).unwrap();
        unknown["messages"][0]["credential"] = serde_json::json!("private");
        assert!(serde_json::from_value::<Snapshot>(unknown).is_err());
    }
}
