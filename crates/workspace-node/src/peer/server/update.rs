//! Remote update of this node. Owner devices only, over the verified end-to-end channel:
//! an update runs new code on this machine, so a capability grant is never enough.
use super::*;
use crate::update::{
    manager::{Phase, Updater, default_entry_package},
    npm::Registries,
};
use axum::extract::Json as BodyJson;
use cranibox_contracts::roster::RosterView;
use serde::Deserialize;

fn authorize(app: &App, caller: &Caller, node: &str, encrypted: bool) -> Result<Updater, Failure> {
    check_target(app, node)?;
    if !encrypted || !caller.grant.peer.as_ref().is_some_and(|p| p.verified) {
        return Err(failure(
            StatusCode::FORBIDDEN,
            "verified end-to-end owner channel required",
        ));
    }
    let roster = agent_workspace_runtime::peers::roster::RosterStore::load(
        app.path.with_file_name("models").join("roster.json"),
    )
    .map_err(|_| failure(StatusCode::FORBIDDEN, "owner roster membership required"))?;
    if !roster.is_owner_node(&app.node) || !roster.is_owner_node(&caller.node) {
        return Err(failure(
            StatusCode::FORBIDDEN,
            "owner roster membership required",
        ));
    }
    let root = crate::update::runtime_root(app.path.parent().unwrap_or(&app.path));
    let registries = Registries::from_env().map_err(|_| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "update service unavailable",
        )
    })?;
    Ok(Updater::new(root, registries, default_entry_package()))
}

fn unavailable(error: anyhow::Error) -> Failure {
    failure(StatusCode::SERVICE_UNAVAILABLE, &format!("{error:#}"))
}

pub(super) async fn read(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    encrypted: Option<Extension<Encrypted>>,
    Path(node): Path<String>,
) -> Result<Json<Value>, Failure> {
    let updater = authorize(&app, &caller, &node, encrypted.is_some())?;
    let info = updater.info().map_err(unavailable)?;
    let status = updater.status().map_err(unavailable)?;
    Ok(Json(json!({
        "node": app.node,
        "version": crate::update::product_version(),
        "runtime": info,
        "status": status,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Start {
    spec: String,
    #[serde(default)]
    force: bool,
}

pub(super) async fn start(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    encrypted: Option<Extension<Encrypted>>,
    Path(node): Path<String>,
    BodyJson(body): BodyJson<Start>,
) -> Result<(StatusCode, Json<Value>), Failure> {
    let updater = authorize(&app, &caller, &node, encrypted.is_some())?;
    let spec = body.spec.trim().to_owned();
    if spec.is_empty() || spec.len() > 64 || spec.chars().any(|c| c.is_whitespace() || c == '/') {
        return Err(failure(
            StatusCode::BAD_REQUEST,
            "spec must be a version or a dist-tag",
        ));
    }
    let info = updater.info().map_err(unavailable)?;
    if !info.can_update {
        return Err(failure(
            StatusCode::CONFLICT,
            &format!(
                "this node cannot update itself: {}",
                info.reasons.join("; ")
            ),
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let (task_id, force) = (id.clone(), body.force);
    app.tasks.lock().await.spawn(async move {
        match updater.apply_as(task_id, &spec, force).await {
            // Leave the requester time to read the activated status before the connection drops.
            Ok(status) if matches!(status.phase, Phase::Activated | Phase::Done) => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if !crate::update::request_restart() {
                    eprintln!("update: activated {spec} but this process cannot restart itself");
                }
            }
            // The failure, with its reason, is already persisted for the requester.
            Ok(_) | Err(_) => {}
        }
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"id": id}))))
}

pub(super) async fn progress(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    encrypted: Option<Extension<Encrypted>>,
    Path((node, id)): Path<(String, String)>,
) -> Result<Json<Value>, Failure> {
    let updater = authorize(&app, &caller, &node, encrypted.is_some())?;
    match updater.status().map_err(unavailable)? {
        Some(status) if status.id == id => Ok(Json(json!(status))),
        _ => Err(failure(StatusCode::NOT_FOUND, "unknown update id")),
    }
}

pub(super) async fn rollback(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    encrypted: Option<Extension<Encrypted>>,
    Path(node): Path<String>,
) -> Result<Json<Value>, Failure> {
    let updater = authorize(&app, &caller, &node, encrypted.is_some())?;
    let status = updater
        .rollback("requested by an owner device")
        .map_err(|e| failure(StatusCode::CONFLICT, &format!("{e:#}")))?;
    app.tasks.lock().await.spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        crate::update::request_restart();
    });
    Ok(Json(json!(status)))
}
