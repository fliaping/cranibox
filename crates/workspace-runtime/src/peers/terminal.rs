//! Authenticated node-to-node terminal stream, shared by Web and native clients.
use super::config::Route;
use crate::protocol::MAX_FRAME;
use anyhow::{Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, http::Request, protocol::WebSocketConfig},
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
}
impl Input {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Input { data } => ensure!(data.len() <= 16 * 1024, "terminal input too large"),
            Self::Resize { cols, rows } => ensure!(
                (2..=500).contains(cols) && (2..=300).contains(rows),
                "invalid terminal size"
            ),
        }
        Ok(())
    }
    fn wire(self) -> serde_json::Value {
        match self {
            Self::Input { data } => json!({"type":"input","data":STANDARD.encode(data)}),
            Self::Resize { cols, rows } => json!({"type":"resize","cols":cols,"rows":rows}),
        }
    }
}
pub enum Event {
    Output(Vec<u8>),
    Exit(u32),
    Error(String),
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Output {
    Output { data: String },
    Exit { code: u32 },
}
pub struct Terminal {
    input: mpsc::Sender<Input>,
    pub output: mpsc::Receiver<Event>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Terminal {
    fn drop(&mut self) {
        // Dropping the attachment closes its socket; the target owns the Shell.
        self.task.abort();
    }
}
impl Terminal {
    pub fn send(&self, value: Input) -> Result<()> {
        value.validate()?;
        self.input
            .try_send(value)
            .map_err(|_| anyhow::anyhow!("terminal input unavailable"))
    }
    pub async fn connect(
        route: Route,
        node: &str,
        session: &str,
        cols: u16,
        rows: u16,
        takeover: bool,
    ) -> Result<Self> {
        route
            .api(node)?
            .terminal(node, session, cols, rows, takeover)
            .await
    }
    pub(crate) async fn connect_request(request: Request<()>) -> Result<Self> {
        // No Origin, proxy, or redirect: credentials go only to this configured origin.
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME))
            .max_frame_size(Some(MAX_FRAME));
        let (mut socket, _) = tokio::time::timeout(
            Duration::from_secs(15),
            connect_async_with_config(request, Some(config), true),
        )
        .await??;
        let (input, mut incoming) = mpsc::channel::<Input>(32);
        let (output, events) = mpsc::channel(32);
        let task = tokio::spawn(async move {
            let result: Result<()> = async {
                let mut ping = tokio::time::interval(Duration::from_secs(15));
                let mut seen = Instant::now();
                loop {
                    tokio::select! {
                        _ = output.closed() => break,
                        _ = ping.tick() => {
                            ensure!(seen.elapsed() < Duration::from_secs(60), "terminal connection timed out");
                            tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Ping(vec![].into()))).await??;
                        }
                        value = incoming.recv() => {
                            let Some(value) = value else { break; };
                            tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Text(value.wire().to_string().into()))).await??;
                        }
                        value = socket.next() => {
                            let Some(value) = value else { break; };
                            seen = Instant::now();
                            match value? {
                                Message::Text(text) => {
                                    let event = match serde_json::from_str::<Output>(&text)? {
                                        Output::Output { data } => Event::Output(STANDARD.decode(data)?),
                                        Output::Exit { code } => Event::Exit(code),
                                    };
                                    let ended = matches!(event, Event::Exit(_));
                                    tokio::time::timeout(Duration::from_secs(5), output.send(event)).await??;
                                    if ended { break; }
                                }
                                Message::Ping(data) => { tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Pong(data))).await??; }
                                Message::Pong(_) => {}
                                Message::Close(_) => break,
                                _ => anyhow::bail!("invalid terminal response"),
                            }
                        }
                    }
                }
                Ok(())
            }.await;
            if let Err(error) = result {
                let _ = output.try_send(Event::Error(error.to_string()));
            }
            let _ = tokio::time::timeout(Duration::from_secs(1), socket.close(None)).await;
        });
        Ok(Self {
            input,
            output: events,
            task,
        })
    }
}
