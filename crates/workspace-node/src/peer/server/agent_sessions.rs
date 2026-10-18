use super::*;
use agent_workspace_runtime::peers::agent_sessions::Registry;

pub(super) async fn list(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
) -> Result<Json<Value>, Failure> {
    check_target(&app, &node)?;
    tokio::task::spawn_blocking(move || {
        let _lock = config::lock_credentials(&app.path).map_err(|_| {
            failure(
                StatusCode::SERVICE_UNAVAILABLE,
                "Agent configuration unavailable",
            )
        })?;
        let current = Config::read(&app.path).map_err(|_| {
            failure(
                StatusCode::SERVICE_UNAVAILABLE,
                "Agent configuration unavailable",
            )
        })?;
        if current.node_id != app.node
            || !current.grants.get(&caller.node).is_some_and(|grant| {
                grant.permissions.messages && grant.same_credential(&caller.grant)
            })
        {
            return Err(failure(
                StatusCode::FORBIDDEN,
                "Agent session discovery requires device messages access",
            ));
        }
        Registry::new(&app.path, app.node)
            .catalog()
            .and_then(|catalog| Ok(serde_json::to_value(catalog)?))
            .map(Json)
            .map_err(|_| {
                failure(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Agent registrations unavailable; existing data was preserved",
                )
            })
    })
    .await
    .map_err(|_| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "Agent registrations unavailable",
        )
    })?
}
