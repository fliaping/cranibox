//! Cranibox default network (WS1, `docs/design/tasks/01-iroh-network.md`).
//!
//! - [`IrohNode`]: one iroh 1.3 endpoint per node. The node's ed25519 signing
//!   key (`node-identity.json`) is the iroh `SecretKey`, so the EndpointId is the
//!   pinned signing key and the TLS handshake authenticates the peer.
//! - [`mesh`]: ALPN [`ALPN_PEER_HTTP`] carries one HTTP/1.1 connection (with
//!   Upgrade) per QUIC bidirectional stream, serving the unchanged mesh routes.
//! - [`IrohTransport`]: [`PeerTransport`] over iroh, plus loopback forwards for
//!   the existing out-of-process clients.
//! - [`relay`]: the iroh relay embedded in `cranibox serve` over HTTP(S)/WebSocket
//!   only, with an allowlist of paired EndpointIds and a self-check.
//! - [`ticket`] and [`pair`]: one-time pairing links / QR codes and the pairing
//!   handshake that shows the existing SAS pairing code.
pub mod addr;
pub mod allow;
pub mod forward;
pub mod lan;
pub mod mesh;
pub mod node;
pub mod pair;
pub mod path;
pub mod relay;
pub mod ticket;
pub mod transport;

pub use addr::{endpoint_addr, iroh_addr};
pub use allow::Allowlist;
pub use cranibox_contracts::net::{
    ALPN_PEER_HTTP, IrohAddr, PairPurpose, PairTicket, PathInfo, PathKind, PeerTransport,
};
pub use node::{IrohNode, Options};
pub use transport::IrohTransport;

/// ALPN of the pre-authorization pairing handshake ([`pair`]). Only a holder of a
/// valid one-time ticket secret gets past the first message.
// CONTRACT-REQUEST: add `ALPN_PAIR` next to `ALPN_PEER_HTTP` in cranibox_contracts::net.
pub const ALPN_PAIR: &[u8] = b"cranibox/pair/1";
/// ALPN of loopback forwards ([`forward`]): the same peer routes as
/// [`ALPN_PEER_HTTP`], but the receiver does **not** treat the stream as
/// authenticated by the node key. A forward is reachable by every local user of
/// the dialing machine, so its requests must still carry the per-route bearer or
/// Noise session, exactly as on direct HTTP routes.
// CONTRACT-REQUEST: add `ALPN_PEER_FWD` to cranibox_contracts::net (C2).
pub const ALPN_PEER_FWD: &[u8] = b"cranibox/peer-fwd/1";

/// Which ALPN a pooled connection speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lane {
    /// [`ALPN_PEER_HTTP`]: in-process callers; the receiver injects `Authenticated`.
    Peer,
    /// [`ALPN_PEER_FWD`]: loopback forwards; inner credentials still required.
    Forward,
}
impl Lane {
    pub fn alpn(self) -> &'static [u8] {
        match self {
            Lane::Peer => ALPN_PEER_HTTP,
            Lane::Forward => ALPN_PEER_FWD,
        }
    }
}

/// QUIC application close codes used by this crate.
pub mod close {
    /// The peer's EndpointId is not paired with this node.
    pub const NOT_ALLOWED: u32 = 0x4e41;
    /// Normal shutdown.
    pub const DONE: u32 = 0;
}
