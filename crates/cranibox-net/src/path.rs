//! `PathInfo` (direct / relayed / RTT) of an iroh connection.
use cranibox_contracts::net::{PathInfo, PathKind};
use iroh::{TransportAddr, endpoint::Connection};

pub fn info(conn: &Connection) -> PathInfo {
    let paths = conn.paths();
    let selected = paths.iter().find(|p| p.is_selected());
    match selected {
        Some(path) => PathInfo {
            kind: match path.remote_addr() {
                TransportAddr::Relay(_) => PathKind::Relay,
                TransportAddr::Ip(_) => PathKind::Direct,
                _ => PathKind::Unknown,
            },
            rtt_ms: Some(path.rtt().as_millis().min(u32::MAX as u128) as u32),
        },
        None => PathInfo {
            kind: PathKind::Unknown,
            rtt_ms: None,
        },
    }
}
