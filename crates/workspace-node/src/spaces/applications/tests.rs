use super::*;
use crate::spaces::{Catalog, DEFAULT_IMAGE, Provider, PullPolicy, Space};
use axum::{
    Json, Router,
    extract::State,
    routing::{any, get},
};
use serde_json::json;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn serve(router: Router) -> (Url, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    (
        url,
        Server(tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        })),
    )
}
fn proxy(target: Url) -> (tempfile::TempDir, Proxy) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().join("peers.json"));
    let owner = Config::update(&store.peers, |_| Ok(())).unwrap().node_id;
    let id = Uuid::new_v4();
    store
        .save(&Catalog {
            version: 1,
            installation: Uuid::new_v4(),
            setup: None,
            spaces: vec![Space {
                id,
                name: "Application test".into(),
                provider: Provider::Docker,
                image: DEFAULT_IMAGE.into(),
                pull_policy: PullPolicy::Never,
                node: "node-space".into(),
                owner: owner.clone(),
                password: "space-secret".into(),
                token: "space-token".into(),
                phase: "ready".into(),
                error: None,
                ports: Default::default(),
            }],
        })
        .unwrap();
    (
        directory,
        Proxy {
            store,
            id,
            application: Application::Desktop,
            target: Target {
                url: target,
                container: "owned-container".into(),
                owner,
                password: "space-secret".into(),
            },
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            cookies: Arc::default(),
        },
    )
}
async fn exchange(State(proxy): State<Proxy>, request: Request<Body>) -> Response {
    proxy
        .exchange(request, "/app/", CancellationToken::new())
        .await
        .unwrap()
}
#[tokio::test]
async fn http_strips_controller_credentials_preserves_app_cookies_and_rewrites_redirects() {
    let upstream = Router::new()
        .route(
            "/headers",
            get(|headers: HeaderMap| async move {
                Json(json!({
                    "authorization":headers.get("authorization").and_then(|v| v.to_str().ok()),
                    "cookie":headers.get("cookie").and_then(|v| v.to_str().ok()),
                    "forwarded":headers.get("x-forwarded-host").and_then(|v| v.to_str().ok()),
                    "origin":headers.get("origin").and_then(|v| v.to_str().ok()),
                }))
            }),
        )
        .route(
            "/cookie",
            get(|| async { [("set-cookie", "editor=private; HttpOnly; Path=/")] }),
        )
        .route(
            "/redirect",
            get(|| async { (StatusCode::FOUND, [("location", "/headers?next=%2F")]) }),
        );
    let (target, _upstream) = serve(upstream).await;
    let (_directory, proxy) = proxy(target.clone());
    let (gateway, _gateway) = serve(Router::new().fallback(any(exchange)).with_state(proxy)).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let cookie = client
        .get(gateway.join("app/cookie").unwrap())
        .send()
        .await
        .unwrap();
    assert!(cookie.headers().get("set-cookie").is_none());
    let value: serde_json::Value = client
        .get(gateway.join("app/headers").unwrap())
        .header("authorization", "Bearer controller-secret")
        .header("cookie", "aw_session=controller-secret")
        .header("x-forwarded-host", "attacker.test")
        .header("origin", gateway.origin().ascii_serialization())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    use base64::Engine;
    assert_eq!(
        value["authorization"],
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("agent:space-secret")
        )
    );
    assert_eq!(value["cookie"], "editor=private");
    assert!(value["forwarded"].is_null());
    assert_eq!(value["origin"], target.origin().ascii_serialization());
    let redirect = client
        .get(gateway.join("app/redirect").unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(redirect.headers()["location"], "/app/headers?next=%2F");
}
#[tokio::test]
async fn websocket_carries_binary_and_text_and_closes_when_space_stops() {
    let (target, _upstream) = serve(Router::new().route(
        "/socket",
        get(|ws: WebSocketUpgrade| async {
            ws.on_upgrade(|mut socket| async move {
                while let Some(Ok(message)) = socket.recv().await {
                    if socket.send(message).await.is_err() {
                        break;
                    }
                }
            })
        }),
    ))
    .await;
    let (_directory, proxy) = proxy(target);
    let store = proxy.store.clone();
    let (gateway, _gateway) = serve(Router::new().fallback(any(exchange)).with_state(proxy)).await;
    let mut address = gateway.join("app/socket").unwrap();
    address.set_scheme("ws").unwrap();
    let mut request = address.as_str().into_client_request().unwrap();
    request.headers_mut().insert(
        "sec-websocket-extensions",
        HeaderValue::from_static("permessage-deflate"),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    for message in [
        Frame::Text("hello".into()),
        Frame::Binary(vec![0, 1, 2, 255].into()),
    ] {
        socket.send(message.clone()).await.unwrap();
        assert_eq!(socket.next().await.unwrap().unwrap(), message);
    }
    let mut catalog = store.read().unwrap();
    catalog.spaces[0].phase = "stopped".into();
    store.save(&catalog).unwrap();
    let message = tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .unwrap();
    assert!(matches!(message, None | Some(Ok(Frame::Close(_)))));
}
#[tokio::test]
async fn tickets_are_one_use_scoped_expiring_and_revocable() {
    let (_directory, proxy) = proxy(Url::parse("http://127.0.0.1:1").unwrap());
    let hub = Hub::default();
    let cancel = hub.stop.child_token();
    hub.entries.lock().await.insert(
        "ticket".into(),
        Entry {
            owner: "session".into(),
            prefix: "/app/".into(),
            proxy,
            expires: Instant::now() + Duration::from_secs(30),
            claimed: false,
            cancel: cancel.clone(),
        },
    );
    assert!(hub.access("ticket", "/app/").await.is_err());
    assert!(hub.claim("ticket", "/other/").await.is_err());
    let (cookie, access) = hub.claim("ticket", "/app/").await.unwrap();
    assert_eq!(access.owner, "session");
    assert!(hub.claim("ticket", "/app/").await.is_err());
    assert!(hub.access(&cookie, "/other/").await.is_err());
    assert!(hub.access(&cookie, "/app/").await.is_ok());
    hub.entries.lock().await.get_mut(&cookie).unwrap().expires =
        Instant::now() - Duration::from_secs(1);
    assert!(hub.access(&cookie, "/app/").await.is_err());
    hub.retain(&Default::default()).await;
    assert!(cancel.is_cancelled());
    assert!(hub.access(&cookie, "/app/").await.is_err());
}
#[test]
fn application_paths_and_browser_cookies_are_scoped() {
    let prefix_text = "/cranibox-app-11111111-1111-4111-8111-111111111111-desktop/";
    assert_eq!(
        prefix(&format!("{prefix_text}websockets")),
        Some(prefix_text)
    );
    for path in [
        "/api/spaces",
        "/cranibox-app-invalid-desktop/",
        "/cranibox-app-11111111-1111-4111-8111-111111111111-shell/",
        "/cranibox-app-11111111-1111-4111-8111-111111111111-desktop",
    ] {
        assert!(prefix(path).is_none());
    }
    let response = claimed_response(prefix_text, "opaque-secret", true);
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], prefix_text);
    let cookie = response.headers()["set-cookie"].to_str().unwrap();
    for expected in ["HttpOnly", "SameSite=Strict", "Secure", prefix_text] {
        assert!(cookie.contains(expected));
    }
    assert!(
        !response.headers()["location"]
            .to_str()
            .unwrap()
            .contains("opaque-secret")
    );
}
