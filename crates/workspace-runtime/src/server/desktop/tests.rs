use super::*;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

#[tokio::test]
async fn rendezvous_binds_target_ticket_and_generation_and_releases_cancelled_requests() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("runtime.json");
    let target_token = config::token();
    let other_token = config::token();
    let mut credentials = config::Credentials {
        controller_token: config::token(),
        nodes: [
            ("target".into(), target_token.clone()),
            ("other".into(), other_token.clone()),
        ]
        .into(),
        node_aliases: Default::default(),
        mesh_groups: Default::default(),
    };
    config::write_private(&path, &serde_json::to_vec(&credentials).unwrap(), false).unwrap();
    let gateway = Gateway::new_with_local_node(path.clone(), Some("host".into())).unwrap();
    let (sender, mut commands) = mpsc::channel(32);
    gateway.app.inner.lock().await.nodes.insert(
        "target".into(),
        Node {
            name: "Target".into(),
            os: "linux".into(),
            arch: "test".into(),
            terminal: true,
            terminal_sessions: true,
            desktop_relay: true,
            generation: "first".into(),
            tx: Some(sender),
            last_seen: now(),
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("ws://{}", listener.local_addr().unwrap());
    let router = gateway.router();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let begin = || {
        let gateway = gateway.clone();
        tokio::spawn(async move { gateway.desktop("target").await })
    };
    let pending = begin();
    let command = tokio::time::timeout(Duration::from_secs(3), commands.recv())
        .await
        .unwrap()
        .unwrap();
    let Wire::DesktopOpen { id, caller, ticket } = command else {
        panic!("expected desktop request");
    };
    assert_eq!(caller, "host");
    let request = |token: &str, nonce: &str| {
        let mut request = format!("{origin}/ws/desktops/{id}")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        request.headers_mut().insert(TICKET, nonce.parse().unwrap());
        request
    };
    assert!(
        Stream::connect_request(request(&other_token, &ticket))
            .await
            .is_err()
    );
    assert!(
        Stream::connect_request(request(&target_token, "wrong-ticket"))
            .await
            .is_err()
    );
    let mut browser = request(&target_token, &ticket);
    browser
        .headers_mut()
        .insert("origin", "http://localhost".parse().unwrap());
    assert!(Stream::connect_request(browser).await.is_err());
    gateway
        .app
        .inner
        .lock()
        .await
        .nodes
        .get_mut("target")
        .unwrap()
        .generation = "next".into();
    assert!(
        Stream::connect_request(request(&target_token, &ticket))
            .await
            .is_err()
    );
    gateway
        .app
        .inner
        .lock()
        .await
        .nodes
        .get_mut("target")
        .unwrap()
        .generation = "first".into();
    let mut target = Stream::connect_request(request(&target_token, &ticket))
        .await
        .unwrap();
    assert!(
        Stream::connect_request(request(&target_token, &ticket))
            .await
            .is_err()
    );
    let mut viewer = pending.await.unwrap().unwrap();
    target.send(b"RFB 003.008\n".to_vec()).unwrap();
    let data = tokio::time::timeout(Duration::from_secs(3), viewer.output.recv())
        .await
        .unwrap();
    assert!(matches!(data, Some(Event::Data(bytes)) if bytes == b"RFB 003.008\n"));
    viewer.send(vec![0, 255, 42]).unwrap();
    let data = tokio::time::timeout(Duration::from_secs(3), target.output.recv())
        .await
        .unwrap();
    assert!(matches!(data, Some(Event::Data(bytes)) if bytes == [0, 255, 42]));
    credentials.nodes.insert("target".into(), config::token());
    config::write_private(&path, &serde_json::to_vec(&credentials).unwrap(), true).unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), viewer.output.recv())
        .await
        .unwrap();
    assert!(matches!(event, None | Some(Event::Error(_))));
    assert!(
        matches!(tokio::time::timeout(Duration::from_secs(3), commands.recv()).await.unwrap(), Some(Wire::DesktopClose { id: closed }) if closed == id)
    );
    drop(viewer);
    drop(target);
    let pending = begin();
    let command = tokio::time::timeout(Duration::from_secs(3), commands.recv())
        .await
        .unwrap()
        .unwrap();
    let Wire::DesktopOpen { id, .. } = command else {
        panic!("expected desktop request");
    };
    pending.abort();
    let _ = pending.await;
    assert!(
        matches!(tokio::time::timeout(Duration::from_secs(3), commands.recv()).await.unwrap(), Some(Wire::DesktopClose { id: closed }) if closed == id)
    );
    assert!(gateway.app.inner.lock().await.desktops.is_empty());
    // The native API exposes the target's useful failure, including in Tauri.
    let api = crate::client::Api::new(crate::client::ApiOptions {
        url: origin.replacen("ws://", "http://", 1),
        token_file: None,
        credentials: path,
        allow_insecure: false,
    })
    .unwrap();
    let pending = tokio::spawn(async move { api.desktop("target").await });
    let command = tokio::time::timeout(Duration::from_secs(3), commands.recv())
        .await
        .unwrap()
        .unwrap();
    let Wire::DesktopOpen { id, .. } = command else {
        panic!("expected desktop request");
    };
    failed(
        &gateway.app,
        "target",
        "first",
        &id,
        "desktop capability not granted by target node".into(),
    )
    .await;
    let error = pending.await.unwrap().err().unwrap().to_string();
    assert!(
        error.contains("desktop capability not granted by target node"),
        "{error}"
    );
    server.abort();
}
