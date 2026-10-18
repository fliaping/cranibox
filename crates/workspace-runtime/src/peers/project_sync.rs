//! Receiver-owned background pulls of authoritative project and Agent metadata.
//! Membership and a saved route never opt a device into background reads.
use super::{
    Store,
    config::{Config, Route},
    projects,
};
use crate::{config, protocol::valid_node_id};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_FILE: u64 = 64 * 1024;
pub const MAX_PARALLEL: usize = 4;

/// Separate opt-in policies share bounded scheduling, never authorization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Projects,
    Agents,
}
impl Kind {
    fn policy(self) -> &'static str {
        match self {
            Self::Projects => "project-sync.json",
            Self::Agents => "agent-sync.json",
        }
    }
    fn journal(self) -> &'static str {
        match self {
            Self::Projects => "project-sync-state.json",
            Self::Agents => "agent-sync-state.json",
        }
    }
    fn entries(self) -> &'static str {
        match self {
            Self::Projects => "projects",
            Self::Agents => "sessions",
        }
    }
    fn count(self) -> &'static str {
        match self {
            Self::Projects => "project_count",
            Self::Agents => "session_count",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Status,
    Configure {
        revision: u64,
        nodes: Vec<String>,
        interval_seconds: u32,
    },
    Pause {
        revision: u64,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    version: u32,
    owner: String,
    revision: u64,
    enabled: bool,
    interval_seconds: u32,
    nodes: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Waiting,
    Checking,
    Online,
    Offline,
    Unavailable,
    Error,
    CacheError,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Outcome {
    revision: u64,
    binding: String,
    phase: Phase,
    attempted_at: u64,
    completed_at: Option<u64>,
    last_success: Option<u64>,
    catalog_revision: Option<u64>,
    #[serde(rename = "project_count")]
    entry_count: Option<usize>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    owner: String,
    nodes: BTreeMap<String, Outcome>,
}

pub struct Job {
    pub node: String,
    pub kind: Kind,
    owner: String,
    revision: u64,
    binding: String,
}

fn now() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}
fn interval(value: u32) -> Result<()> {
    ensure!(
        [15, 30, 60, 300].contains(&value),
        "choose a catalog sync interval of 15, 30, 60 or 300 seconds"
    );
    Ok(())
}
fn hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
pub(super) fn binding(owner: &str, node: &str, route: &Route) -> Result<String> {
    // A forgotten connection recreated with the same target token must still
    // require selection again. Managed route credentials get a fresh filename.
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            projects::fingerprint(owner, node, route)?,
            &route.token_file,
        ))?)
    ))
}
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        file.metadata()?.len() <= MAX_FILE,
        "catalog sync data is too large"
    );
    Ok(Some(serde_json::from_reader(file).context(
        "invalid catalog sync data; existing files were preserved",
    )?))
}
fn settings(path: &Path, owner: &str, kind: Kind) -> Result<Settings> {
    let value: Settings = read(&path.with_file_name(kind.policy()))?.unwrap_or_else(|| Settings {
        version: 1,
        owner: owner.into(),
        revision: 0,
        enabled: false,
        interval_seconds: 60,
        nodes: BTreeMap::new(),
    });
    interval(value.interval_seconds)?;
    ensure!(
        value.version == 1
            && value.owner == owner
            && value.nodes.len() <= 32
            && (!value.enabled || !value.nodes.is_empty())
            && value
                .nodes
                .iter()
                .all(|(node, binding)| valid_node_id(node) && node != owner && hash(binding)),
        "invalid catalog sync policy or owner"
    );
    Ok(value)
}
fn journal(path: &Path, owner: &str, kind: Kind) -> Result<Journal> {
    let value: Journal = read(&path.with_file_name(kind.journal()))?.unwrap_or_else(|| Journal {
        version: 1,
        owner: owner.into(),
        nodes: BTreeMap::new(),
    });
    ensure!(
        value.version == 1
            && value.owner == owner
            && value.nodes.len() <= 32
            && value.nodes.iter().all(|(node, outcome)| valid_node_id(node)
                && hash(&outcome.binding)
                && outcome.attempted_at > 0
                && outcome.attempted_at <= 253_402_300_799_000
                && [outcome.completed_at, outcome.last_success]
                    .iter()
                    .flatten()
                    .all(|value| *value > 0 && *value <= 253_402_300_799_000)),
        "invalid catalog sync journal or owner"
    );
    Ok(value)
}
fn write<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() as u64 <= MAX_FILE,
        "catalog sync data is too large"
    );
    config::write_private(path, &bytes, true)
}
fn matches(settings: &Settings, configuration: &Config, node: &str) -> bool {
    configuration
        .routes
        .get(node)
        .and_then(|route| binding(&configuration.node_id, node, route).ok())
        .is_some_and(|binding| settings.nodes.get(node) == Some(&binding))
}
fn view(settings: &Settings, configuration: &Config, journal: &Journal, kind: Kind) -> Value {
    json!({
        "owner":settings.owner, "revision":settings.revision, "enabled":settings.enabled,
        "interval_seconds":settings.interval_seconds,
        "available_nodes":configuration.routes.keys().collect::<Vec<_>>(),
        "nodes":settings.nodes.iter().map(|(node, binding)| {
            let valid = matches(settings, configuration, node);
            let outcome = journal.nodes.get(node).filter(|outcome| &outcome.binding == binding && outcome.revision == settings.revision);
            json!({
                "node":node, "connection_valid":valid,
                "phase":if !valid {json!("connection_changed")} else if !settings.enabled {json!("paused")} else {outcome.map_or(json!("waiting"), |o| json!(o.phase))},
                "last_attempt":outcome.map(|o| o.attempted_at),
                "last_success":outcome.and_then(|o| o.last_success),
                "catalog_revision":outcome.and_then(|o| o.catalog_revision),
                (kind.count()):outcome.and_then(|o| o.entry_count),
            })
        }).collect::<Vec<_>>(),
    })
}

/// Only local operator entry points call this function. It is not exposed by
/// the inbound peer capability API or the task-scoped MCP adapter.
pub fn manage(path: &Path, command: Command) -> Result<Value> {
    manage_kind(path, command, Kind::Projects)
}
pub fn manage_kind(path: &Path, command: Command, kind: Kind) -> Result<Value> {
    // All users of both locks take the peer lock first; none hold them over I/O.
    let _peer = config::lock_credentials(path)?;
    let configuration = Config::read(path)?;
    let _guard = config::lock_credentials(&path.with_file_name(kind.policy()))?;
    let mut saved = settings(path, &configuration.node_id, kind)?;
    let mut state = journal(path, &configuration.node_id, kind)?;
    let revision = match &command {
        Command::Status => None,
        Command::Configure { revision, .. } | Command::Pause { revision } => Some(*revision),
    };
    if let Some(revision) = revision {
        ensure!(
            saved.revision == revision,
            "catalog sync settings changed; reload and review the selected devices again"
        );
    }
    match command {
        Command::Status => {}
        Command::Configure {
            nodes,
            interval_seconds,
            ..
        } => {
            interval(interval_seconds)?;
            ensure!(
                !nodes.is_empty() && nodes.len() <= 32,
                "select one to 32 catalog sync devices"
            );
            let mut bindings = BTreeMap::new();
            for node in nodes {
                ensure!(
                    valid_node_id(&node) && node != configuration.node_id,
                    "choose a different valid catalog sync device"
                );
                let route = configuration
                    .routes
                    .get(&node)
                    .context("selected catalog sync device is no longer connected")?;
                let binding = binding(&configuration.node_id, &node, route)?;
                ensure!(
                    bindings.insert(node, binding).is_none(),
                    "select distinct catalog sync devices"
                );
            }
            saved.nodes = bindings;
            saved.interval_seconds = interval_seconds;
            saved.enabled = true;
        }
        Command::Pause { .. } => saved.enabled = false,
    }
    if revision.is_some() {
        saved.revision = saved
            .revision
            .checked_add(1)
            .context("catalog sync revision exhausted")?;
        // Publish the policy before its derived journal. A crash between the
        // writes ignores old outcomes by revision and safely retries reads.
        write(&path.with_file_name(kind.policy()), &saved)?;
        state.nodes.retain(|node, outcome| {
            if saved.nodes.get(node) != Some(&outcome.binding) {
                return false;
            }
            outcome.revision = saved.revision;
            // A resumed or edited configuration gets a fresh background cycle.
            outcome.phase = Phase::Waiting;
            outcome.completed_at = None;
            true
        });
        write(&path.with_file_name(kind.journal()), &state)?;
    }
    Ok(view(&saved, &configuration, &state, kind))
}

/// Cheap bounded scheduling; the node service limits concurrent network pulls.
pub fn pending(path: &Path) -> Result<Vec<Job>> {
    pending_kind(path, Kind::Projects)
}
pub fn pending_kind(path: &Path, kind: Kind) -> Result<Vec<Job>> {
    let _peer = config::lock_credentials(path)?;
    let configuration = Config::read(path)?;
    let _guard = config::lock_credentials(&path.with_file_name(kind.policy()))?;
    let saved = settings(path, &configuration.node_id, kind)?;
    if !saved.enabled {
        return Ok(vec![]);
    }
    let state = journal(path, &configuration.node_id, kind)?;
    let now = now()?;
    Ok(saved
        .nodes
        .iter()
        .filter(|(node, binding)| {
            matches(&saved, &configuration, node)
                && state.nodes.get(*node).is_none_or(|outcome| {
                    outcome.revision != saved.revision
                        || &outcome.binding != *binding
                        || outcome.completed_at.is_none_or(|at| {
                            at > now
                                || now.saturating_sub(at)
                                    >= u64::from(saved.interval_seconds) * 1000
                        })
                })
        })
        .map(|(node, binding)| Job {
            node: node.clone(),
            kind,
            owner: configuration.node_id.clone(),
            revision: saved.revision,
            binding: binding.clone(),
        })
        .collect())
}
fn current(path: &Path, job: &Job) -> Result<Option<(Settings, Journal)>> {
    let kind = job.kind;
    let configuration = Config::read(path)?;
    let saved = settings(path, &configuration.node_id, kind)?;
    if configuration.node_id != job.owner
        || !saved.enabled
        || saved.revision != job.revision
        || saved.nodes.get(&job.node) != Some(&job.binding)
        || !matches(&saved, &configuration, &job.node)
    {
        return Ok(None);
    }
    Ok(Some((saved, journal(path, &configuration.node_id, kind)?)))
}

pub async fn run(path: &Path, job: Job) -> Result<()> {
    let kind = job.kind;
    let peer_path = path.to_path_buf();
    let node = job.node.clone();
    let binding = job.binding.clone();
    let started = {
        let path = peer_path.clone();
        tokio::task::spawn_blocking(move || {
            let _peer = config::lock_credentials(&path)?;
            let _guard = config::lock_credentials(&path.with_file_name(kind.policy()))?;
            let Some((_, mut state)) = current(&path, &job)? else {
                return Ok(None);
            };
            let previous = state
                .nodes
                .remove(&job.node)
                .filter(|outcome| outcome.binding == job.binding);
            let outcome = Outcome {
                revision: job.revision,
                binding: job.binding.clone(),
                phase: Phase::Checking,
                attempted_at: now()?,
                completed_at: None,
                last_success: previous.as_ref().and_then(|o| o.last_success),
                catalog_revision: previous.as_ref().and_then(|o| o.catalog_revision),
                entry_count: previous.and_then(|o| o.entry_count),
            };
            state.nodes.insert(job.node.clone(), outcome);
            write(&path.with_file_name(kind.journal()), &state)?;
            Ok::<_, anyhow::Error>(Some(job))
        })
        .await??
    };
    let Some(job) = started else {
        return Ok(());
    };
    let result = tokio::time::timeout(std::time::Duration::from_secs(40), async {
        let store = Store::new(peer_path.clone());
        match kind {
            Kind::Projects => store.background_projects(node, binding).await,
            Kind::Agents => store.background_agents(node, binding).await,
        }
    })
    .await;
    tokio::task::spawn_blocking(move || {
        let _peer = config::lock_credentials(&peer_path)?;
        let _guard = config::lock_credentials(&peer_path.with_file_name(kind.policy()))?;
        let Some((_, mut state)) = current(&peer_path, &job)? else {
            return Ok(());
        };
        let outcome = state
            .nodes
            .get_mut(&job.node)
            .context("catalog sync attempt disappeared")?;
        outcome.completed_at = Some(now()?);
        outcome.phase = match result {
            Ok(Err(error)) if error.is::<projects::RefreshBusy>() => {
                // An interactive request owns this device's snapshot lock.
                // Retry after the interval so other due devices get a turn.
                Phase::Waiting
            }
            Ok(Ok(value)) if value["sync"]["state"] == "offline" => Phase::Offline,
            Ok(Ok(value)) => {
                outcome.last_success = outcome.completed_at;
                outcome.catalog_revision = value["revision"].as_u64();
                outcome.entry_count = value[kind.entries()].as_array().map(Vec::len);
                if value["sync"]["warning"].is_string() {
                    Phase::CacheError
                } else {
                    Phase::Online
                }
            }
            Ok(Err(error)) if crate::client::unavailable(&error) => Phase::Unavailable,
            Err(_) => Phase::Unavailable,
            Ok(Err(_)) => Phase::Error,
        };
        write(&peer_path.with_file_name(kind.journal()), &state)
    })
    .await?
}
