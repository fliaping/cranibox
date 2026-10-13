use crate::journal::Journal;
use crate::{config, protocol::*};
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{
        DefaultBodyLimit, Path, Request, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, mpsc, oneshot};

#[derive(Clone)]
struct App {
    credentials: PathBuf,
    inner: Arc<Mutex<Registry>>,
    history: Arc<Mutex<Journal>>,
    proxy: reqwest::Client,
    upstream: Option<String>,
}
#[derive(Default)]
struct Registry {
    nodes: BTreeMap<String, Node>,
    jobs: BTreeMap<String, Job>,
    terminals: BTreeMap<String, TerminalRoute>,
    terminal_requests: BTreeMap<String, TerminalRequest>,
}
struct TerminalRequest {
    node: String,
    generation: String,
    reply: oneshot::Sender<Value>,
}
struct TerminalRoute {
    node: String,
    generation: String,
    tx: mpsc::Sender<Wire>,
}
impl Registry {
    fn prune_jobs(&mut self) {
        self.jobs.retain(|_, job| {
            job.finished_at
                .is_none_or(|at| at.elapsed() < Duration::from_secs(600))
        });
    }
}
struct Node {
    name: String,
    os: String,
    arch: String,
    terminal: bool,
    terminal_sessions: bool,
    generation: String,
    tx: Option<mpsc::Sender<Wire>>,
    last_seen: u64,
}
struct Job {
    node: String,
    generation: String,
    outcome: Outcome,
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    truncated: bool,
    error: Option<String>,
    created_at: u64,
    finished_at: Option<Instant>,
    cancel_requested: bool,
}
impl Job {
    fn view(&self, id: &str) -> Value {
        json!({"id":id, "node":self.node, "outcome":self.outcome, "exit_code":self.exit_code,
            "stdout":display_output(&self.stdout, self.outcome == Outcome::Running), "stderr":display_output(&self.stderr, self.outcome == Outcome::Running),
            "stdout_base64":STANDARD.encode(&self.stdout), "stderr_base64":STANDARD.encode(&self.stderr),
            "truncated":self.truncated, "error":self.error, "created_at":self.created_at,
            "cancel_requested":self.cancel_requested})
    }
    fn finish(&mut self, outcome: Outcome, code: Option<i32>, error: Option<String>) {
        self.outcome = outcome;
        self.exit_code = code;
        self.error = error;
        self.finished_at = Some(Instant::now());
    }
}
// Streaming text must never replace an incomplete character that a later frame completes.
fn display_output(bytes: &[u8], running: bool) -> String {
    let mut end = bytes.len();
    if running {
        let mut offset = 0;
        while offset < bytes.len() {
            match std::str::from_utf8(&bytes[offset..]) {
                Ok(_) => break,
                Err(error) => {
                    offset += error.valid_up_to();
                    if let Some(length) = error.error_len() {
                        offset += length;
                    } else {
                        end = offset;
                        break;
                    }
                }
            }
        }
    }
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
type ApiError = (StatusCode, Json<Value>);
fn error(status: StatusCode, message: &str) -> ApiError {
    (status, Json(json!({"error":message})))
}
fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

async fn controller_auth(State(app): State<App>, req: Request, next: Next) -> Response {
    if req.headers().contains_key("origin")
        || req
            .headers()
            .get("sec-fetch-site")
            .is_some_and(|v| v == "cross-site")
    {
        return error(
            StatusCode::FORBIDDEN,
            "native Agent API does not accept browser requests",
        )
        .into_response();
    }
    let authorized = config::Credentials::read(&app.credentials).is_ok_and(|c| {
        bearer(req.headers()).is_some_and(|token| config::matches(token, &c.controller_token))
    });
    if !authorized {
        return error(
            StatusCode::UNAUTHORIZED,
            "controller authentication required",
        )
        .into_response();
    }
    next.run(req).await
}
async fn nodes(State(app): State<App>) -> Result<Json<Value>, ApiError> {
    let creds = config::Credentials::read(&app.credentials)
        .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "credentials unavailable"))?;
    let registry = app.inner.lock().await;
    let nodes: Vec<_> = creds.nodes.keys().map(|id| {
        if let Some(n) = registry.nodes.get(id) {
            json!({"id":id,"name":n.name,"os":n.os,"arch":n.arch,"terminal":n.terminal,"terminal_sessions":n.terminal_sessions,"online":n.tx.is_some(),"last_seen":n.last_seen})
        } else { json!({"id":id,"online":false}) }
    }).collect();
    Ok(Json(json!({"nodes":nodes, "protocol_version":VERSION})))
}
async fn execute(
    State(app): State<App>,
    Path(node): Path<String>,
    Json(request): Json<ExecRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    request
        .validate()
        .map_err(|e| error(StatusCode::BAD_REQUEST, &e.to_string()))?;
    let enrolled =
        config::Credentials::read(&app.credentials).is_ok_and(|c| c.nodes.contains_key(&node));
    if !enrolled {
        return Err(error(StatusCode::NOT_FOUND, "node is not enrolled"));
    }
    let mut registry = app.inner.lock().await;
    // Keep finished jobs for ten minutes, with a hard bound on total resident jobs.
    registry.prune_jobs();
    if registry.jobs.len() >= MAX_JOBS {
        if let Some(id) = registry
            .jobs
            .iter()
            .filter(|(_, j)| j.finished_at.is_some())
            .min_by_key(|(_, j)| j.finished_at)
            .map(|(id, _)| id.clone())
        {
            registry.jobs.remove(&id);
        } else {
            return Err(error(StatusCode::TOO_MANY_REQUESTS, "job capacity reached"));
        }
    }
    let n = registry
        .nodes
        .get(&node)
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "node has not connected"))?;
    let tx =
        n.tx.clone()
            .ok_or_else(|| error(StatusCode::CONFLICT, "node offline"))?;
    let generation = n.generation.clone();
    if registry
        .jobs
        .values()
        .filter(|j| j.node == node && j.outcome == Outcome::Running)
        .count()
        >= MAX_PARALLEL
    {
        return Err(error(
            StatusCode::TOO_MANY_REQUESTS,
            "node already running four jobs",
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    // Persist before dispatch: an interrupted command is never replayed.
    let initial = json!({"id":id,"node":node,"program":request.program,"args":request.args,"cwd":request.cwd,"outcome":"running","created_at":now(),"stdout":"","stderr":""});
    app.history
        .lock()
        .await
        .insert(initial)
        .map_err(|_| error(StatusCode::INSUFFICIENT_STORAGE, "cannot persist operation"))?;
    // Queue and insert under the same lock so a fast node response cannot race insertion.
    if tx
        .try_send(Wire::Execute {
            id: id.clone(),
            request,
        })
        .is_err()
    {
        let mut history = app.history.lock().await;
        history.update(&id, |v| {
            v["outcome"] = json!("failed");
            v["error"] = json!("node queue unavailable; command was not dispatched");
            v["finished_at"] = json!(now());
        });
        let _ = history.flush();
        return Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "node queue unavailable",
        ));
    }
    let job = Job {
        node,
        generation,
        outcome: Outcome::Running,
        exit_code: None,
        stdout: vec![],
        stderr: vec![],
        truncated: false,
        error: None,
        created_at: now(),
        finished_at: None,
        cancel_requested: false,
    };
    let view = job.view(&id);
    registry.jobs.insert(id, job);
    Ok((StatusCode::ACCEPTED, Json(view)))
}
async fn job(State(app): State<App>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let mut registry = app.inner.lock().await;
    registry.prune_jobs();
    let value = registry.jobs.get(&id).map(|job| job.view(&id));
    drop(registry);
    let mut record = app
        .history
        .lock()
        .await
        .get(&id)
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "job unavailable or expired"))?;
    if let Some(view) = value {
        merge_record(&mut record, view);
    }
    Ok(Json(record))
}
async fn cancel(State(app): State<App>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let mut registry = app.inner.lock().await;
    registry.prune_jobs();
    let Some(job) = registry.jobs.get(&id) else {
        drop(registry);
        return Ok(Json(app.history.lock().await.get(&id).ok_or_else(
            || error(StatusCode::NOT_FOUND, "job unavailable or expired"),
        )?));
    };
    if job.outcome != Outcome::Running {
        return Ok(Json(job.view(&id)));
    }
    let tx = registry
        .nodes
        .get(&job.node)
        .and_then(|n| n.tx.clone())
        .ok_or_else(|| error(StatusCode::CONFLICT, "node offline"))?;
    tx.try_send(Wire::Cancel { id: id.clone() })
        .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "node queue unavailable"))?;
    let job = registry.jobs.get_mut(&id).unwrap();
    job.cancel_requested = true;
    Ok(Json(job.view(&id)))
}
async fn upgrade(
    State(app): State<App>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    if !valid_node_id(&id) || headers.contains_key("origin") {
        return Err(error(
            StatusCode::FORBIDDEN,
            "native node connections required",
        ));
    }
    let token = bearer(&headers).unwrap_or_default().to_owned();
    let creds = config::Credentials::read(&app.credentials)
        .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "credentials unavailable"))?;
    if !creds
        .nodes
        .get(&id)
        .is_some_and(|expected| config::matches(&token, expected))
    {
        return Err(error(
            StatusCode::UNAUTHORIZED,
            "node authentication required",
        ));
    }
    Ok(ws
        .max_message_size(MAX_FRAME)
        .max_frame_size(MAX_FRAME)
        .on_upgrade(move |socket| connection(app, id, token, socket)))
}
async fn send(socket: &mut WebSocket, wire: Wire) -> anyhow::Result<()> {
    tokio::time::timeout(
        Duration::from_secs(5),
        socket.send(Message::Text(serde_json::to_string(&wire)?.into())),
    )
    .await??;
    Ok(())
}
async fn connection(app: App, id: String, token: String, mut socket: WebSocket) {
    let hello = tokio::time::timeout(Duration::from_secs(5), socket.recv()).await;
    let Some(Wire::Hello {
        version: VERSION,
        name,
        os,
        arch,
        terminal,
        terminal_sessions,
    }) = hello.ok().flatten().and_then(Result::ok).and_then(|m| {
        if let Message::Text(s) = m {
            serde_json::from_str(&s).ok()
        } else {
            None
        }
    })
    else {
        return;
    };
    if name.len() > 128 || arch.len() > 32 || !["linux", "macos", "windows"].contains(&os.as_str())
    {
        return;
    }
    let generation = uuid::Uuid::new_v4().to_string();
    let (tx, mut rx) = mpsc::channel(16);
    {
        let mut registry = app.inner.lock().await;
        if registry.nodes.get(&id).is_some_and(|n| n.tx.is_some()) {
            return;
        }
        if registry.nodes.len() >= 128 && !registry.nodes.contains_key(&id) {
            registry.nodes.retain(|_, n| n.tx.is_some());
            if registry.nodes.len() >= 128 {
                return;
            }
        }
        registry.nodes.insert(
            id.clone(),
            Node {
                name,
                os,
                arch,
                terminal,
                terminal_sessions,
                generation: generation.clone(),
                tx: Some(tx),
                last_seen: now(),
            },
        );
    }
    // All exits after registration must run disconnect cleanup.
    let mut last_seen = Instant::now();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(HEARTBEAT_SECS));
    let mut ready = send(&mut socket, Wire::Welcome { version: VERSION })
        .await
        .is_ok();
    while ready {
        tokio::select! {
            outbound = rx.recv() => {
                if let Some(wire) = outbound { ready = send(&mut socket, wire).await.is_ok(); } else { break; }
            }
            _ = heartbeat.tick() => {
                let authorized = config::Credentials::read(&app.credentials).is_ok_and(|c| c.nodes.get(&id).is_some_and(|expected| config::matches(&token, expected)));
                if !authorized || last_seen.elapsed() > Duration::from_secs(DEAD_SECS) { break; }
                ready = send(&mut socket, Wire::Heartbeat {}).await.is_ok();
            }
            incoming = socket.recv() => {
                let Some(Ok(message)) = incoming else { break; };
                match message {
                    Message::Text(text) => {
                        let Ok(wire) = serde_json::from_str::<Wire>(&text) else { break; };
                        match wire {
                            Wire::Heartbeat {} => {
                                last_seen = Instant::now();
                                if let Some(n) = app.inner.lock().await.nodes.get_mut(&id) { n.last_seen = now(); }
                            }
                            Wire::Output { id:job_id, stream, data } => {
                                let Ok(bytes) = STANDARD.decode(data) else { break; };
                                let mut registry = app.inner.lock().await;
                                let Some(job) = registry.jobs.get_mut(&job_id).filter(|j| j.node == id && j.generation == generation && j.outcome == Outcome::Running) else { break; };
                                let available = MAX_OUTPUT.saturating_sub(job.stdout.len() + job.stderr.len());
                                job.truncated |= bytes.len() > available;
                                match stream { Stream::Stdout => &mut job.stdout, Stream::Stderr => &mut job.stderr }.extend_from_slice(&bytes[..bytes.len().min(available)]);
                                let view = job.view(&job_id);
                                app.history.lock().await.update(&job_id, |v| merge_record(v, view));
                            }
                            Wire::Finished { id:job_id, exit_code, outcome, error } => {
                                if ![Outcome::Exited,Outcome::Failed,Outcome::Cancelled,Outcome::TimedOut].contains(&outcome) || error.as_ref().is_some_and(|e| e.len() > 2048) { break; }
                                let mut registry = app.inner.lock().await;
                                let Some(job) = registry.jobs.get_mut(&job_id).filter(|j| j.node == id && j.generation == generation && j.outcome == Outcome::Running) else { break; };
                                job.finish(outcome, exit_code, error);
                                let view = job.view(&job_id);
                                let mut history = app.history.lock().await;
                                history.update(&job_id, |v| merge_record(v, view));
                                if history.flush().is_err() { eprintln!("operation history flush failed"); }
                            }
                            Wire::TerminalResult { id: request_id, result } => {
                                let mut registry = app.inner.lock().await;
                                if registry.terminal_requests.get(&request_id).is_some_and(|r| r.node == id && r.generation == generation)
                                    && let Some(request) = registry.terminal_requests.remove(&request_id) {
                                    let _ = request.reply.send(result);
                                }
                            }
                            wire @ (Wire::TerminalOutput { .. } | Wire::TerminalExit { .. }) => {
                                let terminal_id = match &wire { Wire::TerminalOutput { id, data } => {
                                    if STANDARD.decode(data).is_err() { break; } id.clone()
                                }, Wire::TerminalExit { id, .. } => id.clone(), _ => unreachable!() };
                                let finished = matches!(&wire, Wire::TerminalExit { .. });
                                let mut registry = app.inner.lock().await;
                                if let Some(route) = registry.terminals.get(&terminal_id).filter(|r| r.node == id && r.generation == generation) {
                                    if route.tx.try_send(wire).is_err() {
                                        if let Some(node) = registry.nodes.get(&id).and_then(|n| n.tx.as_ref()) { close_terminal(node, terminal_id.clone()); }
                                        registry.terminals.remove(&terminal_id);
                                    } else if finished { registry.terminals.remove(&terminal_id); }
                                }
                            }
                            _ => break,
                        }
                    }
                    Message::Ping(_) | Message::Pong(_) => { /* JSON heartbeats are authoritative. */ }
                    Message::Close(_) | Message::Binary(_) => break,
                }
            }
        }
    }
    let mut registry = app.inner.lock().await;
    if let Some(node) = registry
        .nodes
        .get_mut(&id)
        .filter(|n| n.generation == generation)
    {
        node.tx = None;
    }
    for job in registry
        .jobs
        .values_mut()
        .filter(|j| j.node == id && j.generation == generation && j.outcome == Outcome::Running)
    {
        job.finish(
            Outcome::Disconnected,
            None,
            Some("node disconnected; command is not replayed".into()),
        );
    }
    registry
        .terminals
        .retain(|_, r| !(r.node == id && r.generation == generation));
    registry
        .terminal_requests
        .retain(|_, r| !(r.node == id && r.generation == generation));
    let mut history = app.history.lock().await;
    for (job_id, job) in &registry.jobs {
        history.update(job_id, |v| merge_record(v, job.view(job_id)));
    }
    if history.flush().is_err() {
        eprintln!("operation history flush failed");
    }
}

fn strip_hop_headers(headers: &mut HeaderMap) {
    let extra: Vec<_> = headers
        .get_all("connection")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(',').map(|s| s.trim().to_owned()))
        .collect();
    for name in extra {
        headers.remove(name);
    }
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
}
async fn proxy(State(app): State<App>, request: Request) -> Response {
    let Some(upstream) = &app.upstream else {
        return error(
            StatusCode::NOT_FOUND,
            "unknown endpoint; no legacy Web UI configured",
        )
        .into_response();
    };
    let (mut parts, body) = request.into_parts();
    if parts.headers.contains_key("upgrade") {
        return error(
            StatusCode::BAD_REQUEST,
            "WebSocket is available only at /ws/nodes/{id}",
        )
        .into_response();
    }
    strip_hop_headers(&mut parts.headers);
    for name in [
        "forwarded",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-forwarded-for",
        "authorization",
    ] {
        parts.headers.remove(name);
    }
    let body = match to_bytes(body, MAX_FRAME).await {
        Ok(b) => b,
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "request too large").into_response(),
    };
    let url = format!(
        "{}{}",
        upstream,
        parts.uri.path_and_query().map_or("/", |p| p.as_str())
    );
    // Preserve Host/Origin for the existing UI's explicit origin and CSRF checks.
    let response = match tokio::time::timeout(
        Duration::from_secs(40),
        app.proxy
            .request(parts.method, url)
            .headers(parts.headers)
            .body(body)
            .send(),
    )
    .await
    {
        Ok(Ok(r)) => r,
        _ => {
            return error(StatusCode::BAD_GATEWAY, "legacy Web UI unavailable").into_response();
        }
    };
    let status = response.status();
    let mut headers = response.headers().clone();
    strip_hop_headers(&mut headers);
    let mut result = Response::new(Body::from_stream(response.bytes_stream()));
    *result.status_mut() = status;
    *result.headers_mut() = headers;
    result
}

#[derive(Clone)]
pub struct Gateway {
    app: App,
    router: Router,
}
impl Gateway {
    pub fn new(credentials: PathBuf) -> anyhow::Result<Self> {
        let upstream: Option<String> = None;
        config::Credentials::read(&credentials)?;
        if let Some(value) = &upstream {
            let url = config::check_url(value, false, false)?;
            anyhow::ensure!(
                url.scheme() == "http"
                    && url.host_str().is_some_and(|s| s == "localhost"
                        || s.parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback()))
                    && url.path() == "/",
                "Web upstream must be a loopback HTTP origin"
            );
        }
        let history = Journal::open(credentials.with_extension("history"))?;
        let app = App {
            history: Arc::new(Mutex::new(history)),
            credentials,
            inner: Arc::new(Mutex::new(Registry::default())),
            proxy: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .build()?,
            upstream: upstream.map(|s| s.trim_end_matches('/').to_owned()),
        };
        let api = Router::new()
            .route("/nodes", get(nodes))
            .route("/nodes/{node}/exec", post(execute))
            .route("/jobs", get(jobs))
            .route("/jobs/{id}", get(job))
            .route("/jobs/{id}/cancel", post(cancel))
            .route_layer(middleware::from_fn_with_state(app.clone(), controller_auth));
        let router = Router::new()
            .nest("/api/v1", api)
            .route("/ws/nodes/{id}", get(upgrade))
            .route(
                "/healthz",
                get(|| async { Json(json!({"ok":true,"protocol_version":VERSION})) }),
            )
            .layer(DefaultBodyLimit::max(MAX_FRAME))
            .with_state(app.clone());
        Ok(Self { app, router })
    }
    pub fn router(&self) -> Router {
        self.router.clone()
    }
    pub async fn maintenance(&self) {
        let _ = self.app.history.lock().await.flush();
    }
    pub async fn shutdown(&self) {
        let mut registry = self.app.inner.lock().await;
        registry.terminals.clear();
        registry.terminal_requests.clear();
        for node in registry.nodes.values_mut() {
            node.tx = None;
        }
        for (id, job) in &mut registry.jobs {
            if job.outcome == Outcome::Running {
                job.finish(
                    Outcome::Disconnected,
                    None,
                    Some("gateway stopped; command was not replayed".into()),
                );
            }
            self.app
                .history
                .lock()
                .await
                .update(id, |v| merge_record(v, job.view(id)));
        }
        let _ = self.app.history.lock().await.flush();
    }
    pub async fn terminal(
        &self,
        node: &str,
        cols: u16,
        rows: u16,
        session: String,
        takeover: bool,
    ) -> anyhow::Result<RemoteTerminal> {
        anyhow::ensure!(
            (2..=500).contains(&cols) && (2..=300).contains(&rows),
            "invalid terminal size"
        );
        anyhow::ensure!(
            config::Credentials::read(&self.app.credentials)?
                .nodes
                .contains_key(node),
            "node is not enrolled"
        );
        let mut registry = self.app.inner.lock().await;
        registry.terminals.retain(|_, r| !r.tx.is_closed());
        anyhow::ensure!(
            registry.terminals.len() < 32
                && registry
                    .terminals
                    .values()
                    .filter(|r| r.node == node)
                    .count()
                    < 4,
            "terminal capacity reached"
        );
        let n = registry
            .nodes
            .get(node)
            .ok_or_else(|| anyhow::anyhow!("unknown node"))?;
        anyhow::ensure!(
            n.terminal_sessions,
            "update this node to cranibox-node with persistent terminal support"
        );
        let tx =
            n.tx.clone()
                .ok_or_else(|| anyhow::anyhow!("node offline"))?;
        let generation = n.generation.clone();
        let id = uuid::Uuid::new_v4().to_string();
        let (output, receiver) = mpsc::channel(32);
        tx.try_send(Wire::TerminalOpen {
            id: id.clone(),
            cols,
            rows,
            session,
            takeover,
        })
        .map_err(|_| anyhow::anyhow!("node queue unavailable"))?;
        registry.terminals.insert(
            id.clone(),
            TerminalRoute {
                node: node.into(),
                generation,
                tx: output,
            },
        );
        Ok(RemoteTerminal {
            id,
            tx,
            output: receiver,
        })
    }
    pub async fn terminal_manage(&self, node: &str, request: Value) -> anyhow::Result<Value> {
        anyhow::ensure!(
            config::Credentials::read(&self.app.credentials)?
                .nodes
                .contains_key(node),
            "node is not enrolled"
        );
        let id = uuid::Uuid::new_v4().to_string();
        let (reply, receiver) = oneshot::channel();
        {
            let mut registry = self.app.inner.lock().await;
            registry
                .terminal_requests
                .retain(|_, r| !r.reply.is_closed());
            anyhow::ensure!(
                registry.terminal_requests.len() < 32,
                "too many terminal requests"
            );
            let n = registry
                .nodes
                .get(node)
                .ok_or_else(|| anyhow::anyhow!("unknown node"))?;
            anyhow::ensure!(
                n.terminal_sessions,
                "update this node to cranibox-node with persistent terminal support"
            );
            let tx =
                n.tx.clone()
                    .ok_or_else(|| anyhow::anyhow!("node offline"))?;
            let generation = n.generation.clone();
            tx.try_send(Wire::TerminalManage {
                id: id.clone(),
                request,
            })
            .map_err(|_| anyhow::anyhow!("node queue unavailable"))?;
            registry.terminal_requests.insert(
                id.clone(),
                TerminalRequest {
                    node: node.into(),
                    generation,
                    reply,
                },
            );
        }
        let result = tokio::time::timeout(Duration::from_secs(20), receiver).await;
        self.app.inner.lock().await.terminal_requests.remove(&id);
        let value = result
            .map_err(|_| anyhow::anyhow!("node terminal request timed out"))?
            .map_err(|_| anyhow::anyhow!("node disconnected"))?;
        anyhow::ensure!(
            value.get("error").is_none(),
            "{}",
            value["error"]
                .as_str()
                .unwrap_or("node terminal request failed")
        );
        Ok(value)
    }
    pub async fn api(
        &self,
        method: axum::http::Method,
        path: &str,
        body: Option<Value>,
    ) -> anyhow::Result<Value> {
        use tower::ServiceExt;
        let creds = config::Credentials::read(&self.app.credentials)?;
        let request = axum::http::Request::builder()
            .method(method)
            .uri(format!("/api/v1{path}"))
            .header(
                "authorization",
                format!("Bearer {}", creds.controller_token),
            )
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body.unwrap_or(json!({})))?))?;
        let response = self.router.clone().oneshot(request).await?;
        let status = response.status();
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 2 * 1024 * 1024).await?)?;
        anyhow::ensure!(
            status.is_success(),
            "{}",
            value["error"].as_str().unwrap_or("Node operation failed")
        );
        Ok(value)
    }
}
pub struct RemoteTerminal {
    pub id: String,
    tx: mpsc::Sender<Wire>,
    pub output: mpsc::Receiver<Wire>,
}
impl RemoteTerminal {
    pub fn send(&self, wire: Wire) -> anyhow::Result<()> {
        self.tx
            .try_send(wire)
            .map_err(|_| anyhow::anyhow!("node queue unavailable"))
    }
}
impl Drop for RemoteTerminal {
    fn drop(&mut self) {
        close_terminal(&self.tx, self.id.clone());
    }
}
fn close_terminal(tx: &mpsc::Sender<Wire>, id: String) {
    if let Err(mpsc::error::TrySendError::Full(wire)) = tx.try_send(Wire::TerminalClose { id }) {
        let tx = tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(wire).await;
        });
    }
}
pub async fn serve(
    listen: SocketAddr,
    credentials: PathBuf,
    upstream: Option<String>,
) -> anyhow::Result<()> {
    if let Some(value) = &upstream {
        let url = config::check_url(value, false, false)?;
        anyhow::ensure!(
            url.scheme() == "http"
                && url.host_str().is_some_and(|s| s == "localhost"
                    || s.parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback()))
                && url.path() == "/",
            "Web upstream must be a loopback HTTP origin"
        );
    }
    let mut gateway = Gateway::new(credentials)?;
    gateway.app.upstream = upstream.map(|s| s.trim_end_matches('/').to_owned());
    let router = gateway
        .router()
        .fallback(axum::routing::any(proxy).with_state(gateway.app.clone()));
    let listener = tokio::net::TcpListener::bind(listen).await?;
    eprintln!("Agent Workspace Rust gateway: {}", listener.local_addr()?);
    let flush = gateway.clone();
    let task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        loop {
            interval.tick().await;
            flush.maintenance().await;
        }
    });
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            crate::shutdown_signal().await;
            gateway.shutdown().await;
        })
        .await?;
    task.abort();
    Ok(())
}

fn merge_record(record: &mut Value, view: Value) {
    if let (Some(record), Some(view)) = (record.as_object_mut(), view.as_object()) {
        record.extend(view.clone());
        if record.get("outcome").is_some_and(|v| v != "running") {
            record.insert("finished_at".into(), json!(now()));
        }
    }
}
async fn jobs(State(app): State<App>) -> Json<Value> {
    let values: Vec<_> = app
        .history
        .lock()
        .await
        .list()
        .into_iter()
        .map(|mut v| {
            if let Some(obj) = v.as_object_mut() {
                for key in ["stdout", "stderr", "stdout_base64", "stderr_base64"] {
                    obj.remove(key);
                }
            }
            v
        })
        .collect();
    Json(json!({"jobs":values}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(directory: &std::path::Path, tx: mpsc::Sender<Wire>) -> App {
        let credentials = directory.join("runtime.json");
        config::write_private(
            &credentials,
            &serde_json::to_vec(&config::Credentials {
                controller_token: config::token(),
                nodes: BTreeMap::from([("one".into(), config::token())]),
            })
            .unwrap(),
            false,
        )
        .unwrap();
        App {
            credentials,
            history: Arc::new(Mutex::new(
                Journal::open(directory.join("history")).unwrap(),
            )),
            proxy: reqwest::Client::new(),
            upstream: None,
            inner: Arc::new(Mutex::new(Registry {
                nodes: BTreeMap::from([(
                    "one".into(),
                    Node {
                        name: "One".into(),
                        os: "linux".into(),
                        arch: "x86_64".into(),
                        terminal: true,
                        terminal_sessions: true,
                        generation: "generation".into(),
                        tx: Some(tx),
                        last_seen: now(),
                    },
                )]),
                ..Registry::default()
            })),
        }
    }
    fn request() -> ExecRequest {
        ExecRequest {
            program: "echo".into(),
            args: vec!["hello".into()],
            cwd: None,
            env: Default::default(),
            timeout_secs: 5,
        }
    }
    #[tokio::test]
    async fn queue_failure_is_terminal_and_does_not_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel(4);
        drop(rx);
        let app = fixture(dir.path(), tx);
        let failure = execute(State(app.clone()), Path("one".into()), Json(request()))
            .await
            .unwrap_err();
        assert_eq!(failure.0, StatusCode::SERVICE_UNAVAILABLE);
        let record = app.history.lock().await.list().remove(0);
        assert_eq!(record["outcome"], "failed");
        assert!(app.inner.lock().await.jobs.is_empty());
        assert_eq!(
            Journal::open(dir.path().join("history")).unwrap().list()[0]["outcome"],
            "failed"
        );
    }
    #[tokio::test]
    async fn live_and_restored_job_preserve_command_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = mpsc::channel(4);
        let app = fixture(dir.path(), tx);
        let (_, Json(value)) = execute(State(app.clone()), Path("one".into()), Json(request()))
            .await
            .unwrap();
        let id = value["id"].as_str().unwrap().to_owned();
        assert!(matches!(rx.recv().await, Some(Wire::Execute { .. })));
        let Json(value) = job(State(app.clone()), Path(id.clone())).await.unwrap();
        assert_eq!(value["program"], "echo");
        assert_eq!(value["args"], json!(["hello"]));
        app.inner.lock().await.jobs.clear();
        let Json(value) = job(State(app.clone()), Path(id)).await.unwrap();
        assert_eq!(value["program"], "echo");
        assert_eq!(
            Journal::open(dir.path().join("history")).unwrap().list()[0]["outcome"],
            "interrupted"
        );
    }
    #[test]
    fn streaming_partial_utf8_and_binary_output() {
        let bytes = "中文".as_bytes();
        assert_eq!(display_output(&bytes[..1], true), "");
        assert_eq!(display_output(&bytes[..4], true), "中");
        assert_eq!(display_output(bytes, true), "中文");
        assert_eq!(display_output(&[255, bytes[0]], true), "�");
        assert_eq!(display_output(&[255, bytes[0]], false), "��");
    }
}
