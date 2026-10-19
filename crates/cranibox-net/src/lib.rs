//! iroh default network, embedded relay and link pairing. **Owned by WS1.**
//!
//! Phase 0 skeleton. WS1 provides here (see `docs/design/tasks/01-iroh-network.md`):
//! - `IrohTransport: cranibox_contracts::net::PeerTransport`
//! - `serve_mesh(endpoint, mesh_router)` speaking [`ALPN_PEER_HTTP`]
//! - `relay::router(allowlist)` mounted at `/relay` by `workspace-web/src/relay.rs`
//! - `PairTicket` encode/decode and the iroh back end of `/api/v1/app/pair/*`
pub use cranibox_contracts::net::{ALPN_PEER_HTTP, IrohAddr, PairTicket, PeerTransport};
