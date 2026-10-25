//! Remembered details of paired devices (`devices.json` beside `peers.json`), so offline
//! devices keep their names and the time they were last seen.
use anyhow::Result;
use cranibox_contracts::app::DeviceKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Known {
    /// The name the device reports for itself.
    pub name: Option<String>,
    /// A name the owner chose here; wins over `name`.
    pub alias: Option<String>,
    /// When `alias` was last set or cleared (ms). Names sync between owner devices; the
    /// latest change wins, and a cleared name stays as a dated blank so it wins too.
    pub alias_at: Option<u64>,
    pub kind: Option<String>,
    pub os_version: Option<String>,
    pub model: Option<String>,
    pub last_seen: Option<u64>,
    /// What the device last said it shares with this one (`desktop`, `computer`,
    /// `browser`): granted to us and configured on that device.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub offers: BTreeMap<String, bool>,
}
impl Known {
    pub fn display(&self, node: &str) -> String {
        self.alias
            .clone()
            .or_else(|| self.name.clone())
            .unwrap_or_else(|| node.to_owned())
    }
    /// Whether the device shares `key`; unknown until it was probed once.
    pub fn offers(&self, key: &str) -> bool {
        self.offers.get(key).copied().unwrap_or(true)
    }
    pub fn kind(&self) -> DeviceKind {
        match self.kind.as_deref() {
            Some("laptop") => DeviceKind::Laptop,
            Some("server") => DeviceKind::Server,
            Some("desktop") => DeviceKind::Desktop,
            _ => DeviceKind::Unknown,
        }
    }
    /// Take the self-description from a peer `/api/v1/nodes` answer.
    pub fn learn(&mut self, answer: &Value, now: u64) {
        let node = &answer["nodes"][0];
        let text = |key: &str| {
            node[key]
                .as_str()
                .map(|s| s.chars().take(128).collect::<String>())
                .filter(|s| !s.is_empty())
        };
        // Older nodes report their ID as name; keep only a real name.
        if let Some(name) = text("name").filter(|n| Some(n.as_str()) != node["id"].as_str()) {
            self.name = Some(name);
        }
        self.kind = text("kind").or(self.kind.take());
        self.os_version = text("os_version").or(self.os_version.take());
        self.model = text("model").or(self.model.take());
        for key in ["desktop", "computer", "browser"] {
            if let Some(offered) = node[key].as_bool() {
                self.offers.insert(key.to_owned(), offered);
            }
        }
        self.last_seen = Some(now);
    }
}

/// The names this device knows, for owner devices to merge: `{node: {label, at}}`.
pub fn labels(peers: &Path) -> Value {
    load(peers)
        .into_iter()
        .filter_map(|(node, known)| {
            let at = known.alias_at?;
            Some((
                node,
                serde_json::json!({"label": known.alias.unwrap_or_default(), "at": at}),
            ))
        })
        .collect::<serde_json::Map<_, _>>()
        .into()
}

/// Take newer names from an owner device's answer. Only the owner's own devices may rename.
pub fn merge_labels(peers: &Path, remote: &Value) -> Result<()> {
    let Some(remote) = remote.as_object() else {
        return Ok(());
    };
    for (node, entry) in remote.iter().take(256) {
        let valid =
            node.len() <= 96 && node.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        let (Some(label), Some(at)) = (entry["label"].as_str(), entry["at"].as_u64()) else {
            continue;
        };
        if !valid || label.chars().count() > 64 {
            continue;
        }
        update(peers, node, |k| {
            if k.alias_at.is_none_or(|have| at > have) {
                k.alias = (!label.trim().is_empty()).then(|| label.trim().to_owned());
                k.alias_at = Some(at);
            }
        })?;
    }
    Ok(())
}

pub fn path(peers: &Path) -> std::path::PathBuf {
    peers.with_file_name("devices.json")
}
pub fn load(peers: &Path) -> BTreeMap<String, Known> {
    std::fs::read(path(peers))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}
/// Apply `edit` to one device's record and save it if anything changed.
pub fn update(peers: &Path, node: &str, edit: impl FnOnce(&mut Known)) -> Result<()> {
    let mut all = load(peers);
    let before = all.get(node).cloned();
    edit(all.entry(node.to_owned()).or_default());
    if all.get(node) != before.as_ref() {
        agent_workspace_runtime::config::write_private(
            &path(peers),
            &serde_json::to_vec_pretty(&all)?,
            true,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn learns_real_names_and_keeps_the_owners_alias() {
        let dir = tempfile::tempdir().unwrap();
        let peers = dir.path().join("peers.json");
        let answer = json!({"nodes":[{"id":"node-a","name":"Studio","kind":"laptop","os_version":"macOS 26.6","model":"Mac16,8"}]});
        update(&peers, "node-a", |k| k.learn(&answer, 10)).unwrap();
        let known = load(&peers)["node-a"].clone();
        assert_eq!(known.display("node-a"), "Studio");
        assert!(matches!(known.kind(), DeviceKind::Laptop));
        update(&peers, "node-a", |k| {
            k.alias = Some("Work Mac".into());
            k.alias_at = Some(100);
        })
        .unwrap();
        let legacy = json!({"nodes":[{"id":"node-a","name":"node-a"}]});
        update(&peers, "node-a", |k| k.learn(&legacy, 20)).unwrap();
        let known = load(&peers)["node-a"].clone();
        assert_eq!(known.display("node-a"), "Work Mac");
        assert_eq!(known.name.as_deref(), Some("Studio"), "an ID is not a name");
        assert_eq!(known.os_version.as_deref(), Some("macOS 26.6"));
        assert_eq!(known.last_seen, Some(20));
    }

    #[test]
    fn newer_names_win_and_a_cleared_name_is_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let peers = dir.path().join("peers.json");
        update(&peers, "node-a", |k| {
            k.alias = Some("Old".into());
            k.alias_at = Some(10);
        })
        .unwrap();
        merge_labels(&peers, &json!({"node-a": {"label": "Mac mini", "at": 20}, "node-b": {"label": "NAS", "at": 5}})).unwrap();
        merge_labels(
            &peers,
            &json!({"node-a": {"label": "Stale", "at": 15}, "bad id!": {"label": "x", "at": 1}}),
        )
        .unwrap();
        let known = load(&peers);
        assert_eq!(known["node-a"].display("node-a"), "Mac mini");
        assert_eq!(known["node-b"].display("node-b"), "NAS");
        assert!(!known.contains_key("bad id!"));
        merge_labels(&peers, &json!({"node-a": {"label": "", "at": 30}})).unwrap();
        assert_eq!(load(&peers)["node-a"].display("node-a"), "node-a");
        assert_eq!(labels(&peers)["node-a"], json!({"label": "", "at": 30}));
    }
}
