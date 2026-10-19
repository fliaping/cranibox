use axum::{
    Json, Router,
    extract::{Request, State},
    response::{IntoResponse, Response},
    routing::any,
};
use cranibox_contracts::{llm::*, mock::MemoryAccountStore, vault::AccountStore};
use cranibox_llm::{
    auth::{OAuth, OAuthConfig},
    upstream::{
        apikey::{ApiKey, Credential},
        codex::Codex,
        copilot::Copilot,
        gemini::Gemini,
        kimi::Kimi,
    },
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};
mod lifecycle;
#[derive(Clone)]
struct Recorded {
    path: String,
    headers: http::HeaderMap,
    body: Vec<u8>,
}
#[derive(Default)]
struct StateData {
    calls: Mutex<Vec<Recorded>>,
    replies: Mutex<HashMap<String, VecDeque<(u16, Value)>>>,
}
struct Mock {
    base: String,
    state: Arc<StateData>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Mock {
    async fn new() -> Self {
        let state = Arc::new(StateData::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        async fn handler(State(s): State<Arc<StateData>>, request: Request) -> Response {
            let path = request.uri().path().to_string();
            let headers = request.headers().clone();
            let body = axum::body::to_bytes(request.into_body(), 1024 * 1024)
                .await
                .unwrap()
                .to_vec();
            s.calls.lock().unwrap().push(Recorded {
                path: path.clone(),
                headers,
                body,
            });
            let(status,value)=s.replies.lock().unwrap().get_mut(&path).and_then(VecDeque::pop_front).unwrap_or((200,json!({"id":"ok","choices":[{"message":{"role":"assistant","content":"OK"},"finish_reason":"stop"}]})));
            (http::StatusCode::from_u16(status).unwrap(), Json(value)).into_response()
        }
        let router = Router::new()
            .fallback(any(handler))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self { base, state, task }
    }
    fn replies(&self, path: &str, values: Vec<(u16, Value)>) {
        self.state
            .replies
            .lock()
            .unwrap()
            .insert(path.into(), values.into());
    }
    fn calls(&self) -> Vec<Recorded> {
        self.state.calls.lock().unwrap().clone()
    }
}
fn cx(provider: &str, credential: Credential, preset: Option<Preset>) -> UpstreamCx {
    UpstreamCx {
        account: cranibox_llm::auth::signin::summary(provider, "local", None),
        secret: credential.secret(),
        preset,
        node: "local".into(),
    }
}
fn req(wire: Wire) -> CanonicalRequest {
    CanonicalRequest {
        wire,
        body: json!({"model":"model","messages":[{"role":"user","content":"Hello"}],"max_tokens":1}),
        model: "model".into(),
        stream: false,
        session_id: Some("session".into()),
        client_agent: None,
    }
}
fn token() -> Value {
    json!({"access_token":"fake-access","refresh_token":"fake-refresh","expires_in":3600})
}
fn no_input() -> SignInInput {
    SignInInput {
        callback_url: None,
        code: None,
    }
}
fn browser(start: SignInStart) -> (String, url::Url) {
    match start {
        SignInStart::Browser { flow_id, url } | SignInStart::PasteCallback { flow_id, url } => {
            (flow_id, url.parse().unwrap())
        }
        _ => panic!("browser expected"),
    }
}
fn callback(url: &url::Url, state: Option<&str>) -> String {
    let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
    let mut redirect: url::Url = pairs["redirect_uri"].parse().unwrap();
    redirect.query_pairs_mut().extend_pairs([
        ("code", "fake-code"),
        ("state", state.unwrap_or(&pairs["state"])),
    ]);
    redirect.into()
}
#[tokio::test]
async fn every_api_key_preset_uses_exact_endpoint_and_native_auth() {
    let mock = Mock::new().await;
    for mut preset in cranibox_llm::catalog::presets() {
        for wire in [Wire::Chat, Wire::Responses, Wire::Messages] {
            let field = match wire {
                Wire::Chat => &mut preset.chat,
                Wire::Responses => &mut preset.responses,
                Wire::Messages => &mut preset.anthropic,
            };
            let Some(original) = field.clone() else {
                continue;
            };
            let path = url::Url::parse(&original)
                .unwrap()
                .path()
                .trim_end_matches('/')
                .to_string();
            *field = Some(format!("{}{path}", mock.base));
            let mut context = cx(
                &preset.id,
                Credential {
                    access_token: "fake-key".into(),
                    ..Default::default()
                },
                Some(preset.clone()),
            );
            context.account.kind = AccountKind::ApiKey;
            let response = ApiKey.send(&context, req(wire)).await.unwrap();
            assert_eq!(response.status, 200);
            let call = mock.calls().pop().unwrap();
            let suffix = match wire {
                Wire::Chat => "/chat/completions",
                Wire::Responses => "/responses",
                Wire::Messages => {
                    if path.ends_with("/v1") {
                        "/messages"
                    } else {
                        "/v1/messages"
                    }
                }
            };
            assert_eq!(
                call.path,
                format!("{path}{suffix}"),
                "{} {wire:?}",
                preset.id
            );
            if preset.quirks.iter().any(|q| q == "mimo_api_key") {
                assert_eq!(call.headers["api-key"], "fake-key");
                assert!(!call.headers.contains_key("authorization"));
                assert!(!call.headers.contains_key("x-api-key"));
            } else if wire == Wire::Messages && preset.quirks.iter().any(|q| q == "messages_bearer")
            {
                assert_eq!(call.headers["authorization"], "Bearer fake-key");
                assert!(!call.headers.contains_key("x-api-key"));
            } else if wire == Wire::Messages {
                assert_eq!(call.headers["x-api-key"], "fake-key");
            } else {
                assert_eq!(call.headers["authorization"], "Bearer fake-key");
            }
            if wire == Wire::Messages {
                assert_eq!(call.headers["anthropic-version"], "2023-06-01");
            }
        }
    }
}
#[tokio::test]
async fn zhipu_envelope_and_1113_are_detected_and_endpoint_is_remembered() {
    let mock = Mock::new().await;
    let mut preset = cranibox_llm::catalog::preset("zhipu").unwrap();
    preset.chat = Some(format!("{}/api/paas/v4", mock.base));
    let context = cx(
        "zhipu",
        Credential {
            access_token: "fake".into(),
            ..Default::default()
        },
        Some(preset),
    );
    mock.replies(
        "/api/paas/v4/chat/completions",
        vec![(
            200,
            json!({"success":false,"error":{"code":"1113","message":"no balance"}}),
        )],
    );
    for _ in 0..2 {
        assert_eq!(
            ApiKey.send(&context, req(Wire::Chat)).await.unwrap().status,
            200
        );
    }
    assert_eq!(
        mock.calls()
            .iter()
            .map(|r| r.path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "/api/paas/v4/chat/completions",
            "/api/coding/paas/v4/chat/completions",
            "/api/coding/paas/v4/chat/completions"
        ]
    );
    mock.replies(
        "/api/coding/paas/v4/chat/completions",
        vec![
            (200, json!({"success":false,"code":"1004"})),
            (200, json!({"success":false,"code":1302})),
        ],
    );
    assert!(matches!(
        ApiKey.send(&context, req(Wire::Chat)).await,
        Err(UpstreamError::Auth)
    ));
    assert!(matches!(
        ApiKey.send(&context, req(Wire::Chat)).await,
        Err(UpstreamError::RateLimited { .. })
    ));
}
#[test]
fn schema_inference_preserves_instance_data_and_handles_nested_properties() {
    let preset = cranibox_llm::catalog::preset("kimi-code").unwrap();
    let mut body = json!({"model":"kimi-for-coding","temperature":0.3,"top_p":0.9,"tools":[{"name":"x","input_schema":{"properties":{"enum":{"enum":["a","b"]},"n":{"const":3},"obj":{"default":{"enum":[1]},"properties":{"b":{"enum":[true,false]}}}}}}]});
    cranibox_llm::upstream::cn::normalize(Some(&preset), &mut body).unwrap();
    let schema = &body["tools"][0]["input_schema"];
    assert_eq!(schema["properties"]["enum"]["type"], "string");
    assert_eq!(schema["properties"]["n"]["type"], "integer");
    assert_eq!(
        schema["properties"]["obj"]["properties"]["b"]["type"],
        "boolean"
    );
    assert!(schema["properties"]["obj"]["default"].get("type").is_none());
    assert!(body.get("temperature").is_none());
    assert!(body.get("top_p").is_none());
}
#[tokio::test]
async fn subscription_adapter_headers_and_payloads() {
    let mock = Mock::new().await;
    let adapter = Codex::for_loopback(mock.base.clone(), format!("{}/token", mock.base)).unwrap();
    let context = cx(
        "codex",
        Credential {
            access_token: "fake".into(),
            account_id: Some("account-id".into()),
            ..Default::default()
        },
        None,
    );
    adapter.send(&context, req(Wire::Responses)).await.unwrap();
    let call = mock.calls().pop().unwrap();
    assert_eq!(call.path, "/responses");
    assert_eq!(call.headers["chatgpt-account-id"], "account-id");
    let body: Value = serde_json::from_slice(&call.body).unwrap();
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    let adapter = Kimi::for_loopback(mock.base.clone(), format!("{}/token", mock.base)).unwrap();
    let context = cx(
        "kimi",
        Credential {
            access_token: "fake".into(),
            device_id: Some("fake-device".into()),
            ..Default::default()
        },
        None,
    );
    adapter.send(&context, req(Wire::Messages)).await.unwrap();
    let call = mock.calls().pop().unwrap();
    assert_eq!(call.path, "/v1/messages");
    assert_eq!(call.headers["authorization"], "Bearer fake");
    assert_eq!(
        call.headers
            .keys()
            .filter(|h| h.as_str().starts_with("x-msh-"))
            .count(),
        5
    );
    mock.replies("/v1/messages", vec![(402, json!({"error":"quota"}))]);
    assert!(matches!(
        adapter.send(&context, req(Wire::Messages)).await,
        Err(UpstreamError::QuotaExhausted { .. })
    ));
    let adapter = Gemini::for_loopback(mock.base.clone(), format!("{}/token", mock.base)).unwrap();
    let context = cx(
        "gemini",
        Credential {
            access_token: "fake".into(),
            project: Some("project".into()),
            ..Default::default()
        },
        None,
    );
    mock.replies("/v1internal:generateContent",vec![(200,json!({"response":{"candidates":[{"content":{"role":"model","parts":[{"text":"OK"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1}}}))]);
    let response = adapter.send(&context, req(Wire::Messages)).await.unwrap();
    let bytes = response.body.into_stream().next().await.unwrap().unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(value.to_string().contains("OK"));
    let call = mock.calls().pop().unwrap();
    let body: Value = serde_json::from_slice(&call.body).unwrap();
    assert_eq!(body["project"], "project");
    assert_eq!(body["model"], "model");
}
#[tokio::test]
async fn codex_pkce_callback_state_rotation_revocation_and_explicit_import() {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let mock = Mock::new().await;
    mock.replies(
        "/token",
        vec![
            (200, token()),
            (
                200,
                json!({"access_token":"rotated","refresh_token":"rotated-refresh","expires_in":60}),
            ),
            (400, json!({"error":"refresh_token_reused"})),
        ],
    );
    let store = Arc::new(MemoryAccountStore::default());
    let oauth = OAuth::new(
        "local".into(),
        store.clone(),
        OAuthConfig::for_loopback(&mock.base).unwrap(),
    );
    let (flow, url) = browser(
        oauth
            .start(
                "codex",
                SignInOpts {
                    region: None,
                    label: None,
                },
            )
            .await
            .unwrap(),
    );
    assert!(matches!(
        oauth
            .complete(
                &flow,
                SignInInput {
                    callback_url: Some(callback(&url, Some("wrong"))),
                    code: None
                }
            )
            .await,
        Err(MgmtError::Invalid(_))
    ));
    assert!(mock.calls().is_empty());
    let response = reqwest::get(callback(&url, None)).await.unwrap();
    assert_eq!(response.status(), 200);
    let account = oauth.complete(&flow, no_input()).await.unwrap();
    assert_eq!(
        oauth.complete(&flow, no_input()).await.unwrap().id,
        account.id
    );
    let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
    let calls = mock.calls();
    let form: HashMap<_, _> = url::form_urlencoded::parse(&calls[0].body)
        .into_owned()
        .collect();
    assert_eq!(pairs["code_challenge_method"], "S256");
    assert_eq!(
        pairs["code_challenge"],
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(form["code_verifier"].as_bytes()))
    );
    let secret = store.get_secret(&account.id).unwrap();
    let refreshed = cranibox_llm::auth::refresh(
        &secret,
        &format!("{}/token", mock.base),
        cranibox_llm::upstream::codex::CLIENT_ID,
        None,
    )
    .await
    .unwrap();
    let credential = Credential::decode(&refreshed.secret).unwrap();
    assert_eq!(credential.refresh_token.as_deref(), Some("rotated-refresh"));
    assert!(matches!(
        cranibox_llm::auth::refresh(
            &refreshed.secret,
            &format!("{}/token", mock.base),
            cranibox_llm::upstream::codex::CLIENT_ID,
            None
        )
        .await,
        Err(UpstreamError::Auth)
    ));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    std::fs::write(
        &path,
        json!({"tokens":{"access_token":"imported","refresh_token":"refresh"}}).to_string(),
    )
    .unwrap();
    assert!(cranibox_llm::auth::import_codex(store.as_ref(), "local", &path, false).is_err());
    assert!(cranibox_llm::auth::import_codex(store.as_ref(), "local", &path, true).is_ok());
}
#[tokio::test]
async fn codex_paste_callback_works_when_loopback_port_is_busy() {
    let mock = Mock::new().await;
    mock.replies("/token", vec![(200, token())]);
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = occupied.local_addr().unwrap().port();
    let store = Arc::new(MemoryAccountStore::default());
    let oauth = OAuth::new(
        "local".into(),
        store,
        OAuthConfig::for_loopback_with_callback_port(&mock.base, port).unwrap(),
    );
    let start = oauth
        .start(
            "codex",
            SignInOpts {
                region: None,
                label: None,
            },
        )
        .await
        .unwrap();
    assert!(matches!(start, SignInStart::PasteCallback { .. }));
    let (flow, url) = browser(start);
    assert!(
        oauth
            .complete(
                &flow,
                SignInInput {
                    callback_url: Some(callback(&url, None)),
                    code: None
                }
            )
            .await
            .is_ok()
    );
}
async fn start_device(mock: &Mock, oauth: &OAuth, provider: &str, expires: u64) -> String {
    mock.replies("/device",vec![(200,json!({"device_code":"fake-device","user_code":"TEST-ONLY","verification_uri":"https://example.com/verify","expires_in":expires,"interval":1}))]);
    match oauth
        .start(
            provider,
            SignInOpts {
                region: Some("cn".into()),
                label: None,
            },
        )
        .await
        .unwrap()
    {
        SignInStart::DeviceCode { flow_id, .. } => flow_id,
        _ => panic!("device code expected"),
    }
}
#[tokio::test]
async fn kimi_device_pending_slowdown_denied_expired_and_success() {
    let mock = Mock::new().await;
    let store = Arc::new(MemoryAccountStore::default());
    let oauth = OAuth::new(
        "local".into(),
        store.clone(),
        OAuthConfig::for_loopback(&mock.base).unwrap(),
    );
    mock.replies(
        "/token",
        vec![
            (400, json!({"error":"authorization_pending"})),
            (400, json!({"error":"slow_down"})),
            (200, token()),
        ],
    );
    let flow = start_device(&mock, &oauth, "kimi", 600).await;
    assert!(matches!(
        oauth.complete(&flow, no_input()).await,
        Err(MgmtError::Pending)
    ));
    assert_eq!(mock.calls().len(), 1);
    tokio::time::sleep(Duration::from_millis(1050)).await;
    assert!(matches!(
        oauth.complete(&flow, no_input()).await,
        Err(MgmtError::Pending)
    ));
    tokio::time::sleep(Duration::from_millis(1050)).await;
    assert!(matches!(
        oauth.complete(&flow, no_input()).await,
        Err(MgmtError::Pending)
    ));
    let count = mock.calls().len();
    tokio::time::sleep(Duration::from_millis(1050)).await;
    assert!(matches!(
        oauth.complete(&flow, no_input()).await,
        Err(MgmtError::Pending)
    ));
    assert_eq!(mock.calls().len(), count);
    tokio::time::sleep(Duration::from_millis(5050)).await;
    let account = oauth.complete(&flow, no_input()).await.unwrap();
    assert_eq!(account.provider, "kimi");
    assert_eq!(
        Credential::decode(&store.get_secret(&account.id).unwrap())
            .unwrap()
            .region
            .as_deref(),
        Some("cn")
    );
    for call in mock.calls() {
        assert_eq!(
            call.headers
                .keys()
                .filter(|h| h.as_str().starts_with("x-msh-"))
                .count(),
            5
        );
    }
    mock.replies("/token", vec![(400, json!({"error":"access_denied"}))]);
    let flow = start_device(&mock, &oauth, "kimi", 600).await;
    tokio::time::sleep(Duration::from_millis(1050)).await;
    assert!(matches!(
        oauth.complete(&flow, no_input()).await,
        Err(MgmtError::Upstream(UpstreamError::Auth))
    ));
    let flow = start_device(&mock, &oauth, "kimi", 0).await;
    assert!(matches!(
        oauth.complete(&flow, no_input()).await,
        Err(MgmtError::Invalid(_))
    ));
}
#[tokio::test]
async fn gemini_discovery_can_retry_without_reusing_authorization_code() {
    let mock = Mock::new().await;
    mock.replies("/token", vec![(200, token())]);
    mock.replies(
        "/v1internal:loadCodeAssist",
        vec![
            (503, json!({"error":"temporary"})),
            (200, json!({"cloudaicompanionProject":{"id":"project-id"}})),
        ],
    );
    let store = Arc::new(MemoryAccountStore::default());
    let oauth = OAuth::new(
        "local".into(),
        store.clone(),
        OAuthConfig::for_loopback(&mock.base).unwrap(),
    );
    let (flow, url) = browser(
        oauth
            .start(
                "gemini",
                SignInOpts {
                    region: None,
                    label: None,
                },
            )
            .await
            .unwrap(),
    );
    let input = SignInInput {
        callback_url: Some(callback(&url, None)),
        code: None,
    };
    assert!(oauth.complete(&flow, input).await.is_err());
    let account = oauth.complete(&flow, no_input()).await.unwrap();
    assert_eq!(
        mock.calls().iter().filter(|c| c.path == "/token").count(),
        1
    );
    assert_eq!(
        Credential::decode(&store.get_secret(&account.id).unwrap())
            .unwrap()
            .project
            .as_deref(),
        Some("project-id")
    );
}
#[tokio::test]
async fn copilot_login_exchanges_short_token_and_blocks_untrusted_endpoints() {
    let mock = Mock::new().await;
    mock.replies(
        "/token",
        vec![(
            200,
            json!({"access_token":"fake-github","token_type":"bearer"}),
        )],
    );
    mock.replies(
        "/copilot-token",
        vec![(
            200,
            json!({"token":"fake-copilot","expires_at":cranibox_llm::auth::now()+1200}),
        )],
    );
    let store = Arc::new(MemoryAccountStore::default());
    let oauth = OAuth::new(
        "local".into(),
        store.clone(),
        OAuthConfig::for_loopback(&mock.base).unwrap(),
    );
    let flow = start_device(&mock, &oauth, "copilot", 600).await;
    tokio::time::sleep(Duration::from_millis(1050)).await;
    let account = oauth.complete(&flow, no_input()).await.unwrap();
    let credential = Credential::decode(&store.get_secret(&account.id).unwrap()).unwrap();
    assert_eq!(credential.access_token, "fake-copilot");
    assert_eq!(credential.github_token.as_deref(), Some("fake-github"));
    let adapter =
        Copilot::for_loopback(mock.base.clone(), format!("{}/copilot-token", mock.base)).unwrap();
    let context = cx("copilot", credential, None);
    adapter.send(&context, req(Wire::Chat)).await.unwrap();
    assert_eq!(
        mock.calls().pop().unwrap().headers["authorization"],
        "Bearer fake-copilot"
    );
    for base in [
        "https://evil.example",
        "https://api.githubcopilot.com.evil.example",
        "https://api.githubcopilot.com:444",
        "http://api.githubcopilot.com",
    ] {
        let context = cx(
            "copilot",
            Credential {
                access_token: "must-not-leak".into(),
                endpoints: [(Wire::Chat, base.into())].into(),
                ..Default::default()
            },
            None,
        );
        assert!(matches!(
            Copilot::default().send(&context, req(Wire::Chat)).await,
            Err(UpstreamError::Auth)
        ));
        assert!(matches!(
            Copilot::default().models(&context).await,
            Err(UpstreamError::Auth)
        ));
    }
}
