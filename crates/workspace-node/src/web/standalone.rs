//! The device's own loopback web server. The CLI node prints its sign-in link; the desktop
//! window opens the same link. A long-lived access token is exchanged once for an
//! HttpOnly session cookie, so it never stays in the address bar or browser history.
use super::{Services, SessionStore, error, header};
use crate::local::LocalNode;
use anyhow::{Context, Result};
use async_trait::async_trait;
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{Query, Request, State as ExtractState, WebSocketUpgrade},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::sync::{Mutex, watch};
use tokio_util::sync::CancellationToken;

/// Default loopback address of the CLI node's interface.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:7377";
const MAX_SESSIONS: usize = 32;

struct Entry {
    csrf: String,
    seen: Instant,
}
struct Sessions {
    cookie: String,
    map: Mutex<HashMap<String, Entry>>,
}
impl Sessions {
    fn id(&self, headers: &HeaderMap) -> Option<String> {
        let cookie = headers.get("cookie")?.to_str().ok()?;
        let prefix = format!("{}=", self.cookie);
        cookie
            .split(';')
            .map(str::trim)
            .find_map(|s| s.strip_prefix(prefix.as_str()))
            .map(str::to_owned)
    }
    async fn create(&self) -> (String, String) {
        let mut map = self.map.lock().await;
        if map.len() >= MAX_SESSIONS
            && let Some(oldest) = map
                .iter()
                .min_by_key(|(_, entry)| entry.seen)
                .map(|(id, _)| id.clone())
        {
            map.remove(&oldest);
        }
        let id = agent_workspace_runtime::config::token();
        let csrf = agent_workspace_runtime::config::token();
        map.insert(
            id.clone(),
            Entry {
                csrf: csrf.clone(),
                seen: Instant::now(),
            },
        );
        (id, csrf)
    }
}
#[async_trait]
impl SessionStore for Sessions {
    async fn session(&self, headers: &HeaderMap) -> Option<String> {
        let id = self.id(headers)?;
        let mut map = self.map.lock().await;
        let entry = map.get_mut(&id)?;
        entry.seen = Instant::now();
        Some(id)
    }
    async fn csrf(&self, session: &str) -> Option<String> {
        self.map.lock().await.get(session).map(|e| e.csrf.clone())
    }
    async fn alive(&self, session: &str) -> bool {
        self.map.lock().await.contains_key(session)
    }
}

#[derive(Clone)]
struct State {
    services: Services,
    sessions: Arc<Sessions>,
    token: Arc<String>,
    hosts: Arc<HashSet<String>>,
    origins: Arc<HashSet<String>>,
    node: String,
    peers: std::path::PathBuf,
    shell: Option<Arc<dyn Shell>>,
    /// Separate loopback origin for desktop pages, started on first use.
    gateway: Arc<tokio::sync::OnceCell<crate::spaces::applications::Gateway>>,
}

/// Read the access token beside the node configuration, creating it once (0600).
pub fn access_token(peers: &Path) -> Result<String> {
    let path = peers.with_file_name("ui.token");
    if let Ok(text) = std::fs::read_to_string(&path) {
        let token = text.trim();
        if token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(token.to_owned());
        }
    }
    let token = agent_workspace_runtime::config::token();
    agent_workspace_runtime::config::write_private(&path, token.as_bytes(), true)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(token)
}

/// The sign-in link for a server listening on `address`.
pub fn sign_in_url(address: SocketAddr, token: &str) -> String {
    let host = match address.ip() {
        std::net::IpAddr::V6(ip) if ip.is_loopback() => "[::1]".to_owned(),
        std::net::IpAddr::V4(ip) if ip.is_unspecified() || ip.is_loopback() => "127.0.0.1".into(),
        ip => ip.to_string(),
    };
    format!("http://{host}:{}/?token={token}", address.port())
}

async fn bind(listen: Option<SocketAddr>) -> Result<tokio::net::TcpListener> {
    let address = listen.unwrap_or_else(|| DEFAULT_LISTEN.parse().expect("default listen address"));
    match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => Ok(listener),
        // Several devices may share one machine (tests, isolated identities): without an
        // explicit address, take any free loopback port and record it for `ui-url`.
        Err(e) if listen.is_none() && e.kind() == std::io::ErrorKind::AddrInUse => {
            Ok(tokio::net::TcpListener::bind("127.0.0.1:0").await?)
        }
        Err(e) => Err(e).with_context(|| format!("listen on {address}")),
    }
}

/// Optional relay of this device (`serve --relay --public-url … --relay-listen …`).
#[derive(Clone, Default)]
pub struct RelayFlags {
    pub relay: bool,
    /// `https://` origin devices dial for the relay and pairing links.
    pub public_url: Option<String>,
    /// Separate public listener carrying only the relay and read-only identity; the
    /// interface itself stays on loopback. Without it the relay shares the interface
    /// listener behind a reverse proxy.
    pub listen: Option<SocketAddr>,
}

/// Run this device: the node core and its interface.
pub async fn run(
    peers: std::path::PathBuf,
    listen: Option<SocketAddr>,
    mut allow_hosts: Vec<String>,
    relay: RelayFlags,
) -> Result<()> {
    let shell = super::http_shell::HttpShell::from_env()?;
    // Register for SIGTERM before starting services: a stop request that arrives while
    // the node restores its networks must still run the cleanup below.
    let stop = CancellationToken::new();
    let signal = stop.clone();
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::spawn(async move {
        #[cfg(unix)]
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
        #[cfg(not(unix))]
        let _ = tokio::signal::ctrl_c().await;
        signal.cancel();
    });
    // The node's iroh service reads the relay settings when it starts.
    let embedded = super::relay::setup(
        relay.relay,
        false,
        relay.public_url.as_deref().unwrap_or(""),
    );
    if relay.listen.is_none()
        && let Some(host) = embedded
            .public_url()
            .and_then(|u| url::Url::parse(u).ok())
            .and_then(|u| u.host_str().map(str::to_owned))
    {
        // The interface is reached through the same proxy as the relay.
        allow_hosts.push(host);
    }
    let local = Arc::new(LocalNode::start(peers.clone()).await?);
    // Identity, roster trust and account sync run from the start, not on first use.
    if let Err(error) = local.models().await {
        eprintln!("model service: {error:#}");
    }
    let token = if shell.is_some() {
        agent_workspace_runtime::config::token()
    } else {
        access_token(&peers)?
    };
    let listener = match bind(listen).await {
        Ok(listener) => listener,
        Err(e) => {
            let _ = local.shutdown().await;
            return Err(e);
        }
    };
    let address = listener.local_addr()?;
    // Service logs are often world-readable; print the secret link only to a terminal.
    if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        eprintln!(
            "Cranibox is running. Open: {}",
            sign_in_url(address, &token)
        );
    } else {
        eprintln!(
            "Cranibox device interface on http://{address} (run `cranibox ui-url` for the sign-in link)"
        );
    }
    if let Some(address) = relay.listen.filter(|_| embedded.enabled()) {
        let public = match tokio::net::TcpListener::bind(address).await {
            Ok(listener) => listener,
            Err(e) => {
                let _ = local.shutdown().await;
                return Err(e).with_context(|| format!("relay listen on {address}"));
            }
        };
        let router = public_router(
            public_identity(&local, embedded.public_url()).await,
            local.pair_requests.clone(),
        );
        let (relay, shutdown) = (embedded.clone(), stop.clone());
        tokio::spawn(async move {
            if let Err(error) = relay
                .serve(public, router, async move { shutdown.cancelled().await })
                .await
            {
                eprintln!("relay listener: {error}");
            }
        });
        eprintln!("iroh relay listening on {address}");
    }
    embedded.spawn_self_check();
    // Remote update: let an activated version restart this process through the launcher, and
    // report this version healthy once it has stayed up.
    let runtime = crate::update::runtime_root(peers.parent().unwrap_or(&peers));
    crate::update::install_restart_hook(stop.clone());
    crate::update::watch_restart_file(runtime.clone(), stop.clone());
    if std::env::var_os("CRANIBOX_RUNTIME_MANAGED").is_some() {
        #[cfg(unix)]
        crate::update::stop_with_launcher(stop.clone());
        crate::update::confirm_when_stable(
            runtime,
            std::time::Duration::from_secs(20),
            stop.clone(),
        );
    }
    let ui = serve(
        local.clone(),
        listener,
        Options {
            token,
            allow_hosts,
            shell,
            relay: relay.listen.is_none().then(|| embedded.clone()),
        },
        stop.clone(),
    );
    let result = ui.await;
    let shutdown = local.shutdown().await;
    result.and(shutdown)
}

/// Native operations of a windowed shell (autostart, external links, window, quit).
/// Only the desktop app provides one; pages see `"shell": true` in their configuration.
#[async_trait]
pub trait Shell: Send + Sync + 'static {
    async fn call(&self, operation: &str, body: Value) -> Result<Value>;
}

pub struct Options {
    pub token: String,
    /// Extra `host[:port]` values accepted from a trusted reverse proxy.
    pub allow_hosts: Vec<String>,
    pub shell: Option<Arc<dyn Shell>>,
    /// Serve the relay paths in front of the interface (behind a reverse proxy).
    pub relay: Option<super::relay::Embedded>,
}

/// Serve this device's interface on an already bound loopback listener until `stop`.
pub async fn serve(
    local: Arc<LocalNode>,
    listener: tokio::net::TcpListener,
    options: Options,
    stop: CancellationToken,
) -> Result<()> {
    let address = listener.local_addr()?;
    anyhow::ensure!(
        address.ip().is_loopback(),
        "the device interface only listens on loopback; use an SSH tunnel or trusted proxy"
    );
    let _endpoint =
        super::endpoint::Registration::device(local.config_path(), address, &options.token)?;
    let port = address.port();
    let relay = options.relay.clone();
    let mut hosts = HashSet::new();
    let mut origins = HashSet::new();
    for host in ["127.0.0.1", "localhost", "[::1]"] {
        hosts.insert(format!("{host}:{port}"));
        origins.insert(format!("http://{host}:{port}"));
    }
    for host in options.allow_hosts {
        origins.insert(format!("https://{host}"));
        origins.insert(format!("http://{host}"));
        hosts.insert(host);
    }
    let sessions = Arc::new(Sessions {
        cookie: format!("cb_session_{port}"),
        map: Mutex::default(),
    });
    let (closing, closed) = watch::channel(false);
    let mut services = Services::new(
        None,
        local.config_path().to_path_buf(),
        sessions.clone(),
        closed,
    );
    let node = local.id.clone();
    let local_peers = local.config_path().to_path_buf();
    services.set_local(local);
    let state = State {
        services,
        sessions,
        token: Arc::new(options.token),
        hosts: Arc::new(hosts),
        origins: Arc::new(origins),
        node,
        peers: local_peers,
        shell: options.shell,
        gateway: Arc::default(),
    };
    let shutdown = stop.clone();
    let shutdown = async move { shutdown.cancelled().await };
    let result = match relay.filter(|r| r.enabled()) {
        Some(relay) => relay.serve(listener, router(state), shutdown).await,
        None => {
            axum::serve(listener, router(state))
                .with_graceful_shutdown(shutdown)
                .await
        }
    };
    let _ = closing.send(true);
    result.context("device interface server")
}

/// What anyone may learn about this device from its public relay address: enough to
/// recognise it before pairing, nothing that grants access.
async fn public_identity(local: &LocalNode, public_url: Option<&str>) -> Value {
    let identity = local
        .manage_peers(agent_workspace_runtime::peers::Command::Identity)
        .await
        .unwrap_or_default();
    json!({
        "node_id": local.id,
        "name": crate::device_info::local().name,
        "fingerprint": identity["identity"]["fingerprint"],
        "endpoint_id": identity["identity"]["public"]["signing_key"],
        "relay": true,
        "public_url": public_url,
        "protocol": 1,
    })
}
const PAIR_PAGE: &str = "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\">\
<title>Cranibox</title><body style=\"font:16px system-ui,'PingFang SC',sans-serif;max-width:560px;margin:15vh auto;padding:0 16px;line-height:1.6\">\
<h1 style=\"font-size:22px\">在 Cranibox 中打开 · Open in Cranibox</h1>\
<p>复制浏览器地址栏中的完整链接，在另一台设备的 Cranibox「添加设备」中粘贴。</p>\
<p>Copy the full link from the address bar and paste it into Add device in Cranibox on your other device.</p></body></html>";
/// The public listener: relay paths are handled in front of this router.
fn public_router(identity: Value, requests: Arc<crate::pair_url::Requests>) -> Router {
    let create = requests.clone();
    Router::new()
        .route(
            "/pair/request",
            axum::routing::post(move |body: Json<crate::pair_url::Request>| {
                let requests = create.clone();
                async move {
                    match requests.create(body.0) {
                        Ok(id) => Json(json!({"request_id": id})).into_response(),
                        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
                    }
                }
            }),
        )
        .route(
            "/pair/request/{id}",
            axum::routing::post(
                move |axum::extract::Path(id): axum::extract::Path<String>, body: Json<Value>| {
                    let requests = requests.clone();
                    async move {
                        match requests.status(&id, body["secret"].as_str().unwrap_or_default()) {
                            Ok(value) => Json(value).into_response(),
                            Err(e) => error(StatusCode::NOT_FOUND, format!("{e:#}")),
                        }
                    }
                },
            ),
        )
        .route("/healthz", get(|| async { Json(json!({"ok": true})) }))
        .route(
            "/.well-known/cranibox",
            get(move || {
                let identity = identity.clone();
                async move { Json(identity) }
            }),
        )
        .route(
            "/pair",
            get(|| async { ([("content-type", "text/html; charset=utf-8")], PAIR_PAGE) }),
        )
        .fallback(|| async { error(StatusCode::NOT_FOUND, "Not found") })
        .layer(axum::extract::DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn(public_headers))
}
async fn public_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    for (key, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "no-referrer"),
    ] {
        response
            .headers_mut()
            .insert(key, HeaderValue::from_static(value));
    }
    response
}

fn router(state: State) -> Router {
    Router::new()
        .route("/api/desktop/ws", get(desktop_ws))
        .route("/api/terminal/ws", get(terminal_ws))
        .route("/api/v1/app/{*rest}", any(app))
        .fallback(any(handle))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

async fn guard(ExtractState(state): ExtractState<State>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let origin = header(headers, "origin");
    let navigation = request.method() == "GET"
        && header(headers, "sec-fetch-mode") == "navigate"
        && header(headers, "sec-fetch-dest") == "document";
    let mut response = if !state.hosts.contains(&header(headers, "host")) {
        error(StatusCode::FORBIDDEN, "Host not allowed")
    } else if (header(headers, "sec-fetch-site") == "cross-site" && !navigation)
        || (!origin.is_empty() && !state.origins.contains(&origin))
    {
        error(StatusCode::FORBIDDEN, "Cross-origin request rejected")
    } else {
        next.run(request).await
    };
    for (key, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "no-referrer"),
    ] {
        response
            .headers_mut()
            .insert(key, HeaderValue::from_static(value));
    }
    response
}

async fn app(ExtractState(state): ExtractState<State>, request: Request) -> Response {
    super::app_api::handle(state.services, request).await
}
async fn desktop_ws(
    ExtractState(state): ExtractState<State>,
    headers: HeaderMap,
    Query(query): Query<super::desktop::Connection>,
    ws: WebSocketUpgrade,
) -> Response {
    super::desktop::upgrade(state.services, headers, query, ws).await
}
async fn terminal_ws(
    ExtractState(state): ExtractState<State>,
    headers: HeaderMap,
    Query(query): Query<super::terminal::Connection>,
    ws: WebSocketUpgrade,
) -> Response {
    super::terminal::upgrade(state.services, headers, query, ws).await
}

fn language(headers: &HeaderMap) -> &'static str {
    let accept = header(headers, "accept-language").to_ascii_lowercase();
    if accept.trim_start().starts_with("zh") {
        "zh-cn"
    } else {
        "en"
    }
}
fn query_token(request: &Request) -> Option<String> {
    url::form_urlencoded::parse(request.uri().query()?.as_bytes())
        .find(|(key, _)| key == "token")
        .map(|(_, value)| value.into_owned())
}
fn is_page(path: &str) -> bool {
    path == "/" || path == "/app" || path.starts_with("/app/")
}

async fn handle(ExtractState(state): ExtractState<State>, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    if path == "/api/network/bootstrap" {
        return super::bootstrap(&state.services, request).await;
    }
    let headers = request.headers().clone();
    if request.method() == "GET" {
        match path.as_str() {
            "/healthz" => return Json(json!({"ok":true,"node":state.node})).into_response(),
            "/web.js" => {
                return (
                    [("content-type", "text/javascript; charset=utf-8")],
                    super::WEB_JS,
                )
                    .into_response();
            }
            "/sign-in" => return sign_in_required(StatusCode::UNAUTHORIZED),
            "/web.css" => {
                return (
                    [("content-type", "text/css; charset=utf-8")],
                    super::WEB_CSS,
                )
                    .into_response();
            }
            page if is_page(page) => {
                let token = query_token(&request);
                // A terminal window signs in independently after a runtime restart.
                let destination = if url::form_urlencoded::parse(
                    request.uri().query().unwrap_or_default().as_bytes(),
                )
                .any(|(key, value)| key == "window" && value == "terminal")
                {
                    format!("{path}?window=terminal")
                } else {
                    path.clone()
                };
                return page_response(&state, &headers, &destination, token).await;
            }
            _ => {}
        }
    }
    let Some(session) = state.sessions.session(&headers).await else {
        return session_expired();
    };
    let csrf = header(&headers, "x-workspace-csrf");
    let expected = state.sessions.csrf(&session).await.unwrap_or_default();
    if !bool::from(expected.as_bytes().ct_eq(csrf.as_bytes())) {
        return session_expired();
    }
    if request.method() != "POST" {
        return error(StatusCode::METHOD_NOT_ALLOWED, "POST required");
    }
    if header(&headers, "content-type") != "application/json" {
        return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "JSON required");
    }
    let limit = if path == "/api/shell" || path == "/api/desktop/page" {
        Some(16384)
    } else {
        Services::limit(&path)
    };
    let Some(limit) = limit else {
        return error(StatusCode::NOT_FOUND, "Not found");
    };
    let bytes = match tokio::time::timeout(
        Duration::from_secs(10),
        to_bytes(request.into_body(), limit),
    )
    .await
    {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return error(StatusCode::PAYLOAD_TOO_LARGE, "Request too large"),
        Err(_) => return error(StatusCode::REQUEST_TIMEOUT, "Request timed out"),
    };
    let body = match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) if value.is_object() => value,
        Ok(_) => return error(StatusCode::BAD_REQUEST, "Object required"),
        Err(_) => return error(StatusCode::BAD_REQUEST, "Invalid JSON"),
    };
    if path == "/api/shell" {
        let Some(shell) = &state.shell else {
            return error(
                StatusCode::NOT_FOUND,
                "Not available outside the desktop app",
            );
        };
        let operation = body["operation"].as_str().unwrap_or_default().to_owned();
        return match shell.call(&operation, body).await {
            Ok(value) => Json(value).into_response(),
            Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        };
    }
    if path == "/api/desktop/page" {
        return match desktop_page(&state, &headers, body).await {
            Ok(value) => Json(value).into_response(),
            Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        };
    }
    match state.services.api(&session, &path, body).await {
        Some(Ok(value)) => Json(value).into_response(),
        Some(Err(e)) => super::remote_error(StatusCode::BAD_REQUEST, e),
        None => error(StatusCode::NOT_FOUND, "Not found"),
    }
}

/// `POST /api/desktop/page`: a one-time link to a shared desktop's own web page on
/// a separate loopback origin, so the page never runs with this console's session.
async fn desktop_page(state: &State, headers: &HeaderMap, body: Value) -> Result<Value> {
    let host = header(headers, "host");
    let loopback = url::Url::parse(&format!("http://{host}/"))
        .ok()
        .and_then(|url| {
            url.host().map(|host| match host {
                url::Host::Domain(name) => name == "localhost",
                url::Host::Ipv4(ip) => ip.is_loopback(),
                url::Host::Ipv6(ip) => ip.is_loopback(),
            })
        })
        .unwrap_or(false);
    anyhow::ensure!(
        loopback,
        "Open remote desktops from this device's own Cranibox window"
    );
    let gateway = state
        .gateway
        .get_or_try_init(crate::spaces::applications::Gateway::start)
        .await?;
    // CSP source expressions cannot name IPv6 literals.
    let mut ancestors: Vec<_> = state
        .origins
        .iter()
        .filter(|origin| !origin.contains('['))
        .cloned()
        .collect();
    ancestors.sort();
    let page = state
        .services
        .desktop_page(body, ancestors.join(" "))
        .await?;
    let protocol = page.protocol();
    Ok(json!({"url": gateway.open_desktop(page).await?, "protocol": protocol}))
}

async fn page_response(
    state: &State,
    headers: &HeaderMap,
    path: &str,
    token: Option<String>,
) -> Response {
    if let Some(token) = token {
        if !bool::from(token.as_bytes().ct_eq(state.token.as_bytes())) {
            return sign_in_redirect(path);
        }
        // Exchange the link once for a cookie and drop the token from the address bar.
        let (id, _) = state.sessions.create().await;
        let mut response = (
            StatusCode::SEE_OTHER,
            [(axum::http::header::LOCATION, path.to_owned())],
        )
            .into_response();
        response.headers_mut().insert(
            "set-cookie",
            HeaderValue::from_str(&format!(
                "{}={id}; HttpOnly; SameSite=Lax; Path=/",
                state.sessions.cookie
            ))
            .expect("session cookie"),
        );
        return response;
    }
    let Some(session) = state.sessions.session(headers).await else {
        return sign_in_redirect(path);
    };
    let csrf = state.sessions.csrf(&session).await.unwrap_or_default();
    let preferences = crate::app::preferences::read(&state.peers);
    let language = match preferences.language.as_deref() {
        Some("zh-cn") => "zh-cn",
        Some("en") => "en",
        _ => language(headers),
    };
    let config = json!({
        "csrf": csrf,
        "workspaceUrl": "",
        "desktopUrl": "",
        "language": language,
        "cursor": 0,
        "edition": "node",
        "node": state.node,
        "shell": state.shell.is_some(),
        "preferences": preferences,
    });
    (
        [("content-type", "text/html; charset=utf-8")],
        // Desktop pages come from this device's own loopback gateway.
        super::page(language, &config, "http://127.0.0.1:*"),
    )
        .into_response()
}

/// The page's session is gone; the page reloads through `/sign-in`.
fn session_expired() -> Response {
    let mut response = error(
        StatusCode::UNAUTHORIZED,
        "Session expired. Reload this page",
    );
    response.headers_mut().insert(
        super::SIGN_IN_HEADER.0,
        HeaderValue::from_static(super::SIGN_IN_HEADER.1),
    );
    response
}

/// Pages without a session land on one fixed address, so the desktop window can
/// recognise a lost session (runtime restart, dropped cookie) and sign in again.
fn sign_in_redirect(destination: &str) -> Response {
    let location = if destination.ends_with("?window=terminal") {
        "/sign-in?window=terminal"
    } else {
        "/sign-in"
    };
    (
        StatusCode::SEE_OTHER,
        [(axum::http::header::LOCATION, location)],
    )
        .into_response()
}

fn sign_in_required(status: StatusCode) -> Response {
    let body = "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>Cranibox</title><body style=\"font:16px system-ui,'PingFang SC',sans-serif;max-width:560px;margin:15vh auto;padding:0 16px;line-height:1.6\">\
<h1 style=\"font-size:22px\">需要登录链接 · Sign-in link required</h1>\
<p>在运行 Cranibox 的设备上执行 <code>cranibox ui-url</code>，打开它输出的链接。</p>\
<p>Run <code>cranibox ui-url</code> on the device running Cranibox and open the printed link.</p></body></html>";
    (status, [("content-type", "text/html; charset=utf-8")], body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn desktop_client_uses_sessions_csrf_and_terminal_sign_in() {
        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let token = "a".repeat(64);
        let sessions = Arc::new(Sessions {
            cookie: "desktop_test".into(),
            map: Mutex::default(),
        });
        let (_closing, closed) = watch::channel(false);
        let state = State {
            services: Services::new(None, peers.clone(), sessions.clone(), closed),
            sessions,
            token: Arc::new(token.clone()),
            hosts: Arc::new(HashSet::from([address.to_string()])),
            origins: Arc::new(HashSet::from([format!("http://{address}")])),
            node: "test".into(),
            peers: peers.clone(),
            shell: None,
            gateway: Arc::default(),
        };
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, router(state))
                .with_graceful_shutdown(stopped.cancelled_owned())
                .await
                .unwrap();
        });
        let sign_in = sign_in_url(address, &token);
        let interface = super::super::http_shell::LocalInterface::connect(&sign_in)
            .await
            .unwrap();
        let identity = interface
            .request("POST", "/api/peers", json!({"operation":"identity"}))
            .await
            .unwrap();
        assert!(
            identity["node_id"].is_string(),
            "Cookie and CSRF authenticate the existing API"
        );
        // This fixture has no LocalNode: reaching dispatch proves the App API accepted the session.
        let error = interface
            .request("GET", "/api/v1/app/approvals", Value::Null)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Local device service unavailable")
        );
        let _ = interface
            .request(
                "POST",
                "/api/peers",
                json!({
                    "operation":"computer_owner", "command":{"action":"emergency_stop"}
                }),
            )
            .await;
        assert!(
            crate::peer::config::Config::read(&peers)
                .unwrap()
                .computer_paused,
            "Emergency stop reaches the existing owner API and persists its fence"
        );
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let login = client
            .get(format!("{sign_in}&window=terminal"))
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::SEE_OTHER);
        assert_eq!(login.headers()["location"], "/?window=terminal");
        stop.cancel();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn sign_in_link_session_and_browser_guards() {
        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        let local = Arc::new(LocalNode::start(peers.clone()).await.unwrap());
        let token = access_token(&peers).unwrap();
        assert_eq!(access_token(&peers).unwrap(), token, "token is stable");
        // Like the GUI, this launch may use a token different from the CLI token.
        let token = agent_workspace_runtime::config::token();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let stop = CancellationToken::new();
        let server = tokio::spawn(serve(
            local.clone(),
            listener,
            Options {
                token: token.clone(),
                allow_hosts: vec![],
                shell: None,
                relay: None,
            },
            stop.clone(),
        ));
        let base = format!("http://{address}");
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        let page = client.get(&base).send().await.unwrap();
        assert_eq!(page.status(), 303, "no session, no page");
        assert_eq!(page.headers()["location"], "/sign-in");
        let wrong = client
            .get(format!("{base}/?token={}&window=terminal", "0".repeat(64)))
            .send()
            .await
            .unwrap();
        assert_eq!(wrong.headers()["location"], "/sign-in?window=terminal");
        let sign_in = client.get(format!("{base}/sign-in")).send().await.unwrap();
        assert_eq!(sign_in.status(), 401);
        assert!(sign_in.text().await.unwrap().contains("cranibox ui-url"));
        let foreign = client
            .get(format!("{base}/healthz"))
            .header("host", "evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(foreign.status(), 403, "DNS rebinding hosts are refused");

        let discovered = super::super::endpoint::ui_url(&peers, None).await.unwrap();
        assert_eq!(discovered, sign_in_url(address, &token));
        let login = client.get(discovered).send().await.unwrap();
        assert_eq!(login.status(), 303);
        assert_eq!(login.headers()["location"], "/");
        let cookie = login.headers()["set-cookie"].to_str().unwrap().to_owned();
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"));
        let cookie = cookie.split(';').next().unwrap().to_owned();

        let page = client
            .get(format!("{base}/app/devices"))
            .header("cookie", &cookie)
            .header("accept-language", "zh-CN,zh;q=0.9")
            .send()
            .await
            .unwrap();
        assert_eq!(page.status(), 200);
        let html = page.text().await.unwrap();
        assert!(html.contains("\"edition\":\"node\"") && html.contains("\"language\":\"zh-cn\""));
        assert!(
            !html.contains(&token),
            "the access token never reaches the page"
        );
        let csrf = html.split("\"csrf\":\"").nth(1).unwrap()[..64].to_owned();
        // A saved language wins over the browser's: the desktop app's origin (port)
        // changes every launch, so browser storage cannot remember it.
        crate::app::preferences::update(&peers, &serde_json::json!({"language": "en"})).unwrap();
        let html = client
            .get(format!("{base}/app/devices"))
            .header("cookie", &cookie)
            .header("accept-language", "zh-CN,zh;q=0.9")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(html.contains("\"language\":\"en\"") && html.contains("\"preferences\":{"));

        let identity = |csrf: Option<&str>, origin: Option<&str>| {
            let mut request = client
                .post(format!("{base}/api/peers"))
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(r#"{"operation":"identity"}"#);
            if let Some(csrf) = csrf {
                request = request.header("x-workspace-csrf", csrf);
            }
            if let Some(origin) = origin {
                request = request.header("origin", origin);
            }
            request.send()
        };
        assert_eq!(
            identity(None, None).await.unwrap().status(),
            401,
            "CSRF required"
        );
        assert_eq!(
            identity(Some(&csrf), Some("https://evil.example"))
                .await
                .unwrap()
                .status(),
            403,
            "cross-origin writes are refused"
        );
        let ok = identity(Some(&csrf), Some(&base)).await.unwrap();
        assert_eq!(ok.status(), 200);
        let value: Value = ok.json().await.unwrap();
        assert_eq!(value["node_id"], local.id.as_str());

        let app = client
            .post(format!("{base}/api/v1/app/approvals/none"))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(app.status(), 403, "App API writes need CSRF too");

        stop.cancel();
        server.await.unwrap().unwrap();
        assert!(!peers.with_file_name("ui.endpoint.json").exists());
        local.shutdown().await.unwrap();
    }
}
