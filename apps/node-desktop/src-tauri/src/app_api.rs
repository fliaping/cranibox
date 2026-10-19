//! The desktop and HTTP adapters dispatch to the same App API and return the C6 JSON shapes.
use crate::AppState;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tauri::{Emitter, State};

async fn call(state: &AppState, method: &str, path: &str, body: Value) -> Result<Value, Value> {
    let local = state.service.lock().await.local.clone();
    cranibox_node::app::dispatch(local, method, path, body)
        .await
        .map_err(|error| json!({"code":error.code,"message":error.message}))
}
#[tauri::command]
pub async fn app_call(
    state: State<'_, Arc<AppState>>,
    method: String,
    path: String,
    body: Option<Value>,
) -> Result<Value, Value> {
    call(&state, &method, &path, body.unwrap_or(Value::Null)).await
}
#[tauri::command]
pub async fn app_status(state: State<'_, Arc<AppState>>) -> Result<Value, Value> {
    call(&state, "GET", "/status", Value::Null).await
}
#[tauri::command]
pub async fn app_devices(state: State<'_, Arc<AppState>>) -> Result<Value, Value> {
    call(&state, "GET", "/devices", Value::Null).await
}
#[tauri::command]
pub async fn app_pair_invite(state: State<'_, Arc<AppState>>, req: Value) -> Result<Value, Value> {
    call(&state, "POST", "/pair/invite", req).await
}
#[tauri::command]
pub async fn app_pair_accept(state: State<'_, Arc<AppState>>, req: Value) -> Result<Value, Value> {
    call(&state, "POST", "/pair/accept", req).await
}
#[tauri::command]
pub async fn app_pair_confirm(
    state: State<'_, Arc<AppState>>,
    flow: String,
    req: Value,
) -> Result<Value, Value> {
    call(&state, "POST", &format!("/pair/{flow}/confirm"), req).await
}
#[tauri::command]
pub async fn app_approvals(state: State<'_, Arc<AppState>>) -> Result<Value, Value> {
    call(&state, "GET", "/approvals", Value::Null).await
}
#[tauri::command]
pub async fn app_approval_decide(
    state: State<'_, Arc<AppState>>,
    id: String,
    req: Value,
) -> Result<Value, Value> {
    call(&state, "POST", &format!("/approvals/{id}"), req).await
}
#[tauri::command]
pub async fn app_agents(state: State<'_, Arc<AppState>>) -> Result<Value, Value> {
    call(&state, "GET", "/agents", Value::Null).await
}
#[tauri::command]
pub async fn app_agent_update(
    state: State<'_, Arc<AppState>>,
    id: String,
    req: Value,
) -> Result<Value, Value> {
    call(&state, "PUT", &format!("/agents/{id}"), req).await
}
#[tauri::command]
pub async fn app_models(
    state: State<'_, Arc<AppState>>,
    method: String,
    path: String,
    body: Option<Value>,
) -> Result<Value, Value> {
    call(
        &state,
        &method,
        &format!("/models{path}"),
        body.unwrap_or(Value::Null),
    )
    .await
}
#[tauri::command]
pub async fn app_events(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), Value> {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let state = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        let mut cursor = cranibox_node::app::EventCursor::default();
        loop {
            let local = state.service.lock().await.local.clone();
            if let Ok(events) = cursor.poll(local).await {
                for event in events {
                    let _ = app.emit("app://event", event);
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    });
    Ok(())
}
#[tauri::command]
pub async fn app_open_external(url: String) -> Result<(), String> {
    let parsed = url::Url::parse(&url).map_err(|_| "Invalid sign-in URL")?;
    if !matches!(parsed.scheme(), "https" | "http")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("Invalid sign-in URL".into());
    }
    crate::open_browser(parsed.as_str()).await
}
