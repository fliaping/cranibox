//! Embedded iroh relay at `/relay`, plus `/ping` and `/generate_204` (C5). **Owned by WS1.**
//!
//! Phase 0 mount point: empty, merged outside the browser guard in `server.rs`
//! because relay clients authenticate with their EndpointId, not a session.
use crate::server::State;
use axum::Router;

pub(crate) fn router() -> Router<State> {
    Router::new()
}
