//! In-process coordinator. Credential storage is exclusively the C7 AccountStore.
use crate::{
    Config, catalog, proto,
    route::local::{Pool, retryable},
    upstream::apikey::{ApiKey, Credential},
};
use cranibox_contracts::{
    error::ErrorCode, llm::mgmt::MgmtEvent, llm::*, net::BoxBody, vault::AccountStore,
};
use futures_util::{StreamExt, stream};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, broadcast};

pub(crate) struct Engine {
    pub node: String,
    pub store: Arc<dyn AccountStore>,
    pub sign_in: Option<Arc<dyn SignIn>>,
    pub presets: Vec<Preset>,
    routing: Option<Arc<dyn crate::route::Router>>,
    pending: Mutex<HashMap<String, (SecretBox, SecretBox)>>,
    upstreams: Vec<Arc<dyn Upstream>>,
    pool: Mutex<Pool>,
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    catalog: Mutex<Option<(Instant, Vec<ModelEntry>)>>,
    pub events: broadcast::Sender<MgmtEvent>,
    pub claude_node: Mutex<Option<String>>,
}
impl Engine {
    pub fn new(config: Config) -> Arc<Self> {
        Arc::new(Self {
            node: config.node,
            store: config.store,
            sign_in: config.sign_in,
            presets: config.presets,
            routing: config.routing,
            pending: Mutex::new(HashMap::new()),
            upstreams: config.upstreams,
            pool: Mutex::new(Pool::default()),
            locks: Mutex::new(HashMap::new()),
            catalog: Mutex::new(None),
            events: broadcast::channel(128).0,
            claude_node: Mutex::new(None),
        })
    }
    pub fn preset(&self, id: &str) -> Option<Preset> {
        self.presets.iter().find(|p| p.id == id).cloned()
    }
    pub async fn forget(&self, id: &str) {
        self.pending.lock().await.remove(id);
        self.pool.lock().await.success(id);
    }
    pub async fn account_lock(&self, id: &str) -> Arc<Mutex<()>> {
        self.locks
            .lock()
            .await
            .entry(id.into())
            .or_default()
            .clone()
    }
    pub async fn changed(&self, id: &str) {
        *self.catalog.lock().await = None;
        let _ = self.events.send(MgmtEvent::AccountChanged {
            account_id: id.into(),
        });
        let _ = self.events.send(MgmtEvent::ModelsChanged {
            etag: uuid::Uuid::new_v4().to_string(),
        });
        let mode = if self
            .store
            .list()
            .iter()
            .any(|a| self.enabled(a) && matches!(a.state, AccountState::Ready))
        {
            mgmt::Mode::Local
        } else {
            mgmt::Mode::Offline
        };
        let _ = self.events.send(MgmtEvent::ModeChanged { mode });
    }
    pub fn enabled(&self, a: &AccountSummary) -> bool {
        (a.all_nodes || a.enabled_on.contains(&self.node))
            && (a.kind != AccountKind::ClaudeOAuth
                || (a.home_node == self.node && !a.sync && !a.all_nodes))
    }
    fn adapter(&self, a: &AccountSummary) -> Option<Arc<dyn Upstream>> {
        let provider = if a.kind == AccountKind::ApiKey {
            "apikey"
        } else {
            &a.provider
        };
        self.upstreams
            .iter()
            .find(|u| u.provider() == provider)
            .cloned()
    }
    /// A detached refresh finishes and writes a rotated token even if the HTTP caller disconnects.
    /// The same account lock serializes refresh, deletion and enablement changes on this node.
    async fn context(
        self: &Arc<Self>,
        id: &str,
        force: Option<SecretBox>,
    ) -> Result<UpstreamCx, UpstreamError> {
        let this = self.clone();
        let id = id.to_owned();
        tokio::spawn(async move {
            let lock = this.account_lock(&id).await;
            let _guard = lock.lock().await;
            let account = this.store.get(&id).ok_or(UpstreamError::Auth)?;
            if !this.enabled(&account) || matches!(account.state, AccountState::NeedsSignIn) {
                return Err(UpstreamError::Auth);
            }
            let mut secret = this
                .store
                .get_secret(&id)
                .map_err(|_| UpstreamError::Auth)?;
            let pending = { this.pending.lock().await.get(&id).cloned() };
            if let Some((previous, rotated)) = pending {
                if secret == previous {
                    this.store
                        .put(account.clone(), Some(rotated.clone()))
                        .map_err(|_| {
                            UpstreamError::Network("could not persist refreshed credential".into())
                        })?;
                    secret = rotated;
                }
                this.pending.lock().await.remove(&id);
            }
            let mut cx = UpstreamCx {
                preset: this.preset(&account.provider),
                account,
                secret,
                node: this.node.clone(),
            };
            if cx.account.kind != AccountKind::ApiKey {
                let credential = Credential::decode(&cx.secret)?;
                if force
                    .as_ref()
                    .is_some_and(|expected| *expected == cx.secret)
                    || credential
                        .expires_at
                        .is_some_and(|e| e <= crate::auth::now() + 30)
                {
                    let adapter = this
                        .adapter(&cx.account)
                        .ok_or(UpstreamError::Unsupported)?;
                    match adapter.refresh(&cx, &cx.secret).await {
                        Ok(refreshed) => {
                            // Preserve concurrent vault metadata; never recreate a deleted record.
                            let current = this.store.get(&id).ok_or(UpstreamError::Auth)?;
                            if this
                                .store
                                .get_secret(&id)
                                .map_err(|_| UpstreamError::Auth)?
                                != cx.secret
                            {
                                return Err(UpstreamError::Network(
                                    "credential changed during refresh; retry request".into(),
                                ));
                            }
                            this.pending
                                .lock()
                                .await
                                .insert(id.clone(), (cx.secret.clone(), refreshed.secret.clone()));
                            this.store
                                .put(current.clone(), Some(refreshed.secret.clone()))
                                .map_err(|_| {
                                    UpstreamError::Network(
                                        "could not persist refreshed credential".into(),
                                    )
                                })?;
                            this.pending.lock().await.remove(&id);
                            cx.account = current;
                            cx.secret = refreshed.secret;
                            this.changed(&id).await;
                        }
                        Err(error) => {
                            if matches!(error, UpstreamError::Auth)
                                && let Some(mut current) = this.store.get(&id)
                            {
                                current.state = AccountState::NeedsSignIn;
                                this.store.put(current, None).map_err(|_| {
                                    UpstreamError::Network("could not update account state".into())
                                })?;
                                this.changed(&id).await;
                            }
                            return Err(error);
                        }
                    }
                }
            }
            Ok(cx)
        })
        .await
        .map_err(|_| UpstreamError::Network("credential task interrupted".into()))?
    }
    async fn failed(&self, id: &str, error: &UpstreamError) {
        self.pool.lock().await.failed(id, error);
        if matches!(
            error,
            UpstreamError::Auth | UpstreamError::QuotaExhausted { .. }
        ) {
            let lock = self.account_lock(id).await;
            let _guard = lock.lock().await;
            if let Some(mut account) = self.store.get(id) {
                account.state = match error {
                    UpstreamError::Auth => AccountState::NeedsSignIn,
                    UpstreamError::QuotaExhausted { reset_at: Some(at) } => {
                        AccountState::QuotaExhausted {
                            reset_at: Some(*at),
                        }
                    }
                    // No reset hint is not a permanent ban: retry after the pool's conservative cooldown.
                    _ => AccountState::QuotaExhausted {
                        reset_at: Some(crate::auth::now() + 600),
                    },
                };
                let _ = self.store.put(account, None);
                self.changed(id).await;
            }
        }
    }
    pub async fn probe(
        self: &Arc<Self>,
        id: &str,
        request: crate::mgmt::tools::Probe,
    ) -> Result<Vec<crate::mgmt::tools::ProbeResult>, MgmtError> {
        let account = self.store.get(id).ok_or(MgmtError::NotFound)?;
        if !self.enabled(&account) {
            return Err(MgmtError::Unavailable(ErrorCode::AccountDisabledHere));
        }
        let cx = self.context(id, None).await?;
        let adapter = self.adapter(&account).ok_or(MgmtError::NotFound)?;
        let credential = Credential::decode(&cx.secret)?;
        let model = request
            .model
            .or_else(|| credential.models.first().cloned())
            .or_else(|| cx.preset.as_ref().and_then(|p| p.models.first().cloned()))
            .ok_or_else(|| MgmtError::Invalid("请指定要测试的模型".into()))?;
        let mut results = Vec::new();
        for wire in [Wire::Chat, Wire::Responses, Wire::Messages] {
            if request.wire.is_some_and(|w| w != wire) {
                continue;
            }
            let started = Instant::now();
            let body = match wire {
                Wire::Chat => {
                    serde_json::json!({"model":model,"messages":[{"role":"user","content":"Reply OK"}],"max_tokens":1})
                }
                Wire::Responses => {
                    serde_json::json!({"model":model,"input":"Reply OK","max_output_tokens":1})
                }
                Wire::Messages => {
                    serde_json::json!({"model":model,"messages":[{"role":"user","content":"Reply OK"}],"max_tokens":1})
                }
            };
            let result = if adapter.wires().contains(&wire) {
                tokio::time::timeout(
                    Duration::from_secs(15),
                    adapter.send(
                        &cx,
                        CanonicalRequest {
                            wire,
                            body,
                            stream: false,
                            session_id: None,
                            client_agent: None,
                            model: model.clone(),
                        },
                    ),
                )
                .await
                .unwrap_or_else(|_| Err(UpstreamError::Network("test timed out".into())))
            } else {
                Err(UpstreamError::Unsupported)
            };
            let error = match result {
                Ok(r) if (200..300).contains(&r.status) => None,
                Ok(_) => Some("upstream rejected request".into()),
                Err(e) => Some(
                    match e {
                        UpstreamError::Auth => "needs_sign_in",
                        UpstreamError::RateLimited { .. } => "rate_limited",
                        UpstreamError::QuotaExhausted { .. } => "quota_exhausted",
                        UpstreamError::Unsupported => "unsupported",
                        UpstreamError::Network(_) => "connection_failed",
                        UpstreamError::Status(_, _) => "upstream_rejected",
                    }
                    .into(),
                ),
            };
            results.push(crate::mgmt::tools::ProbeResult {
                ok: error.is_none(),
                wire,
                latency_ms: started.elapsed().as_millis() as u64,
                error,
            });
        }
        Ok(results)
    }
    pub async fn send(
        self: &Arc<Self>,
        provider: &str,
        request: CanonicalRequest,
        count: bool,
    ) -> Result<UpstreamResponse, CallError> {
        if let Some(router) = &self.routing {
            match router.decide(provider, &request).await {
                crate::route::RouteDecision::Local => {}
                crate::route::RouteDecision::Unavailable { code } => {
                    return Err(CallError::Unavailable(code));
                }
                crate::route::RouteDecision::Remote { node } => {
                    if count {
                        return Err(CallError::Upstream(UpstreamError::Unsupported));
                    }
                    return router
                        .forward(&node, provider, request)
                        .await
                        .map_err(CallError::Upstream);
                }
            }
        }
        self.send_local(provider, request, count).await
    }
    pub async fn send_local(
        self: &Arc<Self>,
        provider: &str,
        request: CanonicalRequest,
        count: bool,
    ) -> Result<UpstreamResponse, CallError> {
        let session = request.session_id.clone().unwrap_or_else(|| {
            let user = request
                .body
                .get("messages")
                .and_then(|v| v.as_array())
                .and_then(|a| a.iter().find(|m| m["role"] == "user"))
                .or_else(|| {
                    request
                        .body
                        .get("input")
                        .and_then(|v| v.as_array())
                        .and_then(|a| a.iter().find(|m| m["role"] == "user"))
                });
            format!(
                "{:x}",
                Sha256::digest(
                    serde_json::to_vec(&user.cloned().unwrap_or_else(|| {
                        request.body.get("input").cloned().unwrap_or_default()
                    }))
                    .expect("JSON")
                )
            )
        });
        let candidates = self
            .pool
            .lock()
            .await
            .candidates(self.store.list(), provider, &self.node, &session)
            .map_err(CallError::Unavailable)?;
        let mut last_auth = false;
        for account in candidates {
            let result = async {
                let adapter = self.adapter(&account).ok_or(UpstreamError::Unsupported)?;
                let mut cx = self.context(&account.id, None).await?;
                let endpoints = Credential::decode(&cx.secret)
                    .map(|c| crate::upstream::apikey::endpoints(&cx, &c))?;
                let target = if account.kind == AccountKind::ApiKey {
                    if endpoints.contains_key(&request.wire) {
                        request.wire
                    } else {
                        cx.preset
                            .as_ref()
                            .map(|p| p.default_wire)
                            .or_else(|| endpoints.keys().next().copied())
                            .ok_or(UpstreamError::Unsupported)?
                    }
                } else if adapter.wires().contains(&request.wire) {
                    request.wire
                } else {
                    *adapter.wires().first().ok_or(UpstreamError::Unsupported)?
                };
                if count
                    && (account.kind != AccountKind::ApiKey
                        || !endpoints.contains_key(&Wire::Messages))
                {
                    return Err(UpstreamError::Unsupported);
                }
                let mut outbound = request.clone();
                outbound.wire = if count { Wire::Messages } else { target };
                outbound.body = if count {
                    request.body.clone()
                } else {
                    proto::request(request.clone(), target)?.body
                };
                outbound.body["model"] = request.model.clone().into();
                outbound.session_id = Some(session.clone());
                let initial_cx = UpstreamCx {
                    account: cx.account.clone(),
                    secret: cx.secret.clone(),
                    preset: cx.preset.clone(),
                    node: cx.node.clone(),
                };
                let mut response = if count {
                    ApiKey.send_count(&initial_cx, outbound.clone()).await
                } else {
                    adapter.send(&initial_cx, outbound.clone()).await
                };
                if matches!(response, Err(UpstreamError::Auth))
                    && account.kind != AccountKind::ApiKey
                {
                    cx = self.context(&account.id, Some(cx.secret.clone())).await?;
                    response = if count {
                        ApiKey.send_count(&cx, outbound).await
                    } else {
                        adapter.send(&cx, outbound).await
                    };
                }
                let response = response?;
                if !(200..300).contains(&response.status) {
                    return Err(crate::upstream::apikey::status_error(
                        response.status,
                        None,
                        None,
                    ));
                }
                let response = if count {
                    response
                } else {
                    proto::response(response, request.wire, request.stream, &request.model).await?
                };
                prefetch(response).await
            }
            .await;
            match result {
                Ok(response) => {
                    self.pool.lock().await.success(&account.id);
                    let lock = self.account_lock(&account.id).await;
                    let _guard = lock.lock().await;
                    if let Some(mut current) = self.store.get(&account.id)
                        && matches!(current.state, AccountState::QuotaExhausted { .. })
                    {
                        current.state = AccountState::Ready;
                        let _ = self.store.put(current, None);
                        self.changed(&account.id).await;
                    }
                    return Ok(response);
                }
                Err(error) => {
                    if !retryable(&error) {
                        return Err(CallError::Upstream(error));
                    }
                    last_auth = matches!(error, UpstreamError::Auth);
                    self.failed(&account.id, &error).await;
                }
            }
        }
        Err(CallError::Unavailable(
            if last_auth
                && self
                    .store
                    .list()
                    .iter()
                    .filter(|a| a.provider == provider && self.enabled(a))
                    .all(|a| matches!(a.state, AccountState::NeedsSignIn))
            {
                ErrorCode::NeedsSignIn
            } else {
                ErrorCode::AllAccountsExhausted
            },
        ))
    }
    pub async fn models(self: &Arc<Self>) -> Vec<ModelEntry> {
        // The catalog is bounded and cached; a broken discovery endpoint does not disable inference.
        let mut cache = self.catalog.lock().await;
        if let Some((at, models)) = &*cache
            && at.elapsed() < Duration::from_secs(60)
        {
            return models.clone();
        }
        let accounts = self.store.list();
        let models = stream::iter(accounts)
            .map(|account| async move {
                let available =
                    self.enabled(&account) && matches!(account.state, AccountState::Ready);
                let entries = async {
                    let adapter = self.adapter(&account)?;
                    let secret = self.store.get_secret(&account.id).ok()?;
                    let cx = UpstreamCx {
                        preset: self.preset(&account.provider),
                        account: account.clone(),
                        secret,
                        node: self.node.clone(),
                    };
                    tokio::time::timeout(Duration::from_secs(10), adapter.models(&cx))
                        .await
                        .ok()?
                        .ok()
                }
                .await
                .unwrap_or_else(|| {
                    self.preset(&account.provider)
                        .map(|p| {
                            p.models
                                .iter()
                                .map(|m| catalog::model(&p.id, m, p.default_wire))
                                .collect()
                        })
                        .unwrap_or_default()
                });
                entries
                    .into_iter()
                    .map(|mut m| {
                        if !available {
                            m.availability = Availability::Unavailable {
                                code: if !self.enabled(&account) {
                                    ErrorCode::AccountDisabledHere
                                } else if matches!(account.state, AccountState::NeedsSignIn) {
                                    ErrorCode::NeedsSignIn
                                } else {
                                    ErrorCode::AllAccountsExhausted
                                },
                            };
                        }
                        m
                    })
                    .collect::<Vec<_>>()
            })
            .buffer_unordered(8)
            .collect::<Vec<_>>()
            .await;
        let mut merged = std::collections::BTreeMap::<String, ModelEntry>::new();
        for entry in models.into_iter().flatten() {
            if !merged.contains_key(&entry.id) || matches!(entry.availability, Availability::Local)
            {
                merged.insert(entry.id.clone(), entry);
            }
        }
        let models: Vec<_> = merged.into_values().collect();
        *cache = Some((Instant::now(), models.clone()));
        models
    }
}
#[derive(Debug)]
pub enum CallError {
    Unavailable(ErrorCode),
    Upstream(UpstreamError),
}
async fn prefetch(mut response: UpstreamResponse) -> Result<UpstreamResponse, UpstreamError> {
    let mut stream = response.body.into_stream();
    let first = tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            match stream.next().await {
                Some(Ok(bytes)) if bytes.is_empty() => continue,
                Some(Ok(bytes)) => return Ok(bytes),
                Some(Err(_)) | None => {
                    return Err(UpstreamError::Network(
                        "upstream ended before first response byte".into(),
                    ));
                }
            }
        }
    })
    .await
    .map_err(|_| {
        UpstreamError::Network("upstream timed out before first response byte".into())
    })??;
    response.body = BoxBody::new(stream::once(async move { Ok(first) }).chain(stream));
    Ok(response)
}
