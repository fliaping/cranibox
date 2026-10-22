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
