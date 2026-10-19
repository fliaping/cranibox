//! Loopback data plane; the independent management transport has no TCP listener.
pub(crate) mod engine;
use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, DefaultBodyLimit, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cranibox_contracts::{error::UnavailableBody, llm::*};
use engine::{CallError, Engine};
use serde_json::{Value, json};
use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::sync::CancellationToken;

pub struct Handle {
    pub addresses: Vec<SocketAddr>,
    pub management: PathBuf,
    pub mgmt: crate::mgmt::LocalMgmt,
    cancel: CancellationToken,
    tasks: Vec<JoinHandle<()>>,
    engine: Arc<Engine>,
}
impl Handle {
    /// Called by WS6 only after authenticating and authorizing a mesh request.
    /// Bypasses remote routing so an already-forwarded request cannot loop.
    pub async fn dispatch_local(
        &self,
        provider: &str,
        request: CanonicalRequest,
    ) -> Result<UpstreamResponse, crate::ProxyError> {
        self.engine.send_local(provider, request, false).await
    }
    pub async fn dispatch_count_local(
        &self,
        provider: &str,
        request: CanonicalRequest,
    ) -> Result<UpstreamResponse, crate::ProxyError> {
        self.engine.send_local(provider, request, true).await
    }
    pub async fn shutdown(mut self) {
        self.cancel.cancel();
        for mut task in self.tasks.drain(..) {
            if tokio::time::timeout(std::time::Duration::from_secs(5), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Bind all requested loopback listeners before returning. Never falls back to 0.0.0.0.
pub async fn spawn(config: crate::Config) -> anyhow::Result<Handle> {
    anyhow::ensure!(
        !config.node.is_empty() && !config.listen.is_empty(),
        "node and listener addresses are required"
    );
    for preset in &config.presets {
        crate::catalog::validate(preset).map_err(anyhow::Error::msg)?;
    }
    anyhow::ensure!(
        config.listen.iter().all(
            |a| matches!(a.ip(),IpAddr::V4(ip) if ip==std::net::Ipv4Addr::LOCALHOST)
                || matches!(a.ip(),IpAddr::V6(ip) if ip==std::net::Ipv6Addr::LOCALHOST)
        ),
        "only 127.0.0.1 and ::1 may be bound"
    );
    let mut listeners = Vec::new();
    for address in &config.listen {
        listeners.push(TcpListener::bind(address).await?);
    }
    let management = config.run_dir.join(LLM_MGMT_SOCKET);
    let mgmt_listener = crate::mgmt::Listener::bind(&management).await?;
    let engine = Engine::new(config);
    let mgmt = crate::mgmt::LocalMgmt::new(engine.clone());
    let cancel = CancellationToken::new();
    let mut tasks = Vec::new();
    let mut addresses = Vec::new();
    for listener in listeners {
        let address = listener.local_addr()?;
        addresses.push(address);
        let router = data_router(engine.clone())
            .layer(middleware::from_fn_with_state(address.port(), guard));
        let stop = cancel.clone();
        tasks.push(tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(stop.cancelled_owned())
            .await;
        }));
    }
    tasks.push(tokio::spawn(crate::mgmt::serve(
        mgmt_listener,
        mgmt.clone(),
        cancel.clone(),
    )));
    Ok(Handle {
        addresses,
        management,
        mgmt,
        cancel,
        tasks,
        engine,
    })
}
fn data_router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(chat))
        .route("/v1/responses", post(responses))
        .route("/v1/messages", post(messages))
        .route("/v1/messages/count_tokens", post(count_tokens))
        .route("/v1/models", get(models))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(engine)
}
fn forbidden_headers(headers: &HeaderMap) -> bool {
    headers.keys().any(|h| {
        let h = h.as_str();
        matches!(
            h,
            "forwarded" | "x-real-ip" | "cf-connecting-ip" | "true-client-ip" | "origin"
        ) || h.starts_with("x-forwarded-")
            || h.starts_with("tailscale-")
            || h.starts_with("sec-fetch-")
    })
}
async fn guard(State(port): State<u16>, request: Request, next: Next) -> Response {
    let peer = request.extensions().get::<ConnectInfo<SocketAddr>>();
    let host = request
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.parse::<http::uri::Authority>().ok());
    let valid_host = host.is_some_and(|h| {
        matches!(
            h.host().to_ascii_lowercase().as_str(),
            "127.0.0.1" | "[::1]" | "localhost"
        ) && h.port_u16().unwrap_or(80) == port
    });
    if peer.is_none_or(|p| !p.0.ip().is_loopback())
        || forbidden_headers(request.headers())
        || !valid_host
    {
        return error(
            StatusCode::FORBIDDEN,
            "local_clients_only",
            "loopback clients without forwarding or browser headers are required",
        );
    }
    next.run(request).await
}
async fn chat(State(e): State<Arc<Engine>>, h: HeaderMap, Json(v): Json<Value>) -> Response {
    call(e, h, v, Wire::Chat, false).await
}
async fn responses(State(e): State<Arc<Engine>>, h: HeaderMap, Json(v): Json<Value>) -> Response {
    call(e, h, v, Wire::Responses, false).await
}
async fn messages(State(e): State<Arc<Engine>>, h: HeaderMap, Json(v): Json<Value>) -> Response {
    call(e, h, v, Wire::Messages, false).await
}
async fn count_tokens(
    State(e): State<Arc<Engine>>,
    h: HeaderMap,
    Json(v): Json<Value>,
) -> Response {
    call(e, h, v, Wire::Messages, true).await
}
async fn call(
    engine: Arc<Engine>,
    headers: HeaderMap,
    mut body: Value,
    wire: Wire,
    count: bool,
) -> Response {
    let Some(model) = body.get("model").and_then(Value::as_str) else {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "model is required",
        );
    };
    let Some((provider, model)) = model.split_once('/') else {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "model must be <provider>/<model>",
        );
    };
    if provider.is_empty()
        || !provider
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        || model.is_empty()
    {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "invalid provider or model",
        );
    }
    let (provider, model) = (provider.to_string(), model.to_string());
    body["model"] = model.clone().into();
    let stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false) && !count;
    let session_id = headers
        .get("x-session-id")
        .and_then(|h| h.to_str().ok())
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .map(str::to_owned);
    let key = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .or_else(|| headers.get("x-api-key").and_then(|h| h.to_str().ok()));
    let client_agent = key
        .and_then(|k| k.strip_prefix(LOCAL_KEY_PREFIX))
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .map(str::to_owned);
    match engine
        .send(
            &provider,
            CanonicalRequest {
                wire,
                body,
                stream,
                session_id,
                client_agent,
                model,
            },
            count,
        )
        .await
    {
        Ok(upstream) => {
            let mut builder = Response::builder().status(upstream.status);
            for (name, value) in upstream.headers {
                if matches!(
                    name.as_str(),
                    "content-type" | "retry-after" | "x-request-id" | "request-id"
                ) {
                    builder = builder.header(name, value);
                }
            }
            builder
                .header("cache-control", "no-store")
                .body(Body::from_stream(upstream.body.into_stream()))
                .unwrap_or_else(|_| {
                    error(
                        StatusCode::BAD_GATEWAY,
                        "upstream_error",
                        "invalid upstream response",
                    )
                })
        }
        Err(CallError::Unavailable(code)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(UnavailableBody::new(
                code,
                code.as_str(),
                Some(engine.node.clone()),
            )),
        )
            .into_response(),
        Err(CallError::Upstream(e)) => upstream_error(e),
    }
}
async fn models(State(engine): State<Arc<Engine>>) -> Json<Value> {
    Json(
        json!({"object":"list","data":engine.models().await.into_iter().filter(|m|matches!(m.availability,Availability::Local)).map(|m|json!({"id":m.id,"object":"model","owned_by":m.id.split('/').next().unwrap_or("cranibox"),"cranibox":m})).collect::<Vec<_>>()}),
    )
}
pub(crate) fn error(status: StatusCode, kind: &str, message: &str) -> Response {
    (
        status,
        Json(json!({"error":{"type":kind,"message":message}})),
    )
        .into_response()
}
pub(crate) fn upstream_error(error_value: UpstreamError) -> Response {
    let (status, message) = match error_value {
        UpstreamError::Unsupported => (
            StatusCode::NOT_IMPLEMENTED,
            "this operation is not supported by the selected upstream",
        ),
        UpstreamError::Status(status, _) => (
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
            "upstream rejected request",
        ),
        _ => (StatusCode::BAD_GATEWAY, "upstream request failed"),
    };
    error(status, "upstream_error", message)
}

#[cfg(test)]
mod access_tests {
    use super::*;
    use tower::ServiceExt;
    #[tokio::test]
    async fn non_loopback_or_missing_peer_is_rejected_even_with_local_host() {
        for peer in [
            None,
            Some("192.0.2.1:1234"),
            Some("[2001:db8::1]:1234"),
            Some("127.0.0.1:1234"),
            Some("[::1]:1234"),
        ] {
            let app = Router::new()
                .route("/", get(|| async { "OK" }))
                .layer(middleware::from_fn_with_state(3424u16, guard));
            let mut request = Request::builder()
                .uri("/")
                .header("host", "localhost:3424")
                .body(Body::empty())
                .unwrap();
            if let Some(peer) = peer {
                request
                    .extensions_mut()
                    .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
            }
            let response = app.oneshot(request).await.unwrap();
            assert_eq!(
                response.status(),
                if peer.is_some_and(|p| p.starts_with("127.") || p.starts_with("[::1]")) {
                    200
                } else {
                    403
                }
            );
        }
    }
}
