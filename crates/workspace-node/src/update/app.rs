//! App update routes, shared by the cookie adapter and native IPC.
use super::{manager::Updater, npm::Registries};
use crate::app::error;
use agent_workspace_runtime::{client::Fleet, protocol::valid_node_id};
use cranibox_contracts::app::AppError;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{LazyLock, Mutex},
};

pub(crate) struct Context<'a> {
    pub node: &'a str,
    pub peers: &'a Path,
    pub tasks: &'a tokio::sync::Mutex<tokio::task::JoinSet<()>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Action {
    Apply,
    Rollback,
    Adopt,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    node: Option<String>,
    action: Action,
    spec: Option<String>,
    #[serde(default)]
    force: bool,
}

fn unavailable(e: impl std::fmt::Display) -> AppError {
    error(503, "unavailable", e.to_string())
}
fn conflict(e: impl std::fmt::Display) -> AppError {
    error(409, "conflict", e.to_string())
}
fn remote(e: anyhow::Error) -> AppError {
    error(502, "remote", format!("{e:#}"))
}
fn missing() -> AppError {
    error(404, "not_found", "Unknown update id")
}

pub(crate) async fn route(
    context: Option<Context<'_>>,
    method: &str,
    path: &str,
    query: &str,
    body: Value,
) -> Result<Value, AppError> {
    let mut node = None;
    let mut check = false;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "node" => node = Some(value.into_owned()),
            "check" if value == "0" || value == "1" => check = value == "1",
            "check" => return Err(error(400, "invalid", "check must be 0 or 1")),
            _ => {}
        }
    }
    let request = if method == "POST" && path == "/update" {
        let request: Request = serde_json::from_value(body)
            .map_err(|_| error(400, "invalid", "Invalid update request"))?;
        node.clone_from(&request.node);
        Some(request)
    } else if method == "GET" {
        None
    } else {
        return Err(error(405, "invalid", "GET or POST required"));
    };
    let local_node = context.as_ref().map_or(MOCK_SELF, |c| c.node);
    let node = node.as_deref().unwrap_or(local_node);
    let is_self = node == "self" || node == local_node;
    let node = if is_self { local_node } else { node };
    if !valid_node_id(node) {
        return Err(error(400, "invalid", "Invalid device ID"));
    }
    let id = path.strip_prefix("/update/");
    if let Some(id) = id
        && (id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'))
    {
        return Err(missing());
    }
    if let Some(request) = &request {
        if matches!(request.action, Action::Adopt) && !is_self {
            return Err(error(
                400,
                "invalid",
                "Adopt is only available on this device",
            ));
        }
        if matches!(request.action, Action::Apply)
            && !super::valid_spec(request.spec.as_deref().unwrap_or("latest"))
        {
            return Err(error(
                400,
                "invalid",
                "spec must be latest, next, dev or an exact version",
            ));
        }
    }
    let Some(context) = context else {
        return mock(node, is_self, id, check, request);
    };
    if !is_self {
        let fleet = Fleet::new(context.peers.to_owned());
        if let Some(id) = id {
            return fleet
                .update_progress(node, id)
                .await
                .map_err(remote)?
                .ok_or_else(missing);
        }
        if let Some(request) = request {
            let (suffix, body) = match request.action {
                Action::Apply => (
                    "",
                    json!({"spec":request.spec.unwrap_or_else(|| "latest".into()), "force":request.force}),
                ),
                Action::Rollback => ("/rollback", json!({})),
                Action::Adopt => unreachable!(),
            };
            return fleet
                .post(node, &format!("/api/v1/nodes/{node}/update{suffix}"), body)
                .await
                .map_err(remote);
        }
        let mut value = fleet
            .get(
                node,
                &format!("/api/v1/nodes/{node}/update?check={}", u8::from(check)),
            )
            .await
            .map_err(remote)?;
        value["self"] = false.into();
        return Ok(value);
    }
    let root = super::runtime_root(context.peers.parent().unwrap_or(context.peers));
    let updater = Updater::new(
        root.clone(),
        Registries::from_env().map_err(unavailable)?,
        super::manager::default_entry_package(),
    );
    if let Some(id) = id {
        return updater
            .status()
            .map_err(unavailable)?
            .filter(|s| s.id == id)
            .map(|s| json!(s))
            .ok_or_else(missing);
    }
    if let Some(request) = request {
        return match request.action {
            Action::Adopt => {
                if updater.info().map_err(unavailable)?.managed {
                    return Err(conflict("This device already has managed versions"));
                }
                let version = tokio::task::spawn_blocking(move || super::cli::adopt(&root))
                    .await
                    .map_err(unavailable)?
                    .map_err(conflict)?;
                Ok(json!({"adopted":version, "needs_restart":true}))
            }
            Action::Apply => {
                let info = serving_info(&updater)?;
                if !info.can_update {
                    return Err(conflict(format!(
                        "This device cannot update itself: {}",
                        info.reasons.join("; ")
                    )));
                }
                let (id, task) = updater
                    .start(
                        request.spec.unwrap_or_else(|| "latest".into()),
                        request.force,
                    )
                    .map_err(conflict)?;
                let mut tasks = context.tasks.lock().await;
                while tasks.try_join_next().is_some() {}
                tasks.spawn(task);
                Ok(json!({"id":id}))
            }
            Action::Rollback => {
                let status = updater
                    .rollback("requested on this device")
                    .map_err(conflict)?;
                context
                    .tasks
                    .lock()
                    .await
                    .spawn(super::restart_after_response());
                Ok(json!(status))
            }
        };
    }
    let mut value = updater
        .snapshot(node, super::product_version(), check)
        .await
        .map_err(unavailable)?;
    value["runtime"] = json!(serving_info(&updater)?);
    value["self"] = true.into();
    Ok(value)
}

fn serving_info(updater: &Updater) -> Result<super::manager::RuntimeInfo, AppError> {
    let mut info = updater.info().map_err(unavailable)?;
    if info.managed && std::env::var_os("CRANIBOX_RUNTIME_MANAGED").is_none() {
        info.can_update = false;
        info.reasons.push("this process was not started by the cranibox launcher; restart with cranibox update launch".into());
    }
    Ok(info)
}

const MOCK_SELF: &str = "node-1f2e3d4c5b6a79880716253443526170";
const MOCK_MANAGED: &str = "node-0a1b2c3d4e5f60718293a4b5c6d7e8f9";
const MOCK_UNMANAGED: &str = "node-2c3d4e5f60718293a4b5c6d7e8f90a1b";
static MOCK_UPDATES: LazyLock<Mutex<BTreeMap<String, (Value, usize)>>> =
    LazyLock::new(Default::default);

fn mock(
    node: &str,
    is_self: bool,
    id: Option<&str>,
    check: bool,
    request: Option<Request>,
) -> Result<Value, AppError> {
    if ![MOCK_SELF, MOCK_MANAGED, MOCK_UNMANAGED].contains(&node) {
        return Err(error(502, "remote", "Device unavailable"));
    }
    let managed = node != MOCK_UNMANAGED;
    let mut updates = MOCK_UPDATES.lock().unwrap();
    if let Some(id) = id {
        let (status, poll) = updates
            .get_mut(node)
            .filter(|(s, _)| s["id"] == id)
            .ok_or_else(missing)?;
        if status["phase"] != "rolled_back" {
            status["phase"] =
                ["resolving", "downloading", "activated", "done"][(*poll).min(3)].into();
            status["downloaded"] = json!(if *poll == 0 {
                0
            } else if *poll == 1 {
                512
            } else {
                1024
            });
            status["updated_at"] = json!(1_792_800_000_u64 + *poll as u64);
            *poll += 1;
        }
        return Ok(status.clone());
    }
    if let Some(request) = request {
        return match request.action {
            Action::Adopt => Ok(json!({"adopted":"1.1.0", "needs_restart":true})),
            Action::Apply if !managed => Err(error(
                if is_self { 409 } else { 502 },
                "cannot_update",
                "This device cannot update itself: not started by the cranibox launcher",
            )),
            Action::Apply | Action::Rollback => {
                let rollback = matches!(request.action, Action::Rollback);
                let id = uuid::Uuid::new_v4().to_string();
                let spec = request.spec.unwrap_or_else(|| "latest".into());
                let to = if rollback {
                    "1.0.0"
                } else {
                    match spec.as_str() {
                        "latest" => "1.1.0",
                        "next" | "dev" => "1.1.1-dev.7",
                        version => version,
                    }
                };
                let status = json!({"id":id, "spec":spec, "phase":if rollback {"rolled_back"} else {"resolving"},
                    "from":"1.1.0", "to":to, "error":null,
                    "downloaded":0, "total":1024, "started_at":1_792_800_000_u64, "updated_at":1_792_800_000_u64});
                updates.insert(node.into(), (status.clone(), 0));
                Ok(if rollback { status } else { json!({"id":id}) })
            }
        };
    }
    Ok(json!({"node":node, "self":is_self, "version":"1.1.0",
        "runtime":{"managed":managed, "state":if managed {Some("confirmed")} else {None}, "version":if managed {Some("1.1.0")} else {None},
            "os":if is_self {"macos"} else {"linux"}, "arch":if is_self {"aarch64"} else {"x86_64"}, "can_update":managed,
            "reasons":if managed {vec![]} else {vec!["not started by the cranibox launcher"]}},
        "status":updates.get(node).map(|(s, _)| s),
        "available":if check {Some(json!({"latest":"1.1.0", "next":null, "dev":"1.1.1-dev.7"}))} else {None}, "available_error":null}))
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod tests {
    use super::*;
    use crate::update::{layout::Layout, manager::tests::Fixture};
    use std::time::Duration;

    const TEST: &str = "update::app::tests::self_routes_adopt_apply_restart_and_rollback";

    #[tokio::test]
    async fn remote_routes_forward_queries_actions_and_errors() {
        use agent_workspace_runtime::peers::config::{Config, Route};
        use axum::{Json, Router, body::Body, http::Request, http::StatusCode};
        use std::sync::Arc;

        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        let credential = root.path().join("fixture-credential");
        std::fs::write(&credential, "a".repeat(64)).unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = calls.clone();
        let mesh = Router::new().fallback(move |request: Request<Body>| {
            let seen = seen.clone();
            async move {
                let path = request.uri().path().to_owned();
                // A legacy fake endpoint exercises Fleet's HTTP forwarding. The
                // existing mesh tests separately exercise owner authorization over Noise.
                if path == "/api/v1/mesh/keys" {
                    return (StatusCode::NOT_FOUND, Json(json!({"error":"no keys"})));
                }
                let method = request.method().to_string();
                let uri = request.uri().to_string();
                let bytes = axum::body::to_bytes(request.into_body(), 4096).await.unwrap();
                let body: Value = if bytes.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes).unwrap()
                };
                seen.lock().unwrap().push((method.clone(), uri, body.clone()));
                if path.ends_with("/missing") {
                    (StatusCode::NOT_FOUND, Json(json!({"error":"unknown update id"})))
                } else if body["spec"] == "next" {
                    (StatusCode::CONFLICT, Json(json!({"error":"target cannot restart"})))
                } else if path.ends_with("/rollback") || path.ends_with("/task") {
                    (StatusCode::OK, Json(json!({"id":"task", "phase":"rolled_back"})))
                } else if method == "POST" {
                    (StatusCode::ACCEPTED, Json(json!({"id":"task"})))
                } else {
                    (StatusCode::OK, Json(json!({"node":MOCK_MANAGED, "version":"1.1.0",
                        "runtime":{"managed":true}, "status":null,
                        "available":{"latest":null,"next":null,"dev":"1.1.1-dev.7"}, "available_error":null})))
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, mesh).await.unwrap() });
        Config::update(&peers, |config| {
            config.routes.insert(
                MOCK_MANAGED.into(),
                Route {
                    url,
                    token_file: credential,
                    allow_insecure: false,
                    local_forward_lock: None,
                    alternatives: vec![],
                    target_identity: None,
                    config_path: None,
                },
            );
            Ok(())
        })
        .unwrap();
        let tasks = tokio::sync::Mutex::new(tokio::task::JoinSet::new());
        let context = || {
            Some(Context {
                node: MOCK_SELF,
                peers: &peers,
                tasks: &tasks,
            })
        };
        let query = format!("node={MOCK_MANAGED}&check=1");
        let info = route(context(), "GET", "/update", &query, Value::Null)
            .await
            .unwrap();
        assert_eq!(info["self"], false);
        assert_eq!(info["available"]["dev"], "1.1.1-dev.7");
        let started = route(
            context(),
            "POST",
            "/update",
            "",
            json!({"node":MOCK_MANAGED,"action":"apply","spec":"dev","force":true}),
        )
        .await
        .unwrap();
        assert_eq!(started["id"], "task");
        assert_eq!(
            route(context(), "GET", "/update/task", &query, Value::Null)
                .await
                .unwrap()["id"],
            "task"
        );
        assert_eq!(
            route(context(), "GET", "/update/missing", &query, Value::Null)
                .await
                .unwrap_err()
                .status,
            404
        );
        let refused = route(
            context(),
            "POST",
            "/update",
            "",
            json!({"node":MOCK_MANAGED,"action":"apply","spec":"next"}),
        )
        .await
        .unwrap_err();
        assert_eq!(refused.status, 502);
        assert!(refused.message.contains("target cannot restart"));
        let rolled = route(
            context(),
            "POST",
            "/update",
            "",
            json!({"node":MOCK_MANAGED,"action":"rollback"}),
        )
        .await
        .unwrap();
        assert_eq!(rolled["phase"], "rolled_back");
        assert_eq!(
            route(
                context(),
                "POST",
                "/update",
                "",
                json!({"node":MOCK_MANAGED,"action":"adopt"})
            )
            .await
            .unwrap_err()
            .status,
            400
        );
        {
            let calls = calls.lock().unwrap();
            assert_eq!(
                calls[0].1,
                format!("/api/v1/mesh/nodes/{MOCK_MANAGED}/update?check=1")
            );
            assert_eq!(calls[1].2, json!({"spec":"dev", "force":true}));
            assert_eq!(calls.len(), 6, "adopt must not be forwarded");
        }
        server.abort();
        let _ = server.await;
        assert_eq!(
            route(context(), "GET", "/update", &query, Value::Null)
                .await
                .unwrap_err()
                .status,
            502
        );
    }

    #[tokio::test]
    async fn self_routes_adopt_apply_restart_and_rollback() {
        if let Ok(mode) = std::env::var("CRANIBOX_APP_UPDATE_TEST_MODE") {
            let root = std::env::var_os("CRANIBOX_RUNTIME_ROOT").unwrap();
            let peers = Path::new(&root).join("peers.json");
            let tasks = tokio::sync::Mutex::new(tokio::task::JoinSet::new());
            let context = || {
                Some(Context {
                    node: MOCK_SELF,
                    peers: &peers,
                    tasks: &tasks,
                })
            };
            let info = route(context(), "GET", "/update", "check=1", Value::Null)
                .await
                .unwrap();
            assert_eq!(info["self"], true);
            assert_eq!(info["available"]["dev"], "1.2.0-dev.17");
            if mode == "adopt" {
                assert_eq!(info["runtime"]["managed"], false);
                let refused = route(
                    context(),
                    "POST",
                    "/update",
                    "",
                    json!({"action":"apply","spec":"dev"}),
                )
                .await
                .unwrap_err();
                assert_eq!(refused.status, 409);
                assert!(refused.message.contains("launcher"));
                let adopted = route(context(), "POST", "/update", "", json!({"action":"adopt"}))
                    .await
                    .unwrap();
                assert_eq!(adopted["adopted"], super::super::product_version());
                assert_eq!(adopted["needs_restart"], true);
                assert!(Layout::new(&root).pointer().unwrap().is_some());
                assert_eq!(
                    route(context(), "POST", "/update", "", json!({"action":"adopt"}))
                        .await
                        .unwrap_err()
                        .status,
                    409
                );
                let info = route(context(), "GET", "/update", "", Value::Null)
                    .await
                    .unwrap();
                assert_eq!(
                    info["runtime"]["can_update"], false,
                    "adoption alone cannot restart this process"
                );
            } else {
                let stop = tokio_util::sync::CancellationToken::new();
                super::super::install_restart_hook(stop.clone());
                let started = route(
                    context(),
                    "POST",
                    "/update",
                    "",
                    json!({"action":"apply","spec":"1.1.0"}),
                )
                .await
                .unwrap();
                let path = format!("/update/{}", started["id"].as_str().unwrap());
                let first = route(context(), "GET", &path, "", Value::Null)
                    .await
                    .unwrap();
                assert_eq!(
                    first["id"], started["id"],
                    "the status exists as soon as apply returns"
                );
                let duplicate = route(
                    context(),
                    "POST",
                    "/update",
                    "",
                    json!({"action":"apply","spec":"1.1.0"}),
                )
                .await
                .unwrap_err();
                assert_eq!(duplicate.status, 409);
                tokio::time::timeout(Duration::from_secs(20), stop.cancelled())
                    .await
                    .unwrap();
                assert!(super::super::restart_requested());
                assert_eq!(
                    route(context(), "GET", &path, "", Value::Null)
                        .await
                        .unwrap()["phase"],
                    "activated"
                );
                let rolled = route(
                    context(),
                    "POST",
                    "/update",
                    "",
                    json!({"action":"rollback"}),
                )
                .await
                .unwrap();
                assert_eq!(rolled["phase"], "rolled_back");
                assert_eq!(rolled["to"], "1.0.0");
                assert_eq!(
                    route(context(), "GET", "/update/missing", "", Value::Null)
                        .await
                        .unwrap_err()
                        .status,
                    404
                );
            }
            return;
        }
        // Child processes keep edition-2024 environment changes out of parallel tests.
        let fixture = Fixture::new("1.0.0", false, false, false).await;
        let adoption = tempfile::tempdir().unwrap();
        for (mode, root) in [("adopt", adoption.path()), ("managed", fixture.root.path())] {
            let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", TEST, "--nocapture"])
                .env("CRANIBOX_APP_UPDATE_TEST_MODE", mode)
                .env("CRANIBOX_NPM_REGISTRY", &fixture.base)
                .env("CRANIBOX_RUNTIME_ROOT", root)
                .env_remove("CRANIBOX_RUNTIME_VERSION")
                .env_remove("CRANIBOX_NPM_PACKAGE")
                .kill_on_drop(true);
            if mode == "adopt" {
                command.env_remove("CRANIBOX_RUNTIME_MANAGED");
            } else {
                command.env("CRANIBOX_RUNTIME_MANAGED", "1");
            }
            let output = tokio::time::timeout(Duration::from_secs(60), command.output())
                .await
                .expect("self update child timed out")
                .unwrap();
            assert!(
                output.status.success(),
                "{mode}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
