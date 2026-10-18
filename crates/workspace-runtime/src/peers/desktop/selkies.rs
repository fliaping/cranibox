//! Selkies websocket mode, carried through the same bounded, authorized desktop
//! stream as RFB. The destination is always the owner's configured loopback port.
use super::{Event, MAX_CHUNK, Stream};
use anyhow::{Context, Result, ensure};
use futures_util::{SinkExt, StreamExt};
use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};
use tokio::sync::OwnedSemaphorePermit;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

pub const MAGIC: &[u8] = b"CRANIBOX-SELKIES/1\n";
const MAX_FRAME: usize = 16 * 1024 * 1024;
const MAX_INPUT: usize = 16 * 1024;
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn socket(address: SocketAddr) -> Result<Socket> {
    ensure!(
        address.ip().is_loopback() && address.port() != 0,
        "Selkies must use a loopback address and a nonzero port"
    );
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME));
    let (socket, _) = tokio::time::timeout(Duration::from_secs(5), connect_async_with_config(
        format!("ws://{address}/websockets"), Some(config), true,
    )).await.context("Local Selkies connection timed out")?
        .context("Local Selkies websocket server is unavailable (use its internal websocket port, usually 8082)")?;
    Ok(socket)
}

/// A probe never sends SETTINGS, takes over a display, or changes its resolution.
pub async fn check(address: SocketAddr) -> Result<()> {
    let mut socket = socket(address).await?;
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(message) = socket.next().await {
            match message? {
                Message::Text(text) if text.as_str() == "MODE websockets" => return Ok(()),
                Message::Ping(data) => socket.send(Message::Pong(data)).await?,
                _ => anyhow::bail!("Expected Selkies websocket mode without token authentication; use a dedicated loopback Selkies service"),
            }
        }
        anyhow::bail!("Selkies closed before reporting websocket mode")
    }).await.context("Selkies did not report websocket mode")?;
    let _ = tokio::time::timeout(Duration::from_secs(1), socket.close(None)).await;
    result
}

pub async fn connect(
    address: SocketAddr,
    permit: OwnedSemaphorePermit,
    valid: impl Fn() -> bool + Send + Sync + 'static,
) -> Result<Stream> {
    let mut socket = socket(address).await?;
    ensure!(valid(), "desktop sharing stopped or access revoked");
    Ok(Stream::spawn(move |mut incoming, outgoing| async move {
        let _permit = permit;
        let io = async {
            outgoing.send(Event::Data(MAGIC.to_vec())).await?;
            let mut input = Decoder::default();
            let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
            let mut seen = Instant::now();
            loop {
                tokio::select! {
                    _ = outgoing.closed() => break,
                    _ = heartbeat.tick() => {
                        ensure!(seen.elapsed() < Duration::from_secs(60), "Selkies connection timed out");
                        tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Ping(vec![].into()))).await??;
                    }
                    data = incoming.recv() => {
                        let Some(data) = data else { break; };
                        ensure!(valid(), "desktop sharing stopped or access revoked");
                        for text in input.push(&data)? {
                            tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Text(text.into()))).await??;
                        }
                    }
                    message = socket.next() => {
                        let Some(message) = message else { break; };
                        seen = Instant::now();
                        let (kind, payload) = match message? {
                            Message::Text(data) => (0, data.as_bytes().to_vec()),
                            Message::Binary(data) => (1, data.to_vec()),
                            Message::Ping(data) => {
                                tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Pong(data))).await??;
                                continue;
                            }
                            Message::Pong(_) => continue,
                            Message::Close(_) => break,
                            _ => anyhow::bail!("Invalid Selkies websocket message"),
                        };
                        ensure!(payload.len() <= MAX_FRAME, "Selkies frame too large");
                        let mut framed = Vec::with_capacity(5 + payload.len());
                        framed.push(kind);
                        framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                        framed.extend_from_slice(&payload);
                        for chunk in framed.chunks(MAX_CHUNK) {
                            tokio::time::timeout(Duration::from_secs(5), outgoing.send(Event::Data(chunk.to_vec()))).await??;
                        }
                    }
                }
            }
            Ok(())
        };
        // Independent of slow consumers, decoders and the local websocket.
        let guard = async {
            let mut tick = tokio::time::interval(Duration::from_millis(250));
            loop {
                tick.tick().await;
                ensure!(valid(), "desktop sharing stopped or access revoked");
            }
        };
        let result = tokio::select! { result = io => result, result = guard => result };
        let _ = tokio::time::timeout(Duration::from_secs(1), socket.close(None)).await;
        result
    }))
}

/// Only text control messages are accepted. File, audio and device streams are
/// deliberately outside this display/input adapter. Size is checked before allocation.
#[derive(Default)]
struct Decoder {
    buffer: Vec<u8>,
}
impl Decoder {
    fn push(&mut self, data: &[u8]) -> Result<Vec<String>> {
        ensure!(
            self.buffer.len() + data.len() <= MAX_INPUT + MAX_CHUNK + 5,
            "Selkies input buffer too large"
        );
        self.buffer.extend_from_slice(data);
        let mut result = Vec::new();
        while self.buffer.len() >= 5 {
            let len = u32::from_be_bytes(self.buffer[1..5].try_into()?) as usize;
            ensure!(
                self.buffer[0] == 0 && len > 0 && len <= MAX_INPUT,
                "Invalid Selkies input frame"
            );
            if self.buffer.len() < 5 + len {
                break;
            }
            let text = std::str::from_utf8(&self.buffer[5..5 + len])?;
            ensure!(allowed(text), "Unsupported Selkies desktop command");
            result.push(text.to_owned());
            self.buffer.drain(..5 + len);
        }
        Ok(result)
    }
}
fn allowed(text: &str) -> bool {
    matches!(text, "START_VIDEO" | "kr")
        || ["kd,", "ku,", "m,", "CLIENT_FRAME_ACK ", "CLIENT_FPS "]
            .iter()
            .any(|prefix| {
                text.strip_prefix(prefix).is_some_and(|s| {
                    !s.is_empty()
                        && s.bytes()
                            .all(|b| b.is_ascii_digit() || b == b',' || b == b'-' || b == b'.')
                })
            })
        || text.strip_prefix("SETTINGS,").is_some_and(|s| {
            serde_json::from_str::<serde_json::Value>(s)
                .ok()
                .and_then(|v| v.as_object().cloned())
                .is_some_and(|settings| {
                    settings.get("encoder").and_then(|v| v.as_str()) == Some("jpeg")
                        && settings.get("displayId").and_then(|v| v.as_str()) == Some("primary")
                        && settings.keys().all(|key| {
                            matches!(
                                key.as_str(),
                                "encoder" | "displayId" | "framerate" | "jpeg_quality"
                            )
                        })
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn framed(text: &str) -> Vec<u8> {
        let mut data = vec![0];
        data.extend_from_slice(&(text.len() as u32).to_be_bytes());
        data.extend_from_slice(text.as_bytes());
        data
    }
    #[test]
    fn input_fragmentation_coalescing_and_limits() {
        let mut decoder = Decoder::default();
        let bytes = [framed("kd,97"), framed("ku,97")].concat();
        assert!(decoder.push(&bytes[..3]).unwrap().is_empty());
        assert_eq!(decoder.push(&bytes[3..]).unwrap(), ["kd,97", "ku,97"]);
        assert!(Decoder::default().push(&[0, 0, 1, 0, 0]).is_err());
        assert!(
            Decoder::default()
                .push(&framed("FILE_UPLOAD_START:secret"))
                .is_err()
        );
        assert!(Decoder::default().push(&framed("SETTINGS,{\"encoder\":\"jpeg\",\"displayId\":\"primary\",\"manual_width\":4000}")).is_err());
        assert!(allowed(
            "SETTINGS,{\"encoder\":\"jpeg\",\"displayId\":\"primary\",\"framerate\":30,\"jpeg_quality\":65}"
        ));
    }
    #[tokio::test]
    async fn slow_consumers_cannot_prevent_revocation_or_keep_controller_lease() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            // Fill the bounded outgoing queue while the viewer never reads.
            loop {
                if ws
                    .send(Message::Binary(vec![3; MAX_FRAME].into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let lease = Arc::new(tokio::sync::Semaphore::new(1));
        let enabled = Arc::new(AtomicBool::new(true));
        let guard = enabled.clone();
        let stream = connect(
            address,
            lease.clone().acquire_owned().await.unwrap(),
            move || guard.load(Ordering::SeqCst),
        )
        .await
        .unwrap();
        assert_eq!(lease.available_permits(), 0);
        enabled.store(false, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(3), async {
            while lease.available_permits() == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        drop(stream);
        server.abort();
        let _ = server.await;
    }
    #[tokio::test]
    async fn probe_requires_mode_and_sends_no_display_settings() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            ws.send(Message::Text("MODE websockets".into()))
                .await
                .unwrap();
            assert!(matches!(
                ws.next().await.unwrap().unwrap(),
                Message::Close(_)
            ));
            let _ = ws.flush().await;
        });
        check(address).await.unwrap();
        server.await.unwrap();
        assert!(check("0.0.0.0:8081".parse().unwrap()).await.is_err());
    }
}
