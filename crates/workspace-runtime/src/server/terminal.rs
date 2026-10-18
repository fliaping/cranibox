//! Native terminal attachments. Shells live on nodes, independently of this transport.
use super::*;
use axum::extract::Query;
use serde::Deserialize;

#[cfg(test)]
mod tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Options {
    session: String,
    #[serde(default = "cols")]
    cols: u16,
    #[serde(default = "rows")]
    rows: u16,
    #[serde(default)]
    takeover: bool,
}
fn cols() -> u16 {
    80
}
fn rows() -> u16 {
    24
}

pub(super) async fn upgrade(
    State(app): State<App>,
    Path(node): Path<String>,
    peer: Option<Extension<Peer>>,
    headers: HeaderMap,
    Query(options): Query<Options>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let node = config::Credentials::read(&app.credentials)
        .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "credentials unavailable"))?
        .resolve_node(&node)
        .to_owned();
    if peer.as_ref().is_some_and(|p| !p.can_access(&node)) {
        return Err(error(StatusCode::FORBIDDEN, "node access denied"));
    }
    let token = bearer(&headers).unwrap_or_default().to_owned();
    let caller = peer.map(|p| p.id.clone());
    if !authorized(&app, caller.as_deref(), &node, &token) {
        return Err(error(StatusCode::FORBIDDEN, "terminal access revoked"));
    }
    let terminal = terminal_open(
        &app,
        &node,
        options.cols,
        options.rows,
        options.session,
        options.takeover,
    )
    .await
    .map_err(|e| error(StatusCode::BAD_REQUEST, &e.to_string()))?;
    Ok(ws
        .max_message_size(MAX_FRAME)
        .max_frame_size(MAX_FRAME)
        .on_upgrade(move |socket| stream(socket, terminal, app, caller, node, token)))
}

pub(super) fn authorized(app: &App, caller: Option<&str>, node: &str, token: &str) -> bool {
    let Ok(credentials) = config::Credentials::read(&app.credentials) else {
        return false;
    };
    let node = credentials.resolve_node(node).to_owned();
    if !credentials.nodes.contains_key(&node) {
        return false;
    }
    match caller {
        Some(id) => {
            let id = credentials.resolve_node(id).to_owned();
            credentials
                .nodes
                .get(&id)
                .is_some_and(|expected| config::matches(token, expected))
                && Peer {
                    id,
                    credentials: Arc::new(credentials),
                }
                .can_access(&node)
        }
        None => config::matches(token, &credentials.controller_token),
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ClientMessage {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
    Detach,
}

async fn send(socket: &mut WebSocket, value: Message) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(5), socket.send(value)).await??;
    Ok(())
}

async fn stream(
    mut socket: WebSocket,
    mut terminal: RemoteTerminal,
    app: App,
    caller: Option<String>,
    node: String,
    token: String,
) {
    let result: anyhow::Result<()> = async {
        let mut check = tokio::time::interval(Duration::from_millis(250));
        let mut seen = Instant::now();
        let mut ping = Instant::now();
        loop {
            tokio::select! {
                _ = check.tick() => {
                    anyhow::ensure!(authorized(&app, caller.as_deref(), &node, &token), "terminal access revoked");
                    anyhow::ensure!(seen.elapsed() < Duration::from_secs(60), "terminal connection timed out");
                    if ping.elapsed() >= Duration::from_secs(15) {
                        send(&mut socket, Message::Ping(vec![].into())).await?;
                        ping = Instant::now();
                    }
                }
                value = socket.recv() => {
                    let Some(value) = value else { break; };
                    seen = Instant::now();
                    match value? {
                        Message::Text(text) => {
                            // Recheck before commands as well as on the idle timer.
                            anyhow::ensure!(authorized(&app, caller.as_deref(), &node, &token), "terminal access revoked");
                            match serde_json::from_str::<ClientMessage>(&text)? {
                                ClientMessage::Input { data } => {
                                    anyhow::ensure!(STANDARD.decode(&data)?.len() <= 16 * 1024, "terminal input too large");
                                    terminal.send(Wire::TerminalInput { id: terminal.id.clone(), data })?;
                                }
                                ClientMessage::Resize { cols, rows } => {
                                    anyhow::ensure!((2..=500).contains(&cols) && (2..=300).contains(&rows), "invalid terminal size");
                                    terminal.send(Wire::TerminalResize { id: terminal.id.clone(), cols, rows })?;
                                }
                                ClientMessage::Detach => break,
                            }
                        }
                        Message::Ping(data) => send(&mut socket, Message::Pong(data)).await?,
                        Message::Pong(_) => {},
                        Message::Close(_) => break,
                        _ => anyhow::bail!("invalid terminal frame"),
                    }
                }
                value = terminal.output.recv() => {
                    let Some(value) = value else { break; };
                    let (message, ended) = match value {
                        Wire::TerminalOutput { data, .. } => (json!({"type":"output","data":data}), false),
                        Wire::TerminalExit { code, .. } => (json!({"type":"exit","code":code}), true),
                        _ => anyhow::bail!("invalid node terminal response"),
                    };
                    anyhow::ensure!(authorized(&app, caller.as_deref(), &node, &token), "terminal access revoked");
                    send(&mut socket, Message::Text(message.to_string().into())).await?;
                    if ended { break; }
                }
            }
        }
        Ok(())
    }.await;
    // Invalid frames, revocation and transport loss only detach this attachment.
    let _ = result;
    drop(terminal);
    let _ = send(&mut socket, Message::Close(None)).await;
}
