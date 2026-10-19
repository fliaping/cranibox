use super::*;
use crate::local::LocalNode;
use agent_workspace_runtime::{
    peers::{Command as PeerCommand, config::Permissions},
    protocol::ExecRequest,
};
use std::path::Path;

async fn request(node: &LocalNode, value: Value) -> Value {
    node.manage_network(serde_json::from_value(value).unwrap())
        .await
        .unwrap()
}
async fn advertised(local: &LocalNode, remote: &str) -> Value {
    let mut latest = Value::Null;
    tokio::time::timeout(Duration::from_secs(40), async {
        loop {
            let value = request(local, json!({"operation":"status"})).await;
            assert!(
                !value.to_string().contains(&"SECRET".repeat(8)),
                "network status exposed its secret"
            );
            if value["easytier"]["devices"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["node"] == remote)
            {
                return value;
            }
            latest = value;
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("real EasyTier discovery timed out: {latest}"))
}
async fn probe(local: &LocalNode, remote: &str) {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if local
                .manage_peers(PeerCommand::Probe {
                    node: remote.into(),
                })
                .await
                .is_ok()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .expect("authorized EasyTier route did not recover")
}
fn provision(peers: &Path, source: &Path) {
    let dest = install::directory(peers);
    std::fs::create_dir_all(&dest).unwrap();
    for name in ["easytier-core", "easytier-cli"] {
        std::fs::copy(source.join(name), dest.join(name)).unwrap();
    }
}

/// Opt in with the two official v2.6.4 binaries. No TUN, Docker, system service or real user configuration.
#[cfg(unix)]
#[tokio::test]
#[ignore = "requires CRANIBOX_TEST_EASYTIER_DIR and permission to create network sockets"]
async fn real_two_node_network_authorization_and_restart() {
    let source = PathBuf::from(
        std::env::var_os("CRANIBOX_TEST_EASYTIER_DIR")
            .expect("set the official v2.6.4 binary directory"),
    );
    let root = tempfile::tempdir().unwrap();
    let a_path = root.path().join("a/peers.json");
    let b_path = root.path().join("b/peers.json");
    provision(&a_path, &source);
    provision(&b_path, &source);
    let a = LocalNode::start(a_path.clone()).await.unwrap();
    let b = LocalNode::start(b_path).await.unwrap();
    let seed = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = seed.local_addr().unwrap().port();
    drop(seed);
    let network = format!("test-{}", uuid::Uuid::new_v4().simple());
    let token = "SECRET".repeat(8);
    request(&a,json!({"operation":"easy_tier_join","network":network,"token":token,"peer":"","listen_port":port})).await;
    request(&b,json!({"operation":"easy_tier_join","network":network,"token":token,"peer":format!("tcp://127.0.0.1:{port}"),"listen_port":null})).await;
    advertised(&a, &b.id).await;
    advertised(&b, &a.id).await;
    let permissions = Permissions {
        exec: true,
        terminals: true,
        projects: true,
        desktop: false,
        computer: false,
        browser: false,
        messages: false,
        spaces: false,
    };
    let b_grant = b
        .manage_peers(PeerCommand::Grant {
            node: a.id.clone(),
            inbound: None,
            permissions: permissions.clone(),
        })
        .await
        .unwrap();
    let a_grant = a
        .manage_peers(PeerCommand::Grant {
            node: b.id.clone(),
            inbound: None,
            permissions,
        })
        .await
        .unwrap();
    // A syntactically valid but incorrect access token must not grant a network member access.
    request(
        &a,
        json!({"operation":"easy_tier_connect","node":b.id,"token":"0".repeat(64)}),
    )
    .await;
    assert!(
        a.manage_peers(PeerCommand::Probe { node: b.id.clone() })
            .await
            .is_err()
    );
    // Adding a provider path cannot silently rotate an existing device grant.
    let error = a
        .manage_network(
            serde_json::from_value(
                json!({"operation":"easy_tier_connect","node":b.id,"token":b_grant["token"]}),
            )
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("same target access token"));
    assert!(
        a.manage_peers(PeerCommand::Probe { node: b.id.clone() })
            .await
            .is_err()
    );
    a.manage_peers(PeerCommand::Forget { node: b.id.clone() })
        .await
        .unwrap();
    request(
        &a,
        json!({"operation":"easy_tier_connect","node":b.id,"token":b_grant["token"]}),
    )
    .await;
    request(
        &b,
        json!({"operation":"easy_tier_connect","node":a.id,"token":a_grant["token"]}),
    )
    .await;
    probe(&a, &b.id).await;
    probe(&b, &a.id).await;
    let job = a
        .manage_peers(PeerCommand::Exec {
            node: b.id.clone(),
            request: ExecRequest {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), "printf EASYTIER_REAL_COMMAND".into()],
                cwd: None,
                env: Default::default(),
                timeout_secs: 10,
            },
        })
        .await
        .unwrap();
    let job_id = job["id"].as_str().unwrap().to_owned();
    let session = a
        .manage_peers(PeerCommand::Sessions {
            node: b.id.clone(),
            command: json!({"operation":"create","name":"survives-provider-restart","cwd":null}),
        })
        .await
        .unwrap();
    let shell_id = session["session"]["id"].clone();
    let first_routes = a.manage_peers(PeerCommand::List).await.unwrap()["routes"].clone();
    let first_identity = a.id.clone();
    a.shutdown().await.unwrap();
    drop(a);
    let a = LocalNode::start(a_path).await.unwrap();
    assert_eq!(a.id, first_identity);
    advertised(&a, &b.id).await;
    advertised(&b, &a.id).await;
    probe(&a, &b.id).await;
    probe(&b, &a.id).await;
    assert_eq!(
        a.manage_peers(PeerCommand::List).await.unwrap()["routes"],
        first_routes
    );
    let output = a
        .manage_peers(PeerCommand::Job {
            node: b.id.clone(),
            id: job_id,
        })
        .await
        .unwrap();
    assert_eq!(output["stdout"], "EASYTIER_REAL_COMMAND");
    let sessions = a
        .manage_peers(PeerCommand::Sessions {
            node: b.id.clone(),
            command: json!({"operation":"list"}),
        })
        .await
        .unwrap();
    assert!(
        sessions["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == shell_id)
    );
    let stopped = request(&a, json!({"operation":"easy_tier_stop"})).await;
    assert_eq!(stopped["easytier"]["phase"], "stopped");
    assert!(
        a.manage_peers(PeerCommand::Probe { node: b.id.clone() })
            .await
            .is_err()
    );
    let sessions = b
        .sessions(agent_workspace_pty::engine::Manage::List)
        .await
        .unwrap();
    assert!(
        sessions["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == shell_id)
    );
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}

/// The original entry is offline before a leaf restarts; a different ordinary member admits it.
#[tokio::test]
#[ignore = "requires CRANIBOX_TEST_EASYTIER_DIR and permission to create network sockets"]
async fn real_three_node_network_entry_failover() {
    let source = PathBuf::from(
        std::env::var_os("CRANIBOX_TEST_EASYTIER_DIR")
            .expect("set the official v2.6.4 binary directory"),
    );
    let root = tempfile::tempdir().unwrap();
    let a_path = root.path().join("a/peers.json");
    let b_path = root.path().join("b/peers.json");
    let c_path = root.path().join("c/peers.json");
    for path in [&a_path, &b_path, &c_path] {
        provision(path, &source);
    }
    let a = LocalNode::start(a_path).await.unwrap();
    let b = LocalNode::start(b_path).await.unwrap();
    let c = LocalNode::start(c_path.clone()).await.unwrap();
    let first = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let second = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let a_port = first.local_addr().unwrap().port();
    let b_port = second.local_addr().unwrap().port();
    drop((first, second));
    let network = format!("test-{}", uuid::Uuid::new_v4().simple());
    let token = "SECRET".repeat(8);
    let entries = vec![
        format!("tcp://127.0.0.1:{a_port}"),
        format!("tcp://127.0.0.1:{b_port}"),
    ];
    request(&a, json!({"operation":"easy_tier_join","network":network,"token":token,"peers":[],"listen_port":a_port})).await;
    request(&b, json!({"operation":"easy_tier_join","network":network,"token":token,"peers":[entries[0]],"listen_port":b_port})).await;
    request(&c, json!({"operation":"easy_tier_join","network":network,"token":token,"peers":entries,"listen_port":null})).await;
    advertised(&b, &c.id).await;
    let status = advertised(&c, &b.id).await;
    assert_eq!(status["easytier"]["peers"], json!(entries));
    let grant = b
        .manage_peers(PeerCommand::Grant {
            node: c.id.clone(),
            inbound: None,
            permissions: Permissions {
                exec: true,
                terminals: true,
                projects: false,
                desktop: false,
                computer: false,
                browser: false,
                messages: false,
                spaces: false,
            },
        })
        .await
        .unwrap();
    request(
        &c,
        json!({"operation":"easy_tier_connect","node":b.id,"token":grant["token"]}),
    )
    .await;
    probe(&c, &b.id).await;
    let shell = c
        .manage_peers(PeerCommand::Sessions {
            node: b.id.clone(),
            command: json!({"operation":"create","name":"survives-entry-loss","cwd":null}),
        })
        .await
        .unwrap()["session"]["id"]
        .clone();
    let identity = c.id.clone();
    let routes = c.manage_peers(PeerCommand::List).await.unwrap()["routes"].clone();

    a.shutdown().await.unwrap();
    drop(a);
    // Restart removes all C's learned topology. It must join using B, despite A being first.
    c.shutdown().await.unwrap();
    drop(c);
    let c = LocalNode::start(c_path).await.unwrap();
    assert_eq!(c.id, identity);
    advertised(&c, &b.id).await;
    probe(&c, &b.id).await;
    assert_eq!(
        c.manage_peers(PeerCommand::List).await.unwrap()["routes"],
        routes
    );
    let sessions = c
        .manage_peers(PeerCommand::Sessions {
            node: b.id.clone(),
            command: json!({"operation":"list"}),
        })
        .await
        .unwrap();
    assert!(
        sessions["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == shell)
    );
    let job = c
        .manage_peers(PeerCommand::Exec {
            node: b.id.clone(),
            request: ExecRequest {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), "printf BACKUP_ENTRY_CONNECTED".into()],
                cwd: None,
                env: Default::default(),
                timeout_secs: 10,
            },
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let output = c
                .manage_peers(PeerCommand::Job {
                    node: b.id.clone(),
                    id: job["id"].as_str().unwrap().into(),
                })
                .await
                .unwrap();
            if output["stdout"] == "BACKUP_ENTRY_CONNECTED" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("command through the second entry did not complete");
    c.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}
