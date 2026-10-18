//! Binary noVNC adapter with explicit receive acknowledgements for IPC backpressure.
use agent_workspace_runtime::peers::{
    Store,
    desktop::{Event, MAX_CHUNK},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tauri::{State, ipc::Channel};
use tokio::sync::{Mutex, Semaphore, mpsc};

enum Input {
    Data(Vec<u8>),
    Ack(u64),
}
pub struct Desktops {
    attachments: Mutex<HashMap<String, mpsc::Sender<Input>>>,
    slots: Arc<Semaphore>,
}
impl Default for Desktops {
    fn default() -> Self {
        Self {
            attachments: Mutex::default(),
            slots: Arc::new(Semaphore::new(8)),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    node: String,
    #[serde(default)]
    local: bool,
    #[serde(default)]
    relay: bool,
}
#[tauri::command]
pub async fn desktop_open(
    state: State<'_, Arc<Desktops>>,
    app: State<'_, Arc<super::AppState>>,
    options: Options,
    events: Channel<Value>,
) -> Result<String, String> {
    if options.local && options.relay {
        return Err("Choose one desktop route".into());
    }
    let permit = state
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| "Maximum eight desktop viewers")?;
    let mut desktop = if options.local {
        let local = app
            .service
            .lock()
            .await
            .local
            .clone()
            .ok_or("Local device service unavailable")?;
        if local.id != options.node {
            return Err("Local device identity mismatch".into());
        }
        local.desktop().await
    } else if options.relay {
        let settings = app
            .service
            .lock()
            .await
            .settings
            .clone()
            .ok_or("Configure this device first")?;
        settings
            .api()
            .map_err(|e| e.to_string())?
            .desktop(&options.node)
            .await
    } else {
        let store = Store::new(cranibox_node::peer::config::path_for(
            &cranibox_node::settings::default_path(),
        ));
        let api = store
            .route(&options.node)
            .and_then(|route| route.api(&options.node))
            .map_err(|e| e.to_string())?;
        api.desktop(&options.node).await
    }
    .map_err(|e| e.to_string())?;
    let id = agent_workspace_runtime::config::token();
    let (sender, mut input) = mpsc::channel(32);
    state.attachments.lock().await.insert(id.clone(), sender);
    let owner = state.inner().clone();
    let attachment = id.clone();
    tauri::async_runtime::spawn(async move {
        let _permit = permit;
        let mut sequence = 0u64;
        let mut pending = None;
        let mut delivered = tokio::time::Instant::now();
        if events.send(json!({"type":"ready"})).is_ok() {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep_until(delivered + Duration::from_secs(10)), if pending.is_some() => break,
                    input = input.recv() => match input {
                        Some(Input::Data(data)) => if desktop.send(data).is_err() { break; },
                        Some(Input::Ack(value)) => { if pending != Some(value) { break; } pending = None; },
                        None => break,
                    },
                    output = desktop.output.recv(), if pending.is_none() => match output {
                        Some(Event::Data(data)) => {
                            sequence += 1;
                            if events.send(json!({"type":"data","sequence":sequence,"data":STANDARD.encode(data)})).is_err() { break; }
                            pending = Some(sequence); delivered = tokio::time::Instant::now();
                        }
                        Some(Event::Error(message)) => { let _ = events.send(json!({"type":"error","message":message})); break; }
                        None => break,
                    },
                }
            }
        }
        owner.attachments.lock().await.remove(&attachment);
        drop(desktop);
        let _ = events.send(json!({"type":"closed"}));
    });
    Ok(id)
}
#[tauri::command]
pub async fn desktop_input(
    state: State<'_, Arc<Desktops>>,
    id: String,
    data: String,
) -> Result<(), String> {
    if data.len() > MAX_CHUNK.div_ceil(3) * 4 {
        return Err("Desktop input too large".into());
    }
    let bytes = STANDARD.decode(data).map_err(|_| "Invalid desktop input")?;
    if bytes.is_empty() || bytes.len() > MAX_CHUNK {
        return Err("Invalid desktop input size".into());
    }
    state
        .attachments
        .lock()
        .await
        .get(&id)
        .ok_or("Desktop detached")?
        .try_send(Input::Data(bytes))
        .map_err(|_| "Desktop input unavailable".into())
}
#[tauri::command]
pub async fn desktop_ack(
    state: State<'_, Arc<Desktops>>,
    id: String,
    sequence: u64,
) -> Result<(), String> {
    state
        .attachments
        .lock()
        .await
        .get(&id)
        .ok_or("Desktop detached")?
        .try_send(Input::Ack(sequence))
        .map_err(|_| "Desktop acknowledgement unavailable".into())
}
#[tauri::command]
pub async fn desktop_close(state: State<'_, Arc<Desktops>>, id: String) -> Result<(), String> {
    state.attachments.lock().await.remove(&id);
    Ok(())
}
