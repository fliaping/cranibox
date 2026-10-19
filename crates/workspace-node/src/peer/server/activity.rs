use super::*;

pub(super) async fn read(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
) -> Result<Json<Value>, Failure> {
    check_target(&app, &node)?;
    tokio::task::spawn_blocking(move || {
        let unavailable = || {
            failure(
                StatusCode::SERVICE_UNAVAILABLE,
                "Collaboration records unavailable; existing data was preserved",
            )
        };
        let _lock = config::lock_credentials(&app.path).map_err(|_| unavailable())?;
        let current = Config::read(&app.path).map_err(|_| unavailable())?;
        if current.node_id != app.node
            || !current.grants.get(&caller.node).is_some_and(|grant| {
                grant.permissions.activity && grant.same_credential(&caller.grant)
            })
        {
            return Err(failure(
                StatusCode::FORBIDDEN,
                "Reading collaboration records requires a separate activity grant from the target",
            ));
        }
        agent_workspace_runtime::peers::activity::local(&app.path, &current)
            .and_then(|snapshot| Ok(serde_json::to_value(snapshot)?))
            .map(Json)
            .map_err(|_| unavailable())
    })
    .await
    .map_err(|_| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "Collaboration records unavailable",
        )
    })?
}
