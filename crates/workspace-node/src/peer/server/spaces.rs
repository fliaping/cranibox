use super::*;

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
                "space configuration unavailable",
            )
        })?;
        let current = Config::read(&app.path).map_err(|_| {
            failure(
                StatusCode::SERVICE_UNAVAILABLE,
                "space configuration unavailable",
            )
        })?;
        if current.node_id != app.node
            || !current.grants.get(&caller.node).is_some_and(|grant| {
                grant.permissions.spaces && config::matches(&grant.token, &caller.grant.token)
            })
        {
            return Err(failure(
                StatusCode::FORBIDDEN,
                "space catalog access not granted by target node",
            ));
        }
        crate::spaces::Store::new(app.path.clone())
            .shared_catalog(&app.node)
            .and_then(|catalog| Ok(serde_json::to_value(catalog)?))
            .map(Json)
            .map_err(|_| {
                failure(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "space catalog unavailable; existing data was preserved",
                )
            })
    })
    .await
    .map_err(|_| failure(StatusCode::SERVICE_UNAVAILABLE, "space catalog unavailable"))?
}
