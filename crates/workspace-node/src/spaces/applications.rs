//! A fixed, owned container application. Callers authenticate the browser separately.
#[cfg(test)]
mod tests;
use super::{Store, provision};
use agent_workspace_runtime::peers::config::Config;
use anyhow::{Context, Result, ensure};
use axum::{
    body::Body,
    extract::{FromRequestParts, WebSocketUpgrade, ws::Message},
    http::{HeaderMap, HeaderValue, Request, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    net::Ipv4Addr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message as Frame, handshake::derive_accept_key, protocol::Role},
};
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Application {
    Desktop,
    Code,
}
impl Application {
    pub fn name(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Code => "code",
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
struct Target {
    url: Url,
    container: String,
    owner: String,
    password: String,
}
#[derive(Clone)]
pub struct Proxy {
    store: Store,
    id: Uuid,
    application: Application,
    target: Target,
    client: reqwest::Client,
    cookies: Arc<Mutex<BTreeMap<String, String>>>,
}
impl Store {
    async fn application_target(&self, id: Uuid, application: Application) -> Result<Target> {
        let catalog = self.read()?;
        let space = catalog
            .spaces
            .iter()
            .find(|s| s.id == id)
            .context("Space not found")?;
        ensure!(
            space.phase == "ready",
            "Start the space before opening its applications"
        );
        ensure!(
            Config::read(&self.peers)?.node_id == space.owner,
            "Space owner changed"
        );
        let runner = space.provider.runner()?;
        let access = runner.access(space.provider).await?;
        let container = provision::inspect(&runner, "container", &space.container())
            .await?
            .context("Space container not found")?;
        provision::check_labels(&container["Config"]["Labels"], catalog.installation, id)?;
        ensure!(
            container["State"]["Running"] == true,
            "Space container is stopped"
        );
        let ports = provision::Ports::from_container(&container)?;
        let published = match application {
            Application::Desktop => ports.desktop,
            Application::Code => ports.code,
        }
        .context("Application port is unavailable")?;
        let (host, port) = if let Some(network) = access.network {
            let address: Ipv4Addr = container["NetworkSettings"]["Networks"][&network]["IPAddress"]
                .as_str()
                .context("Space is not on the manager network")?
                .parse()?;
            ensure!(
                address.is_private() || address.is_loopback(),
                "Invalid space network address"
            );
            (
                address.to_string(),
                match application {
                    Application::Desktop => 3001,
                    Application::Code => 8443,
                },
            )
        } else {
            ("127.0.0.1".into(), published)
        };
        let target = Target {
            url: Url::parse(&format!("https://{host}:{port}/"))?,
            container: container["Id"]
                .as_str()
                .context("Missing container identity")?
                .to_owned(),
            owner: space.owner.clone(),
            password: space.password.clone(),
        };
        ensure!(!target.container.is_empty(), "Missing container identity");
        Ok(target)
    }
    /// Only a locally managed space can become a proxy target. No address is accepted.
    pub async fn application(&self, id: Uuid, application: Application) -> Result<Proxy> {
        let target = self.application_target(id, application).await?;
        let proxy = Proxy {
            store: self.clone(),
            id,
            application,
            target,
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                // Only the inspected container's private address is ever contacted.
                .danger_accept_invalid_certs(true)
                .build()?,
            cookies: Arc::default(),
        };
        if application == Application::Code {
            let response = tokio::time::timeout(
                Duration::from_secs(10),
                proxy
                    .client
                    .post(proxy.target.url.join("login")?)
                    .basic_auth("agent", Some(&proxy.target.password))
                    .form(&[("password", &proxy.target.password)])
                    .send(),
            )
            .await??;
            proxy.remember_cookies(response.headers()).await;
            ensure!(
                response.status().is_success() || response.status().is_redirection(),
                "Editor login failed"
            );
        }
        Ok(proxy)
    }
}
impl Proxy {
    /// Cheap check used while streaming. New requests additionally inspect Docker.
    pub fn active(&self) -> bool {
        self.store.read().ok().is_some_and(|catalog| {
            catalog.spaces.iter().any(|space| {
                space.id == self.id
                    && space.phase == "ready"
                    && space.owner == self.target.owner
                    && space.password == self.target.password
            })
        }) && Config::read(&self.store.peers)
            .ok()
            .is_some_and(|config| config.node_id == self.target.owner)
    }
    async fn remember_cookies(&self, headers: &HeaderMap) {
        let mut cookies = self.cookies.lock().await;
        for value in headers.get_all("set-cookie") {
            let Ok(value) = value.to_str() else { continue };
            let Some((name, value)) = value.split(';').next().and_then(|v| v.split_once('='))
            else {
                continue;
            };
            if !name.is_empty() && name.len() < 128 && value.len() < 4096 && cookies.len() < 32 {
                cookies.insert(name.into(), value.into());
            }
        }
    }
    /// Prefix includes its trailing slash. Browser cookies/credentials never reach the app.
    pub async fn forward(
        &self,
        request: Request<Body>,
        prefix: &str,
        cancel: CancellationToken,
    ) -> Result<Response> {
        ensure!(!cancel.is_cancelled(), "Application access expired");
        ensure!(
            self.store
                .application_target(self.id, self.application)
                .await?
                == self.target,
            "Space container changed; reopen the application"
        );
        self.exchange(request, prefix, cancel).await
    }
    async fn exchange(
        &self,
        request: Request<Body>,
        prefix: &str,
        cancel: CancellationToken,
    ) -> Result<Response> {
        let relative = request
            .uri()
            .path()
            .strip_prefix(prefix)
            .context("Invalid application path")?;
        let mut target = self.target.url.clone();
        target.set_path(&format!("/{relative}"));
        target.set_query(request.uri().query());
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
        if parts.headers.contains_key("origin") {
            headers.insert(
                "origin",
                HeaderValue::from_str(&self.target.url.origin().ascii_serialization())?,
            );
        }
        let cookies = self
            .cookies
            .lock()
            .await
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ");
        if !cookies.is_empty() {
            headers.insert("cookie", HeaderValue::from_str(&cookies)?);
        }
        if websocket {
            headers.insert("connection", HeaderValue::from_static("Upgrade"));
            headers.insert("upgrade", HeaderValue::from_static("websocket"));
        }
        let send_body = !websocket && parts.method != "GET" && parts.method != "HEAD";
        let mut upstream = self
            .client
            .request(parts.method, target)
            .headers(headers)
            .basic_auth("agent", Some(&self.target.password));
        if send_body {
            upstream = upstream.body(reqwest::Body::wrap_stream(body.into_data_stream()));
        }
        let response = tokio::select! {
            _ = cancel.cancelled() => anyhow::bail!("Application access expired"),
            result = tokio::time::timeout(Duration::from_secs(30), upstream.send()) => result.context("Application timed out")?.context("Application unavailable")?,
        };
        self.remember_cookies(response.headers()).await;
        if let Some(mut upgrade) = upgrade {
            ensure!(
                response.status() == StatusCode::SWITCHING_PROTOCOLS,
                "Application rejected WebSocket"
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
                "Invalid application WebSocket handshake"
            );
            if let Some(protocol) = response.headers().get("sec-websocket-protocol") {
                let protocol = protocol.to_str()?;
                ensure!(
                    parts
                        .headers
                        .get("sec-websocket-protocol")
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|v| v.split(',').any(|v| v.trim() == protocol)),
                    "Invalid application WebSocket protocol"
                );
                upgrade = upgrade.protocols([protocol.to_owned()]);
            }
            let upstream = response.upgrade().await?;
            let proxy = self.clone();
            return Ok(upgrade.on_upgrade(move |mut browser| async move {
                let mut upstream = WebSocketStream::from_raw_socket(upstream, Role::Client, None).await;
                let mut check = tokio::time::interval(Duration::from_secs(1));
                loop {
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = check.tick() => { if !proxy.active() { break; } },
                        frame = browser.recv() => {
                            let frame = match frame {
                                Some(Ok(Message::Text(s))) => Frame::Text(s.as_str().into()),
                                Some(Ok(Message::Binary(b))) => Frame::Binary(b),
                                Some(Ok(Message::Ping(b))) => Frame::Ping(b),
                                Some(Ok(Message::Pong(b))) => Frame::Pong(b),
                                _ => break,
                            };
                            if !matches!(tokio::time::timeout(Duration::from_secs(10), upstream.send(frame)).await, Ok(Ok(()))) { break; }
                        },
                        frame = upstream.next() => {
                            let frame = match frame {
                                Some(Ok(Frame::Text(s))) => Message::Text(s.as_str().into()),
                                Some(Ok(Frame::Binary(b))) => Message::Binary(b),
                                Some(Ok(Frame::Ping(b))) => Message::Ping(b),
                                Some(Ok(Frame::Pong(b))) => Message::Pong(b),
                                Some(Ok(Frame::Frame(_))) => continue,
                                _ => break,
                            };
                            if !matches!(tokio::time::timeout(Duration::from_secs(10), browser.send(frame)).await, Ok(Ok(()))) { break; }
                        },
                    }
                }
                let _ = tokio::time::timeout(Duration::from_secs(1), browser.send(Message::Close(None))).await;
                let _ = tokio::time::timeout(Duration::from_secs(1), upstream.close(None)).await;
            }).into_response());
        }
        let mut output = Response::builder().status(response.status());
        for (name, value) in response.headers() {
            if blocked_response(name.as_str(), response.headers()) {
                continue;
            }
            if name == "location" {
                let mut location = self.target.url.join(value.to_str()?)?;
                if location.origin() == self.target.url.origin() {
                    let path = format!("{prefix}{}", location.path().trim_start_matches('/'));
                    location.set_path(&path);
                    let local = &location[url::Position::BeforePath..];
                    output = output.header(name, local);
                } else {
                    output = output.header(name, value);
                }
            } else {
                output = output.header(name, value);
            }
        }
        let proxy = self.clone();
        let mut stream = response.bytes_stream();
        let mut check = tokio::time::interval(Duration::from_secs(1));
        let stream = futures_util::stream::poll_fn(move |cx| {
            if cancel.is_cancelled() {
                return std::task::Poll::Ready(None);
            }
            if check.poll_tick(cx).is_ready() && !proxy.active() {
                return std::task::Poll::Ready(None);
            }
            stream.poll_next_unpin(cx)
        });
        Ok(output.body(Body::from_stream(stream))?)
    }
}
fn connection_header(name: &str, headers: &HeaderMap) -> bool {
    headers
        .get_all("connection")
        .iter()
        .filter_map(|h| h.to_str().ok())
        .any(|v| v.split(',').any(|v| v.trim().eq_ignore_ascii_case(name)))
}
fn hop(name: &str, headers: &HeaderMap) -> bool {
    matches!(
        name,
        "connection"
            | "upgrade"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
    ) || connection_header(name, headers)
}
fn blocked_request(name: &str, headers: &HeaderMap) -> bool {
    hop(name, headers)
        || matches!(
            name,
            "host"
                | "authorization"
                | "cookie"
                | "referer"
                | "origin"
                | "forwarded"
                | "content-length"
                | "sec-websocket-extensions"
        )
        || name.starts_with("x-forwarded-")
        || name.starts_with("sec-fetch-")
}
fn blocked_response(name: &str, headers: &HeaderMap) -> bool {
    hop(name, headers)
        || matches!(
            name,
            "set-cookie"
                | "www-authenticate"
                | "access-control-allow-origin"
                | "access-control-allow-credentials"
                | "clear-site-data"
        )
}

/// Session-bound application leases. A browser receives only an opaque, scoped cookie.
#[derive(Clone, Default)]
pub struct Hub {
    entries: Arc<Mutex<HashMap<String, Entry>>>,
    stop: CancellationToken,
}
struct Entry {
    owner: String,
    prefix: String,
    proxy: Proxy,
    expires: Instant,
    claimed: bool,
    cancel: CancellationToken,
}
pub struct Access {
    pub owner: String,
    pub proxy: Proxy,
    pub cancel: CancellationToken,
}
pub struct Invitation {
    pub prefix: String,
    pub ticket: String,
}
impl Hub {
    pub async fn issue(
        &self,
        store: &Store,
        owner: String,
        id: Uuid,
        application: Application,
    ) -> Result<Invitation> {
        ensure!(!self.stop.is_cancelled(), "Application gateway stopped");
        let proxy = store.application(id, application).await?;
        let mut entries = self.entries.lock().await;
        entries.retain(|_, entry| {
            let keep = entry.expires > Instant::now() && entry.proxy.active();
            if !keep {
                entry.cancel.cancel();
            }
            keep
        });
        ensure!(entries.len() < 64, "Too many open applications");
        let ticket = agent_workspace_runtime::config::token();
        let prefix = format!("/cranibox-app-{id}-{}/", application.name());
        entries.insert(
            ticket.clone(),
            Entry {
                owner,
                prefix: prefix.clone(),
                proxy,
                expires: Instant::now() + Duration::from_secs(30),
                claimed: false,
                cancel: self.stop.child_token(),
            },
        );
        Ok(Invitation { prefix, ticket })
    }
    pub async fn claim(&self, ticket: &str, prefix: &str) -> Result<(String, Access)> {
        let mut entries = self.entries.lock().await;
        let entry = entries
            .get(ticket)
            .context("Application link expired; reopen it from Spaces")?;
        ensure!(
            !entry.claimed
                && entry.prefix == prefix
                && entry.expires > Instant::now()
                && entry.proxy.active(),
            "Application link expired; reopen it from Spaces"
        );
        let mut entry = entries.remove(ticket).unwrap();
        let cookie = agent_workspace_runtime::config::token();
        entry.claimed = true;
        entry.expires = Instant::now() + Duration::from_secs(12 * 3600);
        let access = Access {
            owner: entry.owner.clone(),
            proxy: entry.proxy.clone(),
            cancel: entry.cancel.clone(),
        };
        entries.insert(cookie.clone(), entry);
        Ok((cookie, access))
    }
    pub async fn access(&self, cookie: &str, prefix: &str) -> Result<Access> {
        let entries = self.entries.lock().await;
        let entry = entries
            .get(cookie)
            .context("Application session expired; reopen it from Spaces")?;
        ensure!(
            entry.claimed
                && entry.prefix == prefix
                && entry.expires > Instant::now()
                && entry.proxy.active(),
            "Application session expired; reopen it from Spaces"
        );
        Ok(Access {
            owner: entry.owner.clone(),
            proxy: entry.proxy.clone(),
            cancel: entry.cancel.clone(),
        })
    }
    pub async fn retain(&self, owners: &std::collections::HashSet<String>) {
        self.entries.lock().await.retain(|_, entry| {
            let keep = owners.contains(&entry.owner)
                && entry.expires > Instant::now()
                && entry.proxy.active();
            if !keep {
                entry.cancel.cancel();
            }
            keep
        });
    }
    pub fn shutdown(&self) {
        self.stop.cancel();
    }
}
/// All application paths have one strict segment; unrelated HTTP routes never enter here.
pub fn prefix(path: &str) -> Option<&str> {
    let segment = path.strip_prefix("/cranibox-app-")?.split('/').next()?;
    let (id, application) = segment.rsplit_once('-')?;
    if Uuid::parse_str(id).is_err() || !matches!(application, "desktop" | "code") {
        return None;
    }
    let end = 1 + "cranibox-app-".len() + segment.len();
    (path.as_bytes().get(end) == Some(&b'/')).then(|| &path[..end + 1])
}
pub fn cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|s| s.strip_prefix("cranibox_application="))
}
pub fn claimed_response(prefix: &str, cookie: &str, secure: bool) -> Response {
    let mut response = (StatusCode::SEE_OTHER, [("location", prefix)]).into_response();
    let cookie = format!(
        "cranibox_application={cookie}; Path={prefix}; HttpOnly; SameSite=Strict{}",
        if secure { "; Secure" } else { "" }
    );
    response
        .headers_mut()
        .insert("set-cookie", HeaderValue::from_str(&cookie).unwrap());
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response
}

/// Native clients use a separate ephemeral loopback origin, owned by the process.
pub struct Gateway {
    hub: Hub,
    origin: String,
}
impl Drop for Gateway {
    fn drop(&mut self) {
        self.hub.shutdown();
    }
}
impl Gateway {
    pub async fn start() -> Result<Self> {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let hub = Hub::default();
        let router = axum::Router::new()
            .fallback(axum::routing::any(native_request))
            .with_state((hub.clone(), origin.clone()));
        let stop = hub.stop.clone();
        let maintenance = hub.clone();
        tokio::spawn(async move {
            let mut check = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    _ = maintenance.stop.cancelled() => break,
                    _ = check.tick() => maintenance.retain(&std::collections::HashSet::from(["native".into()])).await,
                }
            }
        });
        tokio::spawn(async move {
            let server = axum::serve(listener, router)
                .with_graceful_shutdown(stop.clone().cancelled_owned());
            tokio::select! {
                _ = stop.cancelled() => {},
                _ = server => {},
            }
        });
        Ok(Self { hub, origin })
    }
    pub async fn open(&self, store: &Store, id: Uuid, application: Application) -> Result<String> {
        self.hub
            .retain(&std::collections::HashSet::from(["native".into()]))
            .await;
        let invitation = self
            .hub
            .issue(store, "native".into(), id, application)
            .await?;
        Ok(format!(
            "{}{}?open={}",
            self.origin, invitation.prefix, invitation.ticket
        ))
    }
}
async fn native_request(
    axum::extract::State((hub, origin)): axum::extract::State<(Hub, String)>,
    request: Request<Body>,
) -> Response {
    let run = async {
        let expected = origin.strip_prefix("http://").unwrap();
        ensure!(
            request.headers().get("host").and_then(|v| v.to_str().ok()) == Some(expected),
            "Host rejected"
        );
        let prefix = prefix(request.uri().path())
            .context("Application not found")?
            .to_owned();
        if let Some(query) = request.uri().query() {
            let parameters: Vec<_> = url::form_urlencoded::parse(query.as_bytes()).collect();
            if parameters.iter().any(|(key, _)| key == "open") {
                ensure!(
                    request.method() == "GET" && parameters.len() == 1,
                    "Invalid application link"
                );
                let (cookie, _) = hub.claim(&parameters[0].1, &prefix).await?;
                return Ok(claimed_response(&prefix, &cookie, false));
            }
        }
        let request_origin = request
            .headers()
            .get("origin")
            .and_then(|v| v.to_str().ok());
        ensure!(
            request_origin.is_none_or(|v| v == origin),
            "Origin rejected"
        );
        ensure!(
            request
                .headers()
                .get("sec-fetch-site")
                .is_none_or(|v| v != "cross-site"),
            "Cross-site application request rejected"
        );
        ensure!(
            request.method() == "GET"
                || request.method() == "HEAD"
                || request_origin == Some(origin.as_str()),
            "Origin required"
        );
        let access = hub
            .access(
                cookie(request.headers()).context("Open this application from Spaces")?,
                &prefix,
            )
            .await?;
        access.proxy.forward(request, &prefix, access.cancel).await
    }
    .await;
    match run {
        Ok(response) => response,
        Err(error) => (StatusCode::FORBIDDEN, error.to_string()).into_response(),
    }
}
