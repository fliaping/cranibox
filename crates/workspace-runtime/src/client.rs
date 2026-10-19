use crate::{config, protocol::*};
use clap::Args;
use std::path::PathBuf;
#[derive(Args, Clone)]
pub struct ApiOptions {
    #[arg(
        long,
        env = "AGENT_WORKSPACE_RUNTIME_URL",
        default_value = "http://127.0.0.1:9090"
    )]
    pub url: String,
    #[arg(long, env = "AGENT_WORKSPACE_RUNTIME_TOKEN_FILE")]
    pub token_file: Option<PathBuf>,
    #[arg(long, default_value_os_t = config::default_credentials())]
    pub credentials: PathBuf,
    #[arg(long)]
    pub allow_insecure: bool,
}

use anyhow::Result;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Debug)]
struct HttpError {
    status: u16,
    message: String,
}
impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "API {}: {}", self.status, self.message)
    }
}
impl std::error::Error for HttpError {}

pub(crate) fn not_found(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<HttpError>()
        .is_some_and(|e| e.status == 404)
}

/// Only transport outages and explicit upstream outages may use an offline
/// catalog. Authorization, identity and protocol failures must remain failures.
/// The target asked us to slow down (HTTP 429); the same request may be retried later.
pub(crate) fn rate_limited(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<HttpError>()
        .is_some_and(|e| e.status == 429)
}
pub(crate) fn unavailable(error: &anyhow::Error) -> bool {
    error.is::<PathsUnavailable>()
        || error.is::<crate::peers::local_forward::Unavailable>()
        || error
            .downcast_ref::<reqwest::Error>()
            .is_some_and(|e| e.is_connect() || e.is_timeout())
        || error
            .downcast_ref::<HttpError>()
            .is_some_and(|e| matches!(e.status, 502..=504))
}
#[derive(Debug)]
struct PathsUnavailable;
impl std::fmt::Display for PathsUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "All configured paths are unavailable. Check the device and its network connections."
        )
    }
}
impl std::error::Error for PathsUnavailable {}

pub struct Api {
    options: ApiOptions,
    client: reqwest::Client,
    mesh: bool,
    target: Option<String>,
    endpoints: Vec<crate::peers::config::Endpoint>,
    e2e: Option<Box<E2e>>,
}
/// Local identity and the pinned target key for end-to-end encrypted direct calls.
#[derive(Clone)]
struct E2e {
    config_path: PathBuf,
    pinned: Option<crate::peers::config::Pinned>,
}
type SessionKey = (PathBuf, String, String);
static SESSIONS: std::sync::LazyLock<
    std::sync::Mutex<
        std::collections::HashMap<SessionKey, std::sync::Arc<crate::peers::e2e::ClientSession>>,
    >,
> = std::sync::LazyLock::new(Default::default);
/// Targets that answered without end-to-end support (older nodes), rechecked later.
static LEGACY: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<SessionKey, std::time::Instant>>,
> = std::sync::LazyLock::new(Default::default);
const LEGACY_RECHECK: Duration = Duration::from_secs(120);
impl Api {
    pub fn new(options: ApiOptions) -> Result<Self> {
        let url = config::check_url(&options.url, false, options.allow_insecure)?;
        anyhow::ensure!(
            url.path() == "/",
            "API URL must be an origin without a path"
        );
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()?;
        let endpoints = vec![crate::peers::config::Endpoint {
            url: options.url.clone(),
            allow_insecure: options.allow_insecure,
            local_forward_lock: None,
        }];
        Ok(Self {
            options,
            client,
            mesh: false,
            target: None,
            endpoints,
            e2e: None,
        })
    }
    pub fn mesh(options: ApiOptions) -> Result<Self> {
        let mut api = Self::new(options)?;
        api.mesh = true;
        Ok(api)
    }
    /// Direct calls name the expected target before any operation can run.
    pub fn direct(options: ApiOptions, target: String) -> Result<Self> {
        anyhow::ensure!(valid_node_id(&target), "invalid target node ID");
        let mut api = Self::mesh(options)?;
        api.target = Some(target);
        Ok(api)
    }
    /// Enable end-to-end encryption for a direct target when it supports it. A pinned
    /// target key makes encryption mandatory (no plaintext fallback).
    pub fn with_identity(
        mut self,
        config_path: PathBuf,
        pinned: Option<crate::peers::config::Pinned>,
    ) -> Self {
        if self.target.is_some() {
            self.e2e = Some(Box::new(E2e {
                config_path,
                pinned,
            }));
        }
        self
    }
    fn session_key(&self, e2e: &E2e, endpoint: &crate::peers::config::Endpoint) -> SessionKey {
        (
            e2e.config_path.clone(),
            self.target.clone().unwrap_or_default(),
            endpoint.url.clone(),
        )
    }
    /// The target's verified public identity, or None for an older target.
    async fn target_key(
        &self,
        e2e: &E2e,
        endpoint: &crate::peers::config::Endpoint,
    ) -> Result<Option<crate::peers::identity::PublicIdentity>> {
        if let Some(pinned) = &e2e.pinned {
            return Ok(Some(pinned.identity.clone()));
        }
        let key = self.session_key(e2e, endpoint);
        if LEGACY
            .lock()
            .unwrap()
            .get(&key)
            .is_some_and(|at| at.elapsed() < LEGACY_RECHECK)
        {
            return Ok(None);
        }
        let target = self.target.clone().unwrap_or_default();
        let response = self
            .client
            .get(format!(
                "{}/api/v1/mesh/keys",
                endpoint.url.trim_end_matches('/')
            ))
            .header("x-cranibox-target", &target)
            .timeout(Duration::from_secs(5))
            .send()
            .await?;
        if !response.status().is_success() {
            // Older target (no key endpoint). Only reachable while no key is pinned.
            LEGACY
                .lock()
                .unwrap()
                .insert(key, std::time::Instant::now());
            return Ok(None);
        }
        let bytes = Self::bounded(response, 4096).await?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        if value.get("signing_key").is_none() || value.get("noise_key").is_none() {
            // Not a Cranibox key response (older node or a plain HTTP service): no E2E yet.
            // Once a key is pinned this path is never taken, so it cannot be used to downgrade.
            LEGACY
                .lock()
                .unwrap()
                .insert(key, std::time::Instant::now());
            return Ok(None);
        }
        let identity: crate::peers::identity::PublicIdentity = serde_json::from_value(value)?;
        identity.verify()?;
        anyhow::ensure!(identity.node_id == target, "unexpected target identity");
        // Trust on first use: pin the key; later changes are refused until re-paired.
        let path = e2e.config_path.clone();
        let pinned = identity.clone();
        tokio::task::spawn_blocking(move || {
            crate::peers::config::Config::update(&path, |config| {
                if let Some(route) = config.routes.get_mut(&target) {
                    match &route.target_identity {
                        Some(existing) => anyhow::ensure!(
                            existing.identity.same_keys(&pinned),
                            "target key changed; verify the device and pair again"
                        ),
                        None => {
                            route.target_identity =
                                Some(crate::peers::config::Pinned::new(pinned.clone()))
                        }
                    }
                }
                Ok(())
            })
        })
        .await??;
        Ok(Some(identity))
    }
    async fn bounded(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
        let mut bytes = vec![];
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(
                bytes.len() + chunk.len() <= limit,
                "response exceeds allowed size"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
    async fn session(
        &self,
        e2e: &E2e,
        target: &crate::peers::identity::PublicIdentity,
        endpoint: &crate::peers::config::Endpoint,
    ) -> Result<std::sync::Arc<crate::peers::e2e::ClientSession>> {
        let secret = self.token(endpoint)?;
        let mut key = self.session_key(e2e, endpoint);
        // A rotated credential must open a new session rather than reuse the old one.
        key.2 = format!(
            "{}#{:x}",
            key.2,
            <sha2::Sha256 as sha2::Digest>::digest(secret.as_bytes())
        );
        if let Some(session) = SESSIONS.lock().unwrap().get(&key).filter(|s| s.fresh()) {
            return Ok(session.clone());
        }
        let path = e2e.config_path.clone();
        let local = tokio::task::spawn_blocking(move || {
            let file = crate::peers::identity::path_for(&path);
            if !file.try_exists()? {
                // Older configuration: migrate it, which creates the node key once.
                crate::peers::config::Config::update(&path, |_| Ok(()))?;
            }
            crate::peers::config::Config::identity(&path)
        })
        .await??;
        let (initiator, hello) = crate::peers::e2e::Initiator::start(&local, target, &secret)?;
        let response = self
            .client
            .post(format!(
                "{}/api/v1/mesh/nodes/{}/e2e/handshake",
                endpoint.url.trim_end_matches('/'),
                target.node_id
            ))
            .header("x-cranibox-target", &target.node_id)
            .header("content-type", "application/octet-stream")
            .timeout(Duration::from_secs(15))
            .body(hello)
            .send()
            .await?;
        let status = response.status();
        let bytes = Self::bounded(response, crate::peers::e2e::MAX_HANDSHAKE).await?;
        if !status.is_success() {
            return Err(Self::http_error(status.as_u16(), &bytes));
        }
        let session = std::sync::Arc::new(initiator.finish(&bytes)?);
        SESSIONS.lock().unwrap().insert(key, session.clone());
        Ok(session)
    }
    fn http_error(status: u16, bytes: &[u8]) -> anyhow::Error {
        HttpError {
            status,
            message: serde_json::from_slice::<Value>(bytes)
                .ok()
                .and_then(|v| v["error"].as_str().map(str::to_owned))
                .unwrap_or_else(|| "request failed".into()),
        }
        .into()
    }
    /// Header authorizing a WebSocket upgrade through an end-to-end session, if any.
    async fn stream_auth(
        &self,
        endpoint: &crate::peers::config::Endpoint,
        path: &str,
    ) -> Result<Option<(String, crate::peers::e2e::stream::Cipher)>> {
        let Some(e2e) = &self.e2e else {
            return Ok(None);
        };
        let Some(target) = self.target_key(e2e, endpoint).await? else {
            return Ok(None);
        };
        let session = self.session(e2e, &target, endpoint).await?;
        let (proof, secret) = session.stream_proof(path)?;
        Ok(Some((
            format!("{}.{proof}", session.id),
            crate::peers::e2e::stream::Cipher::caller(&secret, path),
        )))
    }
    async fn sealed(
        &self,
        (e2e, target, endpoint): (
            &E2e,
            &crate::peers::identity::PublicIdentity,
            &crate::peers::config::Endpoint,
        ),
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        limit: usize,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        let inner = crate::peers::e2e::Inner {
            method: method.as_str().to_owned(),
            path: path.replacen("/api/v1", "/api/v1/mesh", 1),
            body: match &body {
                Some(body) => serde_json::to_vec(body)?,
                None => vec![],
            },
        };
        for attempt in 0..2 {
            let session = self.session(e2e, target, endpoint).await?;
            let (nonce, frame) = session.seal_request(&inner)?;
            let mut request = self
                .client
                .post(format!(
                    "{}/api/v1/mesh/nodes/{}/e2e/call",
                    endpoint.url.trim_end_matches('/'),
                    target.node_id
                ))
                .header("x-cranibox-target", &target.node_id)
                .header(crate::peers::e2e::SESSION_HEADER, &session.id)
                .header("content-type", "application/octet-stream")
                .body(frame);
            if let Some(timeout) = timeout {
                request = request.timeout(timeout);
            } else if limit > 1024 * 1024 {
                request = request.timeout(Duration::from_secs(120));
            }
            let response = request.send().await?;
            let status = response.status().as_u16();
            if status == crate::peers::e2e::SESSION_GONE && attempt == 0 {
                // The target could not decrypt, so nothing ran: redo the handshake once.
                SESSIONS.lock().unwrap().retain(|_, s| s.id != session.id);
                continue;
            }
            let bytes = Self::bounded(response, limit + limit / 32 + 64 * 1024).await?;
            if status != 200 {
                return Err(Self::http_error(status, &bytes));
            }
            let (status, bytes) = session.open_response(nonce, &bytes)?;
            anyhow::ensure!(bytes.len() <= limit, "response exceeds allowed size");
            if !(200..300).contains(&status) {
                return Err(Self::http_error(status, &bytes));
            }
            return Ok(serde_json::from_slice(&bytes)?);
        }
        anyhow::bail!("end-to-end session could not be renewed")
    }
    fn token(&self, endpoint: &crate::peers::config::Endpoint) -> Result<String> {
        if let Some(lease) = &endpoint.local_forward_lock {
            crate::peers::local_forward::ensure_running(lease)?;
        }
        match &self.options.token_file {
            Some(path) => config::read_token(path),
            None => Ok(config::Credentials::read(&self.options.credentials)?.controller_token),
        }
    }
    pub(crate) fn with_paths(
        mut self,
        endpoints: Vec<crate::peers::config::Endpoint>,
    ) -> Result<Self> {
        anyhow::ensure!(
            !endpoints.is_empty() && endpoints.len() <= crate::peers::config::MAX_PATHS,
            "invalid node paths"
        );
        for endpoint in &endpoints {
            endpoint.validate()?;
        }
        // Preserve eager lease validation for existing single-path callers.
        if endpoints.len() == 1
            && let Some(lease) = &endpoints[0].local_forward_lock
        {
            crate::peers::local_forward::ensure_running(lease)?;
        }
        self.endpoints = endpoints;
        Ok(self)
    }
    /// Only read-only authorization probes may try another path. Actual requests
    /// and WebSocket upgrades are sent once, even if their response is lost.
    async fn select_endpoint(&self) -> Result<&crate::peers::config::Endpoint> {
        if self.endpoints.len() == 1 {
            return Ok(&self.endpoints[0]);
        }
        let target = self
            .target
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("multiple paths require a direct target"))?;
        for endpoint in &self.endpoints {
            if endpoint
                .local_forward_lock
                .as_ref()
                .is_some_and(|lease| crate::peers::local_forward::ensure_running(lease).is_err())
            {
                continue;
            }
            match self
                .request_at(
                    reqwest::Method::GET,
                    "/api/v1/nodes",
                    None,
                    4096,
                    endpoint,
                    Some(Duration::from_secs(2)),
                )
                .await
            {
                Ok(value) => {
                    Self::check_identity(&value, target)?;
                    return Ok(endpoint);
                }
                Err(error) if unavailable(&error) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(PathsUnavailable.into())
    }
    fn check_identity(value: &Value, target: &str) -> Result<()> {
        anyhow::ensure!(
            value["nodes"]
                .as_array()
                .is_some_and(|nodes| nodes.len() == 1 && nodes[0]["id"].as_str() == Some(target)),
            "unexpected target identity"
        );
        Ok(())
    }
    pub async fn probe(&self) -> Result<Value> {
        let endpoint = self.select_endpoint().await?;
        let mut value = self
            .request_at(
                reqwest::Method::GET,
                "/api/v1/nodes",
                None,
                4096,
                endpoint,
                Some(Duration::from_secs(2)),
            )
            .await?;
        if let Some(target) = &self.target {
            Self::check_identity(&value, target)?;
        }
        value["connection"] = json!({"url":endpoint.url, "fallback":endpoint.url != self.endpoints[0].url, "path_count":self.endpoints.len()});
        Ok(value)
    }
    /// Native attachment over either the gateway or an explicitly bound direct route.
    pub async fn terminal(
        &self,
        node: &str,
        session: &str,
        cols: u16,
        rows: u16,
        takeover: bool,
    ) -> Result<crate::peers::terminal::Terminal> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        anyhow::ensure!(valid_node_id(node), "invalid node ID");
        anyhow::ensure!(uuid::Uuid::parse_str(session).is_ok(), "invalid session ID");
        anyhow::ensure!(
            self.target.as_ref().is_none_or(|target| target == node),
            "terminal target mismatch"
        );
        crate::peers::terminal::Input::Resize { cols, rows }.validate()?;
        let endpoint = self.select_endpoint().await?;
        let mut url = config::check_url(&endpoint.url, false, endpoint.allow_insecure)?;
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme)
            .map_err(|_| anyhow::anyhow!("invalid terminal URL"))?;
        let prefix = if self.mesh { "/api/v1/mesh" } else { "/api/v1" };
        url.set_path(&format!("{prefix}/nodes/{node}/terminal/ws"));
        url.query_pairs_mut()
            .append_pair("session", session)
            .append_pair("cols", &cols.to_string())
            .append_pair("rows", &rows.to_string())
            .append_pair("takeover", &takeover.to_string());
        let mut request = url.as_str().into_client_request()?;
        let cipher = self
            .authorize_stream(&mut request, endpoint, url.path())
            .await?;
        if let Some(target) = &self.target {
            request
                .headers_mut()
                .insert("x-cranibox-target", target.parse()?);
        }
        crate::peers::terminal::Terminal::connect_request(request, cipher).await
    }
    /// Use a bound direct target, or a gateway with authenticated caller identity.
    pub async fn desktop(&self, node: &str) -> Result<crate::peers::desktop::Stream> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        anyhow::ensure!(
            crate::protocol::valid_node_id(node)
                && self.target.as_deref().is_none_or(|target| target == node),
            "desktop target mismatch"
        );
        let endpoint = self.select_endpoint().await?;
        let mut url = config::check_url(&endpoint.url, false, endpoint.allow_insecure)?;
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme)
            .map_err(|_| anyhow::anyhow!("invalid desktop URL"))?;
        let prefix = if self.mesh { "/api/v1/mesh" } else { "/api/v1" };
        url.set_path(&format!("{prefix}/nodes/{node}/desktop/ws"));
        let mut request = url.as_str().into_client_request()?;
        let cipher = self
            .authorize_stream(&mut request, endpoint, url.path())
            .await?;
        if let Some(target) = &self.target {
            request
                .headers_mut()
                .insert("x-cranibox-target", target.parse()?);
        }
        crate::peers::desktop::Stream::connect_request(request, cipher).await
    }
    /// Browser CDP is only available on an explicitly bound direct target.
    pub async fn browser(&self, node: &str) -> Result<crate::peers::browser::Socket> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        anyhow::ensure!(
            self.target.as_deref() == Some(node),
            "browser target mismatch"
        );
        let endpoint = self.select_endpoint().await?;
        let mut url = config::check_url(&endpoint.url, false, endpoint.allow_insecure)?;
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme)
            .map_err(|_| anyhow::anyhow!("invalid browser URL"))?;
        url.set_path(&format!("/api/v1/mesh/nodes/{node}/browser/ws"));
        let mut request = url.as_str().into_client_request()?;
        let cipher = self
            .authorize_stream(&mut request, endpoint, url.path())
            .await?;
        request
            .headers_mut()
            .insert("x-cranibox-target", node.parse()?);
        crate::peers::browser::connect(request, cipher).await
    }
    /// Session-bound proof for end-to-end targets (the bearer secret stays off the wire),
    /// or the bearer secret for gateways and older targets.
    async fn authorize_stream(
        &self,
        request: &mut tokio_tungstenite::tungstenite::handshake::client::Request,
        endpoint: &crate::peers::config::Endpoint,
        path: &str,
    ) -> Result<Option<crate::peers::e2e::stream::Cipher>> {
        if let Some((proof, cipher)) = self.stream_auth(endpoint, path).await? {
            request
                .headers_mut()
                .insert(crate::peers::e2e::STREAM_AUTH_HEADER, proof.parse()?);
            Ok(Some(cipher))
        } else {
            request.headers_mut().insert(
                "authorization",
                format!("Bearer {}", self.token(endpoint)?).parse()?,
            );
            Ok(None)
        }
    }
    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        self.request_bounded(method, path, body, 1024 * 1024).await
    }
    async fn request_bounded(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        limit: usize,
    ) -> Result<Value> {
        let endpoint = self.select_endpoint().await?;
        self.request_at(method, path, body, limit, endpoint, None)
            .await
    }
    async fn request_at(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        limit: usize,
        endpoint: &crate::peers::config::Endpoint,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        if let Some(e2e) = &self.e2e
            && let Some(target) = self.target_key(e2e, endpoint).await?
        {
            if let Some(lease) = &endpoint.local_forward_lock {
                crate::peers::local_forward::ensure_running(lease)?;
            }
            return self
                .sealed((e2e, &target, endpoint), method, path, body, limit, timeout)
                .await;
        }
        let mut request = self
            .client
            .request(
                method,
                format!(
                    "{}{}",
                    endpoint.url.trim_end_matches('/'),
                    if self.mesh {
                        path.replacen("/api/v1", "/api/v1/mesh", 1)
                    } else {
                        path.into()
                    }
                ),
            )
            .bearer_auth(self.token(endpoint)?);
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        } else if limit > 1024 * 1024 {
            request = request.timeout(Duration::from_secs(120));
        }
        if let Some(target) = &self.target {
            request = request.header("x-cranibox-target", target);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request.send().await?;
        let status = response.status();
        let value: Result<Value> = async {
            let mut bytes = vec![];
            while let Some(chunk) = response.chunk().await? {
                anyhow::ensure!(
                    bytes.len() + chunk.len() <= limit,
                    "response exceeds allowed size"
                );
                bytes.extend_from_slice(&chunk);
            }
            Ok(serde_json::from_slice(&bytes)?)
        }
        .await;
        // Preserve the HTTP status even when an error body is malformed, too
        // large or times out. A denied request must never become an outage.
        if !status.is_success() {
            return Err(HttpError {
                status: status.as_u16(),
                message: value
                    .as_ref()
                    .ok()
                    .and_then(|v| v["error"].as_str())
                    .unwrap_or("request failed")
                    .to_owned(),
            }
            .into());
        }
        value
    }
    pub async fn get(&self, path: &str) -> Result<Value> {
        self.request(reqwest::Method::GET, path, None).await
    }
    pub async fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.request(reqwest::Method::POST, path, Some(body)).await
    }
    pub(crate) async fn computer_request(&self, path: &str, body: Value) -> Result<Value> {
        let endpoint = self.select_endpoint().await?;
        self.request_at(
            reqwest::Method::POST,
            path,
            Some(body),
            crate::peers::computer::MAX_RESPONSE,
            endpoint,
            Some(Duration::from_secs(15)),
        )
        .await
    }
    /// Only the explicitly bounded code snapshot endpoints use the larger frame.
    pub(crate) async fn copy_request(&self, path: &str, body: Value) -> Result<Value> {
        self.request_bounded(
            reqwest::Method::POST,
            path,
            Some(body),
            crate::projects::copy::MAX_WIRE,
        )
        .await
    }
    pub async fn execute(&self, node: &str, request: ExecRequest, detach: bool) -> Result<Value> {
        anyhow::ensure!(valid_node_id(node), "invalid node ID");
        request.validate()?;
        let timeout = request.timeout_secs;
        let job = self
            .post(
                &format!("/api/v1/nodes/{node}/exec"),
                serde_json::to_value(request)?,
            )
            .await?;
        if detach {
            return Ok(job);
        }
        let id = job["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing job ID"))?;
        let path = job_path(id)?;
        tokio::time::timeout(Duration::from_secs(timeout + DEAD_SECS + 15), async {
            loop {
                let value = self.get(&path).await?;
                if value["outcome"] != "running" {
                    return Ok::<_, anyhow::Error>(value);
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("job {id} still unresolved; query it with 'job {id}'"))?
    }
}
pub fn job_path(id: &str) -> Result<String> {
    anyhow::ensure!(uuid::Uuid::parse_str(id).is_ok(), "invalid job ID");
    Ok(format!("/api/v1/jobs/{id}"))
}
pub fn print(value: Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("JSON value")
    );
}

fn tools() -> Value {
    let job = json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"],"additionalProperties":false});
    json!([
        {"name":"workspace_nodes","description":"List enrolled Windows, macOS and Linux nodes with connection status.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
        {"name":"workspace_exec","description":"Start a command on a connected node and return a job ID. No implicit shell. Use workspace_job to poll output and completion. Commands run with the node client's OS account privileges.",
         "inputSchema":{"type":"object","properties":{
            "node":{"type":"string"},"program":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},
            "cwd":{"type":"string"},"env":{"type":"object","additionalProperties":{"type":"string"}},
            "timeout_secs":{"type":"integer","minimum":1,"maximum":3600,"default":60}},"required":["node","program"],"additionalProperties":false}},
        {"name":"workspace_job","description":"Read a job's current stdout/stderr, exit code and outcome. Output is bounded to 64 KiB; truncated indicates loss. Jobs are in memory and expire after ten minutes.","inputSchema":job},
        {"name":"workspace_cancel","description":"Cancel a running remote command and terminate its process tree. Poll workspace_job for confirmation.","inputSchema":job},
        {"name":"workspace_projects","description":"Read or manage the authorized project catalog. Register records an existing absolute directory without copying files. Bind links an existing terminal UUID; unbind and forget keep files and shells. Direct connections require project permission, plus terminal permission for bind. A ProjectID may explicitly join working copies across nodes.",
         "inputSchema":{"type":"object","properties":{"command":{"type":"object","properties":{
            "operation":{"type":"string","enum":["list","register","bind","unbind","forget"]},
            "name":{"type":"string"},"node":{"type":["string","null"]},"path":{"type":"string"},
            "project_id":{"type":["string","null"]},"replica_id":{"type":"string"},"terminal_id":{"type":"string"}},
            "required":["operation"],"additionalProperties":false}},"required":["command"],"additionalProperties":false}}
    ])
}
async fn call(api: &Api, params: &Value) -> Result<Value> {
    let name = params["name"].as_str().unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    match name {
        "workspace_nodes" => api.get("/api/v1/nodes").await,
        "workspace_projects" => {
            let mut command: crate::projects::Command =
                serde_json::from_value(args["command"].clone())?;
            let path = if let Some(target) = &api.target {
                if let crate::projects::Command::Register { node, .. } = &mut command {
                    anyhow::ensure!(
                        node.as_ref().is_none_or(|node| node == target),
                        "project target mismatch"
                    );
                    *node = Some(target.clone());
                }
                format!("/api/v1/nodes/{target}/projects")
            } else {
                "/api/v1/projects".into()
            };
            api.post(&path, serde_json::to_value(command)?).await
        }
        "workspace_exec" => {
            let mut args = args
                .as_object()
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("arguments must be an object"))?;
            let node = args
                .remove("node")
                .and_then(|v| v.as_str().map(str::to_owned))
                .ok_or_else(|| anyhow::anyhow!("node required"))?;
            api.execute(&node, serde_json::from_value(Value::Object(args))?, true)
                .await
        }
        "workspace_job" | "workspace_cancel" => {
            let path = job_path(args["id"].as_str().unwrap_or_default())?;
            if name == "workspace_cancel" {
                api.post(&format!("{path}/cancel"), json!({})).await
            } else {
                api.get(&path).await
            }
        }
        _ => anyhow::bail!("unknown tool"),
    }
}
pub async fn mcp(api: Api) -> Result<()> {
    mcp_service(McpBackend::Workspace(api)).await
}
/// Local device messaging uses no Workspace administrator or relay credential.
pub async fn messages_mcp(store: crate::peers::Store) -> Result<()> {
    mcp_service(McpBackend::Messages(store)).await
}
/// Explicit process execution is opt-in; the message MCP keeps its existing tools.
pub async fn executions_mcp(store: crate::peers::Store) -> Result<()> {
    mcp_service(McpBackend::Executions(store)).await
}
pub async fn task_worker_mcp(worker: crate::peers::tasks::worker::Worker) -> Result<()> {
    mcp_service(McpBackend::TaskWorker(worker)).await
}
pub async fn computer_mcp(store: crate::peers::Store, node: String) -> Result<()> {
    mcp_service(McpBackend::Computer(crate::peers::computer::Mcp {
        store,
        node,
    }))
    .await
}
enum McpBackend {
    Workspace(Api),
    Messages(crate::peers::Store),
    Executions(crate::peers::Store),
    TaskWorker(crate::peers::tasks::worker::Worker),
    Computer(crate::peers::computer::Mcp),
}
impl McpBackend {
    fn name(&self) -> &'static str {
        match self {
            Self::Workspace(_) => "agent-workspace-nodes",
            Self::Messages(_) => "cranibox-device-messages",
            Self::Executions(_) => "cranibox-device-runs",
            Self::TaskWorker(_) => "cranibox-task-worker",
            Self::Computer(_) => "cranibox-device-computer",
        }
    }
    async fn tools(&self) -> Value {
        match self {
            Self::Workspace(_) => tools(),
            Self::Executions(_) => crate::peers::executions::mcp::tools(),
            Self::TaskWorker(_) => crate::peers::tasks::worker::tools(),
            Self::Computer(computer) => computer.tools().await,
            Self::Messages(_) => {
                let mut tools = crate::peers::messages::tools().as_array().unwrap().clone();
                tools.extend(
                    crate::peers::tasks::tools()
                        .as_array()
                        .unwrap()
                        .iter()
                        .cloned(),
                );
                tools.extend(
                    crate::peers::agent_sessions::tools()
                        .as_array()
                        .unwrap()
                        .iter()
                        .cloned(),
                );
                Value::Array(tools)
            }
        }
    }
    async fn call(&self, parameters: &Value) -> Result<Value> {
        match self {
            Self::Workspace(api) => call(api, parameters).await,
            Self::Executions(store) => store.execution_tool(parameters).await,
            Self::TaskWorker(worker) => worker.call(parameters).await,
            Self::Computer(computer) => computer.call(parameters).await,
            Self::Messages(store) => match parameters["name"].as_str() {
                Some("device_tasks") => store.task_tool(parameters).await,
                Some("device_agent_sessions") => store.agent_session_tool(parameters).await,
                _ => store.message_tool(parameters).await,
            },
        }
    }
}
async fn mcp_service(backend: McpBackend) -> Result<()> {
    let mut input = BufReader::new(tokio::io::stdin());
    let mut output = tokio::io::stdout();
    // JSON escaping can expand a valid 16 KiB prompt beyond 64 KiB. Keep the
    // larger frame bound specific to the opt-in run server; field limits apply below.
    let frame_limit = if matches!(backend, McpBackend::Executions(_)) {
        128 * 1024
    } else {
        MAX_FRAME
    };
    loop {
        // Stdio frames are newline-delimited JSON, with a bound before allocation can grow.
        let mut line = vec![];
        loop {
            let available = input.fill_buf().await?;
            if available.is_empty() {
                break;
            }
            let count = available
                .iter()
                .position(|b| *b == b'\n')
                .map_or(available.len(), |at| at + 1);
            anyhow::ensure!(
                line.len() + count <= frame_limit,
                "MCP request exceeds {} KiB",
                frame_limit / 1024
            );
            let done = available[count - 1] == b'\n';
            line.extend_from_slice(&available[..count]);
            input.consume(count);
            if done {
                break;
            }
        }
        if line.is_empty() {
            break;
        }
        let response = match serde_json::from_slice::<Value>(&line) {
            Err(_) => Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Invalid JSON"}}),
            ),
            Ok(req) if req.get("id").is_none() => None,
            Ok(req) => {
                let id = req["id"].clone();
                let result = match req["method"].as_str().unwrap_or_default() {
                    "initialize" => {
                        let requested = req["params"]["protocolVersion"]
                            .as_str()
                            .unwrap_or("2024-11-05");
                        let protocol =
                            if ["2024-11-05", "2025-03-26", "2025-06-18"].contains(&requested) {
                                requested
                            } else {
                                "2024-11-05"
                            };
                        json!({"protocolVersion":protocol,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":backend.name(),"version":env!("CARGO_PKG_VERSION")}})
                    }
                    "ping" => json!({}),
                    "tools/list" => {
                        json!({"tools":backend.tools().await})
                    }
                    "tools/call" => match backend.call(&req["params"]).await {
                        Ok(value) => {
                            if matches!(backend, McpBackend::Computer(_)) {
                                value
                            } else {
                                json!({"content":[{"type":"text","text":serde_json::to_string(&value)?}]})
                            }
                        }
                        Err(error) => {
                            json!({"isError":true,"content":[{"type":"text","text":error.to_string()}]})
                        }
                    },
                    _ => {
                        let response = json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}});
                        output.write_all(format!("{response}\n").as_bytes()).await?;
                        output.flush().await?;
                        continue;
                    }
                };
                Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
            }
        };
        if let Some(response) = response {
            output.write_all(format!("{response}\n").as_bytes()).await?;
            output.flush().await?;
        }
    }
    Ok(())
}
