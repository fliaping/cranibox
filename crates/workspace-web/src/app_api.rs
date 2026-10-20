//! Cookie-authenticated C6 adapter, sharing its implementation with native IPC.
use crate::server::State;
use axum::{
    Json, Router,
    extract::{Request, State as AppState},
    http::{Method, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::any,
};
use futures_util::{StreamExt, stream};
use serde_json::Value;
use std::{convert::Infallible, time::Duration};
use subtle::ConstantTimeEq;
pub(crate) fn router() -> Router<State> {
    Router::new().route("/api/v1/app/{*rest}", any(handle))
}
async fn handle(AppState(state): AppState<State>, request: Request) -> Response {
    let Some(session) = state.session(request.headers()).await else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"code":"unauthorized","message":"Open Cranibox to sign in"})),
        )
            .into_response();
    };
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        let csrf = request
            .headers()
            .get("x-workspace-csrf")
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default();
        if !bool::from(session.lock().await.csrf.as_bytes().ct_eq(csrf.as_bytes())) {
            return (StatusCode::FORBIDDEN,Json(serde_json::json!({"code":"forbidden","message":"Session expired. Reload the page"}))).into_response();
        }
    }
    let path = request
        .uri()
        .path()
        .strip_prefix("/api/v1/app")
        .unwrap_or_default()
        .to_owned();
    if path == "/events" && request.method() == Method::GET {
        let local = state.local.clone();
        let events = stream::unfold(
            (local, cranibox_node::app::EventCursor::default()),
            |(local, mut cursor)| async move {
                loop {
                    match cursor.poll(local.clone()).await {
                        Ok(events) if !events.is_empty() => {
                            let batch = events
                                .into_iter()
                                .filter_map(|event| {
                                    let value = serde_json::to_value(event).ok()?;
                                    Some(Ok::<_, Infallible>(
                                        Event::default()
                                            .event(value["event"].as_str()?)
                                            .data(value["data"].to_string()),
                                    ))
                                })
                                .collect::<Vec<_>>();
                            return Some((stream::iter(batch), (local, cursor)));
                        }
                        _ => {}
                    }
                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
            },
        )
        .flatten();
        return Sse::new(events)
            .keep_alive(KeepAlive::default())
            .into_response();
    }
    let method = request.method().as_str().to_owned();
    let body = match axum::body::to_bytes(request.into_body(), 1024 * 1024).await {
        Ok(bytes) if bytes.is_empty() => Value::Null,
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"code":"invalid","message":"JSON required"})),
                )
                    .into_response();
            }
        },
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    match cranibox_node::app::dispatch(state.local.clone(), &method, &path, body).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => (
            StatusCode::from_u16(error.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(error),
        )
            .into_response(),
    }
}
