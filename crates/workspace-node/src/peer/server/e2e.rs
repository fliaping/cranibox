//! Target side of end-to-end encrypted calls. See `agent_workspace_runtime::peers::e2e`.
use super::*;
use agent_workspace_runtime::peers::{
    config::Pinned,
    e2e::{self, SESSION_GONE, SESSION_HEADER},
};
use axum::body::Bytes;
use tower::ServiceExt;

/// Marks an internally dispatched, already authenticated and decrypted request. Remote
/// clients cannot set request extensions.
#[derive(Clone)]
pub(super) struct Sealed(pub Caller);

#[derive(Clone)]
pub(super) struct Dispatch {
    pub app: App,
    pub api: Router,
}

fn browser(headers: &HeaderMap) -> bool {
    headers.contains_key("origin")
        || headers
            .get("sec-fetch-site")
            .is_some_and(|h| h == "cross-site")
}
fn targeted(app: &App, headers: &HeaderMap) -> bool {
    headers
        .get("x-cranibox-target")
        .and_then(|h| h.to_str().ok())
        == Some(&app.node)
}
fn precheck(app: &App, headers: &HeaderMap, node: Option<&str>) -> Result<(), Failure> {
    if app.connections.is_cancelled() {
        return Err(failure(StatusCode::SERVICE_UNAVAILABLE, "listener stopped"));
    }
    if browser(headers) {
        return Err(failure(
            StatusCode::FORBIDDEN,
            "direct node API does not accept browser requests",
        ));
    }
    if !targeted(app, headers) || node.is_some_and(|node| node != app.node) {
        return Err(failure(
            StatusCode::CONFLICT,
            "expected target node ID required",
        ));
    }
    Ok(())
}

/// Public, self-signed identity. Requires the expected NodeID so that the listener does
/// not answer blind scans.
pub(super) async fn keys(State(app): State<App>, headers: HeaderMap) -> Response {
    if let Err(error) = precheck(&app, &headers, None) {
        return error.into_response();
    }
    Json(app.identity.public().clone()).into_response()
}

/// The caller's current grant, if the session still matches it (revocation, rotation
/// and key changes end sessions).
fn current(
    app: &App,
    caller: &agent_workspace_runtime::peers::identity::PublicIdentity,
    credential: &str,
) -> Option<Grant> {
    Config::read(&app.path)
        .ok()
        .filter(|config| config.node_id == app.node)
        .and_then(|mut config| config.grants.remove(&caller.node_id))
        .filter(|grant| {
            config::matches(&grant.credential, credential)
                && grant
                    .peer
                    .as_ref()
                    .is_some_and(|peer| peer.identity.same_keys(caller))
        })
}
pub(super) fn session_valid(app: &App, session: &e2e::ServerSession) -> bool {
    current(app, &session.caller, &session.credential).is_some()
}

pub(super) async fn handshake(
    State(app): State<App>,
    Path(node): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(error) = precheck(&app, &headers, Some(&node)) {
        return error.into_response();
    }
    let rejected = || {
        failure(
            StatusCode::UNAUTHORIZED,
            "end-to-end handshake rejected: wrong credential, or this device key is not the one the caller pinned (compare the pairing code and pair again)",
        )
    };
    let Ok(pending) = e2e::respond(&app.identity, &body) else {
        return rejected().into_response();
    };
    let caller = pending.hello.caller.clone();
    let secret = pending.hello.secret.clone();
    let path = app.path.clone();
    let checked = tokio::task::spawn_blocking(move || -> Result<String, Failure> {
        let check = |config: &Config| -> Result<(String, bool), Failure> {
            let grant = config
                .grants
                .get(&caller.node_id)
                .filter(|grant| grant.verifies(&secret))
                .ok_or_else(rejected)?;
            match &grant.peer {
                Some(peer) if peer.identity.same_keys(&caller) => {
                    Ok((grant.credential.clone(), false))
                }
                Some(_) => Err(failure(
                    StatusCode::UNAUTHORIZED,
                    "caller key differs from the paired key; revoke and grant again to re-pair",
                )),
                None => Ok((grant.credential.clone(), true)),
            }
        };
        let unavailable = || {
            failure(
                StatusCode::SERVICE_UNAVAILABLE,
                "peer configuration unavailable",
            )
        };
        let (credential, pin) = check(&Config::read(&path).map_err(|_| unavailable())?)?;
        if pin {
            // Trust on first use: bind the caller key to this grant. Owners confirm it
            // by comparing the pairing code (`peer verify`).
            let mut result = Err(rejected());
            Config::update(&path, |config| {
                result = check(config).map(|(credential, _)| credential);
                if result.is_ok()
                    && let Some(grant) = config.grants.get_mut(&caller.node_id)
                    && grant.peer.is_none()
                {
                    grant.peer = Some(Pinned::new(caller.clone()));
                }
                Ok(())
            })
            .map_err(|_| unavailable())?;
            return result;
        }
        Ok(credential)
    })
    .await;
    let credential = match checked {
        Ok(Ok(credential)) => credential,
        Ok(Err(error)) => return error.into_response(),
        Err(_) => return rejected().into_response(),
    };
    let message = {
        let mut sessions = app.sessions.lock().expect("session table");
        pending.accept(&mut sessions, credential)
    };
    match message {
        Ok(message) => (
            [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
            message,
        )
            .into_response(),
        Err(_) => rejected().into_response(),
    }
}

pub(super) async fn call(
    State(dispatch): State<Dispatch>,
    Path(node): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let app = &dispatch.app;
    if let Err(error) = precheck(app, &headers, Some(&node)) {
        return error.into_response();
    }
    let gone = || {
        (
            StatusCode::from_u16(SESSION_GONE).expect("status"),
            Json(json!({"error":"end-to-end session unknown or expired; renew it"})),
        )
            .into_response()
    };
    let Some(id) = headers
        .get(SESSION_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned)
    else {
        return gone();
    };
    let opened = {
        let mut sessions = app.sessions.lock().expect("session table");
        let Some(session) = sessions.get(&id) else {
            return gone();
        };
        session.open_request(&body).map(|(nonce, inner)| {
            (
                nonce,
                inner,
                session.caller.clone(),
                session.credential.clone(),
            )
        })
    };
    let Ok((nonce, inner, identity, credential)) = opened else {
        return failure(StatusCode::BAD_REQUEST, "encrypted request rejected").into_response();
    };
    let Some(grant) = current(app, &identity, &credential) else {
        // Revoked or rotated since the handshake: drop the session. Nothing ran, so the
        // caller may handshake again with its current credential (which is re-checked).
        app.sessions.lock().expect("session table").remove(&id);
        return gone();
    };
    if !grant.usable() {
        return failure(
            StatusCode::FORBIDDEN,
            "pairing verification pending: compare the pairing code on both devices, then run peer verify",
        )
        .into_response();
    }
    let caller = Caller {
        node: identity.node_id.clone(),
        grant,
    };
    let method = if inner.method == "GET" {
        axum::http::Method::GET
    } else {
        axum::http::Method::POST
    };
    let Ok(mut request) = Request::builder()
        .method(method)
        .uri(&inner.path)
        .header("x-cranibox-target", &app.node)
        .header("content-type", "application/json")
        .body(Body::from(inner.body))
    else {
        return failure(StatusCode::BAD_REQUEST, "encrypted request rejected").into_response();
    };
    request.extensions_mut().insert(Sealed(caller));
    let Ok(response) = dispatch.api.clone().oneshot(request).await;
    let status = response.status().as_u16();
    let Ok(bytes) = axum::body::to_bytes(response.into_body(), e2e::MAX_FRAME - 64).await else {
        return failure(StatusCode::INTERNAL_SERVER_ERROR, "response too large").into_response();
    };
    let sealed = {
        let mut sessions = app.sessions.lock().expect("session table");
        match sessions.get(&id) {
            Some(session) => session.seal_response(nonce, status, &bytes),
            None => return gone(),
        }
    };
    match sealed {
        Ok(frame) => (
            [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
            frame,
        )
            .into_response(),
        Err(_) => failure(StatusCode::INTERNAL_SERVER_ERROR, "encryption failed").into_response(),
    }
}

/// Authorize a WebSocket upgrade with a session-bound proof instead of a bearer secret.
pub(super) fn stream_caller(app: &App, header: &str, path: &str) -> Option<Caller> {
    let (id, proof) = header.split_once('.')?;
    let (identity, credential) = {
        let mut sessions = app.sessions.lock().ok()?;
        let session = sessions.get(id)?;
        session.open_stream_proof(proof, path).ok()?;
        (session.caller.clone(), session.credential.clone())
    };
    let grant = current(app, &identity, &credential).filter(Grant::usable)?;
    Some(Caller {
        node: identity.node_id,
        grant,
    })
}
