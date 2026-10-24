//! Private loopback endpoint for the versioned runtime's native operations.
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::any,
};
use cranibox_node::web::{error, standalone::Shell};
use serde::Deserialize;
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct Bridge {
    shell: Arc<dyn Shell>,
    token: String,
}

pub async fn serve(
    listener: TcpListener,
    token: String,
    shell: Arc<dyn Shell>,
    stop: CancellationToken,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        listener.local_addr()?.ip().is_loopback(),
        "Shell must listen on loopback"
    );
    anyhow::ensure!(!token.is_empty(), "Shell token is required");
    axum::serve(
        listener,
        Router::new()
            .fallback(any(call))
            .with_state(Bridge { shell, token }),
    )
    .with_graceful_shutdown(stop.cancelled_owned())
    .await?;
    Ok(())
}

async fn call(State(bridge): State<Bridge>, request: Request) -> Response {
    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    if !agent_workspace_runtime::config::matches(token, &bridge.token) {
        return error(StatusCode::UNAUTHORIZED, "Invalid shell token");
    }
    if request.uri().path() != "/shell" || request.uri().query().is_some() {
        return error(StatusCode::NOT_FOUND, "Not found");
    }
    if request.method() != "POST" {
        return error(StatusCode::METHOD_NOT_ALLOWED, "POST required");
    }
    if request
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        != Some("application/json")
    {
        return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "JSON required");
    }
    let bytes =
        match tokio::time::timeout(Duration::from_secs(5), to_bytes(request.into_body(), 32768))
            .await
        {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(_)) => return error(StatusCode::PAYLOAD_TOO_LARGE, "Request too large"),
            Err(_) => return error(StatusCode::REQUEST_TIMEOUT, "Request timed out"),
        };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Call {
        operation: String,
        body: Value,
    }
    let call: Call = match serde_json::from_slice(&bytes) {
        Ok(call) => call,
        Err(_) => return error(StatusCode::BAD_REQUEST, "Invalid shell request"),
    };
    match bridge.shell.call(&call.operation, call.body).await {
        Ok(value) => Json(value).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cranibox_node::web::http_shell::HttpShell;
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct Echo;
    #[async_trait::async_trait]
    impl Shell for Echo {
        async fn call(&self, operation: &str, body: Value) -> anyhow::Result<Value> {
            anyhow::ensure!(operation != "fail", "native error");
            Ok(json!({"operation": operation, "body": body}))
        }
    }

    #[tokio::test]
    async fn http_shell_round_trip_and_request_guards() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let stop = CancellationToken::new();
        let server = tokio::spawn(serve(
            listener,
            "test-secret".into(),
            Arc::new(Echo),
            stop.clone(),
        ));
        let base = format!("http://{address}");
        let shell = HttpShell::new(&base, "test-secret".into()).unwrap();
        assert_eq!(
            shell.call("status", json!({"value":42})).await.unwrap(),
            json!({"operation":"status","body":{"value":42}})
        );
        assert!(
            shell
                .call("fail", Value::Null)
                .await
                .unwrap_err()
                .to_string()
                .contains("native error")
        );
        assert!(
            HttpShell::new(&base, "wrong-token".into())
                .unwrap()
                .call("status", Value::Null)
                .await
                .is_err()
        );
        for (method, path, authorization, body, expected) in [
            ("POST", "/shell", "", "{}".into(), 401),
            (
                "GET",
                "/shell",
                "Authorization: Bearer test-secret\r\n",
                "{}".into(),
                405,
            ),
            (
                "OPTIONS",
                "/shell",
                "Authorization: Bearer test-secret\r\n",
                "{}".into(),
                405,
            ),
            (
                "POST",
                "/other",
                "Authorization: Bearer test-secret\r\n",
                "{}".into(),
                404,
            ),
            (
                "POST",
                "/shell",
                "Authorization: Bearer test-secret\r\n",
                "x".repeat(32769),
                413,
            ),
        ] {
            let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
            stream.write_all(format!("{method} {path} HTTP/1.1\r\nHost: {address}\r\n{authorization}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            assert!(
                response.starts_with(&format!("HTTP/1.1 {expected}")),
                "{response}"
            );
            assert!(
                !response
                    .to_ascii_lowercase()
                    .contains("access-control-allow-origin")
            );
        }
        stop.cancel();
        server.await.unwrap().unwrap();
    }
}
