//! Remembered details of paired devices (`devices.json` beside `peers.json`), so offline
//! devices keep their names and the time they were last seen.
use agent_workspace_runtime::peers::config::Config;
use agent_workspace_runtime::peers::roster::RosterStore;
use anyhow::{Result, ensure};
use cranibox_contracts::app::DeviceKind;
use cranibox_contracts::roster::RosterView;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

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
    /// `browser`, `exec`, `terminal`, …): granted to us and configured on that device.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub offers: BTreeMap<String, bool>,
    /// The relationship chosen when the devices were paired. Kept here rather than in
    /// `peers.json`, whose strict format older versions must still read after a rollback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<cranibox_contracts::app::Relation>,
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
        for key in [
            "desktop", "computer", "browser", "exec", "terminal", "projects", "messages",
            "activity", "spaces",
        ] {
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

/// Drop a device that no longer exists (a permanently deleted space). Best effort.
pub fn forget(peers: &Path, node: &str) {
    let mut all = load(peers);
    if all.remove(node).is_some()
        && let Ok(bytes) = serde_json::to_vec_pretty(&all)
    {
        let _ = agent_workspace_runtime::config::write_private(&path(peers), &bytes, true);
    }
}

/// Longest display name, counted in Unicode characters after trimming.
pub const NAME_MAX_CHARS: usize = 64;

/// Trim a name, treat a blank as “clear the alias”, and reject control characters
/// or anything longer than [`NAME_MAX_CHARS`].
pub fn normalize(name: &str) -> std::result::Result<Option<String>, &'static str> {
    let trimmed = name.trim();
    if trimmed.chars().any(|c| c.is_control()) {
        return Err("Name cannot contain control characters");
    }
    if trimmed.chars().count() > NAME_MAX_CHARS {
        return Err("Use at most 64 characters");
    }
    Ok((!trimmed.is_empty()).then(|| trimmed.to_owned()))
}

/// The name this device should advertise: the owner's alias when one is set,
/// otherwise the hostname (or computer name).
pub fn advertised_name(peers: &Path, local_id: &str) -> String {
    load(peers)
        .get(local_id)
        .and_then(|k| k.alias.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::device_info::local().name.clone())
}

/// Every device this node knows, mapped to the name that should be shown for it.
pub fn catalog(peers: &Path) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    if let Ok(config) = Config::read(peers) {
        names.insert(
            config.node_id.clone(),
            advertised_name(peers, &config.node_id),
        );
        for node in config.routes.keys().chain(config.grants.keys()) {
            names.entry(node.clone()).or_insert_with(|| node.clone());
        }
    }
    if let Ok(roster) = RosterStore::load(roster_path(peers)) {
        if let Some(current) = roster.current() {
            for member in current.members {
                names.entry(member.node_id).or_insert(member.name);
            }
        }
    }
    for (node, known) in load(peers) {
        names.insert(node.clone(), known.display(&node));
    }
    if let Ok(config) = Config::read(peers) {
        names.insert(
            config.node_id.clone(),
            advertised_name(peers, &config.node_id),
        );
    }
    names
}

fn roster_path(peers: &Path) -> PathBuf {
    peers.with_file_name("models").join("roster.json")
}

/// Look up a device by node ID or a unique display name (case-insensitive).
pub fn resolve(peers: &Path, value: &str) -> Result<String> {
    let names = catalog(peers);
    if names.contains_key(value) {
        return Ok(value.to_owned());
    }
    let matches: Vec<_> = names
        .iter()
        .filter(|(_, name)| name.eq_ignore_ascii_case(value))
        .map(|(id, _)| id.clone())
        .collect();
    ensure!(
        matches.len() == 1,
        "device name is unknown or ambiguous: {value}; use its node ID"
    );
    Ok(matches[0].clone())
}

pub fn is_duplicate(peers: &Path, node: &str, name: &str) -> bool {
    !name.is_empty()
        && catalog(peers)
            .iter()
            .any(|(id, existing)| id != node && existing.eq_ignore_ascii_case(name))
}

/// Result of setting or clearing a display name.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Rename {
    pub node: String,
    /// The name now used, or `None` if the alias was cleared.
    pub name: Option<String>,
    /// Another known device already uses this display name.
    pub duplicate: bool,
}

impl Rename {
    pub fn to_json(&self) -> Value {
        json!({
            "node": self.node,
            "name": self.name,
            "cleared": self.name.is_none(),
            "duplicate": self.duplicate,
        })
    }
}

/// Set or clear the owner's display name for a known device. The change is stored
/// as an alias in `devices.json` and syncs to other owner devices.
pub fn rename(peers: &Path, node: &str, name: &str) -> Result<Rename> {
    ensure!(catalog(peers).contains_key(node), "device not found");
    let label = normalize(name).map_err(anyhow::Error::msg)?;
    update(peers, node, |k| {
        k.alias = label.clone();
        k.alias_at = Some(cranibox_llm::auth::now().saturating_mul(1000));
    })?;
    let shown = label.clone().unwrap_or_default();
    Ok(Rename {
        node: node.to_owned(),
        name: label,
        duplicate: is_duplicate(peers, node, &shown),
    })
}

/// Prefer owner aliases (and this device's advertised name) in a `cranibox nodes` list.
pub fn apply_to_list(peers: &Path, mut value: Value) -> Value {
    let names = catalog(peers);
    let known = load(peers);
    let Some(nodes) = value["nodes"].as_array_mut() else {
        return value;
    };
    for node in nodes {
        let Some(id) = node["id"].as_str().map(str::to_owned) else {
            continue;
        };
        let Some(name) = names.get(&id) else {
            continue;
        };
        let probed = node["name"].as_str();
        let aliased = known.get(&id).is_some_and(|k| k.alias.is_some());
        if node["this"] == true || aliased || probed.is_none() || probed == Some(id.as_str()) {
            node["name"] = json!(name);
        }
    }
    value
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

    #[test]
    fn normalize_trims_rejects_control_characters_and_clears_a_blank() {
        assert_eq!(
            normalize("  Work Laptop  ").unwrap().as_deref(),
            Some("Work Laptop")
        );
        assert_eq!(normalize("   ").unwrap(), None);
        assert_eq!(normalize("").unwrap(), None);
        assert_eq!(
            normalize(
                "a
b"
            )
            .unwrap_err(),
            "Name cannot contain control characters"
        );
        assert_eq!(
            normalize(&"n".repeat(NAME_MAX_CHARS + 1)).unwrap_err(),
            "Use at most 64 characters"
        );
        assert_eq!(
            normalize(&"你".repeat(NAME_MAX_CHARS))
                .unwrap()
                .unwrap()
                .chars()
                .count(),
            NAME_MAX_CHARS
        );
    }

    #[test]
    fn rename_sets_and_clears_an_alias_and_warns_on_a_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let peers = dir.path().join("peers.json");
        Config::update(&peers, |_| Ok(())).unwrap();
        let me = Config::read(&peers).unwrap().node_id;
        let other = "node-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        update(&peers, other, |k| {
            k.name = Some("Studio".into());
        })
        .unwrap();
        let renamed = rename(&peers, other, "  Work Mac  ").unwrap();
        assert_eq!(renamed.name.as_deref(), Some("Work Mac"));
        assert!(!renamed.duplicate);
        assert_eq!(renamed.to_json()["cleared"], false);
        assert_eq!(load(&peers)[other].display(other), "Work Mac");
        assert_eq!(
            advertised_name(&peers, &me),
            crate::device_info::local().name
        );
        let again = rename(&peers, &me, "Work Mac").unwrap();
        assert!(again.duplicate);
        assert_eq!(advertised_name(&peers, &me), "Work Mac");
        let cleared = rename(&peers, other, "  ").unwrap();
        assert!(cleared.name.is_none());
        assert_eq!(cleared.to_json()["cleared"], true);
        assert_eq!(load(&peers)[other].display(other), "Studio");
        assert!(
            rename(&peers, "node-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "X")
                .unwrap_err()
                .to_string()
                .contains("device not found")
        );
        assert_eq!(resolve(&peers, "work mac").unwrap(), me);
        let list = apply_to_list(
            &peers,
            json!({"nodes":[
                {"id": me, "this": true, "online": true},
                {"id": other, "name": other, "online": false}
            ]}),
        );
        assert_eq!(list["nodes"][0]["name"], "Work Mac");
        assert_eq!(list["nodes"][1]["name"], "Studio");
    }
}
