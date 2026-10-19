use super::config::{Config, Grant, Listener};
use agent_workspace_pty::{
    Event,
    engine::{Attachment, Engine, Manage},
};
use agent_workspace_runtime::{config, protocol::*};
use anyhow::{Context, Result};
use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{
        DefaultBodyLimit, Path, Query, Request, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore, mpsc};
use tokio_util::sync::CancellationToken;
mod activity;
mod agent_sessions;
mod browser;
mod computer;
mod copies;
mod desktop;
mod executions;
mod messages;
mod projects;
mod spaces;
mod tasks;

type Failure = (StatusCode, Json<Value>);
fn failure(status: StatusCode, message: &str) -> Failure {
    (status, Json(json!({"error":message})))
}
#[derive(Clone)]
struct Caller {
    node: String,
    grant: Grant,
}
struct Job {
    caller: Caller,
    request: Vec<u8>,
    cancel: CancellationToken,
    finished: Option<Instant>,
    outcome: Outcome,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    truncated: bool,
    error: Option<String>,
}
impl Job {
    fn visible(&self, caller: &Caller) -> bool {
        self.caller.node == caller.node
            && config::matches(&self.caller.grant.token, &caller.grant.token)
    }
    fn view(&self, id: &str, node: &str) -> Value {
        json!({"id":id,"node":node,"requester":self.caller.node,"outcome":self.outcome,
            "exit_code":self.code,"stdout":String::from_utf8_lossy(&self.stdout),"stderr":String::from_utf8_lossy(&self.stderr),
            "stdout_base64":STANDARD.encode(&self.stdout),"stderr_base64":STANDARD.encode(&self.stderr),
            "truncated":self.truncated,"error":self.error,"cancel_requested":self.cancel.is_cancelled()})
    }
}
#[derive(Clone)]
struct App {
    path: PathBuf,
    node: String,
    engine: Engine,
    catalog: agent_workspace_runtime::projects::Store,
    jobs: Arc<Mutex<BTreeMap<String, Job>>>,
    slots: Arc<Semaphore>,
    streams: Arc<Semaphore>,
    desktops: Arc<Semaphore>,
    browsers: Arc<Semaphore>,
    computer: Arc<Mutex<Option<computer::Lease>>>,
    computer_actions: Arc<Mutex<()>>,
    stop: CancellationToken,
    connections: CancellationToken,
    tasks: Arc<Mutex<tokio::task::JoinSet<()>>>,
}
impl App {
    fn permitted(&self, caller: &Caller, terminal: bool) -> bool {
        Config::read(&self.path).is_ok_and(|configuration| {
            configuration.node_id == self.node
                && configuration.grants.get(&caller.node).is_some_and(|grant| {
                    config::matches(&grant.token, &caller.grant.token)
                        && if terminal {
                            grant.permissions.terminals
                        } else {
                            grant.permissions.exec
                        }
                })
        })
    }
}
async fn auth(State(app): State<App>, mut request: Request, next: Next) -> Response {
    if app.connections.is_cancelled() {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "listener stopped").into_response();
    }
    if request.headers().contains_key("origin")
        || request
            .headers()
            .get("sec-fetch-site")
            .is_some_and(|h| h == "cross-site")
    {
        return failure(
            StatusCode::FORBIDDEN,
            "direct node API does not accept browser requests",
        )
        .into_response();
    }
    if request
        .headers()
        .get("x-cranibox-target")
        .and_then(|h| h.to_str().ok())
        != Some(&app.node)
    {
        return failure(StatusCode::CONFLICT, "expected target node ID required").into_response();
    }
    let Some(token) = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
    else {
        return failure(StatusCode::UNAUTHORIZED, "peer credential required").into_response();
    };
    let configuration = match Config::read(&app.path) {
        Ok(value) if value.node_id == app.node => value,
        _ => {
            return failure(
                StatusCode::SERVICE_UNAVAILABLE,
                "peer configuration unavailable",
            )
            .into_response();
        }
    };
    let Some((node, grant)) = configuration
        .grants
        .into_iter()
        .find(|(_, g)| config::matches(token, &g.token))
    else {
        return failure(StatusCode::UNAUTHORIZED, "peer credential rejected").into_response();
    };
    request.extensions_mut().insert(Caller { node, grant });
    next.run(request).await
}
fn check_target(app: &App, node: &str) -> Result<(), Failure> {
    if node != app.node {
        return Err(failure(StatusCode::NOT_FOUND, "target is not this node"));
    }
    Ok(())
}
fn check_permission(caller: &Caller, terminals: bool) -> Result<(), Failure> {
    if if terminals {
        caller.grant.permissions.terminals
    } else {
        caller.grant.permissions.exec
    } {
        Ok(())
    } else {
        Err(failure(
            StatusCode::FORBIDDEN,
            "capability not granted by target node",
        ))
    }
}
async fn nodes(State(app): State<App>, Extension(caller): Extension<Caller>) -> Json<Value> {
    Json(
        json!({"nodes":[{"id":app.node,"name":app.node,"online":true,"os":std::env::consts::OS,
        "arch":std::env::consts::ARCH,"terminal":caller.grant.permissions.terminals,
        "terminal_sessions":caller.grant.permissions.terminals,"exec":caller.grant.permissions.exec,
        "projects":caller.grant.permissions.projects,"desktop":caller.grant.permissions.desktop && Config::read(&app.path).is_ok_and(|c| c.desktop.is_some()),
        "computer":caller.grant.permissions.computer && Config::read(&app.path).is_ok_and(|c| c.computer.is_some()),
        "browser":caller.grant.permissions.browser && Config::read(&app.path).is_ok_and(|c| c.browser.is_some()),
        "messages":caller.grant.permissions.messages,
        "activity":caller.grant.permissions.activity,
        "spaces":caller.grant.permissions.spaces,
        "tasks":caller.grant.permissions.messages,
        "executions":caller.grant.permissions.exec,
        "agent_sessions":caller.grant.permissions.messages,
        "project_copy":caller.grant.permissions.projects && caller.grant.permissions.exec,
        "project_copy_protocol":2,
        "copy_root":if caller.grant.permissions.projects && caller.grant.permissions.exec {Config::read(&app.path).ok().and_then(|c|c.copy_root)} else {None},"path":"direct"}]}),
    )
}
async fn execute(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ExecRequest>,
) -> Result<Json<Value>, Failure> {
    check_target(&app, &node)?;
    check_permission(&caller, false)?;
    request
        .validate()
        .map_err(|e| failure(StatusCode::BAD_REQUEST, &e.to_string()))?;
    let id = match headers.get("idempotency-key") {
        Some(value) => uuid::Uuid::parse_str(value.to_str().unwrap_or_default())
            .map_err(|_| failure(StatusCode::BAD_REQUEST, "idempotency key must be a UUID"))?
            .to_string(),
        None => uuid::Uuid::new_v4().to_string(),
    };
    let encoded = serde_json::to_vec(&request).unwrap();
    let mut jobs = app.jobs.lock().await;
    jobs.retain(|_, job| {
        job.finished
            .is_none_or(|at| at.elapsed() < Duration::from_secs(600))
    });
    if let Some(job) = jobs.get(&id) {
        if !job.visible(&caller) || job.request != encoded {
            return Err(failure(StatusCode::CONFLICT, "request ID already in use"));
        }
        return Ok(Json(job.view(&id, &node)));
    }
    if jobs.len() >= MAX_JOBS {
        return Err(failure(
            StatusCode::TOO_MANY_REQUESTS,
            "node job limit reached",
        ));
    }
    let permit = app.slots.clone().try_acquire_owned().map_err(|_| {
        failure(
            StatusCode::TOO_MANY_REQUESTS,
            "node execution capacity reached",
        )
    })?;
    let cancel = app.stop.child_token();
    jobs.insert(
        id.clone(),
        Job {
            caller,
            request: encoded,
            cancel: cancel.clone(),
            finished: None,
            outcome: Outcome::Running,
            code: None,
            stdout: vec![],
            stderr: vec![],
            truncated: false,
            error: None,
        },
    );
    let response = jobs[&id].view(&id, &node);
    drop(jobs);
    let task_app = app.clone();
    app.tasks.lock().await.spawn(async move {
        let _permit = permit;
        let (tx, mut rx) = mpsc::channel(32);
        let process = crate::node::execute(id.clone(), request, cancel, tx);
        let collect = async {
            while let Some(event) = rx.recv().await {
                let mut jobs = task_app.jobs.lock().await;
                let Some(job) = jobs.get_mut(&id) else {
                    break;
                };
                match event {
                    Wire::Output { stream, data, .. } => {
                        if let Ok(bytes) = STANDARD.decode(data) {
                            let left =
                                MAX_OUTPUT.saturating_sub(job.stdout.len() + job.stderr.len());
                            job.truncated |= bytes.len() > left;
                            let output = match stream {
                                Stream::Stdout => &mut job.stdout,
                                Stream::Stderr => &mut job.stderr,
                            };
                            output.extend_from_slice(&bytes[..left.min(bytes.len())]);
                        }
                    }
                    Wire::Finished {
                        exit_code,
                        outcome,
                        error,
                        ..
                    } => {
                        job.code = exit_code;
                        job.outcome = outcome;
                        job.error = error;
                        job.finished = Some(Instant::now());
                    }
                    _ => {}
                }
            }
        };
        tokio::join!(process, collect);
    });
    Ok(Json(response))
}
async fn job(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Result<Json<Value>, Failure> {
    check_permission(&caller, false)?;
    let jobs = app.jobs.lock().await;
    let job = jobs
        .get(&id)
        .filter(|job| job.visible(&caller))
        .ok_or_else(|| failure(StatusCode::NOT_FOUND, "job unavailable"))?;
    Ok(Json(job.view(&id, &app.node)))
}
async fn cancel(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Result<Json<Value>, Failure> {
    check_permission(&caller, false)?;
    let jobs = app.jobs.lock().await;
    let job = jobs
        .get(&id)
        .filter(|job| job.visible(&caller))
        .ok_or_else(|| failure(StatusCode::NOT_FOUND, "job unavailable"))?;
    if job.finished.is_none() {
        job.cancel.cancel();
    }
    Ok(Json(job.view(&id, &app.node)))
}
async fn sessions(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    Json(request): Json<Manage>,
) -> Result<Json<Value>, Failure> {
    check_target(&app, &node)?;
    check_permission(&caller, true)?;
    app.engine
        .manage(request)
        .await
        .map(Json)
        .map_err(|e| failure(StatusCode::BAD_REQUEST, &e.to_string()))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Open {
    session: String,
    #[serde(default = "columns")]
    cols: u16,
    #[serde(default = "rows")]
    rows: u16,
    #[serde(default)]
    takeover: bool,
}
fn columns() -> u16 {
    80
}
fn rows() -> u16 {
    24
}
async fn attach(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    Query(open): Query<Open>,
    upgrade: WebSocketUpgrade,
) -> Result<Response<Body>, Failure> {
    check_target(&app, &node)?;
    check_permission(&caller, true)?;
    uuid::Uuid::parse_str(&open.session)
        .map_err(|_| failure(StatusCode::BAD_REQUEST, "invalid session ID"))?;
    agent_workspace_pty::validate_size(open.cols, open.rows)
        .map_err(|e| failure(StatusCode::BAD_REQUEST, &e.to_string()))?;
    let permit = app.streams.clone().try_acquire_owned().map_err(|_| {
        failure(
            StatusCode::TOO_MANY_REQUESTS,
            "terminal attachment limit reached",
        )
    })?;
    let pty = app
        .engine
        .attach(&open.session, open.cols, open.rows, open.takeover)
        .await
        .map_err(|e| failure(StatusCode::CONFLICT, &e.to_string()))?;
    Ok(upgrade
        .max_message_size(MAX_FRAME)
        .max_frame_size(MAX_FRAME)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            terminal_stream(socket, pty, app, caller).await;
        }))
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum TerminalMessage {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
    Detach {},
}
async fn send(socket: &mut WebSocket, value: Value) -> Result<()> {
    tokio::time::timeout(
        Duration::from_secs(5),
        socket.send(Message::Text(value.to_string().into())),
    )
    .await??;
    Ok(())
}
async fn terminal_stream(mut socket: WebSocket, mut pty: Attachment, app: App, caller: Caller) {
    let result: Result<()> = async {
        // Retain only a bounded replay. The owning Engine retains the process on detach.
        for data in pty.take_replay() {
            send(&mut socket, json!({"type":"output","data":STANDARD.encode(data)})).await?;
        }
        let mut check = tokio::time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                biased;
                _ = app.connections.cancelled() => break,
                _ = check.tick() => if !app.permitted(&caller, true) { break; },
                incoming = socket.recv() => match incoming {
                    Some(Ok(Message::Text(text))) => match serde_json::from_str::<TerminalMessage>(&text)? {
                        TerminalMessage::Input { data } => { anyhow::ensure!(app.permitted(&caller, true), "grant revoked"); pty.write(STANDARD.decode(data)?)?; },
                        TerminalMessage::Resize { cols, rows } => { anyhow::ensure!(app.permitted(&caller, true), "grant revoked"); pty.resize(cols, rows)?; },
                        TerminalMessage::Detach {} => break,
                    },
                    Some(Ok(Message::Ping(data))) => { tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Pong(data))).await??; },
                    Some(Ok(Message::Pong(_))) => {},
                    _ => break,
                },
                output = pty.recv() => match output {
                    Some(Event::Output(data)) => send(&mut socket, json!({"type":"output","data":STANDARD.encode(data)})).await?,
                    Some(Event::Exit(code)) => { send(&mut socket, json!({"type":"exit","code":code})).await?; break; },
                    None => break,
                },
            }
        }
        Ok(())
    }.await;
    let _ = result;
    drop(pty);
    let _ = tokio::time::timeout(Duration::from_secs(1), socket.send(Message::Close(None))).await;
}

pub struct Running {
    app: App,
    listener: Option<Transport>,
    listener_error: Option<String>,
    monitor: Option<tokio::task::JoinHandle<()>>,
}
struct Transport {
    configuration: Listener,
    address: SocketAddr,
    stop: CancellationToken,
    task: tokio::task::JoinHandle<Result<()>>,
}
/// A provider gets a separate transport, while grants, jobs and shells stay device-owned.
#[derive(Clone)]
pub(crate) struct NetworkEndpoint(App);
pub(crate) struct NetworkListener(Option<Transport>);
impl NetworkEndpoint {
    pub(crate) async fn listen(&self, address: SocketAddr) -> Result<NetworkListener> {
        anyhow::ensure!(!self.0.stop.is_cancelled(), "local node is stopping");
        let listener = Listener {
            address,
            allow_insecure: !address.ip().is_loopback(),
        };
        Ok(NetworkListener(Some(transport(
            &self.0,
            bind(&listener).await?,
            listener,
            true,
        ))))
    }
}
impl NetworkListener {
    pub(crate) fn address(&self) -> SocketAddr {
        self.0.as_ref().unwrap().address
    }
    pub(crate) async fn shutdown(mut self) {
        if let Some(transport) = self.0.take() {
            transport.shutdown().await;
        }
    }
}
impl Drop for NetworkListener {
    fn drop(&mut self) {
        if let Some(transport) = &self.0 {
            transport.stop.cancel();
        }
    }
}
impl Transport {
    async fn shutdown(self) {
        self.stop.cancel();
        if let Err(error) = self.task.await {
            eprintln!("Direct listener shutdown: {error}");
        }
    }
}
impl Running {
    pub(crate) fn computer_owner(
        &self,
        command: agent_workspace_runtime::peers::computer::OwnerCommand,
    ) -> impl std::future::Future<Output = Result<Value>> + Send + use<> {
        let app = self.app.clone();
        async move { computer::owner(&app, command).await }
    }
    pub(crate) fn network_endpoint(&self) -> NetworkEndpoint {
        NetworkEndpoint(self.app.clone())
    }
    pub(crate) async fn desktop(&self) -> Result<agent_workspace_runtime::peers::desktop::Stream> {
        desktop::open(&self.app, None, false).await
    }
    pub(crate) async fn relay_desktop(
        &self,
        caller: &str,
    ) -> Result<agent_workspace_runtime::peers::desktop::Stream> {
        anyhow::ensure!(valid_node_id(caller), "invalid desktop caller");
        let grant = Config::read(&self.app.path)?
            .grants
            .get(caller)
            .filter(|grant| grant.permissions.desktop)
            .cloned()
            .context("desktop capability not granted by target node")?;
        desktop::open(
            &self.app,
            Some(Caller {
                node: caller.into(),
                grant,
            }),
            false,
        )
        .await
    }
    pub async fn wait(mut self) -> Result<()> {
        let result = if let Some(transport) = self.listener.take() {
            let result = transport.task.await;
            transport.stop.cancel();
            result.map_err(anyhow::Error::from).and_then(|r| r)
        } else {
            self.app.stop.cancelled().await;
            Ok(())
        };
        self.app.stop.cancel();
        if let Some(monitor) = self.monitor.take() {
            monitor.await?;
        }
        result
    }
    pub async fn shutdown(mut self) -> Result<()> {
        self.app.stop.cancel();
        if let Some(listener) = self.listener.take() {
            listener.shutdown().await;
        }
        if let Some(monitor) = self.monitor.take() {
            monitor.await?;
        }
        Ok(())
    }
    pub(crate) fn status(&self, configured: &Value) -> Value {
        let active = self
            .listener
            .as_ref()
            .is_some_and(|l| !l.task.is_finished() && !l.stop.is_cancelled());
        let applied = match &self.listener {
            Some(l) => {
                active && serde_json::to_value(&l.configuration).ok().as_ref() == Some(configured)
            }
            None => configured.is_null(),
        };
        json!({"active":active,"address":self.listener.as_ref().filter(|_| active).map(|l| l.address),"applied":applied,"error":self.listener_error})
    }
    /// Reserve the new address and persist it before releasing a working listener.
    /// The device owns jobs and shells; changing this transport does not end them.
    pub(crate) async fn configure(&mut self, listener: Option<Listener>) -> Result<()> {
        anyhow::ensure!(!self.app.stop.is_cancelled(), "local node is stopping");
        let reuse = self.listener.as_ref().is_some_and(|current| {
            !current.task.is_finished()
                && !current.stop.is_cancelled()
                && listener
                    .as_ref()
                    .is_some_and(|next| next.address == current.configuration.address)
        });
        let tcp = if let Some(listener) = &listener {
            listener.validate()?;
            if reuse {
                None
            } else {
                Some(bind(listener).await?)
            }
        } else {
            None
        };
        let path = self.app.path.clone();
        let node = self.app.node.clone();
        let next = listener.clone();
        tokio::task::spawn_blocking(move || {
            Config::update(&path, |config| {
                anyhow::ensure!(config.node_id == node, "local node identity changed");
                config.listener = next;
                Ok(())
            })
        })
        .await??;
        if reuse {
            self.listener.as_mut().unwrap().configuration = listener.unwrap();
        } else {
            let next = tcp
                .zip(listener)
                .map(|(tcp, config)| transport(&self.app, tcp, config, false));
            if let Some(previous) = std::mem::replace(&mut self.listener, next) {
                previous.shutdown().await;
            }
        }
        self.listener_error = None;
        Ok(())
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.app.stop.cancel();
    }
}

pub async fn start(
    path: PathBuf,
    engine: Engine,
    slots: Arc<Semaphore>,
    stop: CancellationToken,
) -> Result<Option<Running>> {
    if !path.try_exists()? || Config::read(&path)?.listener.is_none() {
        return Ok(None);
    }
    let catalog =
        agent_workspace_runtime::projects::Store::new(path.with_file_name("node-projects.json"));
    let running = start_with_catalog(path, engine, slots, stop, catalog).await?;
    if let Some(error) = &running.listener_error {
        anyhow::bail!("{error}");
    }
    Ok(Some(running))
}

pub(crate) async fn start_with_catalog(
    path: PathBuf,
    engine: Engine,
    slots: Arc<Semaphore>,
    stop: CancellationToken,
    catalog: agent_workspace_runtime::projects::Store,
) -> Result<Running> {
    let configuration = Config::read(&path)?;
    let (tcp, listener_error) = if let Some(listener) = &configuration.listener {
        match bind(listener).await {
            Ok(tcp) => (Some(tcp), None),
            Err(error) => (None, Some(format!("{error:#}"))),
        }
    } else {
        (None, None)
    };
    let stop = stop.child_token();
    let app = App {
        path,
        node: configuration.node_id,
        engine,
        catalog,
        jobs: Arc::default(),
        slots,
        streams: Arc::new(Semaphore::new(4)),
        desktops: Arc::new(Semaphore::new(1)),
        browsers: Arc::new(Semaphore::new(1)),
        computer: Arc::default(),
        computer_actions: Arc::default(),
        connections: stop.child_token(),
        stop,
        tasks: Arc::new(Mutex::new(tokio::task::JoinSet::new())),
    };
    let listener = tcp
        .zip(configuration.listener)
        .map(|(tcp, config)| transport(&app, tcp, config, false));
    let monitor_app = app.clone();
    let monitor = tokio::spawn(monitor(monitor_app));
    Ok(Running {
        app,
        listener,
        listener_error,
        monitor: Some(monitor),
    })
}
async fn bind(listener: &Listener) -> Result<tokio::net::TcpListener> {
    listener.validate()?;
    tokio::net::TcpListener::bind(listener.address)
        .await
        .context("bind direct node listener")
}
async fn network_identity(State(app): State<App>, headers: HeaderMap) -> Response {
    if app.connections.is_cancelled()
        || headers.contains_key("origin")
        || headers.contains_key("sec-fetch-site")
    {
        return failure(StatusCode::FORBIDDEN, "network discovery only").into_response();
    }
    Json(json!({"version":1,"node_id":app.node})).into_response()
}
fn transport(
    app: &App,
    tcp: tokio::net::TcpListener,
    configuration: Listener,
    network_discovery: bool,
) -> Transport {
    let mut app = app.clone();
    app.connections = app.stop.child_token();
    let address = tcp.local_addr().expect("bound TCP listener address");
    eprintln!("Direct node {} listening on {}", app.node, address);
    let router = Router::new()
        .route("/nodes", get(nodes))
        .route("/nodes/{node}/exec", post(execute))
        .route("/jobs/{id}", get(job))
        .route("/jobs/{id}/cancel", post(cancel))
        .route("/nodes/{node}/terminal/sessions", post(sessions))
        .route("/nodes/{node}/terminal/ws", get(attach))
        .route("/nodes/{node}/desktop/ws", get(desktop::upgrade))
        .route("/nodes/{node}/computer", post(computer::manage))
        .route("/nodes/{node}/browser", post(browser::manage))
        .route("/nodes/{node}/browser/ws", get(browser::upgrade))
        .route("/nodes/{node}/projects", post(projects::manage))
        .route("/nodes/{node}/projects/sync", post(projects::sync))
        .route("/nodes/{node}/messages", post(messages::receive))
        .route("/nodes/{node}/activity", get(activity::read))
        .route("/nodes/{node}/messages/{id}", get(messages::receipt))
        .route("/nodes/{node}/tasks", post(tasks::receive))
        .route("/nodes/{node}/tasks/{id}/{dispatch}", get(tasks::receipt))
        .route("/nodes/{node}/agent-sessions", get(agent_sessions::list))
        .route(
            "/nodes/{node}/executions",
            post(executions::manage).layer(DefaultBodyLimit::max(128 * 1024)),
        )
        .route("/nodes/{node}/spaces", get(spaces::list))
        .route("/nodes/{node}/projects/export", post(copies::export))
        .route(
            "/nodes/{node}/projects/import",
            post(copies::import).layer(DefaultBodyLimit::max(
                agent_workspace_runtime::projects::copy::MAX_WIRE,
            )),
        )
        .route("/nodes/{node}/projects/copy-status", post(copies::status))
        .route(
            "/nodes/{node}/projects/transfer",
            post(copies::transfer).layer(DefaultBodyLimit::max(
                agent_workspace_runtime::projects::copy::MAX_WIRE,
            )),
        )
        .route_layer(middleware::from_fn_with_state(app.clone(), auth));
    let mut router = Router::new()
        .nest("/api/v1/mesh", router)
        .layer(DefaultBodyLimit::max(MAX_FRAME));
    // Identity alone is public inside the provider network; every capability remains authenticated.
    if network_discovery {
        router = router.route("/api/v1/mesh/identity", get(network_identity));
    }
    let router = router.with_state(app.clone());
    let service_stop = app.connections.clone();
    let task = tokio::spawn(async move {
        let result = axum::serve(tcp, router)
            .with_graceful_shutdown(app.connections.clone().cancelled_owned())
            .await;
        app.connections.cancel();
        result.context("direct node listener")
    });
    Transport {
        configuration,
        address,
        stop: service_stop,
        task,
    }
}
async fn monitor(app: App) {
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! { _ = app.stop.cancelled() => break, _ = interval.tick() => {} }
        let grants = Config::read(&app.path)
            .ok()
            .filter(|config| config.node_id == app.node);
        computer::prune(&app, grants.as_ref()).await;
        let jobs = app.jobs.lock().await;
        for job in jobs.values().filter(|job| job.finished.is_none()) {
            if !grants
                .as_ref()
                .and_then(|config| config.grants.get(&job.caller.node))
                .is_some_and(|grant| {
                    grant.permissions.exec && config::matches(&grant.token, &job.caller.grant.token)
                })
            {
                job.cancel.cancel();
            }
        }
        drop(jobs);
        let mut tasks = app.tasks.lock().await;
        while tasks.try_join_next().is_some() {}
    }
    for job in app.jobs.lock().await.values() {
        job.cancel.cancel();
    }
    *app.computer.lock().await = None;
    let mut tasks = app.tasks.lock().await;
    if tokio::time::timeout(Duration::from_secs(6), async {
        while tasks.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
}
