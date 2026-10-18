use crate::server::State;
use agent_workspace_pty::{
    Event, Session,
    herdr::{Engine as SessionEngine, Manage},
};
use agent_workspace_runtime::{
    peers::{Store, terminal as direct},
    protocol::Wire,
    server::RemoteTerminal,
};
use axum::{
    Json,
    extract::{
        Query, State as ExtractState, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::SinkExt;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore, watch};

#[derive(Clone)]
pub struct Hub {
    tickets: Arc<Mutex<HashMap<String, Ticket>>>,
    slots: Arc<Semaphore>,
    engine: SessionEngine,
}
impl Hub {
    pub(crate) fn engine(&self) -> SessionEngine {
        self.engine.clone()
    }
    pub fn new(root: PathBuf) -> Self {
        Self {
            tickets: Arc::default(),
            slots: Arc::new(Semaphore::new(8)),
            engine: SessionEngine::new(root.clone(), root.join(".local/state/cranibox/terminal")),
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
    pub async fn issue(&self, session: String, body: Value) -> anyhow::Result<Value> {
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
pub async fn upgrade(
    ExtractState(state): ExtractState<State>,
    headers: HeaderMap,
    Query(query): Query<Connection>,
    ws: WebSocketUpgrade,
) -> Response {
    // guard validates allowed Host and Origin before this handler. Native/no-Origin upgrades are rejected.
    if !headers.contains_key("origin") {
        return failure(StatusCode::FORBIDDEN, "Origin required");
    }
    let Some(session) = state.session(&headers).await else {
        return failure(StatusCode::UNAUTHORIZED, "Session expired");
    };
    let session_id = session.lock().await.id.clone();
    let ticket = {
        let mut tickets = state.terminals.tickets.lock().await;
        let valid = tickets
            .get(&query.ticket)
            .is_some_and(|t| t.session == session_id && t.expires > Instant::now());
        if !valid {
            return failure(StatusCode::UNAUTHORIZED, "Terminal ticket expired");
        }
        tickets.remove(&query.ticket).unwrap()
    };
    let Ok(permit) = state.terminals.slots.clone().try_acquire_owned() else {
        return failure(StatusCode::TOO_MANY_REQUESTS, "Maximum eight terminals");
    };
    let closing = state.closing.subscribe();
    ws.max_message_size(16 * 1024)
        .max_frame_size(16 * 1024)
        .on_upgrade(move |socket| async move {
            let _session = session;
            let _permit = permit;
            match open(&state, ticket.options).await {
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
fn failure(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error":message}))).into_response()
}
enum Terminal {
    Local(Session),
    Remote(RemoteTerminal),
    Direct(direct::Terminal),
}
fn peer_store(state: &State) -> Store {
    Store::new(state.root.join(".config/cranibox-node/peers.json"))
}
async fn open(state: &State, options: Options) -> anyhow::Result<Terminal> {
    let local = state
        .local
        .as_ref()
        .is_some_and(|local| options.node.as_deref() == Some(&local.id));
    anyhow::ensure!(!options.local || local, "local node identity changed");
    if let Some(node) = options.node.filter(|_| options.direct || !local) {
        if options.direct {
            return Ok(Terminal::Direct(
                direct::Terminal::connect(
                    peer_store(state).route(&node)?,
                    &node,
                    &options.session_id,
                    options.cols,
                    options.rows,
                    options.takeover,
                )
                .await?,
            ));
        }
        let gateway = state.gateway.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Remote terminals require the unified Cranibox server")
        })?;
        return Ok(Terminal::Remote(
            gateway
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
    let pty = state
        .terminals
        .engine
        .attach(
            &options.session_id,
            options.cols,
            options.rows,
            options.takeover,
        )
        .await?;
    Ok(Terminal::Local(pty))
}
type Input = direct::Input;
impl Terminal {
    fn input(&self, message: Input) -> anyhow::Result<()> {
        message.validate()?;
        match (self, message) {
            (Self::Direct(remote), message) => remote.send(message),
            (Self::Local(pty), Input::Input { data }) => pty.write(data.into_bytes()),
            (Self::Local(pty), Input::Resize { cols, rows }) => pty.resize(cols, rows),
            (Self::Remote(remote), Input::Input { data }) => remote.send(Wire::TerminalInput {
                id: remote.id.clone(),
                data: STANDARD.encode(data),
            }),
            (Self::Remote(remote), Input::Resize { cols, rows }) => {
                agent_workspace_pty::validate_size(cols, rows)?;
                remote.send(Wire::TerminalResize {
                    id: remote.id.clone(),
                    cols,
                    rows,
                })
            }
        }
    }
    async fn output(&mut self) -> Option<Event> {
        match self {
            Self::Local(pty) => pty.output.recv().await,
            Self::Direct(remote) => match remote.output.recv().await? {
                direct::Event::Output(data) => Some(Event::Output(data)),
                direct::Event::Exit(code) => Some(Event::Exit(code)),
                direct::Event::Error(_) => None,
            },
            Self::Remote(remote) => match remote.output.recv().await? {
                Wire::TerminalOutput { data, .. } => STANDARD.decode(data).ok().map(Event::Output),
                Wire::TerminalExit { code, .. } => Some(Event::Exit(code)),
                _ => None,
            },
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
                    None => break,
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
pub async fn manage(state: &State, mut body: Value) -> anyhow::Result<Value> {
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
    let own_node = state
        .local
        .as_ref()
        .is_some_and(|local| node.as_ref().and_then(Value::as_str) == Some(&local.id));
    anyhow::ensure!(
        !local || (own_node && !direct),
        "local node identity changed"
    );
    let request: Manage = serde_json::from_value(body.clone())?;
    match node {
        None | Some(Value::Null) => state.terminals.engine.manage(request).await,
        Some(Value::String(_)) if own_node && !direct => {
            state.terminals.engine.manage(request).await
        }
        Some(Value::String(node)) => {
            anyhow::ensure!(
                agent_workspace_runtime::protocol::valid_node_id(&node),
                "invalid node"
            );
            if direct {
                return peer_store(state)
                    .manage(agent_workspace_runtime::peers::Command::Sessions {
                        node,
                        command: body,
                    })
                    .await;
            }
            state
                .gateway
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("remote sessions require the unified server"))?
                .terminal_manage(&node, body)
                .await
        }
        _ => anyhow::bail!("invalid node"),
    }
}
