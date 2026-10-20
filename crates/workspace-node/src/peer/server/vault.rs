//! Signed encrypted anti-entropy records, restricted to verified owner devices.
use super::{App, Caller, Encrypted, Failure, failure, llm};
use axum::{
    Extension, Json, Router,
    extract::{Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::any,
};
use cranibox_contracts::{roster::RosterView, vault::*};
use serde_json::{Value, json};
use std::collections::HashMap;
pub(super) fn routes(_app: App) -> Router<App> {
    Router::new()
        .route("/", any(handle))
        .route("/{*rest}", any(handle))
}
async fn handle(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    encrypted: Option<Extension<Encrypted>>,
    Path(params): Path<HashMap<String, String>>,
    request: Request,
) -> Response {
    let service = match llm::authorize(
        &app,
        &caller,
        params.get("node").map_or("", String::as_str),
        encrypted.is_some(),
        true,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    let path = params.get("rest").map_or("", String::as_str);
    if request.method() == axum::http::Method::GET && path == "roster" {
        return Json(json!(service.roster.current())).into_response();
    }
    if request.method() != axum::http::Method::POST {
        return failure(StatusCode::METHOD_NOT_ALLOWED, "POST required").into_response();
    }
    let bytes = match axum::body::to_bytes(request.into_body(), 8 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return failure(StatusCode::PAYLOAD_TOO_LARGE, "vault bundle too large")
                .into_response();
        }
    };
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return invalid(),
    };
    let result: Result<Value, Failure> = match path {
        "pull" => serde_json::from_value::<PullReq>(value)
            .map_err(|_| bad())
            .and_then(|q| service.vault.pull(q).map(|r| json!(r)).map_err(|_| bad())),
        "push" => serde_json::from_value::<Push>(value)
            .map_err(|_| bad())
            .and_then(|q| service.vault.push(q).map(|_| json!({})).map_err(|_| bad())),
        "lease" => serde_json::from_value::<Lease>(value)
            .map_err(|_| bad())
            .and_then(|q| {
                service
                    .vault
                    .push(Push {
                        records: vec![SignedRecord::Lease(q)],
                    })
                    .map(|_| json!({}))
                    .map_err(|_| bad())
            }),
        "refresh-request" => match serde_json::from_value::<RefreshRequest>(value) {
            Ok(q) => service.refresh(q).await.map(|r| json!(r)).map_err(|_| {
                failure(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "refresh holder unavailable",
                )
            }),
            Err(_) => Err(bad()),
        },
        "roster" => serde_json::from_value(value)
            .map_err(|_| bad())
            .and_then(|r| {
                service
                    .install_roster(r)
                    .map(|_| json!({}))
                    .map_err(|_| bad())
            }),
        _ => Err(failure(StatusCode::NOT_FOUND, "unknown vault route")),
    };
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}
fn bad() -> Failure {
    failure(StatusCode::BAD_REQUEST, "invalid signed vault record")
}
fn invalid() -> Response {
    bad().into_response()
}
