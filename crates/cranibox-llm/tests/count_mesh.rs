//! Count requests retain their operation across the authenticated mesh transport.
use cranibox_contracts::{
    NodeId,
    llm::*,
    mock::{MemoryAccountStore, MockRoster},
    net::{BoxBody, PathInfo, PeerTransport, TransportError, TransportKind, routes},
    roster::RosterView,
};
use cranibox_llm::route::{Router, health::Mode, remote::RemoteRouter};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU16, Ordering},
};

#[derive(Default)]
struct Transport {
    status: AtomicU16,
    calls: Mutex<Vec<(String, String, Value)>>,
}
#[async_trait::async_trait]
impl PeerTransport for Transport {
    fn kind(&self) -> TransportKind {
        TransportKind::Direct
    }
    async fn request(
        &self,
        peer: &NodeId,
        request: http::Request<BoxBody>,
    ) -> Result<http::Response<BoxBody>, TransportError> {
        assert_eq!(request.method(), http::Method::POST);
        assert_eq!(request.headers()["x-session-id"], "count-session");
        let path = request.uri().path().to_owned();
        let mut bytes = Vec::new();
        let mut stream = request.into_body().into_stream();
        while let Some(chunk) = stream.next().await {
            bytes.extend_from_slice(&chunk.unwrap());
        }
        self.calls.lock().unwrap().push((
            peer.clone(),
            path,
            serde_json::from_slice(&bytes).unwrap(),
        ));
        Ok(http::Response::builder()
            .status(self.status.load(Ordering::SeqCst))
            .body(BoxBody::full(json!({"input_tokens":9}).to_string()))
            .unwrap())
    }
    async fn path(&self, _: &NodeId) -> Option<PathInfo> {
        None
    }
}
fn request() -> CanonicalRequest {
    CanonicalRequest {
        wire: Wire::Messages,
        body: json!({"model":"model", "stream":true, "messages":[]}),
        stream: true,
        session_id: Some("count-session".into()),
        client_agent: None,
        model: "model".into(),
    }
}
fn router() -> (RemoteRouter, Arc<Transport>, NodeId) {
    let roster = Arc::new(MockRoster::fixture());
    let hub = roster.hub().unwrap().node_id;
    let node = roster.current().unwrap().members[1].node_id.clone();
    let transport = Arc::new(Transport {
        status: AtomicU16::new(200),
        ..Default::default()
    });
    (
        RemoteRouter::new(
            node,
            roster,
            Arc::new(MemoryAccountStore::default()),
            transport.clone(),
        ),
        transport,
        hub,
    )
}
#[tokio::test]
async fn count_uses_dedicated_path_and_unsupported_does_not_mark_peer_offline() {
    let (router, transport, hub) = router();
    let response = router
        .forward_count_tokens(&hub, "test", request())
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    let mut bytes = Vec::new();
    let mut stream = response.body.into_stream();
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!({"input_tokens":9})
    );
    transport.status.store(501, Ordering::SeqCst);
    for _ in 0..3 {
        assert!(matches!(
            router.forward_count_tokens(&hub, "test", request()).await,
            Err(UpstreamError::Unsupported)
        ));
    }
    assert_eq!(router.health.get(&hub).mode, Mode::Remote);
    let calls = transport.calls.lock().unwrap();
    assert_eq!(calls.len(), 4);
    for (peer, path, body) in calls.iter() {
        assert_eq!(peer, &hub);
        assert_eq!(path, &routes::llm(&hub, routes::LLM_COUNT_TOKENS));
        assert_eq!(body["model"], "test/model");
        assert!(body.get("stream").is_none());
    }
}
#[tokio::test]
async fn malformed_count_never_reaches_generation_or_transport() {
    let (router, transport, hub) = router();
    let mut invalid = request();
    invalid.wire = Wire::Chat;
    assert!(matches!(
        router.forward_count_tokens(&hub, "test", invalid).await,
        Err(UpstreamError::Unsupported)
    ));
    let mut invalid = request();
    invalid.body = json!([]);
    assert!(matches!(
        router.forward_count_tokens(&hub, "test", invalid).await,
        Err(UpstreamError::Unsupported)
    ));
    assert!(transport.calls.lock().unwrap().is_empty());
}
