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
    sync::{OwnedSemaphorePermit, mpsc, watch},
};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, http::Request, protocol::WebSocketConfig},
};

pub const MAX_CHUNK: usize = 64 * 1024;
/// A target answers this when its shared desktop has no Selkies web page (VNC).
pub const PAGE_UNAVAILABLE: &str = "This desktop has no Selkies web page";
pub enum Event {
    Data(Vec<u8>),
    Error(String),
}
pub struct Stream {
    input: mpsc::Sender<Vec<u8>>,
    failure: watch::Receiver<Option<String>>,
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
        self.input.try_send(bytes).map_err(|error| {
            if let Some(message) = self.failure.borrow().as_ref() {
                return anyhow::anyhow!("{}", message);
            }
            match error {
                mpsc::error::TrySendError::Full(_) => {
                    anyhow::anyhow!("desktop input queue is full")
                }
                mpsc::error::TrySendError::Closed(_) => {
                    anyhow::anyhow!("desktop provider disconnected")
                }
            }
        })
    }
    /// Like [`Stream::send`], waiting for queue space instead of failing.
    pub async fn send_wait(&self, bytes: Vec<u8>) -> Result<()> {
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_CHUNK,
            "invalid desktop input size"
        );
        if self.input.send(bytes).await.is_err() {
            let message = self.failure.borrow().clone();
            anyhow::bail!(message.unwrap_or_else(|| "desktop provider disconnected".into()));
        }
        Ok(())
    }
    /// Byte-stream view of the tunnel, for HTTP to the target's Selkies web page.
    pub fn into_io(mut self) -> tokio::io::DuplexStream {
        let (local, mut remote) = tokio::io::duplex(4 * MAX_CHUNK);
        tokio::spawn(async move {
            let mut buffer = vec![0; MAX_CHUNK];
            loop {
                tokio::select! {
                    read = remote.read(&mut buffer) => match read {
                        Ok(count) if count > 0 => {
                            if self.input.send(buffer[..count].to_vec()).await.is_err() { break; }
                        }
                        _ => break,
                    },
                    event = self.output.recv() => match event {
                        Some(Event::Data(data)) => {
                            if remote.write_all(&data).await.is_err() { break; }
                        }
                        _ => break,
                    },
                }
            }
        });
        local
    }
    pub(crate) fn spawn<F, T>(run: F) -> Self
    where
        F: FnOnce(mpsc::Receiver<Vec<u8>>, mpsc::Sender<Event>) -> T + Send + 'static,
        T: Future<Output = Result<()>> + Send + 'static,
    {
        Self::spawn_with_capacity(32, run)
    }
    pub(crate) fn spawn_with_capacity<F, T>(capacity: usize, run: F) -> Self
    where
        F: FnOnce(mpsc::Receiver<Vec<u8>>, mpsc::Sender<Event>) -> T + Send + 'static,
        T: Future<Output = Result<()>> + Send + 'static,
    {
        let (input, incoming) = mpsc::channel(32);
        let (outgoing, output) = mpsc::channel(capacity);
        // Keep one bounded slot for the terminal error. A slow viewer must not
        // turn a provider failure into a seemingly clean disconnect, or delay
        // releasing the controller lease while waiting for queue space.
        let failure = outgoing.clone().try_reserve_owned().unwrap();
        let (status, last_failure) = watch::channel(None);
        let task = tokio::spawn(async move {
            if let Err(error) = run(incoming, outgoing.clone()).await {
                let message = error.to_string();
                status.send_replace(Some(message.clone()));
                failure.send(Event::Error(message));
            }
        });
        Self {
            input,
            failure: last_failure,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn full_queue_retains_failure_without_holding_controller_lease() {
        let lease = Arc::new(tokio::sync::Semaphore::new(1));
        let permit = lease.clone().acquire_owned().await.unwrap();
        let (finished, completion) = tokio::sync::oneshot::channel();
        let mut stream = Stream::spawn(move |_, outgoing| async move {
            let _permit = permit;
            for _ in 0..31 {
                outgoing.try_send(Event::Data(vec![1])).unwrap();
            }
            assert!(outgoing.try_send(Event::Data(vec![2])).is_err());
            let _ = finished.send(());
            anyhow::bail!("desktop sharing stopped or access revoked");
        });
        tokio::time::timeout(Duration::from_secs(1), completion)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while lease.available_permits() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        (&mut stream.task).await.unwrap();
        assert_eq!(
            stream.send(vec![1]).unwrap_err().to_string(),
            "desktop sharing stopped or access revoked"
        );
        for _ in 0..31 {
            assert!(matches!(stream.output.recv().await, Some(Event::Data(_))));
        }
        assert!(
            matches!(stream.output.recv().await, Some(Event::Error(message)) if message == "desktop sharing stopped or access revoked")
        );
        assert!(stream.output.recv().await.is_none());
    }
}
