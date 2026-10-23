//! Selkies websocket mode, carried through the same bounded, authorized desktop
//! stream as RFB. The destination is always the owner's configured loopback port.
mod output;
use super::{Event, MAX_CHUNK, Stream};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
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
const MAX_CLIPBOARD: usize = 64 * 1024;
const MAX_INPUT: usize = 3 + 4 * MAX_CLIPBOARD.div_ceil(3);
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn socket(address: SocketAddr) -> Result<Socket> {
    ensure!(
        address.ip().is_loopback() && address.port() != 0,
        "Selkies must use a loopback address and a nonzero port"
    );
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME));
    // Selkies 2.x serves the stream at /api/websockets; 1.x at /websockets.
    let mut failure = None;
    for path in ["/api/websockets", "/websockets"] {
        match tokio::time::timeout(
            Duration::from_secs(5),
            connect_async_with_config(format!("ws://{address}{path}"), Some(config), true),
        )
        .await
        {
            Ok(Ok((socket, _))) => return Ok(socket),
            Ok(Err(error)) => failure = Some(anyhow::Error::from(error)),
            Err(_) => anyhow::bail!("Local Selkies connection timed out"),
        }
    }
    Err(failure.unwrap_or_else(|| anyhow::anyhow!("no Selkies endpoint")))
        .context("Local Selkies websocket server is unavailable (use its internal websocket port, usually 8082)")
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
    // Leave only two data chunks ahead of the viewer (plus the reserved error
    // slot). Coalescing below keeps the upstream socket drained instead of
    // accumulating seconds of stale video in its TCP buffers.
    Ok(Stream::spawn_with_capacity(
        3,
        move |mut incoming, outgoing| async move {
            let _permit = permit;
            let io = async {
                outgoing.send(Event::Data(MAGIC.to_vec())).await?;
                let mut input = Decoder::default();
                let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
                let mut seen = Instant::now();
                let mut output = output::Output::default();
                let mut delivered = tokio::time::Instant::now();
                loop {
                    tokio::select! {
                        _ = outgoing.closed() => break,
                        _ = tokio::time::sleep_until(delivered + Duration::from_secs(5)), if !output.is_empty() => {
                            anyhow::bail!("Desktop viewer receive queue stalled for 5 seconds");
                        }
                        permit = outgoing.reserve(), if !output.is_empty() => {
                            permit?.send(Event::Data(output.next_chunk()));
                            delivered = tokio::time::Instant::now();
                        }
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
                            if output.is_empty() { delivered = tokio::time::Instant::now(); }
                            output.push(kind, payload)?;
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
        },
    ))
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
    if let Some(encoded) = text.strip_prefix("cw,") {
        return encoded.len() <= MAX_INPUT - 3
            && STANDARD.decode(encoded).is_ok_and(|bytes| {
                bytes.len() <= MAX_CLIPBOARD && std::str::from_utf8(&bytes).is_ok()
            });
    }
    if text.len() > 16 * 1024 {
        return false;
    }
    matches!(text, "START_VIDEO" | "kr" | "cr")
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
        let mut oversized = vec![0];
        oversized.extend_from_slice(&((MAX_INPUT + 1) as u32).to_be_bytes());
        assert!(Decoder::default().push(&oversized).is_err());
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
    #[test]
    fn clipboard_accepts_bounded_utf8_and_rejects_other_streams() {
        assert!(allowed("cr"));
        assert!(allowed("cw,"));
        assert!(allowed(&format!(
            "cw,{}",
            STANDARD.encode("中文 👋\ncommand")
        )));
        let text = format!("cw,{}", STANDARD.encode(vec![b'a'; MAX_CLIPBOARD]));
        let bytes = framed(&text);
        let mut decoder = Decoder::default();
        assert!(decoder.push(&bytes[..MAX_CHUNK]).unwrap().is_empty());
        assert_eq!(decoder.push(&bytes[MAX_CHUNK..]).unwrap(), [text]);
        for text in [
            format!("cw,{}", STANDARD.encode(vec![b'a'; MAX_CLIPBOARD + 1])),
            format!("cw,{}", STANDARD.encode([0xff, 0xfe])),
            "cw,%%%".into(),
            "cw,YQ".into(),
            "cws,text/plain,100".into(),
            "clipboard_binary,image/png,AAAA".into(),
            "FILE_UPLOAD_START:secret".into(),
        ] {
            assert!(!allowed(&text));
            assert!(Decoder::default().push(&framed(&text)).is_err());
        }
    }
    #[tokio::test]
    async fn slow_viewer_keeps_upstream_drained_and_clipboard_input_live() {
        use std::sync::Arc;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (received, request) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            started.send(()).unwrap();
            // A primary Selkies display may ignore frame ACK backpressure.
            // Its damage burst must not prevent the adapter from reading cr.
            for id in 0..500 {
                let mut jpeg = output::tests::jpeg(id, 0, 160, 100);
                jpeg.resize(MAX_CHUNK, 0);
                ws.send(Message::Binary(jpeg.into())).await.unwrap();
            }
            while let Some(message) = ws.next().await {
                if matches!(message.unwrap(), Message::Text(text) if text == "cr") {
                    ws.send(Message::Text("clipboard,Y3VycmVudA==".into()))
                        .await
                        .unwrap();
                    // Pong proves the adapter read the preceding clipboard
                    // response while its viewer queue was still full. Merely
                    // receiving cr does not establish that ordering: video can
                    // remain in flight before the response is even written.
                    ws.send(Message::Ping(b"clipboard-ready".to_vec().into()))
                        .await
                        .unwrap();
                    loop {
                        if matches!(ws.next().await.unwrap().unwrap(), Message::Pong(data) if data.as_ref() == b"clipboard-ready")
                        {
                            break;
                        }
                    }
                    received.send(()).unwrap();
                    break;
                }
            }
            while ws.next().await.is_some() {}
        });
        let lease = Arc::new(tokio::sync::Semaphore::new(1));
        let mut stream = connect(address, lease.acquire_owned().await.unwrap(), || true)
            .await
            .unwrap();
        ready.await.unwrap();
        // Keep output blocked until the adapter has consumed the response, so
        // the frame bound measures its queue rather than upstream scheduling.
        stream.send(framed("cr")).unwrap();
        tokio::time::timeout(Duration::from_secs(3), request)
            .await
            .unwrap()
            .unwrap();
        let mut bytes = Vec::new();
        let mut frames = 0;
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let Some(Event::Data(data)) = stream.output.recv().await else {
                    panic!("missing clipboard")
                };
                bytes.extend_from_slice(&data);
                if bytes.starts_with(MAGIC) {
                    bytes.drain(..MAGIC.len());
                }
                while bytes.len() >= 5 {
                    let size = u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize;
                    if bytes.len() < 5 + size {
                        break;
                    }
                    frames += 1;
                    if bytes[0] == 0 && &bytes[5..5 + size] == b"clipboard,Y3VycmVudA==" {
                        return;
                    }
                    bytes.drain(..5 + size);
                }
            }
        })
        .await
        .unwrap();
        assert!(
            frames < 10,
            "superseded video delayed the clipboard by {frames} frames"
        );
        drop(stream);
        server.abort();
        let _ = server.await;
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
