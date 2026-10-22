//! Cookie-authenticated C6 adapter (`/api/v1/app/*`), sharing `app::dispatch` with IPC.
use super::Services;
use axum::{
    Json,
    extract::Request,
    http::{Method, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};
use std::{convert::Infallible, time::Duration};
use subtle::ConstantTimeEq;

pub async fn handle(services: Services, request: Request) -> Response {
    let Some(session) = services.sessions.session(request.headers()).await else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"code":"unauthorized","message":"Open Cranibox to sign in"})),
        )
            .into_response();
    };
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        let csrf = request
            .headers()
            .get("x-workspace-csrf")
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default();
        let expected = services.sessions.csrf(&session).await.unwrap_or_default();
        if expected.is_empty() || !bool::from(expected.as_bytes().ct_eq(csrf.as_bytes())) {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"code":"forbidden","message":"Session expired. Reload the page"})),
            )
                .into_response();
        }
    }
    let Some(path) = decode_path(
        request
            .uri()
            .path()
            .strip_prefix("/api/v1/app")
            .unwrap_or_default(),
    ) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"code":"invalid","message":"Invalid path"})),
        )
            .into_response();
    };
    if path == "/events" && request.method() == Method::GET {
        let local = services.local.clone();
        let events = stream::unfold(
            (local, crate::app::EventCursor::default()),
            |(local, mut cursor)| async move {
                loop {
                    if let Ok(events) = cursor.poll(local.clone()).await
                        && !events.is_empty()
                    {
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
                    Json(json!({"code":"invalid","message":"JSON required"})),
                )
                    .into_response();
            }
        },
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    match crate::app::dispatch(services.local.clone(), &method, &path, body).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => (
            StatusCode::from_u16(error.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(error),
        )
            .into_response(),
    }
}

/// Percent-decode each path segment, as browsers encode IDs such as `pair:…` with
/// `encodeURIComponent`. A segment may not decode to `/` or invalid UTF-8, so encoding
/// can never change which route a request reaches.
fn decode_path(path: &str) -> Option<String> {
    let segments = path
        .split('/')
        .map(|segment| {
            let bytes = segment.as_bytes();
            let mut out = Vec::with_capacity(bytes.len());
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'%' {
                    let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
                    out.push(u8::from_str_radix(hex, 16).ok()?);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            let decoded = String::from_utf8(out).ok()?;
            (!decoded.contains('/')).then_some(decoded)
        })
        .collect::<Option<Vec<_>>>()?;
    Some(segments.join("/"))
}

#[cfg(test)]
mod tests {
    use super::decode_path;

    #[test]
    fn approval_ids_are_decoded_without_changing_routes() {
        assert_eq!(
            decode_path("/approvals/pair%3Aabc").as_deref(),
            Some("/approvals/pair:abc")
        );
        assert_eq!(decode_path("/status").as_deref(), Some("/status"));
        assert_eq!(decode_path("/approvals/a%2Fb"), None, "no encoded slash");
        assert_eq!(decode_path("/approvals/%ZZ"), None);
        assert_eq!(
            decode_path("/approvals/%E4%B8%AD").as_deref(),
            Some("/approvals/中")
        );
        assert_eq!(decode_path("/approvals/%FF"), None, "invalid UTF-8");
    }
}
