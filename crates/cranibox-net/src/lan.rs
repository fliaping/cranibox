//! Local-network address lookup. iroh 1.3 has no mDNS, so devices announce their
//! EndpointId and direct addresses by UDP multicast, signed with the endpoint key.
//! Receivers keep only announcements from devices they already trust (paired or in
//! the roster) and hand the addresses to iroh, which then dials them directly without
//! a relay, also on a local network without internet access.
use crate::allow::Allowlist;
use iroh::{Endpoint, EndpointAddr, PublicKey, SecretKey, Signature, address_lookup::MemoryLookup};
use serde::{Deserialize, Serialize};
use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

/// Administratively scoped multicast group and port used by Cranibox devices.
pub const GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 67, 66);
pub const PORT: u16 = 47_821;
const DOMAIN: &[u8] = b"cranibox-lan-v1\0";
const EVERY: Duration = Duration::from_secs(5);
const MAX_AGE_SECS: u64 = 120;
const MAX_ADDRS: usize = 16;

#[derive(Serialize, Deserialize)]
struct Announce {
    v: u8,
    id: [u8; 32],
    addrs: Vec<SocketAddr>,
    ts: u64,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// `payload ‖ ed25519(DOMAIN ‖ payload)`.
fn encode(secret: &SecretKey, addrs: Vec<SocketAddr>, ts: u64) -> Option<Vec<u8>> {
    let announce = Announce {
        v: 1,
        id: *secret.public().as_bytes(),
        addrs,
        ts,
    };
    let mut payload = postcard::to_allocvec(&announce).ok()?;
    let signature = secret.sign(&[DOMAIN, &payload].concat());
    payload.extend_from_slice(&signature.to_bytes());
    Some(payload)
}

/// A verified announcement that is fresh: the sender and its direct addresses.
fn decode(packet: &[u8], now: u64) -> Option<(PublicKey, Vec<SocketAddr>)> {
    let split = packet.len().checked_sub(64)?;
    let (payload, signature) = packet.split_at(split);
    let announce: Announce = postcard::from_bytes(payload).ok()?;
    if announce.v != 1
        || announce.addrs.len() > MAX_ADDRS
        || announce.ts.abs_diff(now) > MAX_AGE_SECS
    {
        return None;
    }
    let key = PublicKey::from_bytes(&announce.id).ok()?;
    let signature = Signature::from_bytes(signature.try_into().ok()?);
    key.verify(&[DOMAIN, payload].concat(), &signature).ok()?;
    Some((key, announce.addrs))
}

fn socket() -> std::io::Result<tokio::net::UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    // Several devices (or isolated identities) may share one machine.
    socket.set_reuse_address(true)?;
    #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
    socket.set_reuse_port(true)?;
    socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PORT).into())?;
    socket.join_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED)?;
    socket.set_multicast_loop_v4(true)?;
    socket.set_multicast_ttl_v4(1)?;
    socket.set_nonblocking(true)?;
    tokio::net::UdpSocket::from_std(socket.into())
}

/// Announce this endpoint and learn trusted peers' local addresses until `stop`.
pub async fn run(
    endpoint: Endpoint,
    secret: SecretKey,
    memory: MemoryLookup,
    trusted: Allowlist,
    stop: CancellationToken,
) {
    let socket = match socket() {
        Ok(socket) => socket,
        Err(error) => {
            eprintln!("local network discovery unavailable: {error}");
            return;
        }
    };
    let me = secret.public();
    let target = SocketAddr::V4(SocketAddrV4::new(GROUP, PORT));
    let mut tick = tokio::time::interval(EVERY);
    let mut buffer = vec![0u8; 2048];
    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            _ = tick.tick() => {
                let addrs: Vec<SocketAddr> = endpoint
                    .addr()
                    .ip_addrs()
                    .filter(|a| !a.ip().is_unspecified())
                    .take(MAX_ADDRS)
                    .copied()
                    .collect();
                if !addrs.is_empty()
                    && let Some(packet) = encode(&secret, addrs, now())
                {
                    let _ = socket.send_to(&packet, target).await;
                }
            }
            received = socket.recv_from(&mut buffer) => {
                let Ok((len, _)) = received else { continue };
                let Some((id, addrs)) = decode(&buffer[..len], now()) else { continue };
                if id == me || !trusted.contains(&id) {
                    continue;
                }
                let mut addr = EndpointAddr::new(id);
                for a in addrs {
                    addr = addr.with_ip_addr(a);
                }
                memory.add_endpoint_info(addr);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announcements_are_signed_fresh_and_bounded() {
        let secret = SecretKey::from_bytes(&[7u8; 32]);
        let addrs = vec!["192.168.1.20:4433".parse().unwrap()];
        let packet = encode(&secret, addrs.clone(), 1000).unwrap();
        let (id, got) = decode(&packet, 1010).unwrap();
        assert_eq!(id, secret.public());
        assert_eq!(got, addrs);
        assert!(decode(&packet, 1000 + MAX_AGE_SECS + 1).is_none(), "stale");
        let mut forged = packet.clone();
        forged[5] ^= 1;
        assert!(decode(&forged, 1010).is_none(), "tampered");
        let other = SecretKey::from_bytes(&[8u8; 32]);
        let mut swapped = encode(&other, addrs, 1000).unwrap();
        let tail = swapped.len() - 64;
        swapped[tail..].copy_from_slice(&packet[packet.len() - 64..]);
        assert!(
            decode(&swapped, 1010).is_none(),
            "signature must match the claimed id"
        );
    }
}
