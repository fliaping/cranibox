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
    shell: Option<Arc<dyn Shell>>,
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

/// Address the running device service recorded, or the default one.
pub fn recorded_address(peers: &Path) -> SocketAddr {
    std::fs::read_to_string(peers.with_file_name("ui.address"))
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or_else(|| DEFAULT_LISTEN.parse().expect("default listen address"))
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

/// Run this device: the node core, its interface, and an optional legacy gateway link.
pub async fn run(
    peers: std::path::PathBuf,
    listen: Option<SocketAddr>,
    allow_hosts: Vec<String>,
    gateway: Option<crate::settings::Settings>,
) -> Result<()> {
    let local = Arc::new(LocalNode::start(peers.clone()).await?);
    let token = access_token(&peers)?;
    let listener = match bind(listen).await {
        Ok(listener) => listener,
        Err(e) => {
            let _ = local.shutdown().await;
            return Err(e);
        }
    };
    let address = listener.local_addr()?;
    agent_workspace_runtime::config::write_private(
        &peers.with_file_name("ui.address"),
        address.to_string().as_bytes(),
        true,
    )?;
    // Service logs are often world-readable; print the secret link only to a terminal.
    if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        eprintln!(
            "Cranibox is running. Open: {}",
            sign_in_url(address, &token)
        );
    } else {
        eprintln!(
            "Cranibox device interface on http://{address} (run `cranibox-node ui-url` for the sign-in link)"
        );
    }
    let control = crate::node::Control::default();
    let stop = control.stop.clone();
    let signal = stop.clone();
    tokio::spawn(async move {
        agent_workspace_runtime::shutdown_signal().await;
        signal.cancel();
    });
    let ui = serve(
        local.clone(),
        listener,
        Options {
            token,
            allow_hosts,
            shell: None,
        },
        stop.clone(),
    );
    let result = match gateway {
        Some(settings) => {
            let link = crate::node::connect_managed(settings, control, local.clone());
            let (ui, link) = tokio::join!(ui, async {
                let result = link.await;
                stop.cancel();
                result
            });
            ui.and(link)
        }
        None => ui.await,
    };
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
    let port = address.port();
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
    services.set_local(local);
    let state = State {
        services,
        sessions,
        token: Arc::new(options.token),
        hosts: Arc::new(hosts),
        origins: Arc::new(origins),
        node,
        shell: options.shell,
    };
    let shutdown = stop.clone();
    let result = axum::serve(listener, router(state))
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await;
    let _ = closing.send(true);
    result.context("device interface server")
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
            "/web.css" => {
                return (
                    [("content-type", "text/css; charset=utf-8")],
                    super::WEB_CSS,
                )
                    .into_response();
            }
            page if is_page(page) => {
                let token = query_token(&request);
                return page_response(&state, &headers, &path, token).await;
            }
            _ => {}
        }
    }
    let Some(session) = state.sessions.session(&headers).await else {
        return error(
            StatusCode::UNAUTHORIZED,
            "Session expired. Reload this page",
        );
    };
    let csrf = header(&headers, "x-workspace-csrf");
    let expected = state.sessions.csrf(&session).await.unwrap_or_default();
    if !bool::from(expected.as_bytes().ct_eq(csrf.as_bytes())) {
        return error(
            StatusCode::UNAUTHORIZED,
            "Session expired. Reload this page",
        );
    }
    if request.method() != "POST" {
        return error(StatusCode::METHOD_NOT_ALLOWED, "POST required");
    }
    if header(&headers, "content-type") != "application/json" {
        return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "JSON required");
    }
    let limit = if path == "/api/shell" {
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
    match state.services.api(&session, &path, body).await {
        Some(Ok(value)) => Json(value).into_response(),
        Some(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        None => error(StatusCode::NOT_FOUND, "Not found"),
    }
}

async fn page_response(
    state: &State,
    headers: &HeaderMap,
    path: &str,
    token: Option<String>,
) -> Response {
    if let Some(token) = token {
        if !bool::from(token.as_bytes().ct_eq(state.token.as_bytes())) {
            return sign_in_required(StatusCode::UNAUTHORIZED);
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
                "{}={id}; HttpOnly; SameSite=Strict; Path=/",
                state.sessions.cookie
            ))
            .expect("session cookie"),
        );
        return response;
    }
    let Some(session) = state.sessions.session(headers).await else {
        return sign_in_required(StatusCode::UNAUTHORIZED);
    };
    let csrf = state.sessions.csrf(&session).await.unwrap_or_default();
    let language = language(headers);
    let config = json!({
        "csrf": csrf,
        "workspaceUrl": "",
        "desktopUrl": "",
        "language": language,
        "cursor": 0,
        "edition": "node",
        "node": state.node,
        "shell": state.shell.is_some(),
    });
    (
        [("content-type", "text/html; charset=utf-8")],
        super::page(language, &config),
    )
        .into_response()
}

fn sign_in_required(status: StatusCode) -> Response {
    let body = "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>Cranibox</title><body style=\"font:16px system-ui,'PingFang SC',sans-serif;max-width:560px;margin:15vh auto;padding:0 16px;line-height:1.6\">\
<h1 style=\"font-size:22px\">需要登录链接 · Sign-in link required</h1>\
<p>在运行 Cranibox 的设备上执行 <code>cranibox-node ui-url</code>，打开它输出的链接。</p>\
<p>Run <code>cranibox-node ui-url</code> on the device running Cranibox and open the printed link.</p></body></html>";
    (status, [("content-type", "text/html; charset=utf-8")], body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sign_in_link_session_and_browser_guards() {
        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        let local = Arc::new(LocalNode::start(peers.clone()).await.unwrap());
        let token = access_token(&peers).unwrap();
        assert_eq!(access_token(&peers).unwrap(), token, "token is stable");
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
            },
            stop.clone(),
        ));
        let base = format!("http://{address}");
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        let page = client.get(&base).send().await.unwrap();
        assert_eq!(page.status(), 401, "no session, no page");
        let wrong = client
            .get(format!("{base}/?token={}", "0".repeat(64)))
            .send()
            .await
            .unwrap();
        assert_eq!(wrong.status(), 401);
        let foreign = client
            .get(format!("{base}/healthz"))
            .header("host", "evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(foreign.status(), 403, "DNS rebinding hosts are refused");

        let login = client
            .get(sign_in_url(address, &token))
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), 303);
        assert_eq!(login.headers()["location"], "/");
        let cookie = login.headers()["set-cookie"].to_str().unwrap().to_owned();
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
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
        local.shutdown().await.unwrap();
    }
}
