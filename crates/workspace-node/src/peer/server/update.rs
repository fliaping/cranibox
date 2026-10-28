//! Remote update of this node. Owner devices only, over the verified end-to-end channel:
//! an update runs new code on this machine, so a capability grant is never enough.
use super::*;
use crate::update::{
    manager::{Updater, default_entry_package},
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
    Ok(Updater::new(root, registries, default_entry_package())
        .with_terminal_state(app.path.parent().unwrap_or(&app.path).to_owned()))
}

fn unavailable(error: anyhow::Error) -> Failure {
    failure(StatusCode::SERVICE_UNAVAILABLE, &format!("{error:#}"))
}

pub(super) async fn read(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    encrypted: Option<Extension<Encrypted>>,
    Path(node): Path<String>,
    axum::extract::Query(query): axum::extract::Query<Check>,
) -> Result<Json<Value>, Failure> {
    let updater = authorize(&app, &caller, &node, encrypted.is_some())?;
    Ok(Json(
        updater
            .snapshot(
                &app.node,
                crate::update::product_version(),
                query.check == 1,
            )
            .await
            .map_err(unavailable)?,
    ))
}

#[derive(Deserialize, Default)]
pub(super) struct Check {
    #[serde(default)]
    check: u8,
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
    Extension(access_key): Extension<access::AccessKey>,
    Extension(caller): Extension<Caller>,
    encrypted: Option<Extension<Encrypted>>,
    Path(node): Path<String>,
    BodyJson(body): BodyJson<Start>,
) -> Result<(StatusCode, Json<Value>), Failure> {
    let updater = authorize(&app, &caller, &node, encrypted.is_some())?;
    let spec = body.spec.trim().to_owned();
    if !crate::update::valid_spec(&spec) {
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
    let access = access::begin(
        &app,
        &caller,
        crate::access::Capability::Update,
        &access_key,
    )?;
    let (id, task) = updater
        .start(spec, body.force)
        .map_err(|e| failure(StatusCode::CONFLICT, &format!("{e:#}")))?;
    access.bind_job(&id).map_err(unavailable)?;
    app.tasks.lock().await.spawn(async move {
        tokio::select! {
            _ = task => {},
            _ = access.revoked() => {},
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
    Extension(access_key): Extension<access::AccessKey>,
    Extension(caller): Extension<Caller>,
    encrypted: Option<Extension<Encrypted>>,
    Path(node): Path<String>,
) -> Result<Json<Value>, Failure> {
    let updater = authorize(&app, &caller, &node, encrypted.is_some())?;
    let _access = access::begin(
        &app,
        &caller,
        crate::access::Capability::Update,
        &access_key,
    )?;
    let status = updater
        .rollback()
        .map_err(|e| failure(StatusCode::CONFLICT, &format!("{e:#}")))?;
    app.tasks
        .lock()
        .await
        .spawn(crate::update::restart_after_response());
    Ok(Json(json!(status)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_workspace_runtime::peers::{
        config::{Permissions, Pinned},
        roster::{OwnerKey, RosterStore},
    };
    use cranibox_contracts::roster::{Role, RosterMember};
    use tower::ServiceExt;

    const TEST: &str = "peer::server::update::tests::mesh_check_keeps_status_on_registry_failure";

    #[tokio::test]
    async fn mesh_check_keeps_status_on_registry_failure() {
        let Ok(root) = std::env::var("CRANIBOX_MESH_CHECK_TEST_ROOT") else {
            use crate::update::npm::tests::{packument, serve_body};
            let root = tempfile::tempdir().unwrap();
            let good = serve_body(StatusCode::OK, serde_json::to_vec(&packument()).unwrap()).await;
            let down = serve_body(StatusCode::SERVICE_UNAVAILABLE, Vec::new()).await;
            for (name, registry) in [("good", &good.base), ("down", &down.base)] {
                let output = tokio::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", TEST, "--nocapture"])
                    .env("CRANIBOX_MESH_CHECK_TEST_ROOT", root.path().join(name))
                    .env("CRANIBOX_NPM_REGISTRY", registry)
                    .env(
                        "CRANIBOX_MESH_CHECK_TEST_DOWN",
                        if name == "down" { "1" } else { "0" },
                    )
                    .env_remove("CRANIBOX_RUNTIME_ROOT")
                    .env_remove("CRANIBOX_NPM_PACKAGE")
                    .kill_on_drop(true)
                    .output()
                    .await
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            return;
        };
        let root = PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("peers.json");
        Config::update(&path, |_| Ok(())).unwrap();
        let identity = Config::identity(&path).unwrap();
        let public = identity.public().clone();
        let owner = OwnerKey::load_or_create(&root.join("fixture-owner")).unwrap();
        let roster_path = root.join("models/roster.json");
        std::fs::create_dir_all(roster_path.parent().unwrap()).unwrap();
        RosterStore::pin(&roster_path, &owner.public()).unwrap();
        RosterStore::load(roster_path)
            .unwrap()
            .issue(
                &owner,
                vec![RosterMember {
                    node_id: public.node_id.clone(),
                    signing_key: public.signing_key.clone(),
                    x25519_pub: public.noise_key.clone(),
                    role: Role::Node,
                    priority: 0,
                    is_claude_node: false,
                    name: "Update fixture".into(),
                    added_at: 0,
                    identity: None,
                }],
            )
            .unwrap();
        let mut pinned = Pinned::new(public.clone());
        pinned.verified = true;
        let mut grant = Grant::issue("fixture-only", Permissions::default());
        grant.peer = Some(pinned);
        let running = start_with_catalog(
            path.clone(),
            Engine::new(root.join("home")),
            Arc::new(Semaphore::new(1)),
            CancellationToken::new(),
            agent_workspace_runtime::projects::Store::new(root.join("projects.json")),
        )
        .await
        .unwrap();
        let app = running.app.clone();
        let caller = Caller {
            node: public.node_id.clone(),
            grant,
        };
        let router = Router::new()
            .route("/api/v1/nodes/{node}/update", get(read))
            .layer(Extension(caller))
            .layer(Extension(Encrypted))
            .with_state(app);
        for check in [0, 1] {
            let response = router
                .clone()
                .oneshot(
                    axum::http::Request::get(format!(
                        "/api/v1/nodes/{}/update?check={check}",
                        public.node_id
                    ))
                    .body(Body::empty())
                    .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let value: Value = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), 65536)
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(value["node"], public.node_id);
            assert!(value["runtime"].is_object());
            if check == 0 {
                assert!(value["available"].is_null());
            } else if std::env::var("CRANIBOX_MESH_CHECK_TEST_DOWN").as_deref() == Ok("1") {
                assert!(value["available"].is_null());
                assert!(value["available_error"].is_string());
            } else {
                assert_eq!(value["available"]["dev"], "1.2.3");
                assert!(value["available_error"].is_null());
            }
        }
        running.shutdown().await.unwrap();
    }
}
