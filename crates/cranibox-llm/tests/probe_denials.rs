//! Background probes stop after capability_off while explicit operations stay usable.
use cranibox_contracts::{
    NodeId,
    llm::{CanonicalRequest, Wire},
    mock::{MemoryAccountStore, MockRoster},
    net::{BoxBody, PathInfo, PeerTransport, TransportError, TransportKind},
    roster::RosterView,
};
use cranibox_llm::route::{Router, remote::RemoteRouter};
use serde_json::json;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct Transport {
    denial: Mutex<Option<&'static str>>,
    calls: AtomicUsize,
}
#[async_trait::async_trait]
impl PeerTransport for Transport {
    fn kind(&self) -> TransportKind {
        TransportKind::Direct
    }
    async fn request(
        &self,
        _: &NodeId,
        request: http::Request<BoxBody>,
    ) -> Result<http::Response<BoxBody>, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let denial = *self.denial.lock().unwrap();
        let denied = request.method() == http::Method::GET && denial.is_some();
        Ok(http::Response::builder()
            .status(if denied { 403 } else { 200 })
            .body(BoxBody::full(if denied {
                json!({"error":{"code":denial.unwrap()}}).to_string()
            } else {
                "{\"input_tokens\":9}".to_owned()
            }))
            .unwrap())
    }
    async fn path(&self, _: &NodeId) -> Option<PathInfo> {
        None
    }
}
fn setup(code: &'static str) -> (RemoteRouter, Arc<Transport>, Arc<MockRoster>, String) {
    let roster = Arc::new(MockRoster::fixture());
    let hub = roster.hub().unwrap().node_id;
    let node = roster.current().unwrap().members[1].node_id.clone();
    let transport = Arc::new(Transport {
        denial: Mutex::new(Some(code)),
        calls: AtomicUsize::new(0),
    });
    (
        RemoteRouter::new(
            node,
            roster.clone(),
            Arc::new(MemoryAccountStore::default()),
            transport.clone(),
        ),
        transport,
        roster,
        hub,
    )
}
#[tokio::test]
async fn capability_off_stops_repeated_probes_until_roster_changes() {
    let (router, transport, roster, hub) = setup("capability_off");
    assert!(!router.probe(&hub).await);
    assert!(!router.probe(&hub).await);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    let mut next = roster.current().unwrap();
    next.epoch += 1;
    roster.set(Some(next));
    *transport.denial.lock().unwrap() = None;
    assert!(router.probe(&hub).await);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn approval_required_does_not_permanently_disable_probes() {
    let (router, transport, _, hub) = setup("approval_required");
    assert!(!router.probe(&hub).await);
    *transport.denial.lock().unwrap() = None;
    assert!(router.probe(&hub).await);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn explicit_operation_bypasses_denial_and_restores_probes() {
    let (router, transport, _, hub) = setup("capability_off");
    assert!(!router.probe(&hub).await);
    router
        .forward_count_tokens(
            &hub,
            "test",
            CanonicalRequest {
                wire: Wire::Messages,
                body: json!({"model":"model", "messages":[]}),
                stream: false,
                session_id: None,
                client_agent: None,
                model: "model".into(),
            },
        )
        .await
        .unwrap();
    *transport.denial.lock().unwrap() = None;
    assert!(router.probe(&hub).await);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 3);
}
