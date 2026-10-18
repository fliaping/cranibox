//! Opt-in discovery of bounded listener ports on online tailnet peers.
use super::tailscale_nodes::{resolve, tailnet_address};
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::IpAddr,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::task::JoinHandle;

const LIMIT: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Candidate {
    addresses: Vec<IpAddr>,
    name: String,
}

fn candidates(state: &Value) -> Vec<Candidate> {
    if state["state"] != "Running" {
        return Vec::new();
    }
    let mut candidates: Vec<_> = state["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .take(512)
        .filter(|device| device["online"] == true)
        .map(|device| {
            let addresses: BTreeSet<_> = device["addresses"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|ip| ip.as_str()?.parse::<IpAddr>().ok())
                .filter(|ip| tailnet_address(*ip))
                .collect();
            Candidate {
                // At most one IPv4 and one IPv6 address, with IPv4 tried first.
                addresses: addresses
                    .iter()
                    .find(|ip| ip.is_ipv4())
                    .copied()
                    .into_iter()
                    .chain(addresses.iter().find(|ip| ip.is_ipv6()).copied())
                    .collect(),
                name: device["name"]
                    .as_str()
                    .filter(|s| s.len() <= 253)
                    .unwrap_or("")
                    .into(),
            }
        })
        .collect();
    // An address attributed to several devices is not a reliable discovery target.
    let mut counts = BTreeMap::new();
    for device in &candidates {
        for address in &device.addresses {
            *counts.entry(*address).or_insert(0) += 1;
        }
    }
    for device in &mut candidates {
        device.addresses.retain(|address| counts[address] == 1);
    }
    candidates.retain(|device| !device.addresses.is_empty());
    candidates.sort();
    candidates
}

#[derive(Default)]
struct Report {
    devices: Vec<Value>,
    checked: usize,
    skipped: usize,
    ambiguous: usize,
}
fn unique(found: Vec<(usize, Value)>) -> (Vec<Value>, usize) {
    let mut nodes: BTreeMap<String, (BTreeSet<usize>, Vec<Value>)> = BTreeMap::new();
    for (source, device) in found {
        let (sources, paths) = nodes
            .entry(device["node_id"].as_str().unwrap_or("").to_owned())
            .or_default();
        sources.insert(source);
        paths.push(device);
    }
    let mut devices = Vec::new();
    let mut ambiguous = 0;
    for (_, (sources, mut found)) in nodes {
        // Several listeners on one tailnet device are legitimate paths. The same
        // identity on different tailnet devices is still ambiguous and hidden.
        if sources.len() > 1 {
            ambiguous += 1;
            continue;
        }
        let mut device = found.remove(0);
        if !found.is_empty() {
            let mut seen = BTreeSet::new();
            let paths: Vec<_> = std::iter::once(&device)
                .chain(found.iter())
                .filter(|path| seen.insert(path["url"].as_str().unwrap_or("")))
                .map(
                    |path| json!({"address":path["address"],"port":path["port"],"url":path["url"]}),
                )
                .collect();
            device["paths"] = json!(paths);
        }
        devices.push(device);
    }
    (devices, ambiguous)
}
async fn scan(candidates: Vec<Candidate>, ports: Vec<u16>, own: String) -> Report {
    let checked = candidates.len().min(LIMIT);
    let skipped = candidates.len().saturating_sub(LIMIT);
    let probes: Vec<_> = candidates
        .into_iter()
        .take(LIMIT)
        .enumerate()
        .flat_map(|(source, device)| {
            ports
                .iter()
                .copied()
                .enumerate()
                .map(move |(priority, port)| (source, priority, device.clone(), port))
        })
        .collect();
    let mut found: Vec<_> =
        stream::iter(probes.into_iter().map(|(source, priority, device, port)| {
            let own = &own;
            async move {
                for address in device.addresses {
                    if let Ok(mut identity) = resolve(address, port, own).await {
                        identity["name"] = json!(device.name);
                        return Some((source, priority, identity));
                    }
                }
                None
            }
        }))
        .buffer_unordered(8)
        .filter_map(|value| async move { value })
        .collect()
        .await;
    // Configuration order defines the primary endpoint, independently of response timing.
    found.sort_by_key(|(source, priority, _)| (*source, *priority));
    let (devices, ambiguous) = unique(
        found
            .into_iter()
            .map(|(source, _, device)| (source, device))
            .collect(),
    );
    Report {
        devices,
        checked,
        skipped,
        ambiguous,
    }
}

pub(super) struct Discovery {
    task: Option<JoinHandle<Report>>,
    snapshot: Option<(Vec<Candidate>, Vec<u16>)>,
    next: Instant,
    report: Report,
    observed_at: Option<u64>,
    error: Option<&'static str>,
    running: bool,
}
impl Default for Discovery {
    fn default() -> Self {
        Self {
            task: None,
            snapshot: None,
            next: Instant::now(),
            report: Report::default(),
            observed_at: None,
            error: None,
            running: false,
        }
    }
}
impl Drop for Discovery {
    fn drop(&mut self) {
        self.reset();
    }
}
impl Discovery {
    pub fn reset(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.snapshot = None;
        self.next = Instant::now();
        self.report = Report::default();
        self.observed_at = None;
        self.error = None;
        self.running = false;
    }
    pub fn status(&self, ports: &[u16]) -> Value {
        let phase = if ports.is_empty() {
            "disabled"
        } else if !self.running {
            "waiting"
        } else if self.task.is_some() {
            "scanning"
        } else if self.error.is_some() {
            "failed"
        } else {
            "ready"
        };
        json!({"enabled":!ports.is_empty(),"port":ports.first(),"ports":ports,"phase":phase,"devices":self.report.devices,
            "observed_at":self.observed_at,"checked":self.report.checked,"skipped":self.report.skipped,
            "ambiguous":self.report.ambiguous,"error":self.error})
    }
    pub async fn reconcile(&mut self, state: &Value, ports: &[u16], own: &str) {
        if ports.is_empty() || state["state"] != "Running" {
            if self.running {
                self.reset();
            }
            return;
        }
        let current = candidates(state);
        let snapshot = (current.clone(), ports.to_vec());
        if self.snapshot.as_ref() != Some(&snapshot) {
            self.reset();
            self.snapshot = Some(snapshot);
        }
        self.running = true;
        if self.task.as_ref().is_some_and(|task| task.is_finished()) {
            match self.task.take().unwrap().await {
                Ok(report) => {
                    self.report = report;
                    self.observed_at = Some(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                    );
                    self.error = None;
                }
                Err(_) => {
                    self.error =
                        Some("Tailscale node discovery stopped; it will retry automatically.")
                }
            }
            self.next = Instant::now() + Duration::from_secs(30);
        }
        if self.task.is_none() && Instant::now() >= self.next {
            self.task = Some(tokio::spawn(scan(current, ports.to_vec(), own.to_owned())));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_excludes_offline_unsafe_and_ambiguous_addresses() {
        let state = json!({"state":"Running","devices":[
            {"name":"dual","online":true,"addresses":["fd7a:115c:a1e0::2","100.64.0.2","100.64.0.2"]},
            {"name":"offline","online":false,"addresses":["100.64.0.3"]},
            {"name":"unsafe","online":true,"addresses":["127.0.0.1","169.254.169.254","::ffff:100.64.0.4"]},
            {"name":"duplicate-a","online":true,"addresses":["100.64.0.5"]},
            {"name":"duplicate-b","online":true,"addresses":["100.64.0.5"]}]});
        let peers = candidates(&state);
        assert_eq!(peers.len(), 1);
        assert_eq!(
            peers[0].addresses,
            [
                "100.64.0.2".parse::<IpAddr>().unwrap(),
                "fd7a:115c:a1e0::2".parse().unwrap()
            ]
        );
        assert!(candidates(&json!({"state":"Stopped","devices":state["devices"]})).is_empty());
        let (devices, ambiguous) = unique(vec![
            (0, json!({"node_id":"duplicate","address":"100.64.0.2"})),
            (1, json!({"node_id":"duplicate","address":"100.64.0.3"})),
            (2, json!({"node_id":"unique"})),
        ]);
        assert_eq!(ambiguous, 1);
        assert_eq!(devices, [json!({"node_id":"unique"})]);
    }
    #[tokio::test]
    async fn background_scan_resolves_fallback_addresses_and_hides_duplicate_identities() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            for node in ["duplicate", "duplicate", "unique-node", "own-node"] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let len = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..len]).to_ascii_lowercase();
                assert!(!request.contains("authorization") && !request.contains("cookie"));
                let body = json!({"version":1,"node_id":node}).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        // Directly exercise the probe runner; public candidate selection excludes loopback.
        let devices = (0..4)
            .map(|n| Candidate {
                addresses: vec!["127.0.0.2".parse().unwrap(), "127.0.0.1".parse().unwrap()],
                name: format!("device-{n}"),
            })
            .collect();
        let report = scan(devices, vec![port], "own-node".into()).await;
        server.await.unwrap();
        assert_eq!(report.checked, 4);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.ambiguous, 1);
        assert_eq!(report.devices.len(), 1);
        assert_eq!(report.devices[0]["node_id"], "unique-node");
    }
    #[tokio::test]
    async fn discovery_clears_old_observations_when_membership_or_configuration_changes() {
        let mut discovery = Discovery::default();
        let state = json!({"state":"Running","devices":[]});
        discovery.reconcile(&state, &[1234], "own").await;
        while !discovery.task.as_ref().unwrap().is_finished() {
            tokio::task::yield_now().await;
        }
        discovery.reconcile(&state, &[1234], "own").await;
        assert_eq!(discovery.status(&[1234])["phase"], "ready");
        assert!(discovery.status(&[1234])["observed_at"].is_number());
        discovery.reconcile(&state, &[1234, 4321], "own").await;
        assert!(discovery.status(&[1234, 4321])["observed_at"].is_null());
        assert_eq!(
            discovery.status(&[1234, 4321])["ports"],
            json!([1234, 4321])
        );
        discovery
            .reconcile(
                &json!({"state":"Unavailable","devices":[]}),
                &[1234, 4321],
                "own",
            )
            .await;
        assert_eq!(discovery.status(&[1234, 4321])["phase"], "waiting");
        assert!(discovery.status(&[1234, 4321])["observed_at"].is_null());
        discovery.reset();
        assert_eq!(discovery.status(&[])["phase"], "disabled");
    }
    #[tokio::test]
    async fn several_ports_find_distinct_nodes_and_merge_same_device_paths_in_configured_order() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let mut ports = Vec::new();
        let mut servers = Vec::new();
        for (node, delay) in [("shared-node", 50), ("second-node", 0), ("shared-node", 0)] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            ports.push(listener.local_addr().unwrap().port());
            servers.push(tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let len = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..len]).to_ascii_lowercase();
                assert!(request.starts_with("get /api/v1/mesh/identity "));
                assert!(!request.contains("authorization") && !request.contains("cookie"));
                tokio::time::sleep(Duration::from_millis(delay)).await;
                let body = json!({"version":1,"node_id":node}).to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }));
        }
        let report = scan(
            vec![Candidate {
                addresses: vec!["127.0.0.1".parse().unwrap()],
                name: "one-device".into(),
            }],
            ports.clone(),
            "own".into(),
        )
        .await;
        for server in servers {
            server.await.unwrap();
        }
        assert_eq!(report.checked, 1);
        assert_eq!(report.ambiguous, 0);
        assert_eq!(report.devices.len(), 2);
        let shared = report
            .devices
            .iter()
            .find(|node| node["node_id"] == "shared-node")
            .unwrap();
        assert_eq!(shared["port"], ports[0]);
        assert_eq!(shared["paths"][0]["port"], ports[0]);
        assert_eq!(shared["paths"][1]["port"], ports[2]);
    }
    #[test]
    fn a_duplicate_identity_on_another_device_hides_all_of_its_ports() {
        let (devices, ambiguous) = unique(vec![
            (
                0,
                json!({"node_id":"copied-node","url":"http://100.64.0.2:1234"}),
            ),
            (
                0,
                json!({"node_id":"copied-node","url":"http://100.64.0.2:1235"}),
            ),
            (
                1,
                json!({"node_id":"copied-node","url":"http://100.64.0.3:1234"}),
            ),
        ]);
        assert!(devices.is_empty());
        assert_eq!(ambiguous, 1);
    }
}
