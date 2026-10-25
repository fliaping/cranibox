//! Bounded, directional path gossip. Only owner probes may merge remote rows.
use crate::peer::transport_iroh::Handle;
use agent_workspace_runtime::peers::{
    config::Config, roster::RosterStore, transport::endpoint_path,
};
use anyhow::Result;
use cranibox_contracts::net::{PathInfo, PathKind, PathScope};
use cranibox_contracts::roster::RosterView;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, io::Read, path::Path};

/// Record an authenticated probe using the endpoint that actually answered it.
/// The connection metadata is attached locally by Api::probe, never by the peer.
pub async fn observe(
    peers: &Path,
    peer: &str,
    answer: &Value,
    fallback_rtt: u32,
    iroh: Option<&Handle>,
) -> bool {
    let Ok(config) = Config::read(peers) else {
        return false;
    };
    let endpoint = config.routes.get(peer).and_then(|route| {
        route
            .endpoints()
            .into_iter()
            .find(|e| Some(e.url.as_str()) == answer["connection"]["url"].as_str())
    });
    let mut path = endpoint.as_ref().map(endpoint_path).unwrap_or(PathInfo {
        kind: PathKind::Unknown,
        scope: None,
        via: None,
        rtt_ms: None,
    });
    let used_iroh = endpoint.is_some() && path.kind == PathKind::Unknown;
    if used_iroh
        && let Some(iroh) = iroh
        && let Some(live) = iroh.peer_path(peer).await
    {
        path = live;
    }
    path.rtt_ms = path.rtt_ms.or(Some(fallback_rtt.min(MAX_RTT)));
    let _ = record(peers, &config.node_id, peer, path, now());
    let _ = super::known::update(peers, peer, |k| k.learn(answer, cranibox_llm::auth::now()));
    let owner = RosterStore::load(peers.with_file_name("models").join("roster.json"))
        .is_ok_and(|r| r.is_owner_node(&config.node_id) && r.is_owner_node(peer));
    if owner {
        let _ = super::known::merge_labels(peers, &answer["nodes"][0]["labels"]);
        let _ = merge(peers, &answer["paths"], &config.node_id);
    }
    used_iroh
}

/// Headless devices contribute their own rows even when nobody opens their UI.
/// Existing UI probes share the store, so recent measurements need no extra probe.
pub fn start(
    peers: std::path::PathBuf,
    iroh: Option<Handle>,
    stop: tokio_util::sync::CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        use futures_util::{StreamExt, stream};
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! { _ = stop.cancelled() => break, _ = interval.tick() => {} }
            let Ok(config) = Config::read(&peers) else {
                continue;
            };
            let rows = load(&peers);
            let at = now();
            let targets: Vec<_> = config
                .routes
                .keys()
                .filter(|id| *id != &config.node_id)
                .take(LIMIT - 1)
                .filter(|id| {
                    rows.get(&config.node_id)
                        .and_then(|row| row.get(*id))
                        .is_none_or(|v| at.saturating_sub(v.at) >= 15_000)
                })
                .cloned()
                .collect();
            let probes = stream::iter(targets).for_each_concurrent(8, |peer| {
                let (peers, iroh) = (&peers, iroh.as_ref());
                async move {
                    let started = std::time::Instant::now();
                    let store = agent_workspace_runtime::peers::Store::new(peers.clone());
                    if let Ok(Ok(answer)) = tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        store.manage(agent_workspace_runtime::peers::Command::Probe {
                            node: peer.clone(),
                        }),
                    )
                    .await
                    {
                        observe(
                            peers,
                            &peer,
                            &answer,
                            started.elapsed().as_millis().min(MAX_RTT as u128) as u32,
                            iroh,
                        )
                        .await;
                    }
                }
            });
            tokio::select! { _ = stop.cancelled() => break, _ = probes => {} }
        }
    })
}

const LIMIT: usize = 64;
const MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_RTT: u32 = 60_000;
const CLOCK_SKEW: u64 = 60_000;
pub type Rows = BTreeMap<String, BTreeMap<String, Measurement>>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Measurement {
    #[serde(flatten)]
    pub path: PathInfo,
    /// Unix milliseconds, preserved when forwarding another device's measurement.
    pub at: u64,
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn valid_id(id: &str) -> bool {
    agent_workspace_runtime::protocol::valid_node_id(id)
}
fn valid(value: &Measurement, now: u64) -> bool {
    value.at > 0
        && value.at <= now.saturating_add(CLOCK_SKEW)
        && value.path.rtt_ms.is_none_or(|ms| ms <= MAX_RTT)
        && value.path.via.as_ref().is_none_or(|s| {
            !s.is_empty()
                && s.len() <= 253
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
        })
        && match value.path.scope {
            Some(PathScope::Relay) => value.path.kind == PathKind::Relay,
            Some(_) => value.path.kind == PathKind::Direct,
            None => true,
        }
}
fn trim(rows: &mut Rows, own: &str) {
    for row in rows.values_mut() {
        while row.len() > LIMIT {
            let oldest = row
                .iter()
                .min_by_key(|(_, v)| v.at)
                .map(|(id, _)| id.clone())
                .unwrap();
            row.remove(&oldest);
        }
    }
    while rows.len() > LIMIT {
        let oldest = rows
            .iter()
            .filter(|(id, _)| id.as_str() != own)
            .min_by_key(|(_, row)| row.values().map(|v| v.at).max().unwrap_or_default())
            .map(|(id, _)| id.clone())
            .unwrap();
        rows.remove(&oldest);
    }
}
/// The local row is authoritative; echoed (or forged) copies can never replace it.
fn merge_into(rows: &mut Rows, remote: &Value, own: &str, now: u64) {
    let Some(remote) = remote.as_object() else {
        return;
    };
    for (from, row) in remote.iter().take(LIMIT) {
        if from == own || !valid_id(from) {
            continue;
        }
        let Some(row) = row.as_object() else { continue };
        for (to, raw) in row.iter().take(LIMIT) {
            if from == to || !valid_id(to) {
                continue;
            }
            let Ok(value) = serde_json::from_value::<Measurement>(raw.clone()) else {
                continue;
            };
            if !valid(&value, now) {
                continue;
            }
            let row = rows.entry(from.clone()).or_default();
            if row.get(to).is_none_or(|old| value.at > old.at) {
                row.insert(to.clone(), value);
            }
        }
    }
    trim(rows, own);
}
pub fn load(peers: &Path) -> Rows {
    let read = || -> Result<Value> {
        let file = std::fs::File::open(peers.with_file_name("paths.json"))?;
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() as u64 <= MAX_BYTES, "path store too large");
        Ok(serde_json::from_slice(&bytes)?)
    };
    let mut rows = Rows::new();
    if let Ok(value) = read() {
        merge_into(&mut rows, &value, "", now());
    }
    rows
}
fn update(peers: &Path, edit: impl FnOnce(&mut Rows)) -> Result<()> {
    let path = peers.with_file_name("paths.json");
    let _lock = agent_workspace_runtime::config::lock_credentials(&path)?;
    let mut rows = load(peers);
    let before = rows.clone();
    edit(&mut rows);
    if rows != before {
        agent_workspace_runtime::config::write_private(&path, &serde_json::to_vec(&rows)?, true)?;
    }
    Ok(())
}
pub fn merge(peers: &Path, remote: &Value, own: &str) -> Result<()> {
    update(peers, |rows| merge_into(rows, remote, own, now()))
}
pub fn record(peers: &Path, own: &str, peer: &str, path: PathInfo, at: u64) -> Result<()> {
    let value = Measurement { path, at };
    if own == peer || !valid_id(own) || !valid_id(peer) || !valid(&value, now()) {
        return Ok(());
    }
    update(peers, |rows| {
        let row = rows.entry(own.into()).or_default();
        if row.get(peer).is_none_or(|old| at > old.at) {
            row.insert(peer.into(), value);
        }
        trim(rows, own);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn entry(at: u64) -> Value {
        json!({"kind":"direct","scope":"lan","via":"private","rtt_ms":12,"at":at})
    }

    #[tokio::test]
    async fn background_probe_records_http_rtt_and_ignores_guest_gossip() {
        use agent_workspace_runtime::peers::config::{Endpoint, Route};
        use tokio_util::sync::CancellationToken;
        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        let credential = root.path().join("fixture-credential");
        std::fs::write(&credential, "0".repeat(64)).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let config = Config::update(&peers, |c| {
            c.routes.insert(
                "peer".into(),
                Route {
                    url: url.clone(),
                    token_file: credential,
                    allow_insecure: false,
                    local_forward_lock: None,
                    alternatives: vec![],
                    target_identity: None,
                    config_path: None,
                },
            );
            Ok(())
        })
        .unwrap();
        let router = axum::Router::new().fallback(|| async {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            axum::Json(json!({"nodes":[{"id":"peer","name":"Fixture"}],
                "paths":{"forged":{"peer":entry(now())}}}))
        });
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let stop = CancellationToken::new();
        let sampler = start(peers.clone(), None, stop.clone());
        let sampled = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if let Some(path) = load(&peers)
                    .get(&config.node_id)
                    .and_then(|row| row.get("peer"))
                    .cloned()
                {
                    break path;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await;
        stop.cancel();
        sampler.await.unwrap();
        server.abort();
        let sampled = sampled.unwrap();
        assert_eq!(sampled.path.scope, Some(PathScope::Lan));
        assert!(sampled.path.rtt_ms.unwrap() >= 20);
        assert!(!load(&peers).contains_key("forged"));

        // A selected overlay fallback must override the primary HTTP route.
        Config::update(&peers, |c| {
            c.routes
                .get_mut("peer")
                .unwrap()
                .alternatives
                .push(Endpoint {
                    url: "http://100.64.0.2".into(),
                    allow_insecure: true,
                    local_forward_lock: None,
                    iroh: None,
                });
            Ok(())
        })
        .unwrap();
        let answer = json!({"nodes":[{"id":"peer"}], "connection":{"url":"http://100.64.0.2"}});
        assert!(!observe(&peers, "peer", &answer, 42, None).await);
        let path = &load(&peers)[&config.node_id]["peer"].path;
        assert_eq!(path.scope, Some(PathScope::Overlay));
        assert_eq!(path.rtt_ms, Some(42));
    }

    #[test]
    fn newer_directional_measurements_win_but_never_overwrite_our_row() {
        let mut rows = Rows::new();
        merge_into(
            &mut rows,
            &json!({"a":{"b":entry(10)},"b":{"a":entry(20)}}),
            "self",
            100,
        );
        merge_into(
            &mut rows,
            &json!({"a":{"b":entry(9)},"b":{"a":entry(21)},"self":{"a":entry(30)}}),
            "self",
            100,
        );
        assert_eq!(rows["a"]["b"].at, 10);
        assert_eq!(rows["b"]["a"].at, 21);
        assert!(!rows.contains_key("self"));
    }
    #[test]
    fn rejects_invalid_ids_fields_numbers_and_future_poisoning() {
        let mut rows = Rows::new();
        let mut bad = vec![
            json!(null),
            json!({}),
            entry(0),
            entry(100 + CLOCK_SKEW + 1),
        ];
        for (field, value) in [
            ("rtt_ms", json!(-1)),
            ("rtt_ms", json!(60_001)),
            ("rtt_ms", json!(1.5)),
            ("at", json!(u64::MAX)),
            ("via", json!("https://user:secret@relay")),
            ("via", json!("x".repeat(254))),
            ("kind", json!("bogus")),
            ("scope", json!("relay")),
        ] {
            let mut value_entry = entry(1);
            value_entry[field] = value;
            bad.push(value_entry);
        }
        for value in bad {
            merge_into(&mut rows, &json!({"a":{"b":value}}), "self", 100);
        }
        for id in ["".into(), "bad/id".into(), "x".repeat(65)] {
            merge_into(
                &mut rows,
                &json!({id.clone():{"b":entry(1)},"a":{id:entry(1)}}),
                "self",
                100,
            );
        }
        merge_into(&mut rows, &json!({"a":{"a":entry(1)}}), "self", 100);
        assert!(rows.is_empty());
    }
    #[test]
    fn bounds_accumulation_and_preserves_local_row() {
        let mut rows = Rows::new();
        rows.insert(
            "self".into(),
            BTreeMap::from([("peer".into(), serde_json::from_value(entry(1)).unwrap())]),
        );
        for batch in 0..3 {
            let remote: serde_json::Map<_, _> = (0..80)
                .map(|i| {
                    let row: serde_json::Map<_, _> = (0..80)
                        .map(|j| (format!("peer-{batch}-{j}"), entry(10 + batch)))
                        .collect();
                    (format!("node-{batch}-{i}"), Value::Object(row))
                })
                .collect();
            merge_into(&mut rows, &Value::Object(remote), "self", 100);
            assert!(rows.len() <= LIMIT && rows.values().all(|row| row.len() <= LIMIT));
            assert!(rows.contains_key("self"));
        }
        for batch in 0..3 {
            let row: serde_json::Map<_, _> = (0..64)
                .map(|i| (format!("target-{batch}-{i}"), entry(20 + batch)))
                .collect();
            merge_into(&mut rows, &json!({"peer":row}), "self", 100);
            assert_eq!(rows["peer"].len(), LIMIT);
            assert!(rows["peer"].values().all(|v| v.at == 20 + batch));
        }
    }
    #[test]
    fn measurements_persist_without_refreshing_their_age() {
        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        let at = now() - 1000;
        record(
            &peers,
            "self",
            "peer",
            PathInfo::direct("10.0.0.1".parse().unwrap()),
            at,
        )
        .unwrap();
        merge(
            &peers,
            &json!({"self":{"peer":entry(at+1)},"peer":{"self":entry(at)}}),
            "self",
        )
        .unwrap();
        assert_eq!(load(&peers)["self"]["peer"].at, at);
        assert_eq!(load(&peers)["peer"]["self"].at, at);
    }
}
