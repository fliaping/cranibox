use super::{
    config,
    forward::{Destination, socks},
};
use agent_workspace_runtime::protocol::valid_node_id;
use anyhow::{Result, ensure};
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Serialize)]
pub(super) struct Device {
    pub node: String,
    pub address: SocketAddrV4,
    pub path: String,
    pub latency_ms: Option<f64>,
}
struct Advertisement {
    fingerprint: String,
    address: SocketAddrV4,
    path: String,
    latency_ms: Option<f64>,
}
fn advertisements(bytes: &[u8], own_id: &str) -> Result<Vec<Advertisement>> {
    let peers: Vec<Value> = serde_json::from_slice(bytes)?;
    ensure!(peers.len() <= 512, "Too many EasyTier advertisements.");
    let own = config::fingerprint(own_id);
    let mut nodes = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    for peer in peers {
        let Some((fingerprint, port)) = peer["hostname"].as_str().and_then(config::parse_hostname)
        else {
            continue;
        };
        if fingerprint == own || peer["cost"] == "Local" {
            continue;
        }
        let Some(ip) = peer["ipv4"]
            .as_str()
            .and_then(|s| s.parse::<Ipv4Addr>().ok())
            .filter(Ipv4Addr::is_private)
        else {
            continue;
        };
        let device = Advertisement {
            fingerprint: fingerprint.into(),
            address: SocketAddrV4::new(ip, port),
            path: if peer["cost"] == "p2p" {
                "direct"
            } else {
                "relay"
            }
            .into(),
            latency_ms: peer["lat_ms"]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .filter(|n| n.is_finite() && *n >= 0.),
        };
        if nodes.insert(fingerprint.to_owned(), device).is_some() {
            duplicates.insert(fingerprint.to_owned());
        }
    }
    for node in duplicates {
        nodes.remove(&node);
    }
    ensure!(
        nodes.len() <= 128,
        "Too many Cranibox nodes in this network."
    );
    Ok(nodes.into_values().collect())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    version: u8,
    node_id: String,
}
fn identity(bytes: &[u8], fingerprint: &str) -> Result<String> {
    let value: Identity = serde_json::from_slice(bytes)?;
    ensure!(
        value.version == 1
            && valid_node_id(&value.node_id)
            && config::fingerprint(&value.node_id) == fingerprint,
        "Discovery identity does not match its advertisement."
    );
    Ok(value.node_id)
}
async fn resolve(ad: Advertisement, proxy: SocketAddr) -> Result<Device> {
    let node = tokio::time::timeout(Duration::from_secs(2), async {
        let mut connection = socks(Destination{socks:proxy,target:ad.address}).await?;
        connection.write_all(b"GET /api/v1/mesh/identity HTTP/1.1\r\nHost: cranibox-network\r\nConnection: close\r\n\r\n").await?;
        let mut bytes=Vec::new(); connection.take(4097).read_to_end(&mut bytes).await?;
        ensure!(bytes.len() <= 4096 && bytes.starts_with(b"HTTP/1.1 200 "), "Invalid network discovery response.");
        let end = bytes.windows(4).position(|w|w == b"\r\n\r\n").ok_or_else(||anyhow::anyhow!("Missing network discovery response body."))?;
        identity(&bytes[end+4..], &ad.fingerprint)
    }).await??;
    Ok(Device {
        node,
        address: ad.address,
        path: ad.path,
        latency_ms: ad.latency_ms,
    })
}
pub(super) async fn discover(bytes: &[u8], own: &str, proxy: SocketAddr) -> Result<Vec<Device>> {
    // Group members are trusted peers, but individual advertisements and responses remain bounded.
    let ads = advertisements(bytes, own)?;
    let mut devices: Vec<_> = stream::iter(ads)
        .map(|ad| resolve(ad, proxy))
        .buffer_unordered(8)
        .filter_map(|r| async { r.ok() })
        .collect()
        .await;
    devices.sort_by(|a, b| a.node.cmp(&b.node));
    Ok(devices)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn duplicate_and_malformed_advertisements_never_choose_a_target() {
        let entry = |node, port, ip, cost| json!({"hostname":config::hostname(node,port),"ipv4":ip,"cost":cost,"lat_ms":"NaN"});
        let input = json!([
            entry("one", 1234, "10.0.0.1", "p2p"),
            entry("one", 5678, "10.0.0.2", "p2p"),
            entry("two", 1234, "127.0.0.1", "p2p"),
            entry("own", 1234, "10.0.0.3", "Local"),
            entry("three", 1234, "10.0.0.4", "2")
        ]);
        let peers = advertisements(&serde_json::to_vec(&input).unwrap(), "own").unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].fingerprint, config::fingerprint("three"));
        assert_eq!(peers[0].path, "relay");
        assert_eq!(peers[0].latency_ms, None);
    }
    #[test]
    fn discovery_requires_the_complete_advertised_identity() {
        let node = "node-0123456789abcdef0123456789abcdef";
        let bytes = serde_json::to_vec(&json!({"version":1,"node_id":node})).unwrap();
        assert_eq!(identity(&bytes, &config::fingerprint(node)).unwrap(), node);
        assert!(identity(&bytes, &config::fingerprint("other")).is_err());
        assert!(
            identity(
                br#"{"version":1,"node_id":"../invalid"}"#,
                &config::fingerprint("../invalid")
            )
            .is_err()
        );
    }
}
