//! Explicit administrator-approved identity migration. Aliases never grant access.
use super::*;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Migration {
    target: String,
}

pub(super) async fn migrate(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(request): Json<Migration>,
) -> Result<Json<Value>, ApiError> {
    if !valid_node_id(&id) || !valid_node_id(&request.target) {
        return Err(error(StatusCode::BAD_REQUEST, "invalid node ID"));
    }
    if app.local_node.as_deref() == Some(&request.target) {
        return Err(error(StatusCode::CONFLICT, "target is this Workspace host"));
    }
    // Serialize the preflight and credential commit with command dispatch. Dropping
    // the old sender after commit makes the device reconnect through its alias.
    let mut registry = app.inner.lock().await;
    let credentials = config::Credentials::read(&app.credentials)
        .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "credentials unavailable"))?;
    let source = credentials.resolve_node(&id).to_owned();
    if app.local_node.as_deref() == Some(&source) {
        return Err(error(
            StatusCode::CONFLICT,
            "cannot migrate this Workspace host",
        ));
    }
    if source != request.target
        && registry.jobs.values().any(|job| {
            job.outcome == Outcome::Running
                && (job.node == source || job.requester.as_deref() == Some(&source))
        })
    {
        return Err(error(
            StatusCode::CONFLICT,
            "wait for this node's gateway tasks to finish before migrating its identity",
        ));
    }
    if source != request.target
        && registry
            .terminal_requests
            .values()
            .any(|request| request.node == source)
    {
        return Err(error(
            StatusCode::CONFLICT,
            "wait for this node's terminal operation to finish before migrating its identity",
        ));
    }
    let path = app.credentials.clone();
    let projects = app.projects.clone();
    let target = request.target;
    let previous = source.clone();
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        let _lock = config::lock_credentials(&path)?;
        let mut credentials = config::Credentials::read(&path)?;
        anyhow::ensure!(
            credentials.resolve_node(&id) == source,
            "node identity changed; refresh before retrying"
        );
        projects.resolve_nodes(&credentials.node_aliases)?;
        if source != target {
            projects.ensure_node_available(&target)?;
        }
        credentials.migrate_node(&id, &target)?;
        config::write_private(&path, &serde_json::to_vec_pretty(&credentials)?, true)?;
        Ok(
            json!({"id":target,"previous_id":source,"aliases":credentials.node_aliases.iter()
            .filter_map(|(alias, owner)| (owner == &target).then_some(alias)).collect::<Vec<_>>()}),
        )
    })
    .await
    .map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "identity migration unavailable",
        )
    })?
    .map_err(|error_value| error(StatusCode::BAD_REQUEST, &error_value.to_string()))?;
    if result["id"].as_str() != Some(previous.as_str()) {
        registry.terminals.retain(|_, route| route.node != previous);
        registry.desktops.retain(|_, route| route.node != previous);
        registry.nodes.remove(&previous);
        app.nodes_changed.send_modify(|revision| *revision += 1);
    }
    Ok(Json(result))
}
