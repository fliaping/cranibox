//! Two nodes and the embedded relay on loopback, relay-only paths.
use axum::{Extension, Router, extract::WebSocketUpgrade, routing::get};
use cranibox_contracts::net::{
    Authenticated, BoxBody, PairPurpose, PathKind, PeerTransport, TransportKind,
};
use cranibox_net::{
    Allowlist, IrohNode, IrohTransport, Options, addr,
    mesh::{self, MeshTarget},
    pair::{FlowState, Outcome, PairHost, Pairing},
    relay::{self, Relay},
};
use futures_util::{SinkExt, StreamExt};
use iroh::EndpointId;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

async fn start_relay(allow: Allowlist) -> (String, Relay) {
    let relay = Relay::new(allow);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().route("/healthz", get(|| async { "ok" }));
    tokio::spawn(relay.clone().serve(listener, app, std::future::pending()));
    (url, relay)
}

async fn node(seed: u8, relay: &str) -> IrohNode {
    let node = IrohNode::bind(
        &[seed; 32],
        Options {
            relays: vec![relay.into()],
            n0_fallback: false,
            relay_only: true,
            no_address_lookup: true,
            ..Options::default()
        },
    )
    .await
    .unwrap();
    node.online(Duration::from_secs(15)).await.unwrap();
    node
}

fn mesh_router() -> Router {
    Router::new()
        .route(
            "/api/v1/mesh/nodes",
            get(|Extension(auth): Extension<Authenticated>| async move {
                axum::Json(
                    json!({"caller": auth.node_id, "transport": auth.transport, "e2e": auth.e2e}),
                )
            }),
        )
        .route(
            "/api/v1/mesh/sse",
            get(|| async {
                let stream = futures_util::stream::iter(
                    (0..3).map(|i| Ok::<_, std::convert::Infallible>(format!("data: {i}\n\n"))),
                )
                .then(|item| async {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    item
                });
                axum::response::Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(axum::body::Body::from_stream(stream))
                    .unwrap()
            }),
        )
        .route(
            "/api/v1/mesh/ws",
            get(|ws: WebSocketUpgrade| async move {
                ws.on_upgrade(|mut socket| async move {
                    while let Some(Ok(message)) = socket.recv().await {
                        if socket.send(message).await.is_err() {
                            break;
                        }
                    }
                })
            }),
        )
}

fn serve(node: &IrohNode, allow: Allowlist, target: MeshTarget, pairing: Option<Pairing>) {
    let endpoint = node.endpoint().clone();
    tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            let allow = allow.clone();
            let target = target.clone();
            let pairing = pairing.clone();
            tokio::spawn(async move {
                let Ok(conn) = incoming.await else { return };
                if conn.alpn() == cranibox_net::ALPN_PAIR {
                    if let Some(pairing) = pairing {
                        let _ = pairing.handle(conn).await;
                    }
                } else {
                    let _ = mesh::serve_connection(conn, &allow, target, CancellationToken::new())
                        .await;
                }
            });
        }
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn peer_http_over_embedded_relay_streams_upgrades_and_enforces_keys() {
    let allow = Allowlist::default();
    let (relay_url, _relay) = start_relay(allow.clone()).await;
    let a_id = iroh::SecretKey::from_bytes(&[1; 32]).public();
    let b_id = iroh::SecretKey::from_bytes(&[2; 32]).public();
    allow.set([a_id, b_id]);
    let a = node(1, &relay_url).await;
    let b = node(2, &relay_url).await;
    assert_eq!(a.id(), a_id, "the signing seed is the iroh SecretKey");
    let mesh_allow = Allowlist::new([b_id]);
    let resolve: mesh::Resolve =
        Arc::new(move |id: &EndpointId| (*id == b_id).then(|| "node-b".to_owned()));
    serve(
        &a,
        mesh_allow.clone(),
        MeshTarget::Router {
            router: mesh_router(),
            resolve,
        },
        None,
    );

    let a_addr = addr::endpoint_addr(&a.addr()).unwrap();
    let lookup_addr = a_addr.clone();
    let transport = IrohTransport::new(
        b.clone(),
        Arc::new(move |peer: &str| (peer == "node-a").then(|| lookup_addr.clone())),
    );
    let response = transport
        .request(
            &"node-a".into(),
            http::Request::get("/api/v1/mesh/nodes")
                .body(BoxBody::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = collect(response.into_body()).await;
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["caller"], "node-b");
    assert_eq!(value["transport"], json!(TransportKind::Iroh));
    assert_eq!(value["e2e"], true);
    let path = transport.path(&"node-a".into()).await.unwrap();
    assert_eq!(
        path.kind,
        PathKind::Relay,
        "relay-only nodes use the relay path"
    );

    // SSE stays streaming: chunks arrive separately.
    let response = transport
        .request(
            &"node-a".into(),
            http::Request::get("/api/v1/mesh/sse")
                .body(BoxBody::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let mut stream = response.into_body().into_stream();
    let mut chunks = vec![];
    while let Some(chunk) = stream.next().await {
        chunks.push(chunk.unwrap());
    }
    assert!(chunks.len() >= 2, "{chunks:?}");

    // WebSocket upgrade through a loopback forward (the out-of-process client path).
    let forward = cranibox_net::forward::Forward::bind(b.clone(), 0, a_addr.clone())
        .await
        .unwrap();
    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://{}/api/v1/mesh/ws", forward.address))
            .await
            .unwrap();
    ws.send(tokio_tungstenite::tungstenite::Message::text("ping"))
        .await
        .unwrap();
    let echoed = ws.next().await.unwrap().unwrap();
    assert_eq!(echoed.into_text().unwrap().as_str(), "ping");

    // Unpairing closes access at the next stream.
    mesh_allow.set([]);
    let result = transport
        .request(
            &"node-a".into(),
            http::Request::get("/api/v1/mesh/nodes")
                .body(BoxBody::empty())
                .unwrap(),
        )
        .await;
    assert!(result.is_err(), "unpaired caller must be refused");
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_refuses_unpaired_endpoints_and_self_check_explains_failures() {
    let allow = Allowlist::default();
    let (relay_url, _relay) = start_relay(allow.clone()).await;
    let paired = iroh::SecretKey::from_bytes(&[5; 32]).public();
    allow.set([paired]);
    let ok = node(5, &relay_url).await;
    let stranger = IrohNode::bind(
        &[6; 32],
        Options {
            relays: vec![relay_url.clone()],
            relay_only: true,
            no_address_lookup: true,
            ..Options::default()
        },
    )
    .await
    .unwrap();
    assert!(
        stranger.online(Duration::from_secs(4)).await.is_err(),
        "relay must deny unpaired keys"
    );
    assert!(ok.relays_connected().iter().any(|(_, c)| *c));

    assert_eq!(relay::self_check(&relay_url, &[]).await, relay::Check::Ok);
    // A server without the relay answers 404.
    let plain = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let plain_url = format!("http://{}", plain.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(plain, Router::new()).await });
    assert!(
        matches!(relay::self_check(&plain_url, &[]).await, relay::Check::Problem(p) if p.contains("404"))
    );
    assert!(
        matches!(relay::diagnose(400, "missing header: upgrade"), relay::Check::Problem(p) if p.contains("Upgrade"))
    );
    assert!(relay::ping(&relay_url, &[]).await);
}

#[derive(Default)]
struct Host {
    key: String,
    applied: Mutex<Vec<Outcome>>,
    /// Joiner: whether it takes the offered identity.
    joins: bool,
    joined: Mutex<Vec<Value>>,
    committed: Mutex<Vec<Value>>,
}
#[async_trait::async_trait]
impl PairHost for Host {
    fn identity(&self) -> Value {
        json!({"node_id": format!("node-{}", &self.key[..32]), "signing_key": self.key})
    }
    fn verify(&self, peer: &Value, endpoint: &EndpointId) -> anyhow::Result<(String, String)> {
        let key = peer["signing_key"].as_str().unwrap_or_default();
        anyhow::ensure!(
            key == addr::hex_id(endpoint),
            "identity does not match the TLS key"
        );
        let mut keys = [self.key.clone(), key.to_owned()];
        keys.sort();
        Ok((
            peer["node_id"].as_str().unwrap().into(),
            format!("{}-{}", &keys[0][..4], &keys[1][..4]),
        ))
    }
    async fn apply(&self, outcome: Outcome) -> anyhow::Result<()> {
        self.applied.lock().unwrap().push(outcome);
        Ok(())
    }
    async fn member(&self) -> Option<Value> {
        Some(json!({"node_id": format!("node-{}", &self.key[..32])}))
    }
    async fn prepare(&self, member: &Value, _peer: &Value) -> Option<Value> {
        Some(json!({"roster": [self.key.clone(), member["node_id"].clone()]}))
    }
    async fn commit(&self, bundle: &Value) -> anyhow::Result<()> {
        self.committed.lock().unwrap().push(bundle.clone());
        Ok(())
    }
    async fn join(&self, _inviter: &str, bundle: &Value) -> anyhow::Result<bool> {
        if self.joins {
            self.joined.lock().unwrap().push(bundle.clone());
        }
        Ok(self.joins)
    }
}

/// Pair two owner devices; the joiner takes the identity only when `joins`.
async fn owner_pairing(seed: u8, joins: bool) -> (Arc<Host>, Arc<Host>) {
    let allow = Allowlist::default();
    let (relay_url, relay) = start_relay(allow.clone()).await;
    let inviter_id = iroh::SecretKey::from_bytes(&[seed; 32]).public();
    allow.set([inviter_id]);
    let inviter = node(seed, &relay_url).await;
    let inviter_host = Arc::new(Host {
        key: addr::hex_id(&inviter_id),
        ..Host::default()
    });
    let pairing_a = Pairing::new(
        inviter.clone(),
        format!("node-{}", &addr::hex_id(&inviter_id)[..32]),
        inviter_host.clone(),
        Some(relay.clone()),
        None,
    );
    serve(
        &inviter,
        Allowlist::default(),
        MeshTarget::Tcp("127.0.0.1:9".parse().unwrap()),
        Some(pairing_a.clone()),
    );
    let joiner = IrohNode::bind(
        &[seed + 1; 32],
        Options {
            relays: vec![relay_url.clone()],
            relay_only: true,
            no_address_lookup: true,
            ..Options::default()
        },
    )
    .await
    .unwrap();
    let joiner_host = Arc::new(Host {
        key: addr::hex_id(&joiner.id()),
        joins,
        ..Host::default()
    });
    let pairing_b = Pairing::new(
        joiner.clone(),
        format!("node-{}", &addr::hex_id(&joiner.id())[..32]),
        joiner_host.clone(),
        None,
        None,
    );
    let invitation = pairing_a.invite(PairPurpose::OwnerDevice, None).unwrap();
    let flow_b = pairing_b.accept(&invitation.link).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let flow_a = pairing_a.flows().pop().unwrap();
    let (a, b) = tokio::join!(
        pairing_a.confirm(&flow_a.flow_id, true),
        pairing_b.confirm(&flow_b.flow_id, true)
    );
    assert_eq!(
        (a.unwrap(), b.unwrap()),
        (FlowState::Paired, FlowState::Paired)
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    (inviter_host, joiner_host)
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_pairing_adds_the_joiner_to_the_identity_only_after_it_joined() {
    let (inviter, joiner) = owner_pairing(21, true).await;
    let joined = joiner.joined.lock().unwrap().clone();
    assert_eq!(joined.len(), 1, "the joiner took the offered identity");
    assert_eq!(
        *inviter.committed.lock().unwrap(),
        joined,
        "and the inviter committed it"
    );
    // Joined during pairing: nothing left for the joiner's apply to do.
    assert!(joiner.applied.lock().unwrap()[0].identity.is_none());

    // A joiner that keeps its own identity leaves the inviter's unchanged.
    let (inviter, joiner) = owner_pairing(31, false).await;
    assert!(joiner.joined.lock().unwrap().is_empty());
    assert!(inviter.committed.lock().unwrap().is_empty());
    assert_eq!(
        inviter.applied.lock().unwrap().len(),
        1,
        "the devices are still paired"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn link_pairing_is_one_time_shows_the_same_code_and_commits_only_on_match() {
    let allow = Allowlist::default();
    let (relay_url, relay) = start_relay(allow.clone()).await;
    let inviter_id = iroh::SecretKey::from_bytes(&[11; 32]).public();
    allow.set([inviter_id]);
    let inviter = node(11, &relay_url).await;
    let inviter_host = Arc::new(Host {
        key: addr::hex_id(&inviter_id),
        ..Host::default()
    });
    let pairing_a = Pairing::new(
        inviter.clone(),
        format!("node-{}", &addr::hex_id(&inviter_id)[..32]),
        inviter_host.clone(),
        Some(relay.clone()),
        None,
    );
    serve(
        &inviter,
        Allowlist::default(),
        MeshTarget::Tcp("127.0.0.1:9".parse().unwrap()),
        Some(pairing_a.clone()),
    );

    // The joiner is not paired: only the ticket's relay token admits it to the Hub relay.
    let joiner = IrohNode::bind(
        &[12; 32],
        Options {
            relays: vec![relay_url.clone()],
            relay_only: true,
            no_address_lookup: true,
            ..Options::default()
        },
    )
    .await
    .unwrap();
    let joiner_id = joiner.id();
    let joiner_host = Arc::new(Host {
        key: addr::hex_id(&joiner_id),
        ..Host::default()
    });
    let pairing_b = Pairing::new(
        joiner.clone(),
        format!("node-{}", &addr::hex_id(&joiner_id)[..32]),
        joiner_host.clone(),
        None,
        None,
    );

    let invitation = pairing_a
        .invite(PairPurpose::OwnerDevice, Some("https://hub.example.com"))
        .unwrap();
    assert!(invitation.qr_svg.contains("<svg"));
    assert!(
        invitation
            .web_link
            .as_deref()
            .unwrap()
            .starts_with("https://hub.example.com/pair#")
    );
    let flow_b = pairing_b.accept(&invitation.link).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let flow_a = pairing_a.flows().pop().unwrap();
    assert_eq!(flow_a.sas, flow_b.sas, "both devices show the same code");
    assert_eq!(flow_a.state, FlowState::Comparing);

    // The link is single use.
    let reused = pairing_b.accept(&invitation.link).await;
    assert!(reused.unwrap_err().to_string().contains("already used"));

    // A ticket holder cannot authorize itself: each device must confirm locally.
    let joiner_confirmation = {
        let pairing = pairing_b.clone();
        let flow_id = flow_b.flow_id.clone();
        tokio::spawn(async move { pairing.confirm(&flow_id, true).await.unwrap() })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(inviter_host.applied.lock().unwrap().is_empty());
    assert!(joiner_host.applied.lock().unwrap().is_empty());
    assert!(!joiner_confirmation.is_finished());
    assert_eq!(
        pairing_a.flow(&flow_a.flow_id).unwrap().state,
        FlowState::Comparing
    );
    assert_eq!(
        pairing_b.flow(&flow_b.flow_id).unwrap().state,
        FlowState::Comparing
    );

    // Confirming on the other device releases both sides to exchange credentials.
    assert_eq!(
        pairing_a.confirm(&flow_a.flow_id, true).await.unwrap(),
        FlowState::Paired
    );
    assert_eq!(joiner_confirmation.await.unwrap(), FlowState::Paired);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let a_out = inviter_host.applied.lock().unwrap().pop().unwrap();
    let b_out = joiner_host.applied.lock().unwrap().pop().unwrap();
    assert_eq!(a_out.issued, b_out.received);
    assert_eq!(
        b_out.issued, a_out.received,
        "owner devices grant each other"
    );
    assert_eq!(a_out.peer_addr.endpoint_id, addr::hex_id(&joiner_id));

    // An inviter's confirmation must still allow the joiner to reject.
    let second = pairing_a.invite(PairPurpose::Guest, None).unwrap();
    let flow = pairing_b.accept(&second.link).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let inviter_flow = pairing_a
        .flows()
        .into_iter()
        .find(|f| f.state == FlowState::Comparing)
        .unwrap();
    let inviter_confirmation = {
        let pairing = pairing_a.clone();
        let flow_id = inviter_flow.flow_id.clone();
        tokio::spawn(async move { pairing.confirm(&flow_id, true).await.unwrap() })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(inviter_host.applied.lock().unwrap().is_empty());
    assert!(joiner_host.applied.lock().unwrap().is_empty());
    assert!(!inviter_confirmation.is_finished());
    assert_eq!(
        pairing_b.confirm(&flow.flow_id, false).await.unwrap(),
        FlowState::Rejected
    );
    assert_eq!(inviter_confirmation.await.unwrap(), FlowState::Rejected);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        pairing_b.flow(&flow.flow_id).unwrap().state,
        FlowState::Rejected
    );
    assert!(inviter_host.applied.lock().unwrap().is_empty());
    assert!(joiner_host.applied.lock().unwrap().is_empty());

    // Guests also require both decisions, and receive only the inviter's grant.
    let guest = pairing_a.invite(PairPurpose::Guest, None).unwrap();
    let guest_b = pairing_b.accept(&guest.link).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let guest_a = pairing_a
        .flows()
        .into_iter()
        .find(|f| f.state == FlowState::Comparing)
        .unwrap();
    let (a, b) = tokio::join!(
        pairing_a.confirm(&guest_a.flow_id, true),
        pairing_b.confirm(&guest_b.flow_id, true)
    );
    assert_eq!(a.unwrap(), FlowState::Paired);
    assert_eq!(b.unwrap(), FlowState::Paired);
    let a_out = inviter_host.applied.lock().unwrap().pop().unwrap();
    let b_out = joiner_host.applied.lock().unwrap().pop().unwrap();
    assert_eq!(a_out.issued, b_out.received);
    assert!(a_out.received.is_none());
    assert!(b_out.issued.is_none());

    // A forged secret is refused; an expired ticket never dials.
    let mut forged = second.ticket.clone();
    forged.secret = [0; 16];
    let link = cranibox_net::ticket::app_link(&forged).unwrap();
    assert!(pairing_b.accept(&link).await.is_err());
    let mut expired = second.ticket.clone();
    expired.expires_at = 1;
    let err = pairing_b
        .accept(&cranibox_net::ticket::app_link(&expired).unwrap())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("expired"));
}

async fn collect(body: BoxBody) -> Vec<u8> {
    let mut stream = body.into_stream();
    let mut out = vec![];
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk.unwrap());
    }
    out
}

/// Needs outbound HTTPS to the n0 relays: `cargo test -p cranibox-net -- --ignored`.
#[tokio::test]
#[ignore = "needs internet access to the n0 public relays"]
async fn n0_relays_take_over_while_the_own_relay_is_down() {
    let node = IrohNode::bind(
        &[9; 32],
        Options {
            // Nothing listens here: the own relay is down from the start.
            relays: vec!["https://127.0.0.1:9".into()],
            n0_fallback: true,
            relay_only: true,
            no_address_lookup: true,
            ..Options::default()
        },
    )
    .await
    .unwrap();
    let mut connected = Vec::new();
    for _ in 0..60 {
        connected = node
            .relays_connected()
            .into_iter()
            .filter(|(_, up)| *up)
            .map(|(url, _)| url)
            .collect::<Vec<_>>();
        if !connected.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(
        connected.iter().any(|u| u.contains("iroh.link")),
        "no n0 fallback relay: {connected:?}"
    );
    node.close().await;
}
