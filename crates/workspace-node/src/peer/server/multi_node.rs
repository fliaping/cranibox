//! Real TCP + Noise integration, using isolated stores and fake credentials only.
mod netns;
use super::*;
use agent_workspace_runtime::peers::{
    config::{Pinned, Route},
    roster::{OwnerKey, RosterStore},
};
use cranibox_contracts::{
    llm::{AccountKind, AccountState, AccountSummary, SecretBox},
    roster::{Role, RosterMember, RosterView},
    vault::{AccountStore, PullReq, StoreError},
};
use cranibox_llm::vault::{VaultKeys, sync::exchange};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn capture_forwarder(
    target: SocketAddr,
    stop: CancellationToken,
    capture: Arc<std::sync::Mutex<Vec<u8>>>,
) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let incoming = tokio::select! {
                _ = stop.cancelled() => break,
                incoming = listener.accept() => incoming,
            };
            let Ok((incoming, _)) = incoming else { break };
            let capture = capture.clone();
            let stop = stop.clone();
            tokio::spawn(async move {
                let Ok(outgoing) = tokio::net::TcpStream::connect(target).await else {
                    return;
                };
                let (mut client_read, mut client_write) = incoming.into_split();
                let (mut server_read, mut server_write) = outgoing.into_split();
                let upload = async {
                    let mut buffer = [0; 16384];
                    loop {
                        let size = client_read.read(&mut buffer).await?;
                        if size == 0 {
                            break;
                        }
                        capture.lock().unwrap().extend_from_slice(&buffer[..size]);
                        server_write.write_all(&buffer[..size]).await?;
                    }
                    server_write.shutdown().await
                };
                let download = tokio::io::copy(&mut server_read, &mut client_write);
                tokio::select! {
                    _ = stop.cancelled() => {},
                    _ = async { let _ = tokio::join!(upload, download); } => {},
                }
            });
        }
    });
    address
}

#[tokio::test]
async fn three_nodes_sync_over_noise_and_keep_local_account_after_hub_shutdown() {
    let root = tempfile::tempdir().unwrap();
    let stop = CancellationToken::new();
    let paths: Vec<_> = (0..3)
        .map(|n| root.path().join(format!("node-{n}/peers.json")))
        .collect();
    let mut identities = Vec::new();
    for path in &paths {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        Config::update(path, |c| {
            c.listener = Some(Listener {
                address: "127.0.0.1:0".parse().unwrap(),
                allow_insecure: false,
            });
            Ok(())
        })
        .unwrap();
        identities.push(Config::identity(path).unwrap());
    }
    let owner = OwnerKey::load_or_create(&root.path().join("owner.key")).unwrap();
    let members: Vec<_> = identities
        .iter()
        .enumerate()
        .map(|(n, identity)| {
            let public = identity.public();
            let keys = VaultKeys::new(
                public.node_id.clone(),
                public.signing_key.clone(),
                *identity.noise_private(),
                Arc::new(|_| String::new()),
            );
            RosterMember {
                node_id: public.node_id.clone(),
                signing_key: public.signing_key.clone(),
                x25519_pub: keys.encryption_public(),
                role: if n == 0 { Role::Hub } else { Role::Node },
                priority: n as u8,
                is_claude_node: n == 2,
                name: format!("Device {n}"),
                added_at: 0,
                identity: None,
            }
        })
        .collect();
    for (path, identity) in paths.iter().zip(&identities) {
        let dir = path.with_file_name("models");
        std::fs::create_dir_all(&dir).unwrap();
        agent_workspace_runtime::config::write_private(
            &dir.join("encryption.key"),
            identity.noise_private(),
            false,
        )
        .unwrap();
        let roster_path = dir.join("roster.json");
        RosterStore::pin(&roster_path, &owner.public()).unwrap();
        RosterStore::load(roster_path)
            .unwrap()
            .issue(&owner, members.clone())
            .unwrap();
    }
    let mut running = Vec::new();
    let mut models = Vec::new();
    let mut addresses = Vec::new();
    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    for path in &paths {
        let node = start_with_catalog(
            path.clone(),
            Engine::new(path.with_file_name("home")),
            Arc::new(Semaphore::new(1)),
            stop.clone(),
            agent_workspace_runtime::projects::Store::new(path.with_file_name("projects.json")),
        )
        .await
        .unwrap();
        let service = crate::models::Models::start_on(
            path.clone(),
            stop.clone(),
            Some(vec!["127.0.0.1:0".parse().unwrap()]),
        )
        .await
        .unwrap();
        assert!(node.app.models.set(service.clone()).is_ok());
        addresses.push(
            capture_forwarder(
                node.listener.as_ref().unwrap().address,
                stop.clone(),
                captured.clone(),
            )
            .await,
        );
        models.push(service);
        running.push(node);
    }
    for (i, path) in paths.iter().enumerate() {
        Config::update(path, |c| {
            for (j, identity) in identities.iter().enumerate() {
                if i == j {
                    continue;
                }
                let public = identity.public().clone();
                let mut pinned = Pinned::new(public.clone());
                pinned.verified = true;
                let secret = format!("{i:032x}{j:032x}");
                let mut grant = Grant::issue(
                    &format!("{j:032x}{i:032x}"),
                    super::super::config::Permissions {
                        llm: true,
                        vault: true,
                        ..Default::default()
                    },
                );
                grant.peer = Some(pinned.clone());
                c.grants.insert(public.node_id.clone(), grant);
                let token_file = path.with_file_name(format!("route-{j}.token"));
                agent_workspace_runtime::config::write_private(
                    &token_file,
                    secret.as_bytes(),
                    false,
                )?;
                c.routes.insert(
                    public.node_id,
                    Route {
                        url: format!("http://{}", addresses[j]),
                        token_file,
                        allow_insecure: false,
                        local_forward_lock: None,
                        alternatives: vec![],
                        target_identity: Some(pinned),
                        config_path: None,
                    },
                );
            }
            Ok(())
        })
        .unwrap();
        for identity in &identities {
            if identity.public().node_id != Config::read(path).unwrap().node_id {
                use crate::access::Capability::*;
                crate::access::allow_fixture(path, &identity.public().node_id, &[Llm, Vault]);
            }
        }
    }
    let a = &models[0];
    let b = &models[1];
    let c = &models[2];
    let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_address = upstream.local_addr().unwrap();
    let upstream_stop = stop.clone();
    let count_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let upstream_counts = count_calls.clone();
    tokio::spawn(async move {
        let app = Router::new().route("/v1/chat/completions", post(|| async {
            Json(json!({"id":"mock-noise", "object":"chat.completion", "model":"test", "choices":[{"index":0,"message":{"role":"assistant","content":"Noise round trip"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}))
        })).route("/v1/messages/count_tokens", post(move |Json(body): Json<Value>| {
            let calls = upstream_counts.clone();
            async move {
                assert_eq!(body["model"], "test");
                assert!(body.get("stream").is_none());
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Json(json!({"input_tokens":7}))
            }
        }));
        let _ = axum::serve(upstream, app)
            .with_graceful_shutdown(upstream_stop.cancelled_owned())
            .await;
    });
    let credential = cranibox_llm::upstream::apikey::Credential {
        access_token: "fake-vault-secret-for-noise-integration".into(),
        endpoints: [
            (
                cranibox_contracts::llm::Wire::Chat,
                format!("http://{upstream_address}/v1"),
            ),
            (
                cranibox_contracts::llm::Wire::Messages,
                format!("http://{upstream_address}/v1"),
            ),
        ]
        .into(),
        models: vec!["test".into()],
        ..Default::default()
    };
    let secret = credential.secret();
    a.vault
        .put(
            AccountSummary {
                id: "noise-account".into(),
                provider: "custom-noise".into(),
                plan: None,
                label: "Private account label".into(),
                kind: AccountKind::ApiKey,
                home_node: a.vault.node().into(),
                sync: true,
                enabled_on: vec![a.vault.node().into()],
                all_nodes: false,
                state: AccountState::Ready,
            },
            Some(SecretBox::new(secret.expose().to_vec())),
        )
        .unwrap();
    for peer in [b, c] {
        exchange(&a.vault, a.router.transport.as_ref(), peer.vault.node())
            .await
            .unwrap();
        assert_eq!(peer.vault.list().len(), 1);
        assert!(matches!(
            peer.vault.get_secret("noise-account"),
            Err(StoreError::NoSecret)
        ));
        assert!(peer.router.probe(a.vault.node()).await);
        assert_eq!(peer.router.candidates("custom-noise"), vec![a.vault.node()]);
    }
    let ask = |service: &Arc<crate::models::Models>| {
        reqwest::Client::new().post(format!("http://{}/v1/chat/completions", service.proxy.addresses[0]))
            .bearer_auth("cbx-pi")
            .json(&json!({"model":"custom-noise/test","messages":[{"role":"user","content":"Hello"}]}))
            .send()
    };
    let count = |service: &Arc<crate::models::Models>| {
        reqwest::Client::new().post(format!("http://{}/v1/messages/count_tokens", service.proxy.addresses[0]))
            .bearer_auth("cbx-pi")
            .json(&json!({"model":"custom-noise/test","stream":true,"messages":[{"role":"user","content":"private count request across Noise"}]}))
            .send()
    };
    // No local secret: this count must reach the Hub's dedicated count endpoint.
    let response = count(c).await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"input_tokens":7})
    );
    // An unsupported count must survive mesh ingress as 501, without making
    // the reachable Hub unhealthy or accidentally generating a completion.
    let mut generation_only = credential.clone();
    generation_only
        .endpoints
        .remove(&cranibox_contracts::llm::Wire::Messages);
    a.vault
        .put(
            a.vault.get("noise-account").unwrap(),
            Some(generation_only.secret()),
        )
        .unwrap();
    for _ in 0..3 {
        assert_eq!(
            count(c).await.unwrap().status(),
            reqwest::StatusCode::NOT_IMPLEMENTED
        );
    }
    assert_eq!(
        c.router.health.get(a.vault.node()).mode,
        cranibox_llm::route::health::Mode::Remote
    );
    a.vault
        .put(
            a.vault.get("noise-account").unwrap(),
            Some(credential.secret()),
        )
        .unwrap();
    let response = ask(c).await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.json::<Value>().await.unwrap()["choices"][0]["message"]["content"],
        "Noise round trip"
    );
    // The Hub's capability edits also advance the enablement version. Observe
    // those edits before enabling B, so background anti-entropy cannot replace
    // B's enablement with a newer Hub record on slower platforms.
    exchange(&a.vault, a.router.transport.as_ref(), c.vault.node())
        .await
        .unwrap();
    exchange(&a.vault, a.router.transport.as_ref(), b.vault.node())
        .await
        .unwrap();
    assert_eq!(
        b.vault.enablement("noise-account").unwrap(),
        a.vault.enablement("noise-account").unwrap()
    );
    let mut account = b.vault.get("noise-account").unwrap();
    account.enabled_on.push(b.vault.node().into());
    b.vault.put(account, None).unwrap();
    exchange(&b.vault, b.router.transport.as_ref(), c.vault.node())
        .await
        .unwrap();
    assert!(
        c.vault
            .get("noise-account")
            .unwrap()
            .enabled_on
            .contains(&b.vault.node().to_owned())
    );
    assert_eq!(
        b.vault.get_secret("noise-account").unwrap().expose(),
        secret.expose()
    );
    assert_eq!(b.router.candidates("custom-noise"), vec![b.vault.node()]);
    // The local listener and credentials survive loss of the Hub transport.
    running.remove(0).shutdown().await.unwrap();
    assert!(!b.router.probe(a.vault.node()).await);
    assert_eq!(b.router.candidates("custom-noise"), vec![b.vault.node()]);
    assert_eq!(
        b.vault.get_secret("noise-account").unwrap().expose(),
        secret.expose()
    );
    let response = ask(b).await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.json::<Value>().await.unwrap()["choices"][0]["message"]["content"],
        "Noise round trip"
    );
    // The same wrapped adapter also counts locally after the Hub disappears.
    let response = count(b).await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"input_tokens":7})
    );
    // A third node can fall back from the stopped Hub to the newly enabled peer.
    let response = count(c).await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"input_tokens":7})
    );
    assert_eq!(count_calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(b.router.candidates("claude"), vec![c.vault.node()]);
    assert!(!a.vault.pull(PullReq::default()).unwrap().records.is_empty());
    {
        let wire = captured.lock().unwrap();
        assert!(!wire.is_empty());
        let route_secret = format!("{:032x}{:032x}", 0, 1);
        for sensitive in [
            credential.access_token.as_bytes(),
            b"Private account label",
            b"private count request across Noise",
            route_secret.as_bytes(),
        ] {
            assert!(
                !wire.windows(sensitive.len()).any(|part| part == sensitive),
                "plaintext leaked on TCP"
            );
        }
    }
    assert!(b.roster.is_owner_node(c.vault.node()));
    stop.cancel();
    for node in running {
        node.shutdown().await.unwrap();
    }
}

// Exercise authenticated mesh ingress without binding sockets or starting providers.
mod access_policy {
    use super::*;
    use crate::access::{Capability, Mode, Store};
    use agent_workspace_runtime::peers::config::Permissions;
    use cranibox_contracts::net::{Authenticated, TransportKind};
    use tower::ServiceExt;

    struct Fixture {
        _root: tempfile::TempDir,
        running: Running,
        caller: String,
        key: [u8; 32],
        store: Store,
    }
    impl Fixture {
        async fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("target/peers.json");
            let other = root.path().join("caller/peers.json");
            for path in [&path, &other] {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                Config::update(path, |_| Ok(())).unwrap();
            }
            let running = start_with_catalog(
                path.clone(),
                Engine::new(root.path().join("home")),
                Arc::new(Semaphore::new(1)),
                CancellationToken::new(),
                agent_workspace_runtime::projects::Store::new(root.path().join("projects.json")),
            )
            .await
            .unwrap();
            let identities = [
                Config::identity(&path).unwrap(),
                Config::identity(&other).unwrap(),
            ];
            let caller = identities[1].public().clone();
            Config::update(&path, |c| {
                let mut grant = Grant::issue(
                    "synthetic-access-fixture",
                    super::super::super::config::Permissions {
                        terminals: true,
                        exec: true,
                        llm: true,
                        vault: true,
                        messages: true,
                        projects: true,
                        desktop: true,
                        computer: true,
                        browser: true,
                        spaces: true,
                        activity: true,
                    },
                );
                let mut pinned = Pinned::new(caller.clone());
                pinned.verified = true;
                grant.peer = Some(pinned);
                c.grants.insert(caller.node_id.clone(), grant);
                Ok(())
            })
            .unwrap();
            let owner = OwnerKey::load_or_create(&root.path().join("fixture-owner")).unwrap();
            let roster = path.with_file_name("models").join("roster.json");
            std::fs::create_dir_all(roster.parent().unwrap()).unwrap();
            RosterStore::pin(&roster, &owner.public()).unwrap();
            RosterStore::load(roster)
                .unwrap()
                .issue(
                    &owner,
                    identities
                        .iter()
                        .map(|i| {
                            let p = i.public();
                            RosterMember {
                                node_id: p.node_id.clone(),
                                signing_key: p.signing_key.clone(),
                                x25519_pub: p.noise_key.clone(),
                                role: Role::Node,
                                priority: 0,
                                is_claude_node: false,
                                name: "Access fixture".into(),
                                added_at: 0,
                                identity: None,
                            }
                        })
                        .collect(),
                )
                .unwrap();
            Self {
                store: Store::new(&path),
                caller: caller.node_id,
                key: agent_workspace_runtime::peers::identity::unhex(&caller.signing_key).unwrap(),
                running,
                _root: root,
            }
        }
        async fn call(&self, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
            let mut request = axum::http::Request::builder()
                .method(method)
                .uri(path)
                .header("x-cranibox-target", &self.running.app.node)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            request.extensions_mut().insert(Authenticated {
                node_id: self.caller.clone(),
                endpoint_id: Some(self.key),
                transport: TransportKind::Iroh,
                e2e: true,
            });
            let response = self.running.mesh_router().oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), 65536)
                .await
                .unwrap();
            (
                status,
                serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            )
        }
        async fn approve(&self, id: &str, scope: &str) {
            crate::access::api::route(
                &self.running.app.path,
                "POST",
                &format!("/access/requests/{id}"),
                "",
                json!({"decision":"approve", "scope":scope}),
            )
            .await
            .unwrap();
        }
        fn terminal(&self, session: &str) -> String {
            format!(
                "/api/v1/mesh/nodes/{}/terminal/ws?session={session}",
                self.running.app.node
            )
        }
        async fn socket(
            &self,
            session: &str,
        ) -> Result<
            tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream>,
            tokio_tungstenite::tungstenite::Error,
        > {
            use tokio_tungstenite::tungstenite::client::IntoClientRequest;
            let (client, server) = tokio::io::duplex(65536);
            let router = self.running.mesh_router();
            let authenticated = Authenticated {
                node_id: self.caller.clone(),
                endpoint_id: Some(self.key),
                transport: TransportKind::Iroh,
                e2e: true,
            };
            tokio::spawn(async move {
                let service = hyper::service::service_fn(
                    move |request: hyper::Request<hyper::body::Incoming>| {
                        let mut request = request.map(Body::new);
                        request.extensions_mut().insert(authenticated.clone());
                        router.clone().oneshot(request)
                    },
                );
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(hyper_util::rt::TokioIo::new(server), service)
                    .with_upgrades()
                    .await
                    .unwrap();
            });
            let mut request = format!("ws://access.fixture{}", self.terminal(session))
                .into_client_request()
                .unwrap();
            request
                .headers_mut()
                .insert("x-cranibox-target", self.running.app.node.parse().unwrap());
            tokio_tungstenite::client_async(request, client)
                .await
                .map(|(socket, _)| socket)
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_attaches_with_session_and_timed_approvals_without_network() {
        const TEST: &str = "peer::server::multi_node::access_policy::terminal_attaches_with_session_and_timed_approvals_without_network";
        if std::env::var_os("CRANIBOX_ACCESS_PTY_TEST").is_none() {
            use std::os::unix::fs::PermissionsExt;
            let root = tempfile::tempdir().unwrap();
            let shell = root.path().join("fixture-shell");
            // This fixture cannot read interactive shell startup files.
            std::fs::write(&shell, "#!/bin/sh\nexec /bin/cat\n").unwrap();
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
            let output = tokio::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--nocapture"])
                .env_clear()
                .env("CRANIBOX_ACCESS_PTY_TEST", "1")
                .env("SHELL", shell)
                .env("PATH", "/usr/bin:/bin")
                .env("TMPDIR", root.path())
                .kill_on_drop(true)
                .output()
                .await
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::{Error, Message};
        let f = Fixture::new().await;
        let home = f._root.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let mut ids = Vec::new();
        for name in ["first", "second", "third"] {
            let created = f
                .running
                .app
                .engine
                .manage(Manage::Create {
                    name: name.into(),
                    cwd: None,
                })
                .await
                .unwrap();
            ids.push(created["session"]["id"].as_str().unwrap().to_owned());
        }
        f.store
            .change(&f.caller, [(Capability::Terminals, Mode::Ask)].into())
            .unwrap();
        let Error::Http(response) = f.socket(&ids[0]).await.unwrap_err() else {
            panic!("HTTP rejection")
        };
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body: Value = serde_json::from_slice(response.body().as_ref().unwrap()).unwrap();
        assert_eq!(body["error"], "approval_required");
        f.approve(body["request_id"].as_str().unwrap(), "session")
            .await;
        let mut first = f.socket(&ids[0]).await.unwrap();
        first
            .send(Message::Text(
                json!({"type":"input","data":STANDARD.encode(b"access-terminal-ok\n")})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Message::Text(text) = first.next().await.unwrap().unwrap() {
                    let output: Value = serde_json::from_str(&text).unwrap();
                    if let Some(data) = output["data"].as_str() {
                        let bytes = STANDARD.decode(data).unwrap();
                        if String::from_utf8_lossy(&bytes).contains("access-terminal-ok") {
                            break;
                        }
                    }
                }
            }
        })
        .await
        .unwrap();
        let Error::Http(response) = f.socket(&ids[1]).await.unwrap_err() else {
            panic!("second session must ask")
        };
        let body: Value = serde_json::from_slice(response.body().as_ref().unwrap()).unwrap();
        assert_eq!(body["error"], "approval_required");
        f.approve(body["request_id"].as_str().unwrap(), "30m").await;
        let mut second = f.socket(&ids[1]).await.unwrap();
        let mut third = f.socket(&ids[2]).await.unwrap();
        assert_eq!(f.store.view().unwrap().sessions.len(), 3);
        f.store.revoke(None, true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            for socket in [&mut first, &mut second, &mut third] {
                while let Some(Ok(message)) = socket.next().await {
                    if matches!(message, Message::Close(_)) {
                        break;
                    }
                }
            }
        })
        .await
        .unwrap();
        for id in ids {
            f.running
                .app
                .engine
                .manage(Manage::End { id })
                .await
                .unwrap();
        }
        f.running.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn default_off_and_nested_routes_are_enforced() {
        let f = Fixture::new().await;
        let (status, body) = f.call("GET", &f.terminal("one"), Value::Null).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "capability_off");
        assert!(body["hint"].as_str().unwrap().contains("Settings > Access"));
        for capability in ["llm/health", "vault/roster", "update"] {
            let (status, body) = f
                .call(
                    "GET",
                    &format!("/api/v1/mesh/nodes/{}/{capability}", f.running.app.node),
                    Value::Null,
                )
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
            assert_eq!(body["error"], "capability_off");
        }
        for (method, route) in [
            ("POST", "exec"),
            ("POST", "executions"),
            ("POST", "files/read"),
            ("POST", "projects"),
            ("POST", "projects/export"),
            ("POST", "projects/sync"),
            ("GET", "desktop/ws"),
            ("GET", "desktop/page"),
            ("POST", "computer"),
            ("POST", "browser"),
            ("GET", "browser/ws"),
            ("GET", "activity"),
            ("GET", "spaces"),
            ("POST", "update"),
            ("POST", "update/rollback"),
            ("GET", "update/job-id"),
        ] {
            let (status, body) = f
                .call(
                    method,
                    &format!("/api/v1/mesh/nodes/{}/{route}", f.running.app.node),
                    Value::Null,
                )
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{route}: {body}");
            assert_eq!(body["error"], "capability_off", "{route}: {body}");
        }
        f.store
            .change(&f.caller, [(Capability::Projects, Mode::Allow)].into())
            .unwrap();
        let (status, body) = f
            .call(
                "POST",
                &format!("/api/v1/mesh/nodes/{}/projects", f.running.app.node),
                json!({"operation":"bind","replica_id":"replica","terminal_id":"terminal"}),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["capability"], "terminals");
        f.store
            .change(&f.caller, [(Capability::Messages, Mode::Off)].into())
            .unwrap();
        for route in ["messages", "tasks"] {
            assert_eq!(
                f.call(
                    "POST",
                    &format!("/api/v1/mesh/nodes/{}/{route}", f.running.app.node),
                    Value::Null
                )
                .await
                .1["error"],
                "capability_off"
            );
        }
        f.store
            .change(&f.caller, [(Capability::Terminals, Mode::Allow)].into())
            .unwrap();
        Config::update(&f.running.app.path, |c| {
            c.grants.get_mut(&f.caller).unwrap().permissions.terminals = false;
            Ok(())
        })
        .unwrap();
        let (status, body) = f.call("GET", &f.terminal("one"), Value::Null).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "capability not granted by target node");
        f.running.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn exec_session_approval_covers_job_polling_but_not_another_execution() {
        let f = Fixture::new().await;
        f.store
            .change(&f.caller, [(Capability::Exec, Mode::Ask)].into())
            .unwrap();
        let path = format!("/api/v1/mesh/nodes/{}/exec", f.running.app.node);
        let command = json!({"program":"/bin/echo", "args":["access-exec-ok"]});
        let (_, pending) = f.call("POST", &path, command.clone()).await;
        f.approve(pending["request_id"].as_str().unwrap(), "session")
            .await;
        let (status, job) = f.call("POST", &path, command.clone()).await;
        assert_eq!(status, StatusCode::OK, "{job}");
        let job_path = format!("/api/v1/mesh/jobs/{}", job["id"].as_str().unwrap());
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let (status, job) = f.call("GET", &job_path, Value::Null).await;
                assert_eq!(status, StatusCode::OK, "{job}");
                if job["outcome"] != "running" {
                    assert_eq!(job["stdout"], "access-exec-ok\n");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            while !f.store.view().unwrap().sessions.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let (status, result) = f.call("GET", &job_path, Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{result}");
        let (status, pending) = f.call("POST", &path, command).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(pending["error"], "approval_required");
        f.store.revoke(None, true).unwrap();
        assert_eq!(
            f.call("GET", &job_path, Value::Null).await.0,
            StatusCode::FORBIDDEN
        );
        f.running.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn session_approval_polling_cooldown_and_emergency_revoke() {
        let f = Fixture::new().await;
        f.store
            .change(&f.caller, [(Capability::Terminals, Mode::Ask)].into())
            .unwrap();
        let (_, first) = f.call("GET", &f.terminal("one"), Value::Null).await;
        assert_eq!(first["error"], "approval_required");
        let id = first["request_id"].as_str().unwrap();
        let (_, pending) = f
            .call(
                "GET",
                &format!("/api/v1/mesh/access/requests/{id}"),
                Value::Null,
            )
            .await;
        assert_eq!(pending["state"], "pending");
        assert!(f.store.request(id, "another-requester").unwrap().is_none());
        let third_path = f._root.path().join("third/peers.json");
        std::fs::create_dir_all(third_path.parent().unwrap()).unwrap();
        Config::update(&third_path, |_| Ok(())).unwrap();
        let third = Config::identity(&third_path).unwrap().public().clone();
        Config::update(&f.running.app.path, |c| {
            let mut grant = Grant::issue(
                "third-fixture",
                Permissions {
                    messages: true,
                    ..Default::default()
                },
            );
            let mut pinned = Pinned::new(third.clone());
            pinned.verified = true;
            grant.peer = Some(pinned);
            c.grants.insert(third.node_id.clone(), grant);
            Ok(())
        })
        .unwrap();
        let mut request = axum::http::Request::get(format!("/api/v1/mesh/access/requests/{id}"))
            .header("x-cranibox-target", &f.running.app.node)
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(Authenticated {
            node_id: third.node_id,
            endpoint_id: Some(
                agent_workspace_runtime::peers::identity::unhex(&third.signing_key).unwrap(),
            ),
            transport: TransportKind::Iroh,
            e2e: true,
        });
        assert_eq!(
            f.running
                .mesh_router()
                .oneshot(request)
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        f.approve(id, "session").await;
        let session = f
            .store
            .start(&f.caller, Capability::Terminals, "one")
            .unwrap();
        assert!(session.valid());
        let (_, second) = f.call("GET", &f.terminal("two"), Value::Null).await;
        assert_eq!(second["error"], "approval_required");
        assert_ne!(first["request_id"], second["request_id"]);
        let second_id = second["request_id"].as_str().unwrap();
        crate::access::api::route(
            &f.running.app.path,
            "POST",
            &format!("/access/requests/{second_id}"),
            "",
            json!({"decision":"deny"}),
        )
        .await
        .unwrap();
        let (_, denied) = f.call("GET", &f.terminal("two"), Value::Null).await;
        assert_eq!(denied["error"], "approval_cooldown");
        computer::owner(
            &f.running.app,
            agent_workspace_runtime::peers::computer::OwnerCommand::EmergencyStop {},
        )
        .await
        .unwrap();
        assert!(!session.valid());
        assert!(f.store.view().unwrap().approvals.is_empty());
        assert!(
            f.store
                .audit(100, None, None, None)
                .unwrap()
                .entries
                .iter()
                .any(|e| e.event == crate::access::EventKind::EmergencyDisconnect)
        );
        f.running.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn timed_approval_allows_second_session_and_mesh_cannot_approve() {
        let f = Fixture::new().await;
        f.store
            .change(&f.caller, [(Capability::Terminals, Mode::Ask)].into())
            .unwrap();
        let (_, body) = f.call("GET", &f.terminal("one"), Value::Null).await;
        let id = body["request_id"].as_str().unwrap();
        for (method, path, body) in [
            ("GET", "/api/v1/app/access".into(), Value::Null),
            (
                "PUT",
                format!("/api/v1/app/access/{}", f.caller),
                json!({"terminals":"allow"}),
            ),
            (
                "POST",
                format!("/api/v1/app/access/requests/{id}"),
                json!({"decision":"approve","scope":"8h"}),
            ),
        ] {
            assert_eq!(f.call(method, &path, body).await.0, StatusCode::FORBIDDEN);
        }
        f.approve(id, "30m").await;
        let first = f
            .store
            .start(&f.caller, Capability::Terminals, "one")
            .unwrap();
        let second = f
            .store
            .start(&f.caller, Capability::Terminals, "two")
            .unwrap();
        assert!(first.valid() && second.valid());
        let (_, approved) = f
            .call(
                "GET",
                &format!("/api/v1/mesh/access/requests/{id}"),
                Value::Null,
            )
            .await;
        assert_eq!(approved["state"], "approved");
        f.running.shutdown().await.unwrap();
    }
}
