//! C3 sign-in flows. No ambient CLI credential discovery.
// OAuth constants/fields adapted from pleaseai/shunt src/auth/{codex,kimi,google}
// @9d42ba251dd1f0651e9d150767e3ff98a1ae042b (MIT OR Apache-2.0), and
// yetone/magpie internal/provider/copilot_signin.go@865b61808690bc9fd716257949414ec97618a5a5 (MIT).
use super::{now, tokens};
use crate::upstream::{
    apikey::{self, Credential},
    codex, copilot, kimi,
};
use base64::Engine;
use cranibox_contracts::{llm::*, vault::AccountStore};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct OAuthConfig {
    codex_authorize: String,
    codex_token: String,
    kimi_device: String,
    kimi_token: String,
    github_device: String,
    github_token: String,
    copilot_token: String,
    copilot_api: Option<String>,
    google_authorize: String,
    google_token: String,
    google_api: String,
    google_client: Option<(String, String)>,
    callback_port: u16,
}
impl Default for OAuthConfig {
    fn default() -> Self {
        Self {
            codex_authorize: "https://auth.openai.com/oauth/authorize".into(),
            codex_token: "https://auth.openai.com/oauth/token".into(),
            kimi_device: "https://auth.kimi.com/api/oauth/device_authorization".into(),
            kimi_token: "https://auth.kimi.com/api/oauth/token".into(),
            github_device: "https://github.com/login/device/code".into(),
            github_token: "https://github.com/login/oauth/access_token".into(),
            copilot_token: "https://api.github.com/copilot_internal/v2/token".into(),
            copilot_api: None,
            google_authorize: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            google_token: "https://oauth2.googleapis.com/token".into(),
            google_api: "https://cloudcode-pa.googleapis.com".into(),
            google_client: std::env::var("CRANIBOX_GOOGLE_CLIENT_ID")
                .ok()
                .zip(std::env::var("CRANIBOX_GOOGLE_CLIENT_SECRET").ok()),
            callback_port: 1455,
        }
    }
}
impl OAuthConfig {
    /// Restricted to loopback: never send production credentials to a test server.
    pub fn for_loopback_with_callback_port(base: &str, port: u16) -> Result<Self, String> {
        let mut config = Self::for_loopback(base)?;
        config.callback_port = port;
        Ok(config)
    }
    pub fn for_loopback(base: &str) -> Result<Self, String> {
        super::require_loopback(base)?;
        let b = base.trim_end_matches('/');
        Ok(Self {
            codex_authorize: format!("{b}/authorize"),
            codex_token: format!("{b}/token"),
            kimi_device: format!("{b}/device"),
            kimi_token: format!("{b}/token"),
            github_device: format!("{b}/device"),
            github_token: format!("{b}/token"),
            copilot_token: format!("{b}/copilot-token"),
            copilot_api: Some(base.into()),
            google_authorize: format!("{b}/authorize"),
            google_token: format!("{b}/token"),
            google_api: base.into(),
            google_client: Some(("test-client".into(), "test-secret".into())),
            callback_port: 0,
        })
    }
}
#[derive(Clone)]
pub struct OAuth {
    node: String,
    store: Arc<dyn AccountStore>,
    config: OAuthConfig,
    flows: Arc<Mutex<HashMap<String, Arc<Mutex<Flow>>>>>,
}
struct Flow {
    provider: String,
    label: Option<String>,
    region: Option<String>,
    expires: u64,
    state: String,
    verifier: String,
    redirect: String,
    code: Option<String>,
    device: Option<String>,
    device_id: String,
    interval: u64,
    next_poll: Instant,
    complete: Option<AccountSummary>,
    credential: Option<Credential>,
    account: Option<AccountSummary>,
    failure: Option<&'static str>,
    stop: tokio_util::sync::CancellationToken,
}
impl Flow {
    fn fail(&mut self, message: &'static str) {
        self.failure = Some(message);
        self.expires = 0;
        self.state.clear();
        self.verifier.clear();
        self.code = None;
        self.device = None;
        self.credential = None;
        self.stop.cancel();
    }
    fn callback(&mut self, query: &HashMap<String, String>) -> Result<(), MgmtError> {
        if self.expires <= now() || query.get("state") != Some(&self.state) {
            return Err(MgmtError::Invalid("invalid or expired callback".into()));
        }
        if self.code.is_some() || self.credential.is_some() || self.complete.is_some() {
            return Err(MgmtError::Invalid("callback already received".into()));
        }
        if query.contains_key("error") {
            self.fail("授权未完成，请重新登录");
            return Err(MgmtError::Invalid("授权未完成，请重新登录".into()));
        }
        let code = query
            .get("code")
            .filter(|code| !code.is_empty())
            .ok_or_else(|| MgmtError::Invalid("callback code missing".into()))?;
        self.code = Some(code.clone());
        self.stop.cancel();
        Ok(())
    }
}
impl Drop for Flow {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
pub fn summary(provider: &str, node: &str, label: Option<String>) -> AccountSummary {
    AccountSummary {
        id: uuid::Uuid::new_v4().to_string(),
        provider: provider.into(),
        plan: None,
        label: label.unwrap_or_else(|| provider.into()),
        kind: if provider == "claude" {
            AccountKind::ClaudeOAuth
        } else {
            AccountKind::OAuth
        },
        home_node: node.into(),
        sync: provider != "claude",
        enabled_on: vec![node.into()],
        all_nodes: false,
        state: AccountState::Ready,
    }
}
impl OAuth {
    pub fn new(node: String, store: Arc<dyn AccountStore>, config: OAuthConfig) -> Self {
        Self {
            node,
            store,
            config,
            flows: Default::default(),
        }
    }
    /// In-process cancellation. The C3 management contract has no cancel operation yet.
    /// An exchange already in progress is allowed to persist its rotating credentials first.
    pub async fn cancel(&self, flow_id: &str) -> Result<(), MgmtError> {
        let flow = self
            .flows
            .lock()
            .await
            .get(flow_id)
            .cloned()
            .ok_or(MgmtError::NotFound)?;
        let mut flow = flow.lock().await;
        if flow.complete.is_some() {
            return Ok(());
        }
        if flow.credential.is_some() {
            return Err(MgmtError::Invalid("登录凭据待保存，请重试完成登录".into()));
        }
        flow.fail("登录已取消，请重试");
        Ok(())
    }
    async fn device(&self, provider: &str, opts: SignInOpts) -> Result<SignInStart, MgmtError> {
        let (url, client) = match provider {
            "kimi" => (&self.config.kimi_device, kimi::CLIENT_ID),
            "copilot" => (&self.config.github_device, copilot::CLIENT_ID),
            _ => return Err(MgmtError::NotFound),
        };
        let device_id = uuid::Uuid::new_v4().to_string();
        let mut request = apikey::client()
            .post(url)
            .header("accept", "application/json")
            .form(&[
                ("client_id", client),
                (
                    "scope",
                    if provider == "copilot" {
                        "read:user"
                    } else {
                        ""
                    },
                ),
            ]);
        if provider == "kimi" {
            request = kimi::msh(request, &device_id);
        }
        let response = request
            .send()
            .await
            .map_err(|_| MgmtError::Other("device authorization connection failed".into()))?;
        let (status, value) = apikey::json_response(response, 65536).await?;
        if status >= 300 {
            return Err(apikey::status_error(status, None, Some(&value)).into());
        }
        let device = value["device_code"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| MgmtError::Other("invalid device authorization response".into()))?;
        let code = value["user_code"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| MgmtError::Other("device user code missing".into()))?;
        let url = value
            .get("verification_uri_complete")
            .or_else(|| value.get("verification_uri"))
            .and_then(Value::as_str)
            .ok_or_else(|| MgmtError::Other("device verification URL missing".into()))?;
        let parsed = url::Url::parse(url)
            .map_err(|_| MgmtError::Other("invalid verification URL".into()))?;
        if parsed.scheme() != "https" && super::require_loopback(url).is_err() {
            return Err(MgmtError::Other("insecure verification URL".into()));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let expires = now().saturating_add(value["expires_in"].as_u64().unwrap_or(600).min(1800));
        let interval = value["interval"].as_u64().unwrap_or(5).clamp(1, 60);
        self.insert(
            id.clone(),
            Flow {
                provider: provider.into(),
                label: opts.label,
                region: opts.region,
                expires,
                state: String::new(),
                verifier: String::new(),
                redirect: String::new(),
                code: None,
                device: Some(device.into()),
                device_id,
                interval,
                next_poll: Instant::now() + Duration::from_secs(interval),
                complete: None,
                credential: None,
                account: None,
                failure: None,
                stop: tokio_util::sync::CancellationToken::new(),
            },
        )
        .await?;
        Ok(SignInStart::DeviceCode {
            flow_id: id,
            code: code.into(),
            url: url.into(),
            expires_at: expires,
        })
    }
    async fn insert(&self, id: String, flow: Flow) -> Result<(), MgmtError> {
        let mut flows = self.flows.lock().await;
        flows.retain(|_, f| {
            f.try_lock().map_or(true, |mut f| {
                if f.expires > now() || f.credential.is_some() {
                    return true;
                }
                f.fail("登录已过期，请重试");
                false
            })
        });
        if flows.len() >= 32 {
            return Err(MgmtError::Invalid("too many pending sign-ins".into()));
        }
        flows.insert(id, Arc::new(Mutex::new(flow)));
        Ok(())
    }
    async fn browser(&self, provider: &str, opts: SignInOpts) -> Result<SignInStart, MgmtError> {
        let (authorize,client,scope)=match provider{
            "codex"=>(self.config.codex_authorize.as_str(),codex::CLIENT_ID,"openid profile email offline_access api.connectors.read api.connectors.invoke"),
            "gemini"=>(self.config.google_authorize.as_str(),self.config.google_client.as_ref().ok_or_else(||MgmtError::Invalid("请配置 CRANIBOX_GOOGLE_CLIENT_ID 和 CRANIBOX_GOOGLE_CLIENT_SECRET（桌面 OAuth 客户端）".into()))?.0.as_str(),"openid email profile https://www.googleapis.com/auth/cloud-platform"),_=>return Err(MgmtError::NotFound)};
        let verifier =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let state =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        let id = uuid::Uuid::new_v4().to_string();
        let listener = tokio::net::TcpListener::bind((
            std::net::Ipv4Addr::LOCALHOST,
            if provider == "codex" {
                self.config.callback_port
            } else {
                0
            },
        ))
        .await;
        let (listener, port) = match listener {
            Ok(l) => {
                let port = l
                    .local_addr()
                    .map_err(|_| MgmtError::Other("callback bind failed".into()))?
                    .port();
                (Some(l), port)
            }
            Err(_) if provider == "codex" && self.config.callback_port != 0 => {
                (None, self.config.callback_port)
            }
            Err(_) => return Err(MgmtError::Other("callback bind failed".into())),
        };
        let redirect = format!("http://localhost:{port}/auth/callback");
        let mut url = url::Url::parse(authorize)
            .map_err(|_| MgmtError::Other("invalid authorization endpoint".into()))?;
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", client),
            ("redirect_uri", &redirect),
            ("scope", scope),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
        ]);
        if provider == "codex" {
            url.query_pairs_mut().extend_pairs([
                ("id_token_add_organizations", "true"),
                ("codex_cli_simplified_flow", "true"),
                ("originator", "codex_cli_rs"),
            ]);
        } else {
            url.query_pairs_mut()
                .extend_pairs([("access_type", "offline"), ("prompt", "consent")]);
        }
        self.insert(
            id.clone(),
            Flow {
                provider: provider.into(),
                label: opts.label,
                region: opts.region,
                expires: now() + 600,
                state,
                verifier,
                redirect,
                code: None,
                device: None,
                device_id: uuid::Uuid::new_v4().to_string(),
                interval: 0,
                next_poll: Instant::now(),
                complete: None,
                credential: None,
                account: None,
                failure: None,
                stop: tokio_util::sync::CancellationToken::new(),
            },
        )
        .await?;
        if let Some(listener) = listener {
            let flow = self.flows.lock().await.get(&id).expect("inserted").clone();
            let stop = flow.lock().await.stop.clone();
            // The listener must not keep the flow alive after the owner is dropped.
            let flow = Arc::downgrade(&flow);
            let expiry = flow.clone();
            let router = axum::Router::new().route(
                "/auth/callback",
                axum::routing::get(
                    move |axum::extract::Query(query): axum::extract::Query<
                        HashMap<String, String>,
                    >| {
                        let flow = flow.clone();
                        async move {
                            let Some(flow) = flow.upgrade() else {
                                return (axum::http::StatusCode::GONE, "Login no longer active");
                            };
                            let mut flow = flow.lock().await;
                            if flow.callback(&query).is_err() {
                                return (
                                    axum::http::StatusCode::BAD_REQUEST,
                                    "Invalid or expired callback",
                                );
                            }
                            (
                                axum::http::StatusCode::OK,
                                "Login received. Return to Cranibox.",
                            )
                        }
                    },
                ),
            );
            let router = router.layer(axum::middleware::from_fn(
                |request, next: axum::middleware::Next| async move {
                    let mut response = next.run(request).await;
                    response
                        .headers_mut()
                        .insert("cache-control", "no-store".parse().unwrap());
                    response
                        .headers_mut()
                        .insert("referrer-policy", "no-referrer".parse().unwrap());
                    response
                },
            ));
            tokio::spawn(async move {
                let _ = axum::serve(listener, router)
                    .with_graceful_shutdown(async move {
                        tokio::select! {
                            _ = stop.cancelled() => {},
                            _ = tokio::time::sleep(Duration::from_secs(600)) => {
                                if let Some(flow) = expiry.upgrade() {
                                    let mut flow = flow.lock().await;
                                    if flow.complete.is_none() && flow.credential.is_none() {
                                        flow.fail("登录已过期，请重试");
                                    }
                                }
                            }
                        }
                    })
                    .await;
            });
            Ok(SignInStart::Browser {
                flow_id: id,
                url: url.into(),
            })
        } else {
            Ok(SignInStart::PasteCallback {
                flow_id: id,
                url: url.into(),
            })
        }
    }
    async fn finish(&self, flow_id: &str, input: SignInInput) -> Result<AccountSummary, MgmtError> {
        let flow = self
            .flows
            .lock()
            .await
            .get(flow_id)
            .cloned()
            .ok_or(MgmtError::NotFound)?;
        let mut flow = flow.lock().await;
        if let Some(account) = &flow.complete {
            return Ok(account.clone());
        }
        if let Some(message) = flow.failure {
            return Err(MgmtError::Invalid(message.into()));
        }
        if flow.expires <= now() && flow.credential.is_none() {
            flow.fail("登录已过期，请重试");
            return Err(MgmtError::Invalid("登录已过期，请重试".into()));
        }
        if let Some(callback) = input.callback_url {
            let url = url::Url::parse(&callback)
                .map_err(|_| MgmtError::Invalid("invalid callback URL".into()))?;
            let expected = url::Url::parse(&flow.redirect)
                .map_err(|_| MgmtError::Invalid("not a browser flow".into()))?;
            if url.origin() != expected.origin() || url.path() != expected.path() {
                return Err(MgmtError::Invalid("callback origin/path mismatch".into()));
            }
            let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
            flow.callback(&pairs)?;
        }
        if input.code.is_some() {
            return Err(MgmtError::Invalid(
                "请粘贴包含 state 的完整回调 URL；设备码请在厂商页面输入".into(),
            ));
        }
        if flow.credential.is_none() {
            let client = match flow.provider.as_str() {
                "codex" => codex::CLIENT_ID,
                "kimi" => kimi::CLIENT_ID,
                "copilot" => copilot::CLIENT_ID,
                "gemini" => self
                    .config
                    .google_client
                    .as_ref()
                    .ok_or(MgmtError::NotFound)?
                    .0
                    .as_str(),
                _ => return Err(MgmtError::NotFound),
            };
            let (url, form) = if let Some(device) = flow.device.clone() {
                if Instant::now() < flow.next_poll {
                    return Err(MgmtError::Pending);
                }
                flow.next_poll = Instant::now() + Duration::from_secs(flow.interval);
                (
                    if flow.provider == "kimi" {
                        self.config.kimi_token.clone()
                    } else {
                        self.config.github_token.clone()
                    },
                    vec![
                        (
                            "grant_type",
                            "urn:ietf:params:oauth:grant-type:device_code".to_string(),
                        ),
                        ("device_code", device),
                        ("client_id", client.into()),
                    ],
                )
            } else {
                let code = flow.code.clone().ok_or(MgmtError::Pending)?;
                let mut form = vec![
                    ("grant_type", "authorization_code".to_string()),
                    ("code", code),
                    ("redirect_uri", flow.redirect.clone()),
                    ("client_id", client.into()),
                    ("code_verifier", flow.verifier.clone()),
                ];
                if flow.provider == "gemini" {
                    form.push((
                        "client_secret",
                        self.config
                            .google_client
                            .as_ref()
                            .expect("configured")
                            .1
                            .clone(),
                    ));
                }
                (
                    if flow.provider == "codex" {
                        self.config.codex_token.clone()
                    } else {
                        self.config.google_token.clone()
                    },
                    form,
                )
            };
            let mut request = apikey::client()
                .post(url)
                .header("accept", "application/json")
                .form(&form);
            if flow.provider == "kimi" {
                request = kimi::msh(request, &flow.device_id);
            }
            let response = request
                .send()
                .await
                .map_err(|_| MgmtError::Other("token exchange connection failed".into()))?;
            let (status, value) = apikey::json_response(response, 1024 * 1024).await?;
            match value.get("error").and_then(Value::as_str) {
                Some("authorization_pending") => return Err(MgmtError::Pending),
                Some("slow_down") => {
                    flow.interval = (flow.interval + 5).min(60);
                    flow.next_poll = Instant::now() + Duration::from_secs(flow.interval);
                    return Err(MgmtError::Pending);
                }
                Some(
                    "access_denied"
                    | "expired_token"
                    | "invalid_grant"
                    | "token_revoked"
                    | "refresh_token_reused",
                ) => {
                    flow.fail("授权已失效，请重新登录");
                    return Err(UpstreamError::Auth.into());
                }
                Some(_) => return Err(apikey::status_error(status, None, Some(&value)).into()),
                None => {}
            }
            if status >= 300 {
                return Err(apikey::status_error(status, None, Some(&value)).into());
            }
            flow.credential = Some(tokens(
                &value,
                Credential {
                    region: flow.region.clone(),
                    device_id: Some(flow.device_id.clone()),
                    ..Default::default()
                },
            )?);
            flow.code = None;
            flow.device = None;
            flow.verifier.clear();
        }
        let mut credential = flow.credential.clone().expect("exchanged");
        if flow.provider == "gemini" && credential.project.is_none() {
            credential.project = Some(
                crate::upstream::gemini::Gemini::discover(
                    &self.config.google_api,
                    &credential.access_token,
                )
                .await?,
            );
        }
        if flow.provider == "copilot" && credential.github_token.is_none() {
            let adapter = if let Some(base) = &self.config.copilot_api {
                copilot::Copilot::for_loopback(base.clone(), self.config.copilot_token.clone())
                    .map_err(MgmtError::Invalid)?
            } else {
                copilot::Copilot::default()
            };
            credential = adapter.exchange(&credential.access_token).await?;
        }
        flow.credential = Some(credential.clone());
        let account = flow
            .account
            .clone()
            .unwrap_or_else(|| summary(&flow.provider, &self.node, flow.label.clone()));
        flow.account = Some(account.clone());
        self.store
            .put(account.clone(), Some(credential.secret()))
            .map_err(|_| MgmtError::Other("vault write failed".into()))?;
        flow.complete = Some(account.clone());
        flow.credential = None;
        flow.stop.cancel();
        Ok(account)
    }
}
#[async_trait::async_trait]
impl SignIn for OAuth {
    async fn start(&self, provider: &str, opts: SignInOpts) -> Result<SignInStart, MgmtError> {
        if opts
            .region
            .as_deref()
            .is_some_and(|r| !matches!(r, "cn" | "global"))
        {
            return Err(MgmtError::Invalid("unknown region".into()));
        }
        match provider {
            "codex" | "gemini" => self.browser(provider, opts).await,
            "kimi" | "copilot" => self.device(provider, opts).await,
            _ => Err(MgmtError::NotFound),
        }
    }
    async fn complete(
        &self,
        flow_id: &str,
        input: SignInInput,
    ) -> Result<AccountSummary, MgmtError> {
        // Once a rotating exchange starts, caller cancellation must not discard its result.
        let this = self.clone();
        let id = flow_id.to_string();
        tokio::spawn(async move { this.finish(&id, input).await })
            .await
            .map_err(|_| MgmtError::Other("sign-in task failed".into()))?
    }
}
