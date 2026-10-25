//! Owner-only model proxy mesh ingress. Forwarded calls always execute locally.
use super::{App, Caller, Encrypted, Failure, check_target, failure};
use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::any,
};
use cranibox_contracts::{
    error::UnavailableBody,
    llm::{mgmt::LlmMgmt, *},
    roster::RosterView,
    vault::AccountStore,
};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};
pub(super) fn routes(_app: App) -> Router<App> {
    Router::new()
        .route("/stream", axum::routing::get(upgrade))
        .route("/", any(handle))
        .route("/{*rest}", any(handle))
}
pub(super) async fn authorize(
    app: &App,
    caller: &Caller,
    node: &str,
    encrypted: bool,
    vault: bool,
) -> Result<Arc<crate::models::Models>, Failure> {
    check_target(app, node)?;
    let permitted = if vault {
        caller.grant.permissions.vault
    } else {
        caller.grant.permissions.llm
    };
    if !encrypted || !permitted || !caller.grant.peer.as_ref().is_some_and(|p| p.verified) {
        return Err(failure(
            StatusCode::FORBIDDEN,
            "verified end-to-end owner channel required",
        ));
    }
    // A capability grant alone never makes a guest an owner. Check membership
    // before starting a proxy or opening the vault on behalf of a remote caller.
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
    let service = app
        .models()
        .await
        .map_err(|_| failure(StatusCode::SERVICE_UNAVAILABLE, "model service unavailable"))?;
    if !service.roster.is_owner_node(&app.node) || !service.roster.is_owner_node(&caller.node) {
        return Err(failure(
            StatusCode::FORBIDDEN,
            "owner roster membership required",
        ));
    }
    Ok(service)
}
async fn handle(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    encrypted: Option<Extension<Encrypted>>,
    Path(params): Path<HashMap<String, String>>,
    request: Request,
) -> Response {
    let service = match authorize(
        &app,
        &caller,
        params.get("node").map_or("", String::as_str),
        encrypted.is_some(),
        false,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    let path = format!("/{}", params.get("rest").map_or("", String::as_str));
    if request.method() == axum::http::Method::GET {
        return match path.as_str() {
            "/health" => Json(json!(LlmHealth {
                version: env!("CARGO_PKG_VERSION").into(),
                hub: service.roster.hub().is_some_and(|m| m.node_id == app.node),
                accounts: service
                    .vault
                    .list()
                    .into_iter()
                    .map(|a| HealthAccount {
                        id: a.id,
                        provider: a.provider,
                        enabled_here: a.all_nodes || a.enabled_on.contains(&app.node)
                    })
                    .collect(),
                models_etag: service.roster.current().map_or(0, |r| r.epoch).to_string()
            }))
            .into_response(),
            "/v1/models" => match service.proxy.mgmt.models().await {
                Ok(models) => Json(json!({"object":"list","data":models})).into_response(),
                Err(_) => {
                    failure(StatusCode::SERVICE_UNAVAILABLE, "models unavailable").into_response()
                }
            },
            _ => failure(StatusCode::NOT_FOUND, "unknown model route").into_response(),
        };
    }
    if request.method() != axum::http::Method::POST {
        return failure(StatusCode::METHOD_NOT_ALLOWED, "POST required").into_response();
    }
    let wire = match path.as_str() {
        "/v1/chat/completions" => Wire::Chat,
        "/v1/responses" => Wire::Responses,
        "/v1/messages" => Wire::Messages,
        "/v1/messages/count_tokens" => Wire::Messages,
        _ => return failure(StatusCode::NOT_FOUND, "unknown model route").into_response(),
    };
    let session_id = request
        .headers()
        .get("x-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = match axum::body::to_bytes(request.into_body(), 32 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return failure(StatusCode::PAYLOAD_TOO_LARGE, "model request too large")
                .into_response();
        }
    };
    let body: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return failure(StatusCode::BAD_REQUEST, "JSON required").into_response(),
    };
    let Some((provider, model)) = body
        .get("model")
        .and_then(Value::as_str)
        .and_then(|m| m.split_once('/'))
    else {
        return failure(StatusCode::BAD_REQUEST, "model must be provider/model").into_response();
    };
    let provider = provider.to_owned();
    let model = model.to_owned();
    let stream = body["stream"].as_bool().unwrap_or(false);
    let req = CanonicalRequest {
        wire,
        body,
        stream,
        session_id,
        client_agent: None,
        model,
    };
    let result = if path.ends_with("/count_tokens") {
        service.proxy.dispatch_count_local(&provider, req).await
    } else {
        service.proxy.dispatch_local(&provider, req).await
    };
    match result {
        Ok(upstream) => {
            let mut response = Response::builder().status(upstream.status);
            for (key, value) in upstream.headers {
                if matches!(
                    key.to_ascii_lowercase().as_str(),
                    "content-type" | "cache-control" | "retry-after"
                ) {
                    response = response.header(key, value);
                }
            }
            response
                .body(Body::from_stream(upstream.body.into_stream()))
                .unwrap_or_else(|_| {
                    failure(StatusCode::BAD_GATEWAY, "invalid model response").into_response()
                })
        }
        Err(cranibox_llm::ProxyError::Unavailable(code)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(UnavailableBody::new(code, code.as_str(), None)),
        )
            .into_response(),
        Err(cranibox_llm::ProxyError::Upstream(UpstreamError::Unsupported)) => {
            failure(StatusCode::NOT_IMPLEMENTED, "model operation unsupported").into_response()
        }
        Err(_) => failure(StatusCode::BAD_GATEWAY, "model upstream unavailable").into_response(),
    }
}

/// Metadata and chunks share the authenticated stream; a close without the sealed
/// end marker is an error. Dropping the downstream body closes upstream work.
async fn upgrade(
    State(app): State<App>,
    Extension(access_key): Extension<super::access::AccessKey>,
    Extension(caller): Extension<Caller>,
    Path(params): Path<HashMap<String, String>>,
    secret: Option<Extension<crate::peer::secure_ws::StreamSecret>>,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> Response {
    let Some(Extension(secret)) = secret else {
        return failure(StatusCode::FORBIDDEN, "encrypted stream required").into_response();
    };
    let node = params.get("node").cloned().unwrap_or_default();
    if let Err(error) = authorize(&app, &caller, &node, true, false).await {
        return error.into_response();
    }
    let Ok(permit) = app.streams.clone().try_acquire_owned() else {
        return failure(StatusCode::TOO_MANY_REQUESTS, "model stream limit reached")
            .into_response();
    };
    let access =
        match super::access::begin(&app, &caller, crate::access::Capability::Llm, &access_key) {
            Ok(access) => access,
            Err(error) => return error.into_response(),
        };
    upgrade
        .max_message_size(33 * 1024 * 1024)
        .max_frame_size(33 * 1024 * 1024)
        .on_upgrade(move |raw| async move {
            let _permit = permit;
            let mut socket = crate::peer::secure_ws::SecureSocket::new(raw, Some(&secret));
            tokio::select! {
                _ = serve_stream(&mut socket, app, caller, node) => {},
                _ = access.revoked() => {},
            }
        })
        .into_response()
}
async fn serve_stream(
    socket: &mut crate::peer::secure_ws::SecureSocket,
    app: App,
    caller: Caller,
    node: String,
) -> anyhow::Result<()> {
    use axum::extract::ws::Message;
    use futures_util::StreamExt;
    use std::time::Duration;
    let message = tokio::time::timeout(Duration::from_secs(5), socket.recv())
        .await?
        .ok_or_else(|| anyhow::anyhow!("stream ended"))??;
    let Message::Text(text) = message else {
        anyhow::bail!("model request frame required")
    };
    let input: Value = serde_json::from_str(text.as_str())?;
    let path = input["path"].as_str().unwrap_or_default();
    anyhow::ensure!(
        matches!(
            path,
            "/health"
                | "/v1/models"
                | "/v1/chat/completions"
                | "/v1/responses"
                | "/v1/messages"
                | "/v1/messages/count_tokens"
        ),
        "invalid model route"
    );
    let method = input["method"].as_str().unwrap_or_default();
    anyhow::ensure!(matches!(method, "GET" | "POST"), "invalid model method");
    let mut request = Request::builder().method(method).uri(path);
    if let Some(headers) = input["headers"].as_array() {
        for pair in headers {
            if pair[0].as_str() == Some("x-session-id")
                && let Some(value) = pair[1].as_str()
            {
                request = request.header("x-session-id", value);
            }
        }
    }
    let request = request.body(Body::from(serde_json::to_vec(&input["body"])?))?;
    let params = HashMap::from([
        ("node".into(), node.clone()),
        ("rest".into(), path.trim_start_matches('/').into()),
    ]);
    let response = handle(
        State(app.clone()),
        Extension(caller.clone()),
        Some(Extension(Encrypted)),
        Path(params),
        request,
    )
    .await;
    let headers: Vec<_> = response
        .headers()
        .iter()
        .filter(|(k, _)| matches!(k.as_str(), "content-type" | "cache-control" | "retry-after"))
        .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str(), v)))
        .collect();
    socket
        .send(Message::Text(
            serde_json::to_string(&json!({"status":response.status().as_u16(),"headers":headers}))?
                .into(),
        ))
        .await?;
    let mut body = response.into_body().into_data_stream();
    loop {
        let chunk = tokio::select! {chunk=tokio::time::timeout(Duration::from_secs(90),body.next())=>chunk?,frame = socket.recv()=>{match frame{Some(Ok(Message::Ping(_) | Message::Pong(_)))=>continue,_=>anyhow::bail!("downstream closed")}},_ = app.stop.cancelled()=>anyhow::bail!("node stopping")};
        let Some(chunk) = chunk else { break };
        let chunk = chunk?;
        let current = agent_workspace_runtime::peers::config::Config::read(&app.path)?;
        anyhow::ensure!(
            current
                .grants
                .get(&caller.node)
                .is_some_and(|g| g.usable() && g.permissions.llm),
            "model permission revoked"
        );
        authorize(&app, &caller, &node, true, false)
            .await
            .map_err(|_| anyhow::anyhow!("owner removed"))?;
        for part in chunk.chunks(64 * 1024) {
            tokio::time::timeout(
                Duration::from_secs(30),
                socket.send(Message::Binary(part.to_vec().into())),
            )
            .await??;
        }
    }
    socket.send(Message::Text("{\"end\":true}".into())).await?;
    Ok(())
}
