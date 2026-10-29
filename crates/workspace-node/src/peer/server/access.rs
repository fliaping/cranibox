use super::*;
use crate::access::{Capability, Operation, Risk, Store};
use sha2::{Digest, Sha256};
use std::sync::{LazyLock, Mutex};

/// The access key a handler re-checks when it starts its session, and whether the
/// operation needs a one-time approval (see `Operation::confirm`).
#[derive(Clone)]
pub(super) struct AccessKey(pub String, pub bool);
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
        .start_for(&caller.node, capability, &key.0, key.1)
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
/// Remote operations a peer may start per minute before this device answers 429. Bounds
/// how fast a misbehaving peer can push older entries out of the audit log.
const OPERATIONS_PER_MINUTE: u32 = 240;
/// Reads are polled constantly; the log keeps one per peer and kind per this many seconds.
const READ_EVERY: u64 = 600;
/// Refusals of the same operation are logged once per minute.
const REFUSAL_EVERY: u64 = 60;
static RECENT: LazyLock<Mutex<std::collections::HashMap<String, u64>>> =
    LazyLock::new(Default::default);
static RATE: LazyLock<Mutex<std::collections::HashMap<String, (u64, u32)>>> =
    LazyLock::new(Default::default);
fn due(key: String, every: u64) -> bool {
    let now = crate::access::now();
    let mut recent = RECENT.lock().unwrap_or_else(|e| e.into_inner());
    if recent.len() > 4096 {
        recent.retain(|_, at| now.saturating_sub(*at) < READ_EVERY);
    }
    match recent.get(&key) {
        Some(at) if now.saturating_sub(*at) < every => false,
        _ => {
            recent.insert(key, now);
            true
        }
    }
}
/// Counts one non-read operation; false once the peer is over its per-minute budget.
fn within_rate(node: &str) -> bool {
    let minute = crate::access::now() / 60;
    let mut rate = RATE.lock().unwrap_or_else(|e| e.into_inner());
    rate.retain(|_, (m, _)| *m == minute);
    let entry = rate.entry(node.to_owned()).or_insert((minute, 0));
    entry.1 += 1;
    entry.1 <= OPERATIONS_PER_MINUTE
}
struct Recorder {
    store: Store,
    node: String,
    capability: Option<Capability>,
    operation: Option<Operation>,
    transport: &'static str,
}
impl Recorder {
    /// Writes the outcome to the target's audit chain. Reads and repeated refusals are
    /// coalesced so polling cannot flood the log.
    async fn finish(self, basis: Option<String>, status: u16) {
        let Some(operation) = self.operation else {
            return;
        };
        let result = match status {
            0..400 => "ok",
            403 => "refused",
            429 => "rate_limited",
            _ => "failed",
        };
        let coalesce = if result == "refused" || result == "rate_limited" {
            Some(REFUSAL_EVERY)
        } else if operation.risk == Risk::Read {
            Some(READ_EVERY)
        } else {
            None
        };
        if let Some(every) = coalesce
            && !due(format!("{}:{}:{result}", self.node, operation.kind), every)
        {
            return;
        }
        let (store, node, capability, transport) =
            (self.store, self.node, self.capability, self.transport);
        let written = tokio::task::spawn_blocking(move || {
            store.record(
                &node,
                capability,
                &operation,
                basis.as_deref(),
                result,
                status,
                transport,
            )
        })
        .await;
        if !matches!(written, Ok(Ok(()))) {
            eprintln!("record remote operation: {written:?}");
        }
    }
}
/// Space ids are opaque to the person reading the log; show the name next to it.
fn space_name(app: &App, operation: &mut Operation) {
    if !operation.kind.starts_with("spaces.") || operation.summary.is_empty() {
        return;
    }
    if let Ok(catalog) = crate::spaces::Store::new(app.path.clone()).shared_catalog(&app.node)
        && let Some(space) = catalog
            .spaces
            .iter()
            .find(|s| s.id.to_string() == operation.summary)
    {
        let name: String = space
            .name
            .chars()
            .filter(|c| !c.is_control())
            .take(80)
            .collect();
        operation.summary = format!("{name} ({})", operation.summary);
    }
}
pub(super) async fn enforce(State(app): State<App>, request: Request, next: Next) -> Response {
    let Some(caller) = request.extensions().get::<Caller>().cloned() else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let route = request
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|p| {
            p.as_str()
                .strip_prefix("/api/v1/mesh")
                .unwrap_or(p.as_str())
                .to_owned()
        });
    let mut recorder = Recorder {
        store: store(&app),
        node: caller.node.clone(),
        capability: None,
        operation: None,
        transport: if request.extensions().get::<Encrypted>().is_some() {
            "e2e"
        } else {
            "bearer"
        },
    };
    let response = guarded(&app, caller, route, request, next, &mut recorder).await;
    let status = response.status().as_u16();
    let basis = response.extensions().get::<Basis>().map(|b| b.0.clone());
    recorder.finish(basis, status).await;
    response
}
#[derive(Clone)]
struct Basis(String);
async fn guarded(
    app: &App,
    caller: Caller,
    route_name: Option<String>,
    mut request: Request,
    next: Next,
    recorder: &mut Recorder,
) -> Response {
    let app = app.clone();
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
    let route_name = route_name.unwrap_or_else(|| path.clone());
    let method = request.method().as_str().to_owned();
    recorder.capability = Some(caps[0]);
    // Until the body is read, describe the call from its route alone.
    recorder.operation =
        crate::access::operations::describe(&method, &route_name, &BTreeMap::new(), &Value::Null);

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
            Ok(Some(decision)) => {
                let mut response = next.run(request).await;
                response.extensions_mut().insert(Basis(decision.basis));
                return response;
            }
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
    recorder.operation = crate::access::operations::describe(&method, &route_name, &query, &value)
        .map(|mut o| {
            space_name(&app, &mut o);
            o
        });
    let once = recorder.operation.as_ref().is_some_and(|o| o.confirm);
    if recorder
        .operation
        .as_ref()
        .is_some_and(|o| o.risk != Risk::Read)
        && !within_rate(&caller.node)
    {
        return failure(
            StatusCode::TOO_MANY_REQUESTS,
            "too many remote operations; try again in a minute",
        )
        .into_response();
    }
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
    let mut basis = None;
    for capability in &caps {
        match store(&app).authorize_for(
            &caller.node,
            *capability,
            &key,
            recorder.operation.as_ref(),
        ) {
            Ok(decision) => {
                basis.get_or_insert(decision.basis);
            }
            Err(e) => {
                if matches!(e, crate::access::Denied::ApprovalRequired { .. }) {
                    // The request_created entry already names the operation.
                    recorder.operation = None;
                }
                return e.response().into_response();
            }
        }
    }
    request = Request::from_parts(parts, Body::from(bytes));
    request
        .extensions_mut()
        .insert(AccessKey(key.clone(), once));
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
            match store(&app).start_for(&caller.node, capability, &key, once) {
                Ok(g) => guards.push(g),
                Err(_) => {
                    return failure(StatusCode::FORBIDDEN, "access changed before use")
                        .into_response();
                }
            }
        }
    }
    let mut response = next.run(request).await;
    if let Some(basis) = basis {
        response.extensions_mut().insert(Basis(basis));
    }
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
