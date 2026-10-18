//! Native adapter for the same terminal UI used by the browser.
use agent_workspace_runtime::peers::{
    Store,
    terminal::{Event, Input, Terminal},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};
use tauri::{State, ipc::Channel};
use tokio::sync::{Mutex, Semaphore, mpsc};

enum Stream {
    Remote(Terminal),
    Local(cranibox_node::local::Terminal),
}
impl Stream {
    fn send(&self, input: Input) -> anyhow::Result<()> {
        match self {
            Self::Remote(value) => value.send(input),
            Self::Local(value) => value.send(input),
        }
    }
    async fn recv(&mut self) -> Option<Event> {
        match self {
            Self::Remote(value) => value.output.recv().await,
            Self::Local(value) => value.output.recv().await,
        }
    }
}

pub struct Terminals {
    attachments: Mutex<HashMap<String, mpsc::Sender<Input>>>,
    slots: Arc<Semaphore>,
}
impl Default for Terminals {
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
    direct: bool,
    #[serde(default)]
    local: bool,
    session_id: String,
    cols: u16,
    rows: u16,
    #[serde(default)]
    takeover: bool,
}
#[tauri::command]
pub async fn terminal_open(
    state: State<'_, Arc<Terminals>>,
    app: State<'_, Arc<super::AppState>>,
    options: Options,
    events: Channel<Value>,
) -> Result<String, String> {
    let permit = state
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| "Maximum eight terminals".to_owned())?;
    if options.direct && options.local {
        return Err("Invalid terminal transport".into());
    }
    let mut terminal = if options.local {
        let local = app
            .service
            .lock()
            .await
            .local
            .clone()
            .ok_or("Local device service unavailable")?;
        if options.node != local.id {
            return Err("Local device identity mismatch".into());
        }
        local
            .attach(
                &options.session_id,
                options.cols,
                options.rows,
                options.takeover,
            )
            .await
            .map(Stream::Local)
    } else if options.direct {
        let store = Store::new(cranibox_node::peer::config::path_for(
            &cranibox_node::settings::default_path(),
        ));
        let route = store.route(&options.node).map_err(|e| e.to_string())?;
        Terminal::connect(
            route,
            &options.node,
            &options.session_id,
            options.cols,
            options.rows,
            options.takeover,
        )
        .await
        .map(Stream::Remote)
    } else {
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
            .terminal(
                &options.node,
                &options.session_id,
                options.cols,
                options.rows,
                options.takeover,
            )
            .await
            .map(Stream::Remote)
    }
    .map_err(|e| e.to_string())?;
    let id = agent_workspace_runtime::config::token();
    let (sender, mut input) = mpsc::channel::<Input>(32);
    state.attachments.lock().await.insert(id.clone(), sender);
    let owner = state.inner().clone();
    let attachment = id.clone();
    tauri::async_runtime::spawn(async move {
        let _permit = permit;
        if events.send(json!({"type":"ready"})).is_ok() {
            loop {
                tokio::select! {
                    value = input.recv() => {
                        let Some(value) = value else { break; };
                        if let Err(error) = terminal.send(value) {
                            let _ = events.send(json!({"type":"error","message":error.to_string()}));
                            break;
                        }
                    }
                    value = terminal.recv() => {
                        let Some(value) = value else { break; };
                        let ended = matches!(value, Event::Exit(_));
                        let message = match value {
                            Event::Output(data) => json!({"type":"output","data":STANDARD.encode(data)}),
                            Event::Exit(code) => json!({"type":"attachment_closed","client_exit_code":code}),
                            Event::Error(message) => json!({"type":"transport_error","message":message}),
                        };
                        if events.send(message).is_err() || ended { break; }
                    }
                }
            }
        }
        owner.attachments.lock().await.remove(&attachment);
        drop(terminal);
        let _ = events.send(json!({"type":"closed"}));
    });
    Ok(id)
}
#[tauri::command]
pub async fn terminal_input(
    state: State<'_, Arc<Terminals>>,
    id: String,
    input: Input,
) -> Result<(), String> {
    input.validate().map_err(|e| e.to_string())?;
    state
        .attachments
        .lock()
        .await
        .get(&id)
        .ok_or("Terminal detached")?
        .try_send(input)
        .map_err(|_| "Terminal input unavailable".to_owned())
}
#[tauri::command]
pub async fn terminal_close(state: State<'_, Arc<Terminals>>, id: String) -> Result<(), String> {
    // Dropping this sender detaches the stream. The target still owns its Shell.
    state.attachments.lock().await.remove(&id);
    Ok(())
}
