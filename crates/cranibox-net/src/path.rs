//! `PathInfo` (direct / relayed / RTT) of an iroh connection.
use cranibox_contracts::net::{PathInfo, PathKind, PathScope};
use iroh::{TransportAddr, endpoint::Connection};

pub fn info(conn: &Connection) -> PathInfo {
    let paths = conn.paths();
    let selected = paths.iter().find(|p| p.is_selected());
    match selected {
        Some(path) => {
            let mut info = address_info(path.remote_addr());
            info.rtt_ms = Some(path.rtt().as_millis().min(u32::MAX as u128) as u32);
            info
        }
        None => unknown(),
    }
}
fn unknown() -> PathInfo {
    PathInfo {
        kind: PathKind::Unknown,
        rtt_ms: None,
        via: None,
        scope: None,
    }
}
fn address_info(addr: &TransportAddr) -> PathInfo {
    match addr {
        TransportAddr::Relay(url) => PathInfo {
            kind: PathKind::Relay,
            rtt_ms: None,
            via: url.host_str().map(str::to_owned),
            scope: Some(PathScope::Relay),
        },
        TransportAddr::Ip(addr) => PathInfo::direct(addr.ip()),
        _ => unknown(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_ipv4_ipv6_and_overlay_boundaries() {
        for (scope, ips) in [
            (
                PathScope::Lan,
                vec![
                    "10.0.0.1",
                    "172.16.0.1",
                    "172.31.255.255",
                    "192.168.1.1",
                    "169.254.1.1",
                    "127.0.0.1",
                    "::1",
                    "fe80::1",
                    "fd00::1",
                    "::ffff:192.168.1.1",
                ],
            ),
            (
                PathScope::Overlay,
                vec![
                    "100.64.0.0",
                    "100.127.255.255",
                    "::ffff:100.64.1.1",
                    "fd7a:115c:a1e0::1",
                ],
            ),
            (
                PathScope::Internet,
                vec![
                    "8.8.8.8",
                    "100.63.255.255",
                    "100.128.0.0",
                    "172.32.0.1",
                    "2606:4700:4700::1111",
                    "::ffff:8.8.8.8",
                ],
            ),
        ] {
            for ip in ips {
                let addr = TransportAddr::Ip(std::net::SocketAddr::new(ip.parse().unwrap(), 443));
                let info = address_info(&addr);
                assert_eq!(info.scope, Some(scope), "{ip}");
                assert_eq!(info.kind, PathKind::Direct);
                assert!(info.rtt_ms.is_none());
            }
        }
    }

    #[test]
    fn relay_exposes_only_host_and_old_paths_still_deserialize() {
        let info = address_info(&TransportAddr::Relay(
            "https://relay.example:443/".parse().unwrap(),
        ));
        assert_eq!(info.scope, Some(PathScope::Relay));
        assert_eq!(info.via.as_deref(), Some("relay.example"));
        let old: PathInfo = serde_json::from_str(r#"{"kind":"direct","rtt_ms":3}"#).unwrap();
        assert_eq!(old.scope, None);
        assert_eq!(old.via, None);
    }
}
