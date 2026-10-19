//! File access deliberately requires command permission as well as project metadata.
use super::*;
use agent_workspace_runtime::projects::copy;
use anyhow::ensure;

fn permitted(app: &App, caller: &Caller, expected_root: Option<&std::path::Path>) -> Result<()> {
    let current = Config::read(&app.path)?;
    ensure!(
        !app.stop.is_cancelled() && !app.connections.is_cancelled() && current.node_id == app.node,
        "node stopped"
    );
    ensure!(
        current
            .grants
            .get(&caller.node)
            .is_some_and(|g| g.same_credential(&caller.grant)
                && g.permissions.exec
                && g.permissions.projects),
        "project and command access required for copying files"
    );
    if let Some(root) = expected_root {
        ensure!(
            current.copy_root.as_deref() == Some(root),
            "project copy root changed or disabled"
        );
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Export {
    replica_id: String,
}
pub(super) async fn export(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    Json(request): Json<Export>,
) -> Result<Json<Value>, Failure> {
    check_target(&app, &node)?;
    permitted(&app, &caller, None).map_err(|e| failure(StatusCode::FORBIDDEN, &e.to_string()))?;
    let slot = app
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| failure(StatusCode::TOO_MANY_REQUESTS, "node capacity reached"))?;
    let value = tokio::task::spawn_blocking(move || {
        let _slot = slot;
        let package = copy::export(&app.catalog, &node, &request.replica_id, || {
            permitted(&app, &caller, None)
        })?;
        Ok::<_, anyhow::Error>(serde_json::to_value(package)?)
    })
    .await
    .map_err(|_| failure(StatusCode::INTERNAL_SERVER_ERROR, "snapshot worker failed"))?
    .map_err(|e| failure(StatusCode::BAD_REQUEST, &e.to_string()))?;
    Ok(Json(value))
}
pub(super) async fn import(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    Json(request): Json<copy::Import>,
) -> Result<Json<Value>, Failure> {
    check_target(&app, &node)?;
    permitted(&app, &caller, Some(&request.expected_root))
        .map_err(|e| failure(StatusCode::FORBIDDEN, &e.to_string()))?;
    let slot = app
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| failure(StatusCode::TOO_MANY_REQUESTS, "node capacity reached"))?;
    let value = tokio::task::spawn_blocking(move || {
        let _slot = slot;
        let root = request.expected_root.clone();
        let receipt = copy::receive(
            &app.path,
            &app.catalog,
            &node,
            &caller.node,
            &caller.grant.credential,
            request,
            || permitted(&app, &caller, Some(&root)),
        )?;
        Ok::<_, anyhow::Error>(receipt.view())
    })
    .await
    .map_err(|_| failure(StatusCode::INTERNAL_SERVER_ERROR, "copy worker failed"))?
    .map_err(|e| failure(StatusCode::BAD_REQUEST, &e.to_string()))?;
    Ok(Json(value))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Status {
    id: String,
}
pub(super) async fn status(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    Json(request): Json<Status>,
) -> Result<Json<Value>, Failure> {
    check_target(&app, &node)?;
    permitted(&app, &caller, None).map_err(|e| failure(StatusCode::FORBIDDEN, &e.to_string()))?;
    let value = tokio::task::spawn_blocking(move || {
        let receipt = copy::status(
            &app.path,
            &request.id,
            &caller.node,
            &caller.grant.credential,
        )?;
        permitted(&app, &caller, None)?;
        Ok::<_, anyhow::Error>(receipt.view())
    })
    .await
    .map_err(|_| failure(StatusCode::INTERNAL_SERVER_ERROR, "copy worker failed"))?
    .map_err(|e| failure(StatusCode::NOT_FOUND, &e.to_string()))?;
    Ok(Json(value))
}

pub(super) async fn transfer(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    Json(command): Json<copy::stream::Command>,
) -> Result<Json<Value>, Failure> {
    check_target(&app, &node)?;
    permitted(&app, &caller, None).map_err(|e| failure(StatusCode::FORBIDDEN, &e.to_string()))?;
    let slot = app
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| failure(StatusCode::TOO_MANY_REQUESTS, "node capacity reached"))?;
    let value = tokio::task::spawn_blocking(move || {
        let _slot = slot;
        copy::stream::manage(
            &app.path,
            &app.catalog,
            &node,
            &caller.node,
            &caller.grant.credential,
            command,
            |root| permitted(&app, &caller, root),
        )
    })
    .await
    .map_err(|_| failure(StatusCode::INTERNAL_SERVER_ERROR, "transfer worker failed"))?
    .map_err(|e| failure(StatusCode::BAD_REQUEST, &e.to_string()))?;
    Ok(Json(value))
}
