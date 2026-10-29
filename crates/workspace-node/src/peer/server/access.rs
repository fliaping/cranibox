use super::*;
use crate::access::{Capability, Store};
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub(super) struct AccessKey(pub String);
pub(super) fn store(app: &App) -> Store {
    Store::new(&app.path)
}
pub(super) fn begin(
    app: &App,
    caller: &Caller,
    capability: Capability,
    key: &AccessKey,
) -> Result<crate::access::Session, Failure> {
    store(app)
        .start(&caller.node, capability, &key.0)
        .map_err(|_| failure(StatusCode::FORBIDDEN, "access changed before session start"))
}
pub(super) async fn request_state(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Result<Json<Value>, Failure> {
    store(&app)
        .request(&id, &caller.node)
        .map_err(|_| failure(StatusCode::SERVICE_UNAVAILABLE, "access store unavailable"))?
        .map(|r| Json(json!(r)))
        .ok_or_else(|| failure(StatusCode::NOT_FOUND, "unknown access request"))
}
/// `None` for a per-node route that names no capability.
fn capabilities(path: &str) -> Option<Vec<Capability>> {
    use Capability::*;
    let segments: Vec<_> = path.trim_start_matches('/').split('/').collect();
    let Some(first) = segments.first() else {
        return Some(vec![]);
    };
    if *first == "jobs" {
        return Some(vec![Exec]);
    }
    if *first != "nodes" || segments.len() < 3 {
        return Some(vec![]);
    }
    Some(match segments[2] {
        "terminal" => vec![Terminals],
        "exec" | "executions" | "files" => vec![Exec],
        "projects" => {
            if segments
                .get(3)
                .is_some_and(|s| matches!(*s, "export" | "import" | "transfer" | "copy-status"))
            {
                vec![Projects, Exec]
            } else {
                vec![Projects]
            }
        }
        "desktop" => vec![Desktop],
        "computer" => vec![Computer],
        "browser" => vec![Browser],
        "update" => vec![Update],
        // Stopping or starting a space is container control on this device.
        "spaces" if segments.get(3) == Some(&"control") => vec![Spaces, Exec],
        "spaces" => vec![Spaces],
        "activity" => vec![Activity],
        "messages" | "tasks" | "agent-sessions" => vec![Messages],
        "llm" => vec![Llm],
        "vault" => vec![Vault],
        _ => return None,
    })
}
pub(super) async fn enforce(State(app): State<App>, mut request: Request, next: Next) -> Response {
    let Some(caller) = request.extensions().get::<Caller>().cloned() else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let uri = request
        .extensions()
        .get::<axum::extract::OriginalUri>()
        .map(|u| &u.0)
        .unwrap_or_else(|| request.uri());
    let path = uri
        .path()
        .strip_prefix("/api/v1/mesh")
        .unwrap_or(uri.path())
        .to_owned();
    let route = request
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|p| p.as_str())
        .unwrap_or(&path);
    // Fail closed: a new per-node route must name its capability in `capabilities`.
    let Some(mut caps) = capabilities(route.strip_prefix("/api/v1/mesh").unwrap_or(route)) else {
        return failure(StatusCode::FORBIDDEN, "route has no access capability").into_response();
    };
    // Another node's path is not served here; say so before checking access.
    if let ["nodes", node, ..] = path.trim_start_matches('/').split('/').collect::<Vec<_>>()[..]
        && node != app.node
    {
        return failure(StatusCode::NOT_FOUND, "target is not this node").into_response();
    }
    if caps.is_empty() {
        return next.run(request).await;
    }

    for capability in &caps {
        if !crate::access::granted(&caller.grant.permissions, *capability) {
            return failure(
                StatusCode::FORBIDDEN,
                "capability not granted by target node",
            )
            .into_response();
        }
    }
    if caps
        .iter()
        .any(|c| matches!(c, Capability::Update | Capability::Llm | Capability::Vault))
    {
        use cranibox_contracts::roster::RosterView;
        let roster = agent_workspace_runtime::peers::roster::RosterStore::load(
            app.path.with_file_name("models").join("roster.json"),
        );
        if request.extensions().get::<Encrypted>().is_none()
            || !caller.grant.peer.as_ref().is_some_and(|p| p.verified)
        {
            return failure(
                StatusCode::FORBIDDEN,
                "verified end-to-end owner channel required",
            )
            .into_response();
        }
        if !roster.is_ok_and(|r| r.is_owner_node(&app.node) && r.is_owner_node(&caller.node)) {
            return failure(StatusCode::FORBIDDEN, "owner roster membership required")
                .into_response();
        }
    }
    let segments: Vec<_> = path.trim_start_matches('/').split('/').collect();
    let job = match segments.as_slice() {
        ["jobs", id] if request.method() == axum::http::Method::GET => Some(*id),
        ["jobs", id, "cancel"] => Some(*id),
        ["nodes", _, "update", id] if request.method() == axum::http::Method::GET => Some(*id),
        _ => None,
    };
    if let Some(id) = job {
        match store(&app).job_decision(&caller.node, caps[0], id) {
            Ok(Some(_)) => return next.run(request).await,
            Ok(None) => {}
            Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        }
    }
    // Read only a bounded body; a digest binds approval to this operation, never to its secrets.
    let (parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(
        body,
        agent_workspace_runtime::projects::copy::MAX_WIRE
            .max(agent_workspace_runtime::peers::files::MAX_WIRE)
            .max(32 * 1024 * 1024),
    )
    .await
    {
        Ok(b) => b,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if caps[0] == Capability::Projects && value["operation"] == "bind" {
        if let Err(error) = check_permission(&caller, true) {
            return error.into_response();
        }
        caps.push(Capability::Terminals);
    }
    let query: BTreeMap<_, _> =
        url::form_urlencoded::parse(parts.uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    // Lease follow-up calls use the key authorized for the acquire request. An unknown or
    // foreign lease is authorized like any other call; the handler then refuses it.
    let lease_key = if caps[0] == Capability::Computer
        && let Some(id) = value["lease"].as_str()
    {
        app.computer
            .lock()
            .await
            .as_ref()
            .filter(|l| l.access_owner(&caller, id))
            .map(|l| l.access_key().to_owned())
    } else {
        None
    };
    let key = if caps[0] == Capability::Terminals && path.ends_with("/ws") {
        query.get("session").cloned().unwrap_or_default()
    } else if let Some(key) = lease_key.clone() {
        key
    } else {
        let nonce = query
            .get("access_session")
            .map(String::as_str)
            .or_else(|| {
                parts
                    .headers
                    .get("idempotency-key")
                    .and_then(|h| h.to_str().ok())
            })
            .unwrap_or("");
        let mut digest = Sha256::new();
        digest.update(format!(
            "{}:{path}:{}:{nonce}:",
            parts.method,
            parts.uri.query().unwrap_or_default()
        ));
        digest.update(&bytes);
        format!("{:x}", digest.finalize())
    };
    for capability in &caps {
        if let Err(e) = store(&app).authorize(&caller.node, *capability, &key) {
            return e.response().into_response();
        }
    }
    request = Request::from_parts(parts, Body::from(bytes));
    request.extensions_mut().insert(AccessKey(key.clone()));
    // Streaming handlers transfer the guard to their actual transport/job/lease.
    let held = path.ends_with("/ws")
        || path.ends_with("/desktop/page")
        || path.ends_with("/llm/stream")
        || path.ends_with("/exec")
        || lease_key.is_some()
        || (caps[0] == Capability::Computer && value["operation"] == "acquire")
        || (caps[0] == Capability::Update && request.method() == axum::http::Method::POST);
    let mut guards = Vec::new();
    if !held {
        for capability in caps {
            match store(&app).start(&caller.node, capability, &key) {
                Ok(g) => guards.push(g),
                Err(_) => {
                    return failure(StatusCode::FORBIDDEN, "access changed before use")
                        .into_response();
                }
            }
        }
    }
    let response = next.run(request).await;
    if guards.is_empty() || !path.contains("/llm/") {
        return response;
    }
    let (parts, body) = response.into_parts();
    let stream = futures_util::stream::unfold(
        (body.into_data_stream(), guards),
        |(mut body, guards)| async move {
            use futures_util::StreamExt;
            let mut check = tokio::time::interval(Duration::from_millis(250));
            loop {
                tokio::select! {
                    biased;
                    _ = check.tick() => if guards.iter().any(|g| !g.valid()) { return None; },
                    chunk = body.next() => return chunk.map(|c| (c, (body, guards))),
                }
            }
        },
    );
    Response::from_parts(parts, Body::from_stream(stream))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stopping_or_starting_a_space_needs_exec_as_well_as_the_space_list() {
        use Capability::*;
        assert_eq!(capabilities("/nodes/node-a/spaces"), Some(vec![Spaces]));
        assert_eq!(
            capabilities("/nodes/node-a/spaces/control"),
            Some(vec![Spaces, Exec])
        );
    }
}
