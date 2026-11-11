//! Devices as routers: when this device reaches one of the owner's devices only through
//! a slow relay (or not at all) but another owner device has a fast direct path to it,
//! go through that device instead (`cranibox_net::hop`). Only the owner's own devices
//! forward and are forwarded to; guests, workers and other identities never are. The
//! forwarding device sees only the end-to-end encrypted stream.
use anyhow::Result;
use cranibox_contracts::net::{PathInfo, PathKind};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path};

/// Saved in `routing.json` beside `peers.json`; both on by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Go through another own device when that is clearly faster than the relay.
    pub use_devices: bool,
    /// Let the owner's other devices go through this one.
    pub forward_for_devices: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            use_devices: true,
            forward_for_devices: true,
        }
    }
}
fn file(peers: &Path) -> std::path::PathBuf {
    peers.with_file_name("routing.json")
}
impl Settings {
    pub fn load(peers: &Path) -> Self {
        let read = || -> Result<Self> {
            let mut bytes = Vec::new();
            std::fs::File::open(file(peers))?
                .take(4096)
                .read_to_end(&mut bytes)?;
            Ok(serde_json::from_slice(&bytes)?)
        };
        read().unwrap_or_default()
    }
    pub fn save(&self, peers: &Path) -> Result<()> {
        agent_workspace_runtime::config::write_private(
            &file(peers),
            &serde_json::to_vec_pretty(self)?,
            true,
        )
    }
}

/// Milliseconds a hop adds besides the two measured legs.
const HOP_COST: u32 = 2;
/// A hop must beat the relay by this much, and by a quarter, to be worth switching.
const MIN_GAIN: u32 = 10;
/// Another device's measurement older than this is not trusted for routing.
pub const FRESH_MS: u64 = 120_000;

/// One candidate device to go through: this device's path to it and its measured
/// path to the target.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub node: String,
    pub to_via: Option<PathInfo>,
    pub via_to_target: Option<PathInfo>,
}
fn direct_rtt(path: &Option<PathInfo>) -> Option<u32> {
    path.as_ref()
        .filter(|p| p.kind == PathKind::Direct)
        .and_then(|p| p.rtt_ms)
}
/// The device to go through and the expected round trip, or `None` to go directly.
/// `current`: the device used now, kept unless another one is clearly better.
/// `direct`: this device's own path to the target (`None` when not connected).
pub fn choose(
    direct: Option<&PathInfo>,
    candidates: &[Candidate],
    current: Option<&str>,
) -> Option<(String, u32)> {
    if direct.is_some_and(|p| p.kind == PathKind::Direct) {
        return None;
    }
    let mut costs: Vec<(String, u32)> = candidates
        .iter()
        .filter_map(|c| {
            Some((
                c.node.clone(),
                direct_rtt(&c.to_via)? + direct_rtt(&c.via_to_target)? + HOP_COST,
            ))
        })
        .collect();
    costs.sort_by_key(|(_, cost)| *cost);
    let (best, cost) = costs.first().cloned()?;
    // Stay with the current device while it still works and is not clearly worse.
    if let Some(kept) = current.and_then(|c| costs.iter().find(|(node, _)| node == c))
        && kept.1 <= cost + cost / 5 + 5
    {
        return Some(kept.clone());
    }
    match direct
        .filter(|p| p.kind == PathKind::Relay)
        .and_then(|p| p.rtt_ms)
    {
        Some(relay) if cost + MIN_GAIN <= relay && cost * 4 <= relay * 3 => Some((best, cost)),
        Some(_) => None,
        // Not connected (or the relay path is not measured yet): a working hop helps.
        None => Some((best, cost)),
    }
}

/// How a hop path is shown: a relayed path whose relay is the named device.
pub fn hop_path(via: &str, rtt_ms: u32) -> PathInfo {
    PathInfo {
        kind: PathKind::Relay,
        rtt_ms: Some(rtt_ms),
        via: Some(format!("device:{via}")),
        scope: Some(cranibox_contracts::net::PathScope::Relay),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn path(kind: PathKind, rtt: u32) -> Option<PathInfo> {
        Some(PathInfo {
            kind,
            rtt_ms: Some(rtt),
            via: None,
            scope: None,
        })
    }
    fn candidate(node: &str, a: u32, b: u32) -> Candidate {
        Candidate {
            node: node.into(),
            to_via: path(PathKind::Direct, a),
            via_to_target: path(PathKind::Direct, b),
        }
    }

    #[test]
    fn hops_only_when_clearly_faster_than_the_relay_and_never_beat_a_direct_path() {
        let c = [candidate("mini", 3, 9), candidate("server", 20, 30)];
        let relay = path(PathKind::Relay, 120);
        assert_eq!(choose(relay.as_ref(), &c, None), Some(("mini".into(), 14)));
        // Direct wins outright, however slow.
        assert_eq!(choose(path(PathKind::Direct, 300).as_ref(), &c, None), None);
        // A relay barely slower than the hop is kept.
        assert_eq!(choose(path(PathKind::Relay, 22).as_ref(), &c, None), None);
        // Not connected at all: any working hop.
        assert_eq!(choose(None, &c, None), Some(("mini".into(), 14)));
        // Candidates without a direct leg on both sides do not count.
        let relayed_leg = Candidate {
            node: "phone".into(),
            to_via: path(PathKind::Relay, 1),
            via_to_target: path(PathKind::Direct, 1),
        };
        assert_eq!(choose(None, &[relayed_leg], None), None);
    }

    #[test]
    fn the_current_device_is_kept_unless_another_is_clearly_better() {
        let c = [candidate("mini", 10, 10), candidate("server", 9, 9)];
        let relay = path(PathKind::Relay, 200);
        assert_eq!(
            choose(relay.as_ref(), &c, Some("mini")),
            Some(("mini".into(), 22))
        );
        let c = [candidate("mini", 40, 40), candidate("server", 5, 5)];
        assert_eq!(
            choose(relay.as_ref(), &c, Some("mini")),
            Some(("server".into(), 12))
        );
    }

    #[test]
    fn settings_default_on_and_survive_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let peers = dir.path().join("peers.json");
        assert_eq!(Settings::load(&peers), Settings::default());
        std::fs::write(
            dir.path().join("routing.json"),
            br#"{"use_devices":false,"later":1}"#,
        )
        .unwrap();
        let loaded = Settings::load(&peers);
        assert!(!loaded.use_devices && loaded.forward_for_devices);
        let hop = hop_path("node-x", 14);
        assert_eq!(hop.via.as_deref(), Some("device:node-x"));
    }
}
