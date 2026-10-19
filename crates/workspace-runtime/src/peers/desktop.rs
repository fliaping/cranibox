//! Bounded desktop transport. Dropping a viewer only detaches from the desktop.
pub mod selkies;
use anyhow::{Result, ensure};
use futures_util::{SinkExt, StreamExt};
use std::{
    future::Future,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{OwnedSemaphorePermit, mpsc},
};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, http::Request, protocol::WebSocketConfig},
};

pub const MAX_CHUNK: usize = 64 * 1024;
pub enum Event {
    Data(Vec<u8>),
    Error(String),
}
pub struct Stream {
    input: mpsc::Sender<Vec<u8>>,
    pub output: mpsc::Receiver<Event>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Stream {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Stream {
    pub fn send(&self, bytes: Vec<u8>) -> Result<()> {
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_CHUNK,
            "invalid desktop input size"
        );
        self.input
            .try_send(bytes)
            .map_err(|_| anyhow::anyhow!("desktop input unavailable"))
    }
    pub(crate) fn spawn<F, T>(run: F) -> Self
    where
        F: FnOnce(mpsc::Receiver<Vec<u8>>, mpsc::Sender<Event>) -> T + Send + 'static,
        T: Future<Output = Result<()>> + Send + 'static,
    {
        let (input, incoming) = mpsc::channel(32);
        let (outgoing, output) = mpsc::channel(32);
        let task = tokio::spawn(async move {
            if let Err(error) = run(incoming, outgoing.clone()).await {
                let _ = outgoing.try_send(Event::Error(error.to_string()));
            }
        });
        Self {
            input,
            output,
            task,
        }
    }
    /// A single controller lease is shared by local and remote viewers. The guard
    /// runs independently of socket backpressure, so revocation always detaches.
    pub fn from_tcp(
        mut tcp: tokio::net::TcpStream,
        permit: OwnedSemaphorePermit,
        valid: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Self {
        Self::spawn(move |mut incoming, outgoing| async move {
            let _permit = permit;
            let io = async {
                let mut buffer = vec![0; MAX_CHUNK];
                loop {
                    tokio::select! {
                        _ = outgoing.closed() => break,
                        input = incoming.recv() => {
                            let Some(input) = input else { break; };
                            ensure!(valid(), "desktop sharing stopped or access revoked");
                            tokio::time::timeout(Duration::from_secs(5), tcp.write_all(&input)).await??;
                        }
                        read = tcp.read(&mut buffer) => {
                            let count = read?;
                            if count == 0 { break; }
                            tokio::time::timeout(Duration::from_secs(5), outgoing.send(Event::Data(buffer[..count].to_vec()))).await??;
                        }
                    }
                }
                Ok(())
            };
            let guard = async {
                let mut check = tokio::time::interval(Duration::from_millis(250));
                loop {
                    check.tick().await;
                    ensure!(valid(), "desktop sharing stopped or access revoked");
                }
            };
            tokio::select! { result = io => result, result = guard => result }
        })
    }
    pub async fn connect_request(
        request: Request<()>,
        cipher: Option<crate::peers::e2e::stream::Cipher>,
    ) -> Result<Self> {
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_CHUNK + crate::peers::e2e::stream::OVERHEAD))
            .max_frame_size(Some(MAX_CHUNK + crate::peers::e2e::stream::OVERHEAD));
        let (socket, _) = tokio::time::timeout(
            Duration::from_secs(10),
            connect_async_with_config(request, Some(config), true),
        )
        .await?
        .map_err(|error| {
            if let tokio_tungstenite::tungstenite::Error::Http(response) = &error
                && let Some(body) = response.body().as_deref().filter(|body| body.len() <= 4096)
                && let Ok(value) = serde_json::from_slice::<serde_json::Value>(body)
                && let Some(message) = value.get("error").and_then(|value| value.as_str())
            {
                return anyhow::anyhow!("{}", message.chars().take(1000).collect::<String>());
            }
            error.into()
        })?;
        let mut socket = crate::peers::e2e::socket::SecureSocket::new(socket, cipher);
        Ok(Self::spawn(move |mut incoming, outgoing| async move {
            let mut ping = tokio::time::interval(Duration::from_secs(15));
            let mut seen = Instant::now();
            let result = async {
                loop {
                    tokio::select! {
                        _ = outgoing.closed() => break,
                        _ = ping.tick() => {
                            ensure!(seen.elapsed() < Duration::from_secs(60), "desktop connection timed out");
                            tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Ping(vec![].into()))).await??;
                        }
                        input = incoming.recv() => {
                            let Some(input) = input else { break; };
                            tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Binary(input.into()))).await??;
                        }
                        event = socket.next() => {
                            let Some(event) = event else { break; };
                            seen = Instant::now();
                            match event? {
                                Message::Binary(data) => {
                                    tokio::time::timeout(Duration::from_secs(5), outgoing.send(Event::Data(data.to_vec()))).await??;
                                }
                                Message::Ping(data) => { tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Pong(data))).await??; }
                                Message::Pong(_) => {},
                                Message::Close(_) => break,
                                _ => anyhow::bail!("invalid desktop stream"),
                            }
                        }
                    }
                }
                Ok(())
            }.await;
            let _ = tokio::time::timeout(Duration::from_secs(1), socket.close(None)).await;
            result
        }))
    }
}
