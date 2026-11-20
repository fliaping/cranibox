//! Who may use whom. Every device knows its own relations (from the pairing it made,
//! or the direction of its grants and routes); owner devices share theirs with each
//! other, bounded like path gossip, so each one can draw the whole network.
use agent_workspace_runtime::peers::config::Config;
use anyhow::Result;
use cranibox_contracts::app::Relation;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, io::Read, path::Path};

use super::known::Known;

const LIMIT: usize = 64;
const MAX_BYTES: u64 = 1024 * 1024;
const MAX_NAME: usize = 80;
const CLOCK_SKEW: u64 = 60_000;

/// The relation of a paired device that is not one of the owner's (roster) devices.
/// `space`: a virtual space this device enrolled. A remembered pairing wins; older
/// pairings are read from the direction of their grant and route.
pub fn of(config: &Config, known: Option<&Known>, node: &str, space: bool) -> Relation {
    if space {
        return Relation::Controlled;
    }
    if let Some(relation) = known.and_then(|k| k.relation) {
        return relation;
    }
    match (config.grants.get(node), config.routes.contains_key(node)) {
        // An older mutual pairing: another identity's device, or (messages only) a guest.
        (Some(grant), true) if grant.permissions.exec && grant.permissions.terminals => {
            Relation::Peer
        }
        (Some(_), _) => Relation::Guest,
        (None, _) => Relation::Host,
    }
}

/// One relation a device reports: the other device, its name and how they relate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub node: String,
    pub name: String,
    pub relation: Relation,
}
/// What one device reported, and when (Unix milliseconds).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub at: u64,
    pub edges: Vec<Edge>,
}
pub type Reports = BTreeMap<String, Report>;

fn valid_id(id: &str) -> bool {
    agent_workspace_runtime::protocol::valid_node_id(id)
}
fn clean(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect()
}

/// This device's relations to devices outside the owner's roster.
pub fn local(peers: &Path) -> Vec<Edge> {
    let Ok(config) = Config::read(peers) else {
        return vec![];
    };
    let roster = agent_workspace_runtime::peers::roster::RosterStore::load(
        peers.with_file_name("models").join("roster.json"),
    )
    .ok();
    let owner = |node: &str| {
        use cranibox_contracts::roster::RosterView;
        roster.as_ref().is_some_and(|r| r.is_owner_node(node))
    };
    let known = super::known::load(peers);
    let spaces: Vec<String> = crate::spaces::Store::new(peers.to_path_buf())
        .enrolled(&config)
        .into_iter()
        .map(|space| space.node)
        .collect();
    let mut nodes: Vec<&String> = config.grants.keys().chain(config.routes.keys()).collect();
    nodes.sort();
    nodes.dedup();
    nodes
        .into_iter()
        .filter(|node| **node != config.node_id && !owner(node))
        .filter_map(|node| {
            let relation = of(&config, known.get(node), node, spaces.contains(node));
            (relation != Relation::Peer).then(|| Edge {
                node: node.clone(),
                name: clean(
                    &known
                        .get(node)
                        .map(|k| k.display(node))
                        .unwrap_or_else(|| node.clone()),
                ),
                relation,
            })
        })
        .take(LIMIT)
        .collect()
}

fn merge_into(reports: &mut Reports, remote: &Value, own: &str, now: u64) {
    let Some(remote) = remote.as_object() else {
        return;
    };
    for (from, raw) in remote.iter().take(LIMIT) {
        if from == own || !valid_id(from) {
            continue;
        }
        let Ok(mut report) = serde_json::from_value::<Report>(raw.clone()) else {
            continue;
        };
        if report.at == 0 || report.at > now.saturating_add(CLOCK_SKEW) {
            continue;
        }
        report
            .edges
            .retain(|e| valid_id(&e.node) && e.node != *from);
        report.edges.truncate(LIMIT);
        for edge in &mut report.edges {
            edge.name = clean(&edge.name);
        }
        if reports.get(from).is_none_or(|old| report.at > old.at) {
            reports.insert(from.clone(), report);
        }
    }
    while reports.len() > LIMIT {
        let oldest = reports
            .iter()
            .min_by_key(|(_, r)| r.at)
            .map(|(id, _)| id.clone())
            .unwrap();
        reports.remove(&oldest);
    }
}

fn file(peers: &Path) -> std::path::PathBuf {
    peers.with_file_name("relations.json")
}
/// Other owner devices' reports, as last received.
pub fn load(peers: &Path) -> Reports {
    let read = || -> Result<Value> {
        let file = std::fs::File::open(file(peers))?;
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() as u64 <= MAX_BYTES, "relation store too large");
        Ok(serde_json::from_slice(&bytes)?)
    };
    let mut reports = Reports::new();
    if let Ok(value) = read() {
        merge_into(&mut reports, &value, "", super::paths::now());
    }
    reports
}
/// Merge reports an owner device sent; this device's own report is never replaced.
pub fn merge(peers: &Path, remote: &Value, own: &str) -> Result<()> {
    let path = file(peers);
    let _lock = agent_workspace_runtime::config::lock_credentials(&path)?;
    let mut reports = load(peers);
    let before = reports.clone();
    merge_into(&mut reports, remote, own, super::paths::now());
    reports.remove(own);
    if reports != before {
        agent_workspace_runtime::config::write_private(
            &path,
            &serde_json::to_vec(&reports)?,
            true,
        )?;
    }
    Ok(())
}
/// What this device tells another owner device: its own report and the ones it holds.
pub fn answer(peers: &Path, own: &str) -> Value {
    let mut reports = load(peers);
    reports.insert(
        own.to_owned(),
        Report {
            at: super::paths::now(),
            edges: local(peers),
        },
    );
    serde_json::to_value(reports).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const A: &str = "node-0123456789abcdef0123456789abcdef";
    const B: &str = "node-fedcba9876543210fedcba9876543210";
    const W: &str = "node-00000000000000000000000000000001";

    #[test]
    fn newer_reports_win_and_own_and_invalid_rows_are_dropped() {
        let mut reports = Reports::new();
        let now = 1_800_000_000_000;
        let report = |at: u64, name: &str| {
            json!({"at": at, "edges": [{"node": W, "name": name, "relation": "controlled"},
                                        {"node": "bad id", "name": "x", "relation": "guest"}]})
        };
        merge_into(&mut reports, &json!({B: report(now - 10, "old")}), A, now);
        merge_into(&mut reports, &json!({B: report(now - 20, "older")}), A, now);
        merge_into(
            &mut reports,
            &json!({A: report(now, "forged"), "not an id": report(now, "x")}),
            A,
            now,
        );
        assert_eq!(reports.len(), 1);
        assert_eq!(
            reports[B].edges,
            [Edge {
                node: W.into(),
                name: "old".into(),
                relation: Relation::Controlled
            }]
        );
        merge_into(
            &mut reports,
            &json!({B: report(now + CLOCK_SKEW + 1, "future")}),
            A,
            now,
        );
        assert_eq!(reports[B].edges[0].name, "old");
        merge_into(&mut reports, &json!({B: report(now, "new\u{7}")}), A, now);
        assert_eq!(reports[B].edges[0].name, "new");
    }
}
