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
                grant.permissions.spaces && grant.same_credential(&caller.grant)
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

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Control {
    id: uuid::Uuid,
    action: agent_workspace_runtime::peers::space_catalog::Action,
}

/// Stop or start one of this device's spaces for a caller holding both the space
/// catalog and the command grant: a command grant can already run the container engine.
pub(super) async fn control(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    axum::extract::Json(request): axum::extract::Json<Control>,
) -> Result<Json<Value>, Failure> {
    use agent_workspace_runtime::peers::space_catalog::Action;
    check_target(&app, &node)?;
    let current = Config::read(&app.path).map_err(|_| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "space configuration unavailable",
        )
    })?;
    if current.node_id != app.node
        || !current.grants.get(&caller.node).is_some_and(|grant| {
            grant.permissions.spaces
                && grant.permissions.exec
                && grant.same_credential(&caller.grant)
        })
    {
        return Err(failure(
            StatusCode::FORBIDDEN,
            "this device does not allow you to stop or start its spaces",
        ));
    }
    let store = crate::spaces::Store::new(app.path.clone());
    let known = store
        .shared_catalog(&app.node)
        .map_err(|_| failure(StatusCode::SERVICE_UNAVAILABLE, "space catalog unavailable"))?;
    if !known.spaces.iter().any(|space| space.id == request.id) {
        return Err(failure(StatusCode::NOT_FOUND, "space not found"));
    }
    let command = match request.action {
        Action::Stop => crate::spaces::Command::Stop { id: request.id },
        Action::Resume => crate::spaces::Command::Resume {
            id: request.id,
            image: None,
            registry: None,
            display: None,
            pull_policy: None,
        },
    };
    store
        .manage(command, true)
        .await
        .map_err(|error| failure(StatusCode::CONFLICT, &format!("{error:#}")))?;
    store
        .shared_catalog(&app.node)
        .and_then(|catalog| Ok(serde_json::to_value(catalog)?))
        .map(Json)
        .map_err(|_| failure(StatusCode::SERVICE_UNAVAILABLE, "space catalog unavailable"))
}
