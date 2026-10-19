//! C3 JSON management API. Only the private OS transport serves this router.
use crate::{
    catalog,
    server::{engine::Engine, error, upstream_error},
    upstream::apikey::Credential,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{delete, get, post},
};
use cranibox_contracts::{
    error::{ErrorCode, UnavailableBody},
    llm::mgmt::*,
    llm::*,
};
use serde_json::json;
#[cfg(unix)]
use std::path::Path as FsPath;
use std::{path::PathBuf, sync::Arc};
#[cfg(unix)]
use tokio_util::sync::CancellationToken;
pub mod tools;

#[derive(Clone)]
pub struct LocalMgmt {
    engine: Arc<Engine>,
}
impl LocalMgmt {
    pub(crate) fn new(engine: Arc<Engine>) -> Self {
        Self { engine }
    }
}
fn store_error(_: cranibox_contracts::vault::StoreError) -> MgmtError {
    MgmtError::Other("account store operation failed".into())
}
#[async_trait::async_trait]
impl LlmMgmt for LocalMgmt {
    async fn status(&self) -> Result<Status, MgmtError> {
        let accounts = self.engine.store.list();
        let count = self
            .engine
            .models()
            .await
            .iter()
            .filter(|m| matches!(m.availability, Availability::Local))
            .count() as u32;
        let issues = accounts
            .iter()
            .filter_map(|a| {
                let code = if !self.engine.enabled(a) {
                    Some(ErrorCode::AccountDisabledHere)
                } else {
                    match a.state {
                        AccountState::NeedsSignIn => Some(ErrorCode::NeedsSignIn),
                        AccountState::QuotaExhausted { .. } | AccountState::Error { .. } => {
                            Some(ErrorCode::AllAccountsExhausted)
                        }
                        _ => None,
                    }
                }?;
                Some(Issue {
                    code,
                    message: code.as_str().into(),
                    account_id: Some(a.id.clone()),
                })
            })
            .collect();
        Ok(Status {
            mode: if count > 0 {
                Mode::Local
            } else {
                Mode::Offline
            },
            hub_online: false,
            models_available: count,
            issues,
        })
    }
    async fn accounts(&self) -> Result<Vec<AccountSummary>, MgmtError> {
        Ok(self.engine.store.list())
    }
    async fn add_account(&self, account: NewAccount) -> Result<AccountSummary, MgmtError> {
        let id = uuid::Uuid::new_v4().to_string();
        let (provider, label, credential) = match account {
            NewAccount::ApiKey {
                preset,
                key,
                label,
                region,
            } => {
                let preset = self
                    .engine
                    .preset(&preset)
                    .ok_or_else(|| MgmtError::Invalid("unknown preset".into()))?;
                if region.as_ref().is_some_and(|r| !preset.region.contains(r)) {
                    return Err(MgmtError::Invalid(
                        "choose the preset for the requested region".into(),
                    ));
                }
                validate_key(&key)?;
                if preset.quirks.iter().any(|q| q == "minimax_plan") && key.starts_with("sk-api-") {
                    return Err(MgmtError::Invalid("这是 MiniMax 按量 key，请选择 MiniMax 按量预设；Coding Plan 使用 sk-cp- key".into()));
                }
                let label = label
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| preset.label.clone());
                (
                    preset.id,
                    label,
                    Credential {
                        access_token: key,
                        region,
                        ..Default::default()
                    },
                )
            }
            NewAccount::Custom {
                label,
                key,
                endpoints,
                models,
            } => {
                validate_key(&key)?;
                if label.trim().is_empty()
                    || label.len() > 256
                    || endpoints.is_empty()
                    || models.iter().any(|m| m.trim().is_empty() || m.len() > 512)
                {
                    return Err(MgmtError::Invalid(
                        "label, endpoints and valid model names are required".into(),
                    ));
                }
                for endpoint in endpoints.values() {
                    catalog::validate_url(endpoint).map_err(MgmtError::Invalid)?;
                }
                (
                    format!("custom-{id}"),
                    label,
                    Credential {
                        access_token: key,
                        endpoints,
                        models,
                        ..Default::default()
                    },
                )
            }
        };
        if label.len() > 256 {
            return Err(MgmtError::Invalid("label exceeds 256 bytes".into()));
        }
        let account = AccountSummary {
            id,
            provider,
            plan: None,
            label,
            kind: AccountKind::ApiKey,
            home_node: self.engine.node.clone(),
            sync: true,
            enabled_on: vec![self.engine.node.clone()],
            all_nodes: false,
            state: AccountState::Ready,
        };
        self.engine
            .store
            .put(account.clone(), Some(credential.secret()))
            .map_err(store_error)?;
        self.engine.changed(&account.id).await;
        Ok(account)
    }
    async fn delete_account(&self, id: &str) -> Result<(), MgmtError> {
        let lock = self.engine.account_lock(id).await;
        let _guard = lock.lock().await;
        if self.engine.store.get(id).is_none() {
            return Err(MgmtError::NotFound);
        }
        self.engine.store.remove(id).map_err(store_error)?;
        self.engine.forget(id).await;
        self.engine.changed(id).await;
        Ok(())
    }
    async fn enable(&self, id: &str, req: Enable) -> Result<AccountSummary, MgmtError> {
        if req.node.is_some() == req.all_nodes.is_some() || req.all_nodes == Some(false) {
            return Err(MgmtError::Invalid(
                "specify exactly one node or all_nodes=true".into(),
            ));
        }
        let lock = self.engine.account_lock(id).await;
        let _guard = lock.lock().await;
        let mut account = self.engine.store.get(id).ok_or(MgmtError::NotFound)?;
        if account.kind == AccountKind::ClaudeOAuth
            && (req.all_nodes.is_some() || req.node.as_ref() != Some(&account.home_node))
        {
            return Err(MgmtError::Invalid("Claude credentials can only be enabled on their home node; sign in again to relocate".into()));
        }
        if req.all_nodes.is_some() {
            account.all_nodes = req.on;
            if !req.on {
                account.enabled_on.clear();
            }
        } else if let Some(node) = req.node {
            if node.is_empty() {
                return Err(MgmtError::Invalid("node must not be empty".into()));
            }
            if account.all_nodes && !req.on {
                return Err(MgmtError::Invalid(
                    "disable all_nodes before disabling an individual node".into(),
                ));
            }
            account.enabled_on.retain(|n| n != &node);
            if req.on {
                account.enabled_on.push(node);
            }
        }
        self.engine
            .store
            .put(account.clone(), None)
            .map_err(store_error)?;
        self.engine.changed(id).await;
        Ok(account)
    }
    async fn signin_start(&self, req: SignInStartReq) -> Result<SignInStart, MgmtError> {
        self.engine
            .sign_in
            .as_ref()
            .ok_or_else(|| MgmtError::Other("sign-in adapter is not configured".into()))?
            .start(
                &req.provider,
                SignInOpts {
                    region: req.region,
                    label: None,
                },
            )
            .await
    }
    async fn signin_complete(
        &self,
        flow: &str,
        input: SignInInput,
    ) -> Result<AccountSummary, MgmtError> {
        let account = self
            .engine
            .sign_in
            .as_ref()
            .ok_or(MgmtError::NotFound)?
            .complete(flow, input)
            .await?;
        self.engine.changed(&account.id).await;
        Ok(account)
    }
    async fn models(&self) -> Result<Vec<ModelEntry>, MgmtError> {
        Ok(self.engine.models().await)
    }
    async fn set_claude_node(&self, req: SetClaudeNode) -> Result<(), MgmtError> {
        if req.node != self.engine.node {
            return Err(MgmtError::Unavailable(ErrorCode::ClaudeNodeOffline));
        }
        *self.engine.claude_node.lock().await = Some(req.node);
        Ok(())
    }
    async fn presets(&self) -> Result<Vec<Preset>, MgmtError> {
        Ok(self.engine.presets.clone())
    }
}
fn validate_key(key: &str) -> Result<(), MgmtError> {
    if key.trim().is_empty() || key.len() > 32768 || key.bytes().any(|c| c.is_ascii_control()) {
        Err(MgmtError::Invalid("invalid API key".into()))
    } else {
        Ok(())
    }
}
fn failure(error_value: MgmtError) -> Response {
    match error_value {
        MgmtError::NotFound => error(StatusCode::NOT_FOUND, "not_found", "not found"),
        MgmtError::Pending => {
            (StatusCode::ACCEPTED, Json(json!({"status":"pending"}))).into_response()
        }
        MgmtError::Invalid(message) => {
            error(StatusCode::BAD_REQUEST, "invalid_request_error", &message)
        }
        MgmtError::Unavailable(code) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(UnavailableBody::new(code, code.as_str(), None)),
        )
            .into_response(),
        MgmtError::Upstream(e) => upstream_error(e),
        MgmtError::Other(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "management_error",
            "management operation failed",
        ),
    }
}
fn answer<T: serde::Serialize>(result: Result<T, MgmtError>) -> Response {
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => failure(e),
    }
}
async fn status(State(m): State<LocalMgmt>) -> Response {
    answer(m.status().await)
}
async fn accounts(State(m): State<LocalMgmt>) -> Response {
    answer(m.accounts().await)
}
async fn add(State(m): State<LocalMgmt>, Json(v): Json<NewAccount>) -> Response {
    answer(m.add_account(v).await)
}
async fn remove(State(m): State<LocalMgmt>, Path(id): Path<String>) -> Response {
    match m.delete_account(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(e),
    }
}
async fn enable(
    State(m): State<LocalMgmt>,
    Path(id): Path<String>,
    Json(v): Json<Enable>,
) -> Response {
    answer(m.enable(&id, v).await)
}
async fn signin_start(State(m): State<LocalMgmt>, Json(v): Json<SignInStartReq>) -> Response {
    answer(m.signin_start(v).await)
}
async fn signin_complete(
    State(m): State<LocalMgmt>,
    Path(flow): Path<String>,
    Json(v): Json<SignInInput>,
) -> Response {
    answer(m.signin_complete(&flow, v).await)
}
async fn models(State(m): State<LocalMgmt>) -> Response {
    answer(m.models().await)
}
async fn set_node(State(m): State<LocalMgmt>, Json(v): Json<SetClaudeNode>) -> Response {
    answer(m.set_claude_node(v).await)
}
async fn presets(State(m): State<LocalMgmt>) -> Response {
    answer(m.presets().await)
}
async fn events(State(m): State<LocalMgmt>) -> Response {
    let mut rx = m.engine.events.subscribe();
    let events = async_stream::stream! {loop{match rx.recv().await{
        Ok(event)=>yield Ok::<_,std::convert::Infallible>(Event::default().json_data(event).expect("serializable event")),
        Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>yield Ok(Event::default().event("resync").data("{}")),
        Err(_)=>break,
    }}};
    Sse::new(events)
        .keep_alive(KeepAlive::default())
        .into_response()
}
fn router(mgmt: LocalMgmt) -> Router {
    Router::new()
        .route(STATUS, get(status))
        .route(ACCOUNTS, get(accounts).post(add))
        .route(ACCOUNT, delete(remove))
        .route(ACCOUNT_ENABLE, post(enable))
        .route(SIGNIN_START, post(signin_start))
        .route(SIGNIN_COMPLETE, post(signin_complete))
        .route(MODELS, get(models))
        .route(CLAUDE_SET_NODE, post(set_node))
        .route(PRESETS, get(presets))
        .route(EVENTS, get(events))
        .route("/accounts/import-curl", post(tools::import))
        .route("/accounts/{id}/test", post(tools::probe))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .with_state(mgmt)
}

#[cfg(unix)]
pub(crate) struct Listener {
    inner: tokio::net::UnixListener,
    guard: SocketGuard,
}
#[cfg(unix)]
struct SocketGuard {
    path: PathBuf,
    device: u64,
    inode: u64,
    _lock: std::fs::File,
}
#[cfg(unix)]
impl Drop for SocketGuard {
    fn drop(&mut self) {
        use std::os::unix::fs::MetadataExt;
        if let Ok(meta) = std::fs::symlink_metadata(&self.path)
            && meta.dev() == self.device
            && meta.ino() == self.inode
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
#[cfg(unix)]
impl Listener {
    pub async fn bind(path: &FsPath) -> anyhow::Result<Self> {
        use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("management socket needs a private run directory"))?;
        std::fs::create_dir_all(parent)?;
        let meta = std::fs::symlink_metadata(parent)?;
        anyhow::ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "run directory must be a real directory"
        );
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        // Serialize ownership across bind, stale-socket recovery and shutdown.
        // The OS releases this lock even after SIGKILL; a pathname alone does
        // not tell us whether a crashed service still owns a listener.
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(path.with_extension("lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)?;
        if let Ok(before) = std::fs::symlink_metadata(path) {
            anyhow::ensure!(
                before.file_type().is_socket(),
                "management path is not a socket"
            );
            match tokio::time::timeout(
                std::time::Duration::from_millis(250),
                tokio::net::UnixStream::connect(path),
            )
            .await
            {
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                    let after = std::fs::symlink_metadata(path)?;
                    anyhow::ensure!(
                        before.dev() == after.dev() && before.ino() == after.ino(),
                        "management socket changed during recovery"
                    );
                    std::fs::remove_file(path)?;
                }
                _ => anyhow::bail!("management socket is already in use"),
            }
        }
        let inner = tokio::net::UnixListener::bind(path)?;
        let meta = std::fs::symlink_metadata(path)?;
        let guard = SocketGuard {
            path: path.into(),
            device: meta.dev(),
            inode: meta.ino(),
            _lock: lock,
        };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self { inner, guard })
    }
}
#[cfg(unix)]
pub(crate) async fn serve(listener: Listener, mgmt: LocalMgmt, cancel: CancellationToken) {
    let Listener {
        inner,
        guard: _guard,
    } = listener;
    let _ = axum::serve(inner, router(mgmt))
        .with_graceful_shutdown(cancel.cancelled_owned())
        .await;
}

// Windows management uses a local named pipe. The OS listener implementation is
// separate so no TCP fallback can accidentally expose account management.
#[cfg(windows)]
#[path = "mgmt/windows.rs"]
mod windows;
#[cfg(windows)]
pub(crate) use windows::{Listener, serve};

/// Typed C3 management client. Creates an HTTP/1 connection on the private OS transport.
#[derive(Clone)]
pub struct Client {
    path: PathBuf,
}
impl Client {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: http::Method,
        path: &str,
        value: Option<serde_json::Value>,
    ) -> Result<T, MgmtError> {
        use bytes::Bytes;
        use http_body_util::Full;
        use hyper_util::rt::TokioIo;
        #[cfg(unix)]
        let io = tokio::net::UnixStream::connect(&self.path)
            .await
            .map_err(|_| MgmtError::Other("management socket is unavailable".into()))?;
        #[cfg(windows)]
        let io = windows::connect(&self.path).await?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(io))
            .await
            .map_err(|_| MgmtError::Other("management handshake failed".into()))?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let bytes = value
            .map(|v| serde_json::to_vec(&v).expect("JSON"))
            .unwrap_or_default();
        let request = http::Request::builder()
            .method(method)
            .uri(path)
            .header("host", "localhost")
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(bytes)))
            .map_err(|_| MgmtError::Invalid("invalid management path".into()))?;
        let response = sender
            .send_request(request)
            .await
            .map_err(|_| MgmtError::Other("management request failed".into()))?;
        let status = response.status();
        let body =
            axum::body::to_bytes(axum::body::Body::new(response.into_body()), 4 * 1024 * 1024)
                .await
                .map_err(|_| MgmtError::Other("invalid management response".into()))?;
        if status == StatusCode::ACCEPTED {
            return Err(MgmtError::Pending);
        }
        if status == StatusCode::NOT_FOUND {
            return Err(MgmtError::NotFound);
        }
        if !status.is_success() {
            if let Ok(value) = serde_json::from_slice::<UnavailableBody>(&body) {
                return Err(MgmtError::Unavailable(value.error.code));
            }
            if status == StatusCode::BAD_REQUEST
                && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&body)
                && let Some(message) = value
                    .pointer("/error/message")
                    .and_then(serde_json::Value::as_str)
            {
                return Err(MgmtError::Invalid(message.into()));
            }
            return Err(MgmtError::Other(format!(
                "management returned HTTP {}",
                status.as_u16()
            )));
        }
        let body = if status == StatusCode::NO_CONTENT {
            b"null".as_slice()
        } else {
            body.as_ref()
        };
        serde_json::from_slice(body).map_err(|_| MgmtError::Other("invalid management JSON".into()))
    }
}
fn account_path(id: &str, suffix: &str) -> Result<String, MgmtError> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(MgmtError::Invalid("invalid account identifier".into()));
    }
    Ok(format!("/accounts/{id}{suffix}"))
}
#[async_trait::async_trait]
impl LlmMgmt for Client {
    async fn status(&self) -> Result<Status, MgmtError> {
        self.call(http::Method::GET, STATUS, None).await
    }
    async fn accounts(&self) -> Result<Vec<AccountSummary>, MgmtError> {
        self.call(http::Method::GET, ACCOUNTS, None).await
    }
    async fn add_account(&self, a: NewAccount) -> Result<AccountSummary, MgmtError> {
        self.call(http::Method::POST, ACCOUNTS, Some(json!(a)))
            .await
    }
    async fn delete_account(&self, id: &str) -> Result<(), MgmtError> {
        self.call(http::Method::DELETE, &account_path(id, "")?, None)
            .await
    }
    async fn enable(&self, id: &str, req: Enable) -> Result<AccountSummary, MgmtError> {
        self.call(
            http::Method::POST,
            &account_path(id, "/enable")?,
            Some(json!(req)),
        )
        .await
    }
    async fn signin_start(&self, req: SignInStartReq) -> Result<SignInStart, MgmtError> {
        self.call(http::Method::POST, SIGNIN_START, Some(json!(req)))
            .await
    }
    async fn signin_complete(
        &self,
        flow: &str,
        input: SignInInput,
    ) -> Result<AccountSummary, MgmtError> {
        account_path(flow, "")?;
        self.call(
            http::Method::POST,
            &format!("/signin/{flow}/complete"),
            Some(json!(input)),
        )
        .await
    }
    async fn models(&self) -> Result<Vec<ModelEntry>, MgmtError> {
        self.call(http::Method::GET, MODELS, None).await
    }
    async fn set_claude_node(&self, req: SetClaudeNode) -> Result<(), MgmtError> {
        self.call(http::Method::POST, CLAUDE_SET_NODE, Some(json!(req)))
            .await
    }
    async fn presets(&self) -> Result<Vec<Preset>, MgmtError> {
        self.call(http::Method::GET, PRESETS, None).await
    }
}

#[cfg(all(test, unix))]
mod listener_tests {
    use super::Listener;
    #[tokio::test]
    async fn recovers_crashed_socket_but_preserves_live_listener_and_user_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("run/llm.sock");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let stale = std::os::unix::net::UnixListener::bind(&path).unwrap();
        drop(stale); // SIGKILL leaves the same pathname behind.
        let live = Listener::bind(&path).await.unwrap();
        assert!(Listener::bind(&path).await.is_err());
        assert!(tokio::net::UnixStream::connect(&path).await.is_ok());
        drop(live);
        assert!(!path.exists());
        // Also protect a listener from a pre-lock implementation.
        let external = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(Listener::bind(&path).await.is_err());
        drop(external);
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, "user data").unwrap();
        assert!(Listener::bind(&path).await.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user data");
    }
}
