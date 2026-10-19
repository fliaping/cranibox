//! Mesh `/nodes/{node}/llm/*` (C2): forwarded model proxy calls and `/health`.
//!
//! **Owned by WS6.** Phase 0 placeholder mounted by `mesh_router()`: callers
//! with `Permissions.llm` get 501, others 403. WS6 adds the roster check
//! (`RosterView::is_owner_node`) and the real handlers.
use super::{App, Caller, Failure, check_target, failure};
use axum::{
    Extension, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::any,
};
use std::collections::HashMap;

pub(super) fn routes(_app: App) -> Router<App> {
    Router::new()
        .route("/", any(placeholder))
        .route("/{*rest}", any(placeholder))
}

async fn placeholder(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(params): Path<HashMap<String, String>>,
) -> Failure {
    if let Err(error) = check_target(&app, params.get("node").map_or("", String::as_str)) {
        return error;
    }
    if !caller.grant.permissions.llm {
        return failure(
            StatusCode::FORBIDDEN,
            "capability not granted by target node",
        );
    }
    failure(
        StatusCode::NOT_IMPLEMENTED,
        "llm mesh routes are not implemented yet",
    )
}
