use cranibox_contracts::{llm::*, mock::MemoryAccountStore, net::BoxBody, vault::AccountStore};
use cranibox_llm::{
    Config, Handle,
    route::{RouteDecision, Router},
    upstream::apikey::Credential,
};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct Counter {
    calls: Mutex<Vec<String>>,
    refreshes: AtomicUsize,
}
#[async_trait::async_trait]
impl Upstream for Counter {
    fn provider(&self) -> &'static str {
        "counter"
    }
    fn wires(&self) -> &[Wire] {
        &[Wire::Messages]
    }
    async fn send(
        &self,
        _: &UpstreamCx,
        _: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        panic!("counting must never generate a completion")
    }
    async fn count_tokens(
        &self,
        cx: &UpstreamCx,
        request: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        assert_eq!(request.wire, Wire::Messages);
        assert!(!request.stream);
        assert!(request.body.get("stream").is_none());
        assert_eq!(request.model, "model");
        let token = Credential::decode(&cx.secret)?.access_token;
        self.calls.lock().unwrap().push(token.clone());
        if token == "old" {
            return Err(UpstreamError::Auth);
        }
        Ok(UpstreamResponse {
            status: 200,
            headers: vec![],
            wire: Wire::Messages,
            body: BoxBody::full(json!({"input_tokens":7}).to_string()),
        })
    }
    async fn refresh(&self, _: &UpstreamCx, _: &SecretBox) -> Result<Refreshed, UpstreamError> {
        self.refreshes.fetch_add(1, Ordering::SeqCst);
        Ok(Refreshed {
            secret: Credential {
                access_token: "new".into(),
                ..Default::default()
            }
            .secret(),
            expires_at: None,
        })
    }
    async fn models(&self, _: &UpstreamCx) -> Result<Vec<ModelEntry>, UpstreamError> {
        Ok(vec![])
    }
}
struct Legacy;
#[async_trait::async_trait]
impl Upstream for Legacy {
    fn provider(&self) -> &'static str {
        "counter"
    }
    fn wires(&self) -> &[Wire] {
        &[Wire::Messages]
    }
    async fn send(
        &self,
        _: &UpstreamCx,
        _: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        panic!("legacy adapter must not generate to count")
    }
    async fn models(&self, _: &UpstreamCx) -> Result<Vec<ModelEntry>, UpstreamError> {
        Ok(vec![])
    }
}
fn store(token: &str) -> Arc<MemoryAccountStore> {
    let store = Arc::new(MemoryAccountStore::default());
    let account = cranibox_llm::auth::signin::summary("counter", "local", None);
    store
        .put(
            account,
            Some(
                Credential {
                    access_token: token.into(),
                    ..Default::default()
                }
                .secret(),
            ),
        )
        .unwrap();
    store
}
async fn start(
    store: Arc<MemoryAccountStore>,
    upstream: Arc<dyn Upstream>,
    router: Option<Arc<dyn Router>>,
) -> (tempfile::TempDir, Handle) {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new("local".into(), dir.path().join("run"), store);
    cfg.listen = vec!["127.0.0.1:0".parse().unwrap()];
    cfg.upstreams = vec![upstream];
    cfg.routing = router;
    let handle = cranibox_llm::spawn(cfg).await.unwrap();
    (dir, handle)
}
async fn count(handle: &Handle) -> reqwest::Response {
    reqwest::Client::new().post(format!("http://{}/v1/messages/count_tokens", handle.addresses[0]))
        .json(&json!({"model":"counter/model","stream":true,"messages":[{"role":"user","content":"hello"}]})).send().await.unwrap()
}
#[tokio::test]
async fn oauth_count_uses_count_operation_and_refreshes_after_auth_failure() {
    let store = store("old");
    let adapter = Arc::new(Counter::default());
    let (_dir, handle) = start(store.clone(), adapter.clone(), None).await;
    let response = count(&handle).await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"input_tokens":7})
    );
    assert_eq!(*adapter.calls.lock().unwrap(), ["old", "new"]);
    assert_eq!(adapter.refreshes.load(Ordering::SeqCst), 1);
    let id = &store.list()[0].id;
    assert_eq!(
        Credential::decode(&store.get_secret(id).unwrap())
            .unwrap()
            .access_token,
        "new"
    );
    handle.shutdown().await;
}
#[tokio::test]
async fn unsupported_oauth_count_returns_501_without_generating() {
    let (_dir, handle) = start(store("new"), Arc::new(Legacy), None).await;
    assert_eq!(count(&handle).await.status(), 501);
    handle.shutdown().await;
}
struct LegacyRouter;
#[async_trait::async_trait]
impl Router for LegacyRouter {
    async fn decide(&self, _: &str, _: &CanonicalRequest) -> RouteDecision {
        RouteDecision::Remote {
            node: "peer".into(),
        }
    }
    async fn forward(
        &self,
        _: &str,
        _: &str,
        _: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        panic!("count must not use generation forwarding")
    }
}
#[tokio::test]
async fn legacy_router_does_not_forward_counts_to_generation() {
    let (_dir, handle) = start(store("new"), Arc::new(Legacy), Some(Arc::new(LegacyRouter))).await;
    assert_eq!(count(&handle).await.status(), 501);
    handle.shutdown().await;
}
struct Bridge {
    peer: Arc<Handle>,
    calls: AtomicUsize,
}
#[async_trait::async_trait]
impl Router for Bridge {
    async fn decide(&self, _: &str, _: &CanonicalRequest) -> RouteDecision {
        RouteDecision::Remote {
            node: "peer".into(),
        }
    }
    async fn forward(
        &self,
        _: &str,
        _: &str,
        _: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        panic!("generation forwarding must not be used")
    }
    async fn forward_count_tokens(
        &self,
        node: &str,
        provider: &str,
        request: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        assert_eq!(node, "peer");
        assert_eq!(provider, "counter");
        assert!(!request.stream);
        assert!(request.body.get("stream").is_none());
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.peer
            .dispatch_count_local(provider, request)
            .await
            .map_err(|_| UpstreamError::Network("test peer dispatch failed".into()))
    }
}
#[tokio::test]
async fn remote_count_operation_and_local_dispatch_do_not_loop() {
    let counter = Arc::new(Counter::default());
    // The peer's router would send it back out if local dispatch did not bypass routing.
    let (_peer_dir, peer) =
        start(store("new"), counter.clone(), Some(Arc::new(LegacyRouter))).await;
    let bridge = Arc::new(Bridge {
        peer: Arc::new(peer),
        calls: AtomicUsize::new(0),
    });
    let (_dir, handle) = start(Arc::default(), Arc::new(Legacy), Some(bridge.clone())).await;
    let response = count(&handle).await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"input_tokens":7})
    );
    assert_eq!(bridge.calls.load(Ordering::SeqCst), 1);
    assert_eq!(*counter.calls.lock().unwrap(), ["new"]);
    handle.shutdown().await;
    let Bridge { peer, .. } = Arc::try_unwrap(bridge).ok().unwrap();
    Arc::try_unwrap(peer).ok().unwrap().shutdown().await;
}
