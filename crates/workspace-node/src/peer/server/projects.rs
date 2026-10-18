//! Node-owned catalog. Never writes another device's paths or the gateway catalog.
use super::*;
use agent_workspace_runtime::projects::Command;
use agent_workspace_runtime::projects::sync::{Delta, Index, PROTOCOL};

pub(super) async fn manage(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    Json(command): Json<Command>,
) -> Result<Json<Value>, Failure> {
    check_target(&app, &node)?;
    if !caller.grant.permissions.projects {
        return Err(failure(StatusCode::FORBIDDEN, "project access not granted"));
    }
    if let Command::Bind { .. } = &command {
        check_permission(&caller, true)?;
    }
    let catalog = crate::local::projects_in(app.catalog.clone(), &node, &app.engine, command)
        .await
        .map_err(|e| failure(StatusCode::BAD_REQUEST, &e.to_string()))?;
    Ok(Json(json!({"owner":node, "revision":catalog.revision,
        "projects":catalog.projects, "permissions":caller.grant.permissions, "sync_protocol":PROTOCOL})))
}

pub(super) async fn sync(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    Json(index): Json<Index>,
) -> Result<Json<Value>, Failure> {
    check_target(&app, &node)?;
    if !caller.grant.permissions.projects {
        return Err(failure(StatusCode::FORBIDDEN, "project access not granted"));
    }
    let catalog =
        crate::local::projects_in(app.catalog.clone(), &node, &app.engine, Command::List {})
            .await
            .map_err(|e| failure(StatusCode::BAD_REQUEST, &e.to_string()))?;
    let delta = Delta::between(&catalog, &index)
        .map_err(|e| failure(StatusCode::BAD_REQUEST, &e.to_string()))?;
    // Authentication and ownership filtering are identical to a full read.
    Ok(Json(
        json!({"owner":node, "permissions":caller.grant.permissions,
        "sync_protocol":PROTOCOL, "delta":delta}),
    ))
}
