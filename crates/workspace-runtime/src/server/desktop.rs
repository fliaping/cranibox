//! Outbound-only desktop rendezvous. The gateway asserts identity; the target
//! still owns the desktop grant and the controller lease. Video uses a separate
//! bounded binary socket, never the command/terminal control connection.
#[cfg(test)]
mod tests;
use super::*;
use crate::peers::desktop::{Event, MAX_CHUNK, Stream};

pub(super) const FEATURE: &str = "x-cranibox-desktop-relay";
pub(super) const TICKET: &str = "x-cranibox-desktop-ticket";
pub(super) struct Route {
    pub node: String,
    pub generation: String,
    ticket: Option<String>,
    reply: Option<oneshot::Sender<Result<Stream, String>>>,
}
struct Lease {
    app: App,
    id: String,
    tx: mpsc::Sender<Wire>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let app = self.app.clone();
        let id = self.id.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            app.inner.lock().await.desktops.remove(&id);
            // A saturated control channel must not strand a target controller.
            // The data socket also closes when its Stream is dropped.
            let _ =
                tokio::time::timeout(Duration::from_secs(5), tx.send(Wire::DesktopClose { id }))
                    .await;
        });
    }
}
pub(super) async fn open(
    app: &App,
    target: &str,
    peer: Option<String>,
    token: String,
) -> anyhow::Result<Stream> {
    anyhow::ensure!(valid_node_id(target), "invalid node ID");
    let credentials = config::Credentials::read(&app.credentials)?;
    let node = credentials.resolve_node(target).to_owned();
    anyhow::ensure!(
        super::terminal::authorized(app, peer.as_deref(), &node, &token),
        "desktop access denied"
    );
    let caller = match &peer {
        Some(id) => credentials.resolve_node(id).to_owned(),
        None => app.local_node.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "this gateway has no local node identity; connect using a node credential"
            )
        })?,
    };
    let target_token = credentials
        .nodes
        .get(&node)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("node is not enrolled"))?;
    let id = uuid::Uuid::new_v4().to_string();
    let (reply, receiver) = oneshot::channel();
    let (tx, generation) = {
        let mut registry = app.inner.lock().await;
        anyhow::ensure!(
            registry.desktops.len() < 32,
            "desktop relay capacity reached"
        );
        let n = registry
            .nodes
            .get(&node)
            .ok_or_else(|| anyhow::anyhow!("unknown node"))?;
        let tx =
            n.tx.clone()
                .ok_or_else(|| anyhow::anyhow!("node offline"))?;
        anyhow::ensure!(n.desktop_relay, "update this node to enable desktop relay");
        let generation = n.generation.clone();
        let ticket = config::token();
        tx.try_send(Wire::DesktopOpen {
            id: id.clone(),
            caller,
            ticket: ticket.clone(),
        })
        .map_err(|_| anyhow::anyhow!("node queue unavailable"))?;
        registry.desktops.insert(
            id.clone(),
            Route {
                node: node.clone(),
                generation: generation.clone(),
                ticket: Some(ticket),
                reply: Some(reply),
            },
        );
        (tx, generation)
    };
    let lease = Lease {
        app: app.clone(),
        id: id.clone(),
        tx,
    };
    let mut desktop = tokio::time::timeout(Duration::from_secs(15), receiver)
        .await
        .map_err(|_| anyhow::anyhow!("desktop relay connection timed out"))?
        .map_err(|_| anyhow::anyhow!("desktop node disconnected"))?
        .map_err(anyhow::Error::msg)?;
    let app = app.clone();
    Ok(Stream::spawn(move |mut incoming, outgoing| async move {
        let _lease = lease;
        let io = async {
            loop {
                tokio::select! {
                    _ = outgoing.closed() => break,
                    input = incoming.recv() => { let Some(input) = input else { break; }; desktop.send(input)?; }
                    output = desktop.output.recv() => match output {
                        Some(Event::Data(bytes)) => { tokio::time::timeout(Duration::from_secs(5), outgoing.send(Event::Data(bytes))).await??; }
                        Some(Event::Error(error)) => anyhow::bail!(error),
                        None => break,
                    },
                }
            }
            Ok(())
        };
        let guard = async {
            let mut check = tokio::time::interval(Duration::from_millis(250));
            loop {
                check.tick().await;
                anyhow::ensure!(
                    super::terminal::authorized(&app, peer.as_deref(), &node, &token),
                    "desktop access revoked"
                );
                anyhow::ensure!(
                    config::Credentials::read(&app.credentials).is_ok_and(|c| c
                        .nodes
                        .get(&node)
                        .is_some_and(|t| config::matches(t, &target_token))),
                    "desktop node credential changed"
                );
                let registry = app.inner.lock().await;
                anyhow::ensure!(
                    registry.desktops.contains_key(&id)
                        && registry
                            .nodes
                            .get(&node)
                            .is_some_and(|n| n.generation == generation && n.tx.is_some()),
                    "desktop node disconnected"
                );
            }
        };
        tokio::select! { result = io => result, result = guard => result }
    }))
}
pub(super) async fn upgrade(
    State(app): State<App>,
    Path(node): Path<String>,
    peer: Option<Extension<Peer>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let desktop = open(
        &app,
        &node,
        peer.map(|p| p.id.clone()),
        bearer(&headers).unwrap_or_default().into(),
    )
    .await
    .map_err(|e| error(StatusCode::CONFLICT, &e.to_string()))?;
    Ok(ws.max_message_size(MAX_CHUNK).max_frame_size(MAX_CHUNK).on_upgrade(move |socket| async move {
        let mut socket = transport(socket);
        let mut desktop = desktop;
        let _ = async {
            loop {
                tokio::select! {
                    output = desktop.output.recv() => match output { Some(Event::Data(data)) => socket.send(data)?, _ => break },
                    input = socket.output.recv() => match input { Some(Event::Data(data)) => desktop.send(data)?, _ => break },
                }
            }
            Ok::<_, anyhow::Error>(())
        }.await;
    }))
}
pub(super) async fn attach(
    State(app): State<App>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    if headers.contains_key("origin")
        || headers
            .get("sec-fetch-site")
            .is_some_and(|v| v == "cross-site")
    {
        return Err(error(
            StatusCode::FORBIDDEN,
            "native desktop connection required",
        ));
    }
    let credentials = config::Credentials::read(&app.credentials)
        .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "credentials unavailable"))?;
    let reply = {
        let mut registry = app.inner.lock().await;
        let valid = registry.desktops.get(&id).is_some_and(|route| {
            route.ticket.as_ref().is_some_and(|ticket| {
                headers
                    .get(TICKET)
                    .and_then(|h| h.to_str().ok())
                    .is_some_and(|value| config::matches(value, ticket))
            }) && credentials.nodes.get(&route.node).is_some_and(|token| {
                bearer(&headers).is_some_and(|value| config::matches(value, token))
            }) && registry
                .nodes
                .get(&route.node)
                .is_some_and(|n| n.generation == route.generation && n.tx.is_some())
        });
        if !valid {
            return Err(error(
                StatusCode::UNAUTHORIZED,
                "desktop rendezvous expired or unauthorized",
            ));
        }
        let route = registry.desktops.get_mut(&id).unwrap();
        route.ticket = None;
        route
            .reply
            .take()
            .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "desktop rendezvous already used"))?
    };
    Ok(ws
        .max_message_size(MAX_CHUNK)
        .max_frame_size(MAX_CHUNK)
        .on_upgrade(move |socket| async move {
            let _ = reply.send(Ok(transport(socket)));
        }))
}
pub(super) async fn failed(app: &App, node: &str, generation: &str, id: &str, message: String) {
    let mut registry = app.inner.lock().await;
    if registry
        .desktops
        .get(id)
        .is_some_and(|r| r.node == node && r.generation == generation)
        && let Some(mut route) = registry.desktops.remove(id)
        && let Some(reply) = route.reply.take()
    {
        let _ = reply.send(Err(message.chars().take(1000).collect()));
    }
}
fn transport(mut socket: WebSocket) -> Stream {
    Stream::spawn(move |mut incoming, outgoing| async move {
        let mut check = tokio::time::interval(Duration::from_secs(15));
        let mut seen = Instant::now();
        let run = async {
            loop {
                tokio::select! {
                    _ = outgoing.closed() => break,
                    _ = check.tick() => {
                        anyhow::ensure!(seen.elapsed() < Duration::from_secs(60), "desktop connection timed out");
                        tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Ping(vec![].into()))).await??;
                    }
                    input = incoming.recv() => { let Some(input) = input else { break; }; tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Binary(input.into()))).await??; }
                    event = socket.recv() => {
                        let Some(event) = event else { break; };
                        seen = Instant::now();
                        match event? {
                            Message::Binary(data) => { tokio::time::timeout(Duration::from_secs(5), outgoing.send(Event::Data(data.to_vec()))).await??; }
                            Message::Ping(data) => { tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Pong(data))).await??; }
                            Message::Pong(_) => {},
                            Message::Close(_) => break,
                            _ => anyhow::bail!("invalid desktop frame"),
                        }
                    }
                }
            }
            Ok(())
        }.await;
        let _ =
            tokio::time::timeout(Duration::from_secs(1), socket.send(Message::Close(None))).await;
        run
    })
}
