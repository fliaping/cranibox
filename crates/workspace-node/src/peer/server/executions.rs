use super::*;
use agent_workspace_runtime::peers::executions::{Command, Requester, Store};

pub(super) async fn manage(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    Json(command): Json<Command>,
) -> Result<Json<Value>, Failure> {
    check_target(&app, &node)?;
    check_permission(&caller, false)?;
    tokio::task::spawn_blocking(move || {
        // Serialize admission with revocation. The run ledger has its own lock.
        let _lock = config::lock_credentials(&app.path).map_err(|_| {
            failure(
                StatusCode::SERVICE_UNAVAILABLE,
                "execution configuration unavailable",
            )
        })?;
        if !app.permitted(&caller, false) || app.connections.is_cancelled() {
            return Err(failure(
                StatusCode::FORBIDDEN,
                "execution access was revoked",
            ));
        }
        Store::new(&app.path, app.node)
            .for_requester(Requester::new(caller.node, &caller.grant.token))
            .apply(command)
            .map(Json)
            .map_err(|error| failure(StatusCode::BAD_REQUEST, &error.to_string()))
    })
    .await
    .map_err(|_| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "execution request unavailable",
        )
    })?
}
