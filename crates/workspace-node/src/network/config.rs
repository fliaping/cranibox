use agent_workspace_runtime::{config as secrets, protocol::valid_node_id};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EasyTier {
    pub network: String,
    pub token: String,
    /// Legacy single-entry configurations remain readable. New clients use `peers`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub peer: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peers: Vec<String>,
    pub listen_port: Option<u16>,
}
impl EasyTier {
    pub fn entries(&self) -> &[String] {
        if self.peer.is_empty() {
            &self.peers
        } else {
            std::slice::from_ref(&self.peer)
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.network.is_empty()
                && self.network.len() <= 64
                && self
                    .network
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c)),
            "Network name must use 1–64 letters, numbers, underscores or hyphens."
        );
        ensure!(
            (32..=256).contains(&self.token.len())
                && self.token.bytes().all(|c| c.is_ascii_graphic()),
            "Use a network token with 32–256 printable ASCII characters."
        );
        ensure!(
            self.listen_port.is_none_or(|p| p >= 1024),
            "Listen port must be between 1024 and 65535."
        );
        ensure!(
            self.peer.is_empty() || self.peers.is_empty(),
            "Supply peers or the legacy peer field, not both."
        );
        let entries = self.entries();
        ensure!(
            entries.len() <= 8,
            "Use at most 8 EasyTier entry addresses."
        );
        if entries.is_empty() {
            ensure!(
                self.listen_port.is_some(),
                "Supply an EasyTier peer address or enable an incoming listener."
            );
        }
        let mut seen = std::collections::BTreeSet::new();
        for peer in entries {
            let mut url = url::Url::parse(peer)
                .context("Use an EasyTier tcp://, udp://, ws:// or wss:// address with a port.")?;
            ensure!(
                matches!(url.scheme(), "tcp" | "udp" | "ws" | "wss")
                    && url.host_str().is_some()
                    && url.port_or_known_default().is_some_and(|p| p != 0)
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && peer.len() <= 2048
                    && (!matches!(url.scheme(), "tcp" | "udp")
                        || url.path().is_empty()
                        || url.path() == "/"),
                "Use an EasyTier peer URL without embedded credentials, a query or a fragment."
            );
            if matches!(url.scheme(), "tcp" | "udp") && url.path() == "/" {
                url.set_path("");
            }
            ensure!(
                seen.insert(url.to_string()),
                "Remove duplicate EasyTier entry addresses."
            );
        }
        Ok(())
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Spec {
    pub version: u8,
    pub easytier: Option<EasyTier>,
    /// Legacy single-port discovery remains readable. New clients use `ports`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale_discovery_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tailscale_discovery_ports: Vec<u16>,
    /// Loopback ports are stable across restarts; destinations come from live discovery.
    pub forwards: BTreeMap<String, u16>,
    /// C5 iroh section (`cranibox_contracts::config::IROH_SECTION`), owned by WS1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iroh: Option<cranibox_contracts::config::IrohConfig>,
}
impl Default for Spec {
    fn default() -> Self {
        Self {
            version: 1,
            easytier: None,
            tailscale_discovery_port: None,
            tailscale_discovery_ports: Vec::new(),
            forwards: BTreeMap::new(),
            iroh: None,
        }
    }
}
impl Spec {
    pub fn discovery_ports(&self) -> Vec<u16> {
        self.tailscale_discovery_port
            .map_or_else(|| self.tailscale_discovery_ports.clone(), |port| vec![port])
    }
    pub fn read(path: &Path) -> Result<Self> {
        if !path.try_exists()? {
            return Ok(Self::default());
        }
        let file = std::fs::File::open(path)?;
        ensure!(
            file.metadata()?.len() <= 32 * 1024,
            "Network configuration is too large."
        );
        let value: Self = serde_json::from_reader(file)
            .context("Invalid networks.json; restore it before changing network settings.")?;
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && self.forwards.len() <= 128,
            "Invalid network version or device limit."
        );
        if let Some(value) = &self.easytier {
            value.validate()?;
        }
        ensure!(
            self.tailscale_discovery_port.is_none() || self.tailscale_discovery_ports.is_empty(),
            "Supply discovery ports or the legacy port field, not both."
        );
        validate_discovery_ports(&self.discovery_ports())?;
        let mut ports = std::collections::BTreeSet::new();
        for (node, port) in &self.forwards {
            ensure!(
                valid_node_id(node) && *port >= 1024 && ports.insert(port),
                "Invalid or duplicate network forwarding entry."
            );
        }
        Ok(())
    }
    pub fn write(&self, path: &Path) -> Result<()> {
        self.validate()?;
        // Do not overwrite corrupt state, even when an in-memory copy still exists.
        Self::read(path)?;
        let bytes = serde_json::to_vec_pretty(self)?;
        ensure!(
            bytes.len() <= 32 * 1024,
            "Network configuration is too large."
        );
        secrets::write_private(path, &bytes, true)
    }
}

pub(super) fn validate_discovery_ports(ports: &[u16]) -> Result<()> {
    let unique: std::collections::BTreeSet<_> = ports.iter().collect();
    ensure!(
        ports.len() <= 8 && !ports.contains(&0) && unique.len() == ports.len(),
        "Use up to 8 different Tailscale discovery ports between 1 and 65535."
    );
    Ok(())
}

/// Fit EasyTier's 32-character hostname limit, including arbitrary Cranibox IDs.
/// This is an untrusted discovery hint, not a cryptographic node identity.
pub(super) fn fingerprint(node: &str) -> String {
    format!("{:x}", Sha256::digest(node.as_bytes()))[..20].into()
}
pub(super) fn hostname(node: &str, port: u16) -> String {
    format!("cb1.{}.{port}", fingerprint(node))
}
pub(super) fn parse_hostname(value: &str) -> Option<(&str, u16)> {
    let (fingerprint, port) = value.strip_prefix("cb1.")?.rsplit_once('.')?;
    let port: u16 = port.parse().ok()?;
    (fingerprint.len() == 20 && fingerprint.bytes().all(|c| c.is_ascii_hexdigit()) && port != 0)
        .then_some((fingerprint, port))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tailscale_discovery_is_opt_in_and_preserves_other_network_settings() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("networks.json");
        std::fs::write(&path, r#"{"version":1,"easytier":null,"forwards":{}}"#).unwrap();
        let mut spec = Spec::read(&path).unwrap();
        assert_eq!(spec.tailscale_discovery_port, None);
        spec.tailscale_discovery_port = Some(1234);
        spec.write(&path).unwrap();
        assert_eq!(
            Spec::read(&path).unwrap().tailscale_discovery_port,
            Some(1234)
        );
        let before = std::fs::read(&path).unwrap();
        spec.tailscale_discovery_port = Some(0);
        assert!(spec.write(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        spec.tailscale_discovery_port = None;
        spec.write(&path).unwrap();
        assert_eq!(Spec::read(&path).unwrap().tailscale_discovery_port, None);
    }
    #[test]
    fn discovery_ports_preserve_order_and_reject_ambiguous_or_unbounded_configuration() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("networks.json");
        let mut spec = Spec::default();
        spec.forwards.insert("saved-node".into(), 14000);
        spec.tailscale_discovery_ports = vec![4321, 1234];
        spec.write(&path).unwrap();
        let read = Spec::read(&path).unwrap();
        assert_eq!(read.discovery_ports(), [4321, 1234]);
        assert_eq!(read.forwards["saved-node"], 14000);
        let before = std::fs::read(&path).unwrap();
        for ports in [vec![0], vec![1234, 1234], (1..=9).collect()] {
            spec.tailscale_discovery_ports = ports;
            assert!(spec.write(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), before);
        }
        spec.tailscale_discovery_ports = vec![4321];
        spec.tailscale_discovery_port = Some(1234);
        assert!(spec.write(&path).is_err());
        spec.tailscale_discovery_ports.clear();
        spec.write(&path).unwrap();
        assert_eq!(Spec::read(&path).unwrap().discovery_ports(), [1234]);
    }
    #[test]
    fn malformed_state_and_peer_credentials_are_rejected() {
        let mut spec = Spec {
            easytier: Some(EasyTier {
                network: "home".into(),
                token: "a".repeat(64),
                peer: "tcp://127.0.0.1:11010".into(),
                peers: Vec::new(),
                listen_port: None,
            }),
            ..Spec::default()
        };
        spec.validate().unwrap();
        for url in [
            "https://host:443",
            "tcp://user:secret@host:11010",
            "tcp://host",
            "ws://host/path?token=secret",
        ] {
            spec.easytier.as_mut().unwrap().peer = url.into();
            assert!(spec.validate().is_err(), "{url}");
        }
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("networks.json");
        std::fs::write(&path, "invalid").unwrap();
        assert!(Spec::default().write(&path).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "invalid");
    }
    #[test]
    fn multiple_entries_validate_every_address_and_preserve_legacy_state() {
        let mut spec: EasyTier = serde_json::from_value(serde_json::json!({
            "network":"home","token":"a".repeat(64),
            "peer":"tcp://127.0.0.1:11010","listen_port":null
        }))
        .unwrap();
        spec.validate().unwrap();
        assert_eq!(spec.entries(), ["tcp://127.0.0.1:11010"]);
        spec.peers = vec!["tcp://127.0.0.2:11010".into()];
        assert!(spec.validate().is_err());
        spec.peer.clear();
        spec.validate().unwrap();
        let valid = vec![
            "tcp://127.0.0.1:11010".to_string(),
            "wss://backup.example.com/mesh".to_string(),
        ];
        spec.peers = valid.clone();
        spec.validate().unwrap();
        for bad in [
            "",
            "tcp://host:0",
            "tcp://user:secret@host:11010",
            "wss://host/path?token=secret",
        ] {
            spec.peers = vec![valid[0].clone(), bad.into()];
            assert!(spec.validate().is_err(), "{bad}");
        }
        spec.peers = vec![valid[0].clone(), format!("{}/", valid[0])];
        assert!(spec.validate().is_err());
        spec.peers = (0..9)
            .map(|n| format!("tcp://127.0.0.1:{}", 11010 + n))
            .collect();
        assert!(spec.validate().is_err());
        spec.peers.pop();
        spec.validate().unwrap();
        spec.peers.clear();
        assert!(spec.validate().is_err());
        spec.listen_port = Some(11010);
        spec.validate().unwrap();
    }
    #[test]
    fn hostnames_encode_only_node_and_service_port() {
        let node = "node-0123456789abcdef0123456789abcdef";
        let name = hostname(node, 45678);
        assert!(name.len() <= 32);
        assert_eq!(
            parse_hostname(&name),
            Some((fingerprint(node).as_str(), 45678))
        );
        for value in [
            "cb1.0123456789abcdef0123.0",
            "cb1.0123456789abcdef0123.65536",
            "cb1.invalid.9000",
            "ordinary-device",
        ] {
            assert!(parse_hostname(value).is_none());
        }
    }
}
