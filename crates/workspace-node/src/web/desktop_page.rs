//! A shared desktop's own web interface, shown unchanged on the application origin.
//! Selkies serves its page, API and stream on the shared port, so HTTP passes through
//! the authorized peer tunnel as is. A VNC desktop gets the noVNC application, whose
//! `websockify` socket carries RFB through the same authorized desktop stream.
use super::novnc_files::FILES;
use crate::{
    local::LocalNode,
    spaces::applications::{blocked_request, blocked_response, splice},
};
use agent_workspace_runtime::{
    peers::desktop::{Event, MAX_CHUNK, PAGE_UNAVAILABLE, Stream, selkies::MAGIC},
    protocol::valid_node_id,
};
use anyhow::{Context, Result, bail, ensure};
use axum::{
    body::Body,
    extract::{FromRequestParts, WebSocketUpgrade, ws::Message},
    http::{HeaderMap, HeaderValue, Request, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use hyper::client::conn::http1::SendRequest;
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{handshake::derive_accept_key, protocol::Role},
};
use tokio_util::sync::CancellationToken;

/// Idle HTTP connections kept per open Selkies page.
const IDLE: usize = 6;

#[derive(Clone)]
pub enum Source {
    Local(Arc<LocalNode>),
    Peer { peers: PathBuf, node: String },
}
impl Source {
    async fn page(&self) -> Result<Stream> {
        match self {
            Self::Local(local) => local.desktop_page().await,
            Self::Peer { peers, node } => {
                agent_workspace_runtime::peers::Store::new(peers.clone())
                    .route(node)?
                    .api(node)?
                    .desktop_page(node)
                    .await
            }
        }
    }
    async fn desktop(&self) -> Result<Stream> {
        match self {
            Self::Local(local) => local.desktop().await,
            Self::Peer { peers, node } => {
                agent_workspace_runtime::peers::Store::new(peers.clone())
                    .route(node)?
                    .api(node)?
                    .desktop(node)
                    .await
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    node: String,
    #[serde(default)]
    local: bool,
}
impl super::Services {
    /// Open the desktop of `{node, local}` for its web page. `ancestors` may frame it.
    pub async fn desktop_page(&self, body: Value, ancestors: String) -> Result<Page> {
        let options: Options = serde_json::from_value(body)?;
        ensure!(valid_node_id(&options.node), "invalid node ID");
        let source = if options.local {
            let local = self.local()?;
            ensure!(local.id == options.node, "local node identity mismatch");
            Source::Local(local.clone())
        } else {
            Source::Peer {
                peers: self.peers.clone(),
                node: options.node,
            }
        };
        Page::open(source, ancestors).await
    }
}

#[derive(Clone)]
pub struct Page(Arc<Inner>);
struct Inner {
    source: Source,
    kind: Kind,
    ancestors: String,
    stop: CancellationToken,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
enum Kind {
    Selkies {
        idle: Mutex<Vec<SendRequest<Body>>>,
    },
    /// The stream opened to identify the desktop, with its first bytes, kept for the
    /// page's first connection. Later connections open their own stream.
    Vnc {
        first: Mutex<Option<(Vec<u8>, Stream)>>,
    },
}
async fn handshake(stream: Stream) -> Result<SendRequest<Body>> {
    let (sender, connection) =
        hyper::client::conn::http1::handshake(TokioIo::new(stream.into_io())).await?;
    tokio::spawn(async move {
        let _ = connection.with_upgrades().await;
    });
    Ok(sender)
}
impl Page {
    /// Connects once so that offline targets, missing grants and the desktop kind are
    /// known before the page opens.
    pub async fn open(source: Source, ancestors: String) -> Result<Self> {
        let kind = match source.page().await {
            Ok(stream) => Kind::Selkies {
                idle: Mutex::new(vec![handshake(stream).await?]),
            },
            Err(error) if error.to_string() == PAGE_UNAVAILABLE => {
                let mut stream = source.desktop().await?;
                let first =
                    match tokio::time::timeout(Duration::from_secs(10), stream.output.recv()).await
                    {
                        Ok(Some(Event::Data(data))) => data,
                        Ok(Some(Event::Error(message))) => bail!(message),
                        _ => {
                            bail!("The desktop server did not answer. Check sharing and reconnect.")
                        }
                    };
                ensure!(
                    !first.starts_with(MAGIC),
                    "Update Cranibox on that device to open its Selkies desktop"
                );
                Kind::Vnc {
                    first: Mutex::new(Some((first, stream))),
                }
            }
            Err(error) => return Err(error),
        };
        Ok(Self(Arc::new(Inner {
            source,
            kind,
            ancestors,
            stop: CancellationToken::new(),
        })))
    }
    pub fn protocol(&self) -> &'static str {
        match self.0.kind {
            Kind::Selkies { .. } => "selkies",
            Kind::Vnc { .. } => "vnc",
        }
    }
    pub fn active(&self) -> bool {
        !self.0.stop.is_cancelled()
    }
    pub async fn forward(
        &self,
        request: Request<Body>,
        prefix: &str,
        cancel: CancellationToken,
    ) -> Result<Response> {
        ensure!(
            !cancel.is_cancelled() && self.active(),
            "Desktop page closed; reconnect from Cranibox"
        );
        let relative = request
            .uri()
            .path()
            .strip_prefix(prefix)
            .context("Invalid desktop page path")?
            .to_owned();
        let mut response = match self.0.kind {
            Kind::Selkies { .. } => self.selkies(request, prefix, &relative, cancel).await?,
            Kind::Vnc { .. } => self.vnc(request, &relative, cancel).await?,
        };
        response.headers_mut().append(
            "content-security-policy",
            HeaderValue::from_str(&format!("frame-ancestors {}", self.0.ancestors))?,
        );
        Ok(response)
    }

    async fn connection(&self) -> Result<SendRequest<Body>> {
        handshake(self.0.source.page().await?).await
    }
    async fn idle(&self) -> Result<SendRequest<Body>> {
        let Kind::Selkies { idle } = &self.0.kind else {
            bail!("not a Selkies page");
        };
        loop {
            let Some(mut sender) = idle.lock().unwrap().pop() else {
                break;
            };
            if !sender.is_closed()
                && matches!(
                    tokio::time::timeout(Duration::from_millis(100), sender.ready()).await,
                    Ok(Ok(()))
                )
            {
                return Ok(sender);
            }
        }
        self.connection().await
    }
    fn release(&self, sender: SendRequest<Body>) {
        if let Kind::Selkies { idle } = &self.0.kind {
            let mut idle = idle.lock().unwrap();
            if idle.len() < IDLE && !sender.is_closed() && self.active() {
                idle.push(sender);
            }
        }
    }

    async fn selkies(
        &self,
        request: Request<Body>,
        prefix: &str,
        relative: &str,
        cancel: CancellationToken,
    ) -> Result<Response> {
        let path = match request.uri().query() {
            Some(query) => format!("/{relative}?{query}"),
            None => format!("/{relative}"),
        };
        let (mut parts, body) = request.into_parts();
        let websocket = parts
            .headers
            .get("upgrade")
            .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"websocket"));
        let upgrade = if websocket {
            Some(
                WebSocketUpgrade::from_request_parts(&mut parts, &())
                    .await
                    .map_err(|_| anyhow::anyhow!("Invalid WebSocket upgrade"))?,
            )
        } else {
            None
        };
        let mut headers = HeaderMap::new();
        for (name, value) in &parts.headers {
            if !blocked_request(name.as_str(), &parts.headers) {
                headers.append(name.clone(), value.clone());
            }
        }
        // Selkies listens on the target's loopback interface.
        headers.insert("host", HeaderValue::from_static("localhost"));
        if parts.headers.contains_key("origin") {
            headers.insert("origin", HeaderValue::from_static("http://localhost"));
        }
        let send_body = !websocket && parts.method != "GET" && parts.method != "HEAD";
        if websocket {
            headers.insert("connection", HeaderValue::from_static("Upgrade"));
            headers.insert("upgrade", HeaderValue::from_static("websocket"));
        } else if send_body && let Some(length) = parts.headers.get("content-length") {
            headers.insert("content-length", length.clone());
        }
        let mut outgoing = Request::builder()
            .method(parts.method.clone())
            .uri(path)
            .body(if send_body { body } else { Body::empty() })?;
        *outgoing.headers_mut() = headers;
        // An upgraded connection never returns to the pool.
        let mut sender = if websocket {
            self.connection().await?
        } else {
            self.idle().await?
        };
        let response = tokio::select! {
            _ = cancel.cancelled() => bail!("Desktop page closed"),
            result = tokio::time::timeout(Duration::from_secs(30), async {
                sender.ready().await?;
                sender.send_request(outgoing).await
            }) => result.context("Desktop page timed out")?.context("Desktop page unavailable")?,
        };
        if let Some(mut upgrade) = upgrade {
            ensure!(
                response.status() == StatusCode::SWITCHING_PROTOCOLS,
                "Desktop rejected WebSocket"
            );
            let key = parts
                .headers
                .get("sec-websocket-key")
                .context("Missing WebSocket key")?;
            ensure!(
                response
                    .headers()
                    .get("sec-websocket-accept")
                    .is_some_and(|v| v.as_bytes() == derive_accept_key(key.as_bytes()).as_bytes()),
                "Invalid desktop WebSocket handshake"
            );
            if let Some(protocol) = response.headers().get("sec-websocket-protocol") {
                let protocol = protocol.to_str()?;
                ensure!(
                    parts
                        .headers
                        .get("sec-websocket-protocol")
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|v| v.split(',').any(|v| v.trim() == protocol)),
                    "Invalid desktop WebSocket protocol"
                );
                upgrade = upgrade.protocols([protocol.to_owned()]);
            }
            let upgraded = hyper::upgrade::on(response).await?;
            let page = self.clone();
            return Ok(upgrade
                .on_upgrade(move |browser| async move {
                    let upstream = WebSocketStream::from_raw_socket(
                        TokioIo::new(upgraded),
                        Role::Client,
                        None,
                    )
                    .await;
                    splice(browser, upstream, cancel, move || page.active()).await;
                })
                .into_response());
        }
        let (head, incoming) = response.into_parts();
        let mut output = Response::builder().status(head.status);
        for (name, value) in &head.headers {
            if blocked_response(name.as_str(), &head.headers) {
                continue;
            }
            match value.to_str() {
                Ok(location)
                    if name == "location"
                        && location.starts_with('/')
                        && !location.starts_with("//") =>
                {
                    output = output.header(name, format!("{prefix}{}", &location[1..]));
                }
                _ => output = output.header(name, value),
            }
        }
        // The connection returns to the pool when the server is done with this body;
        // `idle` skips it unless the response was read in full.
        let lease = Lease(self.clone(), Some(sender));
        let page = self.clone();
        let mut body = Body::new(incoming).into_data_stream();
        let mut check = tokio::time::interval(Duration::from_secs(1));
        let stream = futures_util::stream::poll_fn(move |cx| {
            let _ = &lease;
            if cancel.is_cancelled() || (check.poll_tick(cx).is_ready() && !page.active()) {
                return std::task::Poll::Ready(None);
            }
            body.poll_next_unpin(cx)
        });
        Ok(output.body(Body::from_stream(stream))?)
    }

    async fn vnc(
        &self,
        request: Request<Body>,
        relative: &str,
        cancel: CancellationToken,
    ) -> Result<Response> {
        if relative == "websockify" {
            let (mut parts, _) = request.into_parts();
            let upgrade = WebSocketUpgrade::from_request_parts(&mut parts, &())
                .await
                .map_err(|_| anyhow::anyhow!("WebSocket required"))?;
            let Kind::Vnc { first } = &self.0.kind else {
                bail!("not a VNC page");
            };
            let pending = first.lock().unwrap().take();
            let (first, stream) = match pending {
                Some(pending) => pending,
                None => (Vec::new(), self.0.source.desktop().await?),
            };
            let page = self.clone();
            return Ok(upgrade
                .protocols(["binary"])
                .max_message_size(MAX_CHUNK)
                .on_upgrade(move |socket| bridge(socket, first, stream, cancel, page))
                .into_response());
        }
        ensure!(
            request.method() == "GET" || request.method() == "HEAD",
            "Method not allowed"
        );
        let (body, kind): (Vec<u8>, &str) = match relative {
            // noVNC reads these beside vnc.html: connect at once to this page's socket.
            "defaults.json" => (
                json!({"autoconnect": true, "reconnect": true, "resize": "scale", "shared": true})
                    .to_string()
                    .into_bytes(),
                "json",
            ),
            "mandatory.json" => (
                json!({"host": "", "port": 0, "path": "websockify"})
                    .to_string()
                    .into_bytes(),
                "json",
            ),
            path => {
                let path = if path.is_empty() { "vnc.html" } else { path };
                let Some((name, bytes)) = FILES.iter().find(|(name, _)| *name == path) else {
                    return Ok((StatusCode::NOT_FOUND, "Not found").into_response());
                };
                (bytes.to_vec(), name.rsplit('.').next().unwrap_or_default())
            }
        };
        let content_type = match kind {
            "html" => "text/html; charset=utf-8",
            "js" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "json" => "application/json",
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "ico" => "image/x-icon",
            "mp3" => "audio/mpeg",
            "oga" => "audio/ogg",
            "ttf" => "font/ttf",
            "woff" => "font/woff",
            _ => "text/plain; charset=utf-8",
        };
        Ok((
            [
                ("content-type", content_type),
                ("x-content-type-options", "nosniff"),
            ],
            body,
        )
            .into_response())
    }
}

struct Lease(Page, Option<SendRequest<Body>>);
impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(sender) = self.1.take() {
            self.0.release(sender);
        }
    }
}

/// noVNC's `websockify` socket: binary frames carry the RFB stream unchanged.
async fn bridge(
    mut socket: axum::extract::ws::WebSocket,
    first: Vec<u8>,
    mut stream: Stream,
    cancel: CancellationToken,
    page: Page,
) {
    let run: Result<()> = async {
        if !first.is_empty() {
            socket.send(Message::Binary(first.into())).await?;
        }
        let mut check = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = check.tick() => { if !page.active() { break; } },
                event = stream.output.recv() => match event {
                    Some(Event::Data(data)) => {
                        tokio::time::timeout(Duration::from_secs(10), socket.send(Message::Binary(data.into()))).await??;
                    }
                    _ => break,
                },
                message = socket.recv() => match message {
                    Some(Ok(Message::Binary(bytes))) => {
                        for chunk in bytes.chunks(MAX_CHUNK).filter(|chunk| !chunk.is_empty()) {
                            stream.send_wait(chunk.to_vec()).await?;
                        }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    _ => break,
                },
            }
        }
        Ok(())
    }
    .await;
    let _ = run;
    drop(stream);
    let _ = tokio::time::timeout(Duration::from_secs(1), socket.send(Message::Close(None))).await;
}
