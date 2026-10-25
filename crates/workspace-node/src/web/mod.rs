//! The device's own browser interface. The same routes back the CLI node, the
//! desktop window and the Workspace container; each host only supplies sessions.
pub mod app_api;
pub mod desktop;
pub mod desktop_page;
pub mod endpoint;
pub mod http_shell;
mod novnc_files;
pub mod relay;
pub mod standalone;
pub mod terminal;

use crate::local::LocalNode;
use anyhow::Result;
use async_trait::async_trait;
use axum::{
    Json,
    body::to_bytes,
    extract::Request,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::watch;

pub const WEB_JS: &str = include_str!("../../../../addons/control-center/web.js");
pub const WEB_CSS: &str = include_str!("../../../../addons/control-center/web.css");
pub const INDEX_EN: &str = include_str!("../../../../addons/control-center/index.en.html");
pub const INDEX_ZH: &str = include_str!("../../../../addons/control-center/index.zh-cn.html");

/// Browser sessions of the hosting server. Cookies and CSRF tokens stay host-owned.
#[async_trait]
pub trait SessionStore: Send + Sync + 'static {
    /// Session ID of a request that carries a valid session cookie.
    async fn session(&self, headers: &HeaderMap) -> Option<String>;
    async fn csrf(&self, session: &str) -> Option<String>;
    /// Streams stop when their owning browser session ends.
    async fn alive(&self, session: &str) -> bool;
}

#[derive(Clone)]
pub struct Services {
    pub local: Option<Arc<LocalNode>>,
    peers: PathBuf,
    sessions: Arc<dyn SessionStore>,
    desktops: desktop::Hub,
    terminals: terminal::Hub,
    closing: watch::Receiver<bool>,
}
impl Services {
    /// `peers` is the node's peer configuration, used when `local` is not running.
    pub fn new(
        local: Option<Arc<LocalNode>>,
        peers: PathBuf,
        sessions: Arc<dyn SessionStore>,
        closing: watch::Receiver<bool>,
    ) -> Self {
        Self {
            local,
            peers,
            sessions,
            desktops: desktop::Hub::default(),
            terminals: terminal::Hub::default(),
            closing,
        }
    }
    pub fn set_local(&mut self, local: Arc<LocalNode>) {
        self.peers = local.config_path().to_path_buf();
        self.local = Some(local);
    }
    fn peer_store(&self) -> agent_workspace_runtime::peers::Store {
        agent_workspace_runtime::peers::Store::new(self.peers.clone())
    }
    fn local(&self) -> Result<&Arc<LocalNode>> {
        self.local
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Local device service unavailable"))
    }

    /// Body limit of a node JSON route, or `None` when the host must handle the path.
    pub fn limit(path: &str) -> Option<usize> {
        match path {
            // Agent input allows 16 KiB after decoding. JSON escaping can expand it sixfold.
            "/api/peers" => Some(128 * 1024),
            "/api/network" => Some(32768),
            "/api/desktop/service"
            | "/api/spaces"
            | "/api/desktop/ticket"
            | "/api/desktop/cancel"
            | "/api/terminal/ticket"
            | "/api/terminal/sessions" => Some(16384),
            _ => None,
        }
    }

    /// Session- and CSRF-checked JSON operations of the device. `None`: not a node route.
    pub async fn api(&self, session: &str, path: &str, body: Value) -> Option<Result<Value>> {
        Some(match path {
            "/api/network" => match serde_json::from_value(body) {
                Ok(command) => match self.local() {
                    Ok(local) => local.manage_network(command).await,
                    Err(error) => Err(error),
                },
                Err(error) => Err(error.into()),
            },
            "/api/desktop/service" => match serde_json::from_value(body) {
                Ok(command) => match self.local() {
                    Ok(local) => local.manage_desktop_service(command).await,
                    Err(error) => Err(error),
                },
                Err(error) => Err(error.into()),
            },
            "/api/spaces" => match serde_json::from_value(body) {
                Ok(command) => {
                    crate::spaces::Store::new(self.peers.clone())
                        .manage(command, true)
                        .await
                }
                Err(error) => Err(error.into()),
            },
            "/api/peers" => match serde_json::from_value(body) {
                Ok(command) => match &self.local {
                    Some(local) => local.manage_peers(command).await,
                    None => self.peer_store().manage(command).await,
                },
                Err(error) => Err(error.into()),
            },
            "/api/desktop/ticket" => self.desktops.issue(self, session.to_owned(), body).await,
            "/api/desktop/cancel" => self.desktops.cancel(session, body).await,
            "/api/terminal/ticket" => self.terminals.issue(self, session.to_owned(), body).await,
            "/api/terminal/sessions" => terminal::manage(self, body).await,
            _ => return None,
        })
    }
}

pub fn error(status: StatusCode, message: impl ToString) -> Response {
    (status, Json(json!({"error":message.to_string()}))).into_response()
}
pub fn remote_error(status: StatusCode, cause: anyhow::Error) -> Response {
    if let Some((status, body)) = agent_workspace_runtime::client::access_refusal(&cause) {
        return (
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
            Json(body),
        )
            .into_response();
    }
    error(status, format!("{cause:#}"))
}
fn header(headers: &HeaderMap, key: &str) -> String {
    headers
        .get(key)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .into()
}

/// `POST /api/network/bootstrap`: authenticated only by a short-lived invitation, so it
/// sits outside the browser session. All other network operations keep session checks.
pub async fn bootstrap(services: &Services, request: Request) -> Response {
    if request.method() != "POST" {
        return error(StatusCode::METHOD_NOT_ALLOWED, "POST required");
    }
    if request.uri().query().is_some() {
        return error(
            StatusCode::BAD_REQUEST,
            "Do not put invitation credentials in the URL",
        );
    }
    let headers = request.headers();
    if header(headers, "content-type") != "application/json" {
        return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "JSON required");
    }
    let authorization = header(headers, "authorization");
    let Some(token) = authorization.strip_prefix("Bearer ").filter(|token| {
        token.len() == 68
            && token.starts_with("cbi_")
            && token[4..].bytes().all(|c| c.is_ascii_hexdigit())
    }) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "Invitation is invalid, expired or already used",
        );
    };
    let token = token.to_owned();
    let bytes = match tokio::time::timeout(
        Duration::from_secs(5),
        to_bytes(request.into_body(), 128),
    )
    .await
    {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return error(StatusCode::PAYLOAD_TOO_LARGE, "Request too large"),
        Err(_) => return error(StatusCode::REQUEST_TIMEOUT, "Request timed out"),
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Redemption {
        recovery: Option<String>,
    }
    let redemption = match serde_json::from_slice::<Redemption>(&bytes) {
        Ok(value)
            if value.recovery.as_ref().is_none_or(|recovery| {
                recovery.len() == 64 && recovery.bytes().all(|c| c.is_ascii_hexdigit())
            }) =>
        {
            value
        }
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "Use an empty object or a 64 character recovery secret",
            );
        }
    };
    match &services.local {
        Some(local) => match local
            .redeem_network_invitation(token, redemption.recovery)
            .await
        {
            Ok(value) => Json(value).into_response(),
            Err(_) => error(
                StatusCode::UNAUTHORIZED,
                "Invitation is invalid, expired or already used",
            ),
        },
        None => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Local device service unavailable",
        ),
    }
}

/// Fill a page template with its per-response nonce and client configuration.
/// `frames` lists the extra origins the page may frame (desktop pages).
pub fn page(language: &str, config: &Value, frames: &str) -> String {
    let template = if language == "zh-cn" {
        INDEX_ZH
    } else {
        INDEX_EN
    };
    template
        .replace(
            "__WORKSPACE_NONCE__",
            &agent_workspace_runtime::config::token(),
        )
        .replace(
            "__WORKSPACE_CONFIG__",
            &config.to_string().replace('<', "\\u003c"),
        )
        .replace(
            "__WORKSPACE_FRAMES__",
            &frames
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || " :/.*-[]".contains(*c))
                .collect::<String>(),
        )
}
