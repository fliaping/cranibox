use cranibox_contracts::{
    llm::mgmt::*, llm::*, mock::MemoryAccountStore, net::BoxBody, vault::AccountStore,
};
use cranibox_llm::{Config, mgmt::Client, upstream::apikey::Credential};
use futures_util::stream;
use serde_json::json;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
#[derive(Clone)]
enum Reply {
    Ok,
    Auth,
    Rate,
    Before,
    After,
    BadRequest,
}
struct Fake {
    calls: Mutex<Vec<String>>,
    reply: Mutex<HashMap<String, VecDeque<Reply>>>,
    refreshes: AtomicUsize,
}
impl Fake {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            reply: Mutex::new(HashMap::new()),
            refreshes: AtomicUsize::new(0),
        })
    }
}
#[async_trait::async_trait]
impl Upstream for Fake {
    fn provider(&self) -> &'static str {
        "test"
    }
    fn wires(&self) -> &[Wire] {
        &[Wire::Chat]
    }
    async fn send(
        &self,
        cx: &UpstreamCx,
        _req: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        self.calls.lock().unwrap().push(cx.account.id.clone());
        let mode = self
            .reply
            .lock()
            .unwrap()
            .get_mut(&cx.account.id)
            .and_then(VecDeque::pop_front)
            .unwrap_or(Reply::Ok);
        match mode {
            Reply::Auth => return Err(UpstreamError::Auth),
            Reply::Rate => {
                return Err(UpstreamError::RateLimited {
                    retry_after: Some(60),
                });
            }
            Reply::BadRequest => return Err(UpstreamError::Status(400, "invalid".into())),
            _ => {}
        }
        let body = match mode {
            Reply::Before => BoxBody::new(stream::once(async {
                Err(Box::new(std::io::Error::other("broken")) as cranibox_contracts::net::BodyError)
            })),
            Reply::After => BoxBody::new(
                async_stream::stream! {yield Ok(bytes::Bytes::from_static(b"first"));tokio::time::sleep(Duration::from_millis(100)).await;yield Err(Box::new(std::io::Error::other("broken")) as cranibox_contracts::net::BodyError);},
            ),
            _ => BoxBody::full(json!({"account":cx.account.id}).to_string()),
        };
        Ok(UpstreamResponse {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            wire: Wire::Chat,
            body,
        })
    }
    async fn refresh(
        &self,
        _cx: &UpstreamCx,
        secret: &SecretBox,
    ) -> Result<Refreshed, UpstreamError> {
        self.refreshes.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut c = Credential::decode(secret)?;
        c.access_token = "rotated-fake".into();
        c.refresh_token = Some("rotated-refresh".into());
        c.expires_at = Some(cranibox_llm::auth::now() + 3600);
        Ok(Refreshed {
            secret: c.secret(),
            expires_at: c.expires_at,
        })
    }
    async fn models(&self, _: &UpstreamCx) -> Result<Vec<ModelEntry>, UpstreamError> {
        Ok(vec![cranibox_llm::catalog::model(
            "test",
            "model",
            Wire::Chat,
        )])
    }
}
fn insert(store: &MemoryAccountStore, id: &str, expires: Option<u64>) {
    let a = AccountSummary {
        id: id.into(),
        provider: "test".into(),
        plan: None,
        label: id.into(),
        kind: AccountKind::OAuth,
        home_node: "local".into(),
        sync: true,
        enabled_on: vec!["local".into()],
        all_nodes: false,
        state: AccountState::Ready,
    };
    store
        .put(
            a,
            Some(
                Credential {
                    access_token: "fake".into(),
                    refresh_token: Some("fake-refresh".into()),
                    expires_at: expires,
                    ..Default::default()
                }
                .secret(),
            ),
        )
        .unwrap();
}
async fn start(
    store: Arc<MemoryAccountStore>,
    fake: Arc<Fake>,
) -> (tempfile::TempDir, cranibox_llm::Handle) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::new("local".into(), dir.path().join("run"), store);
    config.listen = vec!["127.0.0.1:0".parse().unwrap()];
    config.upstreams = vec![fake];
    let handle = cranibox_llm::spawn(config).await.unwrap();
    (dir, handle)
}
async fn request(address: std::net::SocketAddr) -> reqwest::Response {
    reqwest::Client::new().post(format!("http://{address}/v1/chat/completions")).header("x-session-id","sticky").json(&json!({"model":"test/model/with/slash","messages":[{"role":"user","content":"hello"}]})).send().await.unwrap()
}
#[tokio::test]
async fn loopback_access_matrix_and_private_management() {
    let store = Arc::new(MemoryAccountStore::default());
    let (_dir, handle) = start(store, Fake::new()).await;
    let client = reqwest::Client::new();
    let base = format!("http://{}", handle.addresses[0]);
    for header in [
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-forwarded-anything",
        "x-real-ip",
        "cf-connecting-ip",
        "true-client-ip",
        "tailscale-user-login",
        "tailscale-unknown",
        "origin",
        "sec-fetch-site",
        "sec-fetch-mode",
        "sec-fetch-unknown",
    ] {
        let response = client
            .get(format!("{base}/v1/models"))
            .header(header, "blocked")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 403, "header {header}");
    }
    for host in [
        "example.com",
        "127.0.0.1.evil",
        "localhost.evil",
        "localhost:1",
        "[::ffff:127.0.0.1]",
    ] {
        assert_eq!(
            client
                .get(format!("{base}/v1/models"))
                .header("host", host)
                .send()
                .await
                .unwrap()
                .status(),
            403,
            "host {host}"
        );
    }
    assert_eq!(
        client
            .get(format!("{base}/status"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        client
            .get(format!("{base}/v1/models"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let mgmt = Client::new(&handle.management);
    assert_eq!(mgmt.status().await.unwrap().mode, Mode::Offline);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&handle.management)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let path = handle.management.clone();
    handle.shutdown().await;
    #[cfg(unix)]
    assert!(!path.exists());
}
#[tokio::test]
async fn rejects_non_loopback_bind_before_start() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::new(
        "local".into(),
        dir.path().join("run"),
        Arc::new(MemoryAccountStore::default()),
    );
    config.listen = vec!["0.0.0.0:0".parse().unwrap()];
    assert!(cranibox_llm::spawn(config).await.is_err());
}
#[tokio::test]
async fn affinity_is_stable_for_ten_requests() {
    let store = Arc::new(MemoryAccountStore::default());
    for id in ["a", "b"] {
        insert(&store, id, None);
    }
    let fake = Fake::new();
    let (_dir, handle) = start(store, fake.clone()).await;
    for _ in 0..10 {
        assert_eq!(request(handle.addresses[0]).await.status(), 200);
    }
    let calls = fake.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 10);
    assert!(calls.iter().all(|id| id == &calls[0]));
    handle.shutdown().await;
}
#[tokio::test]
async fn switches_only_before_first_byte_and_for_allowed_errors() {
    for mode in [
        Reply::Auth,
        Reply::Rate,
        Reply::Before,
        Reply::After,
        Reply::BadRequest,
    ] {
        let store = Arc::new(MemoryAccountStore::default());
        for id in ["a", "b"] {
            insert(&store, id, None);
        }
        let order = cranibox_llm::route::local::Pool::default()
            .candidates(store.list(), "test", "local", "sticky")
            .unwrap();
        let fake = Fake::new();
        fake.reply
            .lock()
            .unwrap()
            .insert(order[0].id.clone(), vec![mode.clone(), mode.clone()].into());
        let (_dir, handle) = start(store, fake.clone()).await;
        let response = request(handle.addresses[0]).await;
        match mode {
            Reply::After => {
                assert_eq!(response.status(), 200);
                assert!(response.bytes().await.is_err());
                assert_eq!(fake.calls.lock().unwrap().len(), 1);
            }
            Reply::BadRequest => {
                assert_eq!(response.status(), 400);
                assert_eq!(fake.calls.lock().unwrap().len(), 1);
            }
            _ => {
                assert_eq!(response.status(), 200);
                assert_eq!(
                    response.json::<serde_json::Value>().await.unwrap()["account"],
                    order[1].id
                );
            }
        }
        handle.shutdown().await;
    }
}
#[tokio::test]
async fn refresh_single_flight_and_cancelled_http_preserve_rotation() {
    let store = Arc::new(MemoryAccountStore::default());
    insert(&store, "a", Some(0));
    let fake = Fake::new();
    let (_dir, handle) = start(store.clone(), fake.clone()).await;
    let address = handle.addresses[0];
    let caller = tokio::spawn(async move { request(address).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while fake.refreshes.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    caller.abort();
    let calls = (0..10).map(|_| request(address));
    let replies = futures_util::future::join_all(calls).await;
    assert!(replies.iter().all(|r| r.status() == 200));
    assert_eq!(fake.refreshes.load(Ordering::SeqCst), 1);
    let credential = Credential::decode(&store.get_secret("a").unwrap()).unwrap();
    assert_eq!(credential.refresh_token.as_deref(), Some("rotated-refresh"));
    handle.shutdown().await;
}
#[tokio::test]
async fn account_management_never_returns_keys_and_enforces_claude_scope() {
    let store = Arc::new(MemoryAccountStore::default());
    let (_dir, handle) = start(store.clone(), Fake::new()).await;
    let client = Client::new(&handle.management);
    let a = client
        .add_account(NewAccount::ApiKey {
            preset: "deepseek".into(),
            key: "fake-secret-no-leak".into(),
            label: None,
            region: Some("cn".into()),
        })
        .await
        .unwrap();
    assert!(
        !serde_json::to_string(&client.accounts().await.unwrap())
            .unwrap()
            .contains("fake-secret")
    );
    client
        .enable(
            &a.id,
            Enable {
                node: None,
                all_nodes: Some(true),
                on: true,
            },
        )
        .await
        .unwrap();
    assert!(
        client
            .enable(
                &a.id,
                Enable {
                    node: Some("other".into()),
                    all_nodes: None,
                    on: false
                }
            )
            .await
            .is_err()
    );
    let mut claude = a;
    claude.kind = AccountKind::ClaudeOAuth;
    claude.provider = "claude".into();
    claude.sync = false;
    claude.all_nodes = false;
    store.put(claude.clone(), None).unwrap();
    assert!(
        client
            .enable(
                &claude.id,
                Enable {
                    node: Some("other".into()),
                    all_nodes: None,
                    on: true
                }
            )
            .await
            .is_err()
    );
    assert!(
        client
            .enable(
                &claude.id,
                Enable {
                    node: None,
                    all_nodes: Some(true),
                    on: true
                }
            )
            .await
            .is_err()
    );
    client.delete_account(&claude.id).await.unwrap();
    assert!(store.get_secret(&claude.id).is_err());
    handle.shutdown().await;
}

#[tokio::test]
async fn curl_import_probe_and_minimax_hint_use_private_management() {
    use cranibox_llm::mgmt::tools::{ImportCurl, Probe, parse_curl};
    let command=ImportCurl{label:"我的接口".into(),curl:r#"curl 'https://example.com/v1/chat/completions' -H 'Authorization: Bearer fake-only' --data-raw '{"model":"vendor/model","messages":[]}'"#.into()};
    match parse_curl(&command).unwrap() {
        NewAccount::Custom {
            endpoints, models, ..
        } => {
            assert_eq!(endpoints[&Wire::Chat], "https://example.com/v1");
            assert_eq!(models, vec!["vendor/model"]);
        }
        _ => panic!("custom expected"),
    }
    for curl in [
        "curl https://example.com/v1/responses -d @/etc/passwd",
        "curl https://example.com/v1/responses --config bad",
        "curl http://example.com/v1/responses",
    ] {
        assert!(
            parse_curl(&ImportCurl {
                label: "test".into(),
                curl: curl.into()
            })
            .is_err()
        );
    }
    let store = Arc::new(MemoryAccountStore::default());
    insert(&store, "a", None);
    let (_dir, handle) = start(store, Fake::new()).await;
    let client = Client::new(&handle.management);
    assert!(client.import_curl(command).await.is_ok());
    let result = client
        .test_account(
            "a",
            Probe {
                model: Some("model".into()),
                wire: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(result.len(), 3);
    assert!(result[0].ok);
    assert_eq!(result[1].error.as_deref(), Some("unsupported"));
    let error = client
        .add_account(NewAccount::ApiKey {
            preset: "minimax-coding".into(),
            key: "sk-api-test-only".into(),
            label: None,
            region: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(error,MgmtError::Invalid(message) if message.contains("按量")));
    handle.shutdown().await;
}
#[test]
fn preset_overrides_are_validated_and_replace_by_id() {
    let dir = tempfile::tempdir().unwrap();
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/upstream/presets.toml"
    ))
    .unwrap();
    let first = text.split("[[preset]]").nth(1).unwrap();
    std::fs::write(
        dir.path().join("10-openai.toml"),
        format!(
            "[[preset]]{}",
            first.replace("https://api.openai.com/v1", "https://example.com/api")
        ),
    )
    .unwrap();
    let presets = cranibox_llm::catalog::load_overrides(dir.path()).unwrap();
    assert_eq!(
        presets
            .iter()
            .find(|p| p.id == "openai")
            .unwrap()
            .chat
            .as_deref(),
        Some("https://example.com/api")
    );
    std::fs::write(dir.path().join("20-bad.toml"), "[[preset]]\nid = 'bad'").unwrap();
    assert!(cranibox_llm::catalog::load_overrides(dir.path()).is_err());
}

struct FailsOnceStore {
    inner: MemoryAccountStore,
    fail: std::sync::atomic::AtomicBool,
}
impl AccountStore for FailsOnceStore {
    fn list(&self) -> Vec<AccountSummary> {
        self.inner.list()
    }
    fn get(&self, id: &str) -> Option<AccountSummary> {
        self.inner.get(id)
    }
    fn get_secret(&self, id: &str) -> Result<SecretBox, cranibox_contracts::vault::StoreError> {
        self.inner.get_secret(id)
    }
    fn put(
        &self,
        a: AccountSummary,
        s: Option<SecretBox>,
    ) -> Result<(), cranibox_contracts::vault::StoreError> {
        if s.as_ref()
            .is_some_and(|s| Credential::decode(s).is_ok_and(|c| c.access_token == "rotated-fake"))
            && self.fail.swap(false, Ordering::SeqCst)
        {
            return Err(cranibox_contracts::vault::StoreError::Other(
                "temporary fake write failure".into(),
            ));
        }
        self.inner.put(a, s)
    }
    fn remove(&self, id: &str) -> Result<(), cranibox_contracts::vault::StoreError> {
        self.inner.remove(id)
    }
}
#[tokio::test]
async fn rotated_token_survives_transient_vault_write_failure() {
    let store = Arc::new(FailsOnceStore {
        inner: MemoryAccountStore::default(),
        fail: std::sync::atomic::AtomicBool::new(true),
    });
    insert(&store.inner, "a", Some(0));
    let fake = Fake::new();
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::new("local".into(), dir.path().join("run"), store.clone());
    config.listen = vec!["127.0.0.1:0".parse().unwrap()];
    config.upstreams = vec![fake.clone()];
    let handle = cranibox_llm::spawn(config).await.unwrap();
    assert_eq!(request(handle.addresses[0]).await.status(), 503);
    assert_eq!(fake.refreshes.load(Ordering::SeqCst), 1);
    let client = Client::new(&handle.management);
    let result = client
        .test_account(
            "a",
            cranibox_llm::mgmt::tools::Probe {
                model: Some("model".into()),
                wire: Some(Wire::Chat),
            },
        )
        .await
        .unwrap();
    assert!(result[0].ok);
    assert_eq!(fake.refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(
        Credential::decode(&store.get_secret("a").unwrap())
            .unwrap()
            .refresh_token
            .as_deref(),
        Some("rotated-refresh")
    );
    handle.shutdown().await;
}
struct RemoteOnly;
#[async_trait::async_trait]
impl cranibox_llm::route::Router for RemoteOnly {
    async fn decide(&self, _: &str, _: &CanonicalRequest) -> cranibox_llm::route::RouteDecision {
        cranibox_llm::route::RouteDecision::Remote { node: "hub".into() }
    }
    async fn forward(
        &self,
        node: &str,
        provider: &str,
        _: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        assert_eq!(node, "hub");
        assert_eq!(provider, "test");
        Ok(UpstreamResponse {
            status: 200,
            headers: vec![],
            wire: Wire::Chat,
            body: BoxBody::full("forwarded"),
        })
    }
}
#[tokio::test]
async fn remote_routing_hook_and_local_dispatch_do_not_loop() {
    let store = Arc::new(MemoryAccountStore::default());
    insert(&store, "a", None);
    let fake = Fake::new();
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::new("local".into(), dir.path().join("run"), store);
    config.listen = vec!["127.0.0.1:0".parse().unwrap()];
    config.upstreams = vec![fake.clone()];
    config.routing = Some(Arc::new(RemoteOnly));
    let handle = cranibox_llm::spawn(config).await.unwrap();
    assert_eq!(
        request(handle.addresses[0]).await.text().await.unwrap(),
        "forwarded"
    );
    assert!(fake.calls.lock().unwrap().is_empty());
    handle
        .dispatch_local(
            "test",
            CanonicalRequest {
                wire: Wire::Chat,
                body: json!({"messages":[]}),
                stream: false,
                session_id: None,
                client_agent: None,
                model: "model".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(fake.calls.lock().unwrap().len(), 1);
    handle.shutdown().await;
}
