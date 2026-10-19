//! Binary noVNC adapter with explicit receive acknowledgements for IPC backpressure.
use agent_workspace_runtime::peers::{
    Store,
    desktop::{Event, MAX_CHUNK, Stream},
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
    let desktop = if options.local {
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
    let (sender, input) = mpsc::channel(32);
    state.attachments.lock().await.insert(id.clone(), sender);
    let owner = state.inner().clone();
    let attachment = id.clone();
    tauri::async_runtime::spawn(async move {
        let _permit = permit;
        if let Err(message) =
            forward_desktop(desktop, input, &events, Duration::from_secs(10)).await
        {
            let _ = events.send(json!({"type":"error","message":message}));
        }
        owner.attachments.lock().await.remove(&attachment);
        let _ = events.send(json!({"type":"closed"}));
    });
    Ok(id)
}

async fn forward_desktop(
    mut desktop: Stream,
    mut input: mpsc::Receiver<Input>,
    events: &Channel<Value>,
    acknowledgement_timeout: Duration,
) -> Result<(), String> {
    let mut sequence = 0u64;
    let mut pending = None;
    let mut delivered = tokio::time::Instant::now();
    events
        .send(json!({"type":"ready"}))
        .map_err(|e| e.to_string())?;
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(delivered + acknowledgement_timeout), if pending.is_some() => {
                return Err("Desktop viewer did not acknowledge received data within 10 seconds".into());
            }
            input = input.recv() => match input {
                Some(Input::Data(data)) => desktop.send(data).map_err(|e| e.to_string())?,
                Some(Input::Ack(value)) => {
                    if pending != Some(value) { return Err("Invalid desktop viewer acknowledgement".into()); }
                    pending = None;
                }
                None => return Ok(()),
            },
            output = desktop.output.recv(), if pending.is_none() => match output {
                Some(Event::Data(data)) => {
                    sequence += 1;
                    events.send(json!({"type":"data","sequence":sequence,"data":STANDARD.encode(data)})).map_err(|e| e.to_string())?;
                    pending = Some(sequence);
                    delivered = tokio::time::Instant::now();
                }
                Some(Event::Error(message)) => return Err(message),
                None => return Err("Desktop provider closed the connection".into()),
            },
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::ipc::InvokeResponseBody;
    use tokio::{
        io::AsyncWriteExt,
        net::{TcpListener, TcpStream},
    };

    async fn connection() -> (Stream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (client, server) = tokio::join!(
            TcpStream::connect(listener.local_addr().unwrap()),
            listener.accept()
        );
        let permit = Arc::new(Semaphore::new(1)).acquire_owned().await.unwrap();
        (
            Stream::from_tcp(client.unwrap(), permit, || true),
            server.unwrap().0,
        )
    }
    fn channel() -> (Channel<Value>, mpsc::UnboundedReceiver<Value>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (
            Channel::new(move |body| {
                let InvokeResponseBody::Json(body) = body else {
                    panic!("Expected JSON")
                };
                sender.send(serde_json::from_str(&body).unwrap()).unwrap();
                Ok(())
            }),
            receiver,
        )
    }
    async fn next(receiver: &mut mpsc::UnboundedReceiver<Value>) -> Value {
        tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap()
    }
    #[tokio::test]
    async fn missing_acknowledgement_reports_timeout() {
        let (desktop, mut server) = connection().await;
        let (_sender, input) = mpsc::channel(32);
        let (events, mut receiver) = channel();
        let task = tokio::spawn(async move {
            forward_desktop(desktop, input, &events, Duration::from_millis(100)).await
        });
        assert_eq!(next(&mut receiver).await["type"], "ready");
        server.write_all(b"frame").await.unwrap();
        assert_eq!(next(&mut receiver).await["sequence"], 1);
        assert_eq!(
            task.await.unwrap().unwrap_err(),
            "Desktop viewer did not acknowledge received data within 10 seconds"
        );
    }
    #[tokio::test]
    async fn acknowledgements_allow_next_frame_and_explicit_detach_is_clean() {
        let (desktop, mut server) = connection().await;
        let (sender, input) = mpsc::channel(32);
        let (events, mut receiver) = channel();
        let task = tokio::spawn(async move {
            forward_desktop(desktop, input, &events, Duration::from_secs(10)).await
        });
        assert_eq!(next(&mut receiver).await["type"], "ready");
        server.write_all(b"first").await.unwrap();
        assert_eq!(next(&mut receiver).await["sequence"], 1);
        server.write_all(b"second").await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), receiver.recv())
                .await
                .is_err()
        );
        sender.send(Input::Ack(1)).await.unwrap();
        let second = next(&mut receiver).await;
        assert_eq!(second["sequence"], 2);
        assert_eq!(
            STANDARD.decode(second["data"].as_str().unwrap()).unwrap(),
            b"second"
        );
        drop(sender);
        assert!(task.await.unwrap().is_ok());
    }
    #[tokio::test]
    async fn invalid_acknowledgement_reports_protocol_error() {
        let (desktop, _server) = connection().await;
        let (sender, input) = mpsc::channel(32);
        let (events, mut receiver) = channel();
        let task = tokio::spawn(async move {
            forward_desktop(desktop, input, &events, Duration::from_secs(10)).await
        });
        next(&mut receiver).await;
        sender.send(Input::Ack(9)).await.unwrap();
        assert_eq!(
            task.await.unwrap().unwrap_err(),
            "Invalid desktop viewer acknowledgement"
        );
    }
    #[tokio::test]
    async fn provider_disconnect_reports_reason() {
        let (desktop, server) = connection().await;
        let (_sender, input) = mpsc::channel(32);
        let (events, mut receiver) = channel();
        let task = tokio::spawn(async move {
            forward_desktop(desktop, input, &events, Duration::from_secs(10)).await
        });
        next(&mut receiver).await;
        drop(server);
        assert_eq!(
            task.await.unwrap().unwrap_err(),
            "Desktop provider closed the connection"
        );
    }
}
