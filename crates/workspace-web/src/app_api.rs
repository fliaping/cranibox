//! App API `/api/v1/app/*` for the UI (C6, `cranibox_contracts::app`). **Owned by WS7.**
//!
//! Phase 0 mount point: empty, so every request still reaches the existing
//! handler. WS7 adds the routes here; the mount in `server.rs` does not change.
use crate::server::State;
use axum::Router;

pub(crate) fn router() -> Router<State> {
    Router::new()
}
