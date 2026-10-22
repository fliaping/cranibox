//! The Workspace server's embedded iroh relay; shared with the CLI node and desktop app
//! (see [`cranibox_node::web::relay`]).
use crate::server::State;
use axum::Router;
pub(crate) use cranibox_node::web::relay::setup;

/// The relay paths are dispatched in front of axum, so the mount point stays empty.
pub(crate) fn router() -> Router<State> {
    Router::new()
}
