//! Browser terminals: this device's shells, paired devices, or an optional relay.
use super::{RelayTerminal, Services, error};
use agent_workspace_runtime::peers::terminal::{self as direct, Event, Input};
use axum::{
    extract::{
        WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::SinkExt;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore, watch};

#[derive(Clone)]
pub(crate) struct Hub {
    tickets: Arc<Mutex<HashMap<String, Ticket>>>,
    slots: Arc<Semaphore>,
}
impl Default for Hub {
    fn default() -> Self {
        Self {
            tickets: Arc::default(),
            slots: Arc::new(Semaphore::new(8)),
        }
    }
}
struct Ticket {
    session: String,
    expires: Instant,
    options: Options,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    node: Option<String>,
    #[serde(default)]
    local: bool,
    #[serde(default)]
    direct: bool,
    cols: u16,
    rows: u16,
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    takeover: bool,
}
impl Hub {
    pub(crate) async fn issue(&self, session: String, body: Value) -> anyhow::Result<Value> {
        let options: Options = serde_json::from_value(body)?;
        agent_workspace_pty::validate_size(options.cols, options.rows)?;
        anyhow::ensure!(
            !options.direct || options.node.is_some(),
            "direct terminal requires a node"
        );
        anyhow::ensure!(
            !options.direct || !options.local,
            "conflicting terminal routes"
        );
        if let Some(node) = &options.node {
            anyhow::ensure!(
                agent_workspace_runtime::protocol::valid_node_id(node),
                "invalid node"
            );
        }
        anyhow::ensure!(uuid_is_valid(&options.session_id), "invalid session ID");
        let mut tickets = self.tickets.lock().await;
        tickets.retain(|_, t| t.expires > Instant::now());
        anyhow::ensure!(tickets.len() < 64, "too many terminal requests");
        let token = agent_workspace_runtime::config::token();
        tickets.insert(
            token.clone(),
            Ticket {
                session,
                expires: Instant::now() + Duration::from_secs(30),
                options,
            },
        );
        Ok(json!({"ticket":token}))
    }
}

#[derive(Deserialize)]
pub struct Connection {
    ticket: String,
}

/// `GET /api/terminal/ws?ticket=…`. Native/no-Origin upgrades are rejected.
pub async fn upgrade(
    services: Services,
    headers: HeaderMap,
    query: Connection,
    ws: WebSocketUpgrade,
) -> Response {
    if !headers.contains_key("origin") {
        return error(StatusCode::FORBIDDEN, "Origin required");
    }
    let Some(session) = services.sessions.session(&headers).await else {
        return error(StatusCode::UNAUTHORIZED, "Session expired");
    };
    let ticket = {
        let mut tickets = services.terminals.tickets.lock().await;
        let valid = tickets
            .get(&query.ticket)
            .is_some_and(|t| t.session == session && t.expires > Instant::now());
        if !valid {
            return error(StatusCode::UNAUTHORIZED, "Terminal ticket expired");
        }
        tickets.remove(&query.ticket).unwrap()
    };
    let Ok(permit) = services.terminals.slots.clone().try_acquire_owned() else {
        return error(StatusCode::TOO_MANY_REQUESTS, "Maximum eight terminals");
    };
    let closing = services.closing.clone();
    ws.max_message_size(16 * 1024)
        .max_frame_size(16 * 1024)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            match open(&services, ticket.options).await {
                Ok(terminal) => run(socket, terminal, closing).await,
                Err(error) => {
                    let mut socket = socket;
                    let _ = socket
                        .send(Message::Text(
                            json!({"type":"error","message":error.to_string()})
                                .to_string()
                                .into(),
                        ))
                        .await;
                }
            }
        })
        .into_response()
}

enum Terminal {
    Local(crate::local::Terminal),
    Direct(direct::Terminal),
    Relay(Box<dyn RelayTerminal>),
}
async fn open(services: &Services, options: Options) -> anyhow::Result<Terminal> {
    let own = services
        .local
        .as_ref()
        .is_some_and(|local| options.node.as_deref() == Some(&local.id));
    anyhow::ensure!(!options.local || own, "local node identity changed");
    if let Some(node) = options.node.filter(|_| options.direct || !own) {
        if options.direct {
            return Ok(Terminal::Direct(
                direct::Terminal::connect(
                    services.peer_store().route(&node)?,
                    &node,
                    &options.session_id,
                    options.cols,
                    options.rows,
                    options.takeover,
                )
                .await?,
            ));
        }
        let relay = services
            .relay
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Pair this device to open its terminal"))?;
        return Ok(Terminal::Relay(
            relay
                .terminal(
                    &node,
                    options.cols,
                    options.rows,
                    options.session_id,
                    options.takeover,
                )
                .await?,
        ));
    }
    Ok(Terminal::Local(
        services
            .local()?
            .attach(
                &options.session_id,
                options.cols,
                options.rows,
                options.takeover,
            )
            .await?,
    ))
}
impl Terminal {
    fn input(&self, message: Input) -> anyhow::Result<()> {
        match self {
            Self::Local(local) => local.send(message),
            Self::Direct(remote) => remote.send(message),
            Self::Relay(remote) => {
                message.validate()?;
                remote.send(message)
            }
        }
    }
    async fn output(&mut self) -> Option<Event> {
        match self {
            Self::Local(local) => local.output.recv().await,
            Self::Direct(remote) => remote.output.recv().await,
            Self::Relay(remote) => remote.output().await,
        }
    }
}
async fn run(mut socket: WebSocket, mut terminal: Terminal, mut closing: watch::Receiver<bool>) {
    if !matches!(
        tokio::time::timeout(
            Duration::from_secs(5),
            socket.send(Message::Text(json!({"type":"ready"}).to_string().into()))
        )
        .await,
        Ok(Ok(()))
    ) {
        return;
    }
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    let mut seen = Instant::now();
    let mut activity = Instant::now();
    loop {
        tokio::select! {
            _ = closing.changed() => break,
            _ = heartbeat.tick() => {
                if seen.elapsed() > Duration::from_secs(60) || activity.elapsed() > Duration::from_secs(30 * 60) { break; }
                if !matches!(tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Ping(vec![].into()))).await, Ok(Ok(()))) { break; }
            }
            message = socket.recv() => match message {
                Some(Ok(Message::Text(text))) => {
                    seen = Instant::now(); activity = seen;
                    let Ok(input) = serde_json::from_str(&text) else { break; };
                    if terminal.input(input).is_err() { break; }
                }
                Some(Ok(Message::Pong(_))) | Some(Ok(Message::Ping(_))) => seen = Instant::now(),
                _ => break,
            },
            event = terminal.output() => {
                let message = match event {
                    Some(Event::Output(data)) => Message::Binary(data.into()),
                    Some(Event::Exit(code)) => { let _ = tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Text(json!({"type":"attachment_closed","client_exit_code":code}).to_string().into()))).await; break; },
                    Some(Event::Error(_)) | None => break,
                };
                activity = Instant::now();
                if !matches!(tokio::time::timeout(Duration::from_secs(5), socket.send(message)).await, Ok(Ok(()))) { break; }
            }
        }
    }
    drop(terminal);
    let _ = tokio::time::timeout(Duration::from_secs(1), socket.close()).await;
}
fn uuid_is_valid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}
pub async fn manage(services: &Services, mut body: Value) -> anyhow::Result<Value> {
    let object = body
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("session command must be an object"))?;
    let node = object.remove("node");
    let local = match object.remove("local") {
        None | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        _ => anyhow::bail!("invalid local flag"),
    };
    let direct = match object.remove("direct") {
        None | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        _ => anyhow::bail!("invalid direct flag"),
    };
    anyhow::ensure!(
        !direct || matches!(&node, Some(Value::String(_))),
        "direct terminal requires a node"
    );
    let own = services
        .local
        .as_ref()
        .is_some_and(|local| node.as_ref().and_then(Value::as_str) == Some(&local.id));
    anyhow::ensure!(!local || (own && !direct), "local node identity changed");
    match node {
        None | Some(Value::Null) => {
            services
                .local()?
                .sessions(serde_json::from_value(body)?)
                .await
        }
        Some(Value::String(_)) if own && !direct => {
            services
                .local()?
                .sessions(serde_json::from_value(body)?)
                .await
        }
        Some(Value::String(node)) => {
            anyhow::ensure!(
                agent_workspace_runtime::protocol::valid_node_id(&node),
                "invalid node"
            );
            if direct {
                return services
                    .peer_store()
                    .manage(agent_workspace_runtime::peers::Command::Sessions {
                        node,
                        command: body,
                    })
                    .await;
            }
            services
                .relay
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Pair this device to manage its sessions"))?
                .terminal_manage(&node, body)
                .await
        }
        _ => anyhow::bail!("invalid node"),
    }
}
