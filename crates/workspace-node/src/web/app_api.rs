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
    if request
        .extensions()
        .get::<cranibox_contracts::net::Authenticated>()
        .is_some()
    {
        return StatusCode::FORBIDDEN.into_response();
    }
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
    let Some(mut path) = decode_path(
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
    if let Some(query) = request.uri().query() {
        path.push('?');
        path.push_str(query);
    }
    if path.split('?').next() == Some("/events") && request.method() == Method::GET {
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
        Ok(value) => (success_status(&method, &path, &value), Json(value)).into_response(),
        Err(error) => (
            StatusCode::from_u16(error.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(
                error
                    .body
                    .clone()
                    .unwrap_or_else(|| serde_json::to_value(error).unwrap_or_default()),
            ),
        )
            .into_response(),
    }
}

fn success_status(method: &str, path: &str, value: &Value) -> StatusCode {
    if method == "POST"
        && path.split('?').next() == Some("/update")
        && value["id"].is_string()
        && value.get("phase").is_none()
    {
        StatusCode::ACCEPTED
    } else {
        StatusCode::OK
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

#[cfg(test)]
mod update_tests {
    use super::*;
    use crate::web::SessionStore;
    use axum::{
        body::Body,
        http::{HeaderMap, Request as HttpRequest},
    };
    use std::sync::Arc;

    struct Sessions;
    #[async_trait::async_trait]
    impl SessionStore for Sessions {
        async fn session(&self, headers: &HeaderMap) -> Option<String> {
            (headers.get("cookie")?.to_str().ok()? == "fixture=session").then(|| "session".into())
        }
        async fn csrf(&self, _: &str) -> Option<String> {
            Some("fixture-csrf".into())
        }
        async fn alive(&self, _: &str) -> bool {
            true
        }
    }

    const TEST: &str = "web::app_api::update_tests::mock_http_update_queries_status_codes_and_auth";

    #[tokio::test]
    async fn mock_http_update_queries_status_codes_and_auth() {
        if std::env::var("CRANIBOX_APP_HTTP_TEST").as_deref() != Ok("1") {
            let output = tokio::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--nocapture"])
                .env("CRANIBOX_APP_HTTP_TEST", "1")
                .env("CRANIBOX_APP_MOCK", "1")
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
            return;
        }
        let (_, closing) = tokio::sync::watch::channel(false);
        let services = Services::new(None, Default::default(), Arc::new(Sessions), closing);
        let call = |method: &str, path: &str, body: Value, cookie: bool, csrf: bool| {
            let mut request = HttpRequest::builder()
                .method(method)
                .uri(format!("/api/v1/app{path}"));
            if cookie {
                request = request.header("cookie", "fixture=session");
            }
            if csrf {
                request = request.header("x-workspace-csrf", "fixture-csrf");
            }
            handle(
                services.clone(),
                request.body(Body::from(body.to_string())).unwrap(),
            )
        };
        assert_eq!(
            call("GET", "/update", Value::Null, false, false)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call("POST", "/update", json!({"action":"apply"}), true, false)
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        let info = call(
            "GET",
            "/update?node=node-0a1b2c3d4e5f60718293a4b5c6d7e8f9&check=1",
            Value::Null,
            true,
            false,
        )
        .await;
        assert_eq!(info.status(), StatusCode::OK);
        let info: Value =
            serde_json::from_slice(&axum::body::to_bytes(info.into_body(), 65536).await.unwrap())
                .unwrap();
        assert_eq!(info["self"], false);
        assert_eq!(info["available"]["dev"], "1.1.1-dev.7");
        let started = call(
            "POST",
            "/update",
            json!({"action":"apply","spec":"dev"}),
            true,
            true,
        )
        .await;
        assert_eq!(started.status(), StatusCode::ACCEPTED);
        let started: Value = serde_json::from_slice(
            &axum::body::to_bytes(started.into_body(), 65536)
                .await
                .unwrap(),
        )
        .unwrap();
        let progress = call(
            "GET",
            &format!("/update/{}?node=self", started["id"].as_str().unwrap()),
            Value::Null,
            true,
            false,
        )
        .await;
        assert_eq!(progress.status(), StatusCode::OK);
        assert_eq!(
            call("GET", "/update/unknown", Value::Null, true, false)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call("GET", "/update?check=bad", Value::Null, true, false)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        for action in ["rollback", "adopt"] {
            assert_eq!(
                call("POST", "/update", json!({"action":action}), true, true)
                    .await
                    .status(),
                StatusCode::OK
            );
        }
    }
}

#[cfg(test)]
mod access_tests {
    use super::*;
    use crate::web::SessionStore;
    use axum::body::Body;
    use std::sync::Arc;

    struct Sessions;
    #[async_trait::async_trait]
    impl SessionStore for Sessions {
        async fn session(&self, headers: &axum::http::HeaderMap) -> Option<String> {
            (headers.get("cookie")?.to_str().ok()? == "fixture=session").then(|| "session".into())
        }
        async fn csrf(&self, _: &str) -> Option<String> {
            Some("fixture-csrf".into())
        }
        async fn alive(&self, _: &str) -> bool {
            true
        }
    }
    #[tokio::test]
    async fn access_http_cookie_csrf_and_authenticated_mesh_rejection() {
        if std::env::var("CRANIBOX_ACCESS_HTTP_TEST").as_deref() != Ok("1") {
            let output = tokio::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "web::app_api::access_tests::access_http_cookie_csrf_and_authenticated_mesh_rejection", "--nocapture"])
                .env("CRANIBOX_ACCESS_HTTP_TEST", "1").env("CRANIBOX_APP_MOCK", "1")
                .kill_on_drop(true).output().await.unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let (_, closing) = tokio::sync::watch::channel(false);
        let services = Services::new(None, Default::default(), Arc::new(Sessions), closing);
        for (cookie, csrf, mesh, expected) in [
            (false, false, false, StatusCode::UNAUTHORIZED),
            (true, false, false, StatusCode::FORBIDDEN),
            (true, true, true, StatusCode::FORBIDDEN),
            (true, true, false, StatusCode::OK),
        ] {
            let mut request = Request::builder()
                .method("PUT")
                .uri("/api/v1/app/access/node-0123456789abcdef0123456789abcdef")
                .header("x-forwarded-for", "127.0.0.1");
            if cookie {
                request = request.header("cookie", "fixture=session");
            }
            if csrf {
                request = request.header("x-workspace-csrf", "fixture-csrf");
            }
            let mut request = request.body(Body::from(r#"{"terminals":"ask"}"#)).unwrap();
            if mesh {
                request
                    .extensions_mut()
                    .insert(cranibox_contracts::net::Authenticated {
                        node_id: "remote".into(),
                        endpoint_id: None,
                        transport: cranibox_contracts::net::TransportKind::Iroh,
                        e2e: true,
                    });
            }
            assert_eq!(handle(services.clone(), request).await.status(), expected);
        }
    }

    #[tokio::test]
    async fn requester_http_adapter_preserves_websocket_approval_response() {
        let body = json!({"error":"approval_required","request_id":"request","capability":"terminals","expires_at":123});
        let response = tokio_tungstenite::tungstenite::http::Response::builder()
            .status(403)
            .body(Some(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let error = anyhow::Error::new(tokio_tungstenite::tungstenite::Error::Http(Box::new(
            response,
        )));
        let app_error = crate::app::remote_error(error);
        assert_eq!(app_error.status, 403);
        assert_eq!(app_error.body, Some(body.clone()));
        let response = tokio_tungstenite::tungstenite::http::Response::builder()
            .status(403)
            .body(Some(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let response = crate::web::remote_error(
            StatusCode::BAD_GATEWAY,
            anyhow::Error::new(tokio_tungstenite::tungstenite::Error::Http(Box::new(
                response,
            ))),
        );
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let returned: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(returned, body);
    }
}
