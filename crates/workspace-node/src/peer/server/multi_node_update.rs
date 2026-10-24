//! Remote updates over real TCP + Noise, with owner membership independent of grants.
use super::*;
use crate::update::{
    layout::{Layout, PointerState},
    npm,
    version::Version,
};
use agent_workspace_runtime::{
    client::Fleet,
    peers::{
        config::{Permissions, Pinned, Route},
        roster::{OwnerKey, RosterStore},
    },
};
use cranibox_contracts::roster::{Role, RosterMember, RosterView};
use cranibox_llm::vault::VaultKeys;
use flate2::{Compression, write::GzEncoder};
use sha2::{Digest, Sha512};
use std::{fs, path::Path as FsPath};

const TEST: &str = "peer::server::multi_node_update::owner_updates_and_rolls_back_over_noise";
const CHILD_ROOT: &str = "CRANIBOX_UPDATE_TEST_ROOT";
const SCRIPT: &[u8] = b"#!/bin/sh\n# runtime 1.0.1\nexit 0\n";

/// The runtime payload. macOS refuses unsigned code (`platform::verify_runtime` runs
/// `codesign --verify`), so there a copy of a signed system binary stands in for the script.
fn payload() -> Vec<u8> {
    #[cfg(target_os = "macos")]
    return fs::read("/usr/bin/true").unwrap();
    #[cfg(not(target_os = "macos"))]
    SCRIPT.to_vec()
}

fn tarball(package: &str) -> Vec<u8> {
    let gzip = GzEncoder::new(Vec::new(), Compression::default());
    let mut archive = tar::Builder::new(gzip);
    let manifest = serde_json::to_vec(&json!({
        "name": package, "version": "1.0.1"
    }))
    .unwrap();
    for (path, bytes, mode) in [
        ("package/bin/cranibox", payload().as_slice(), 0o755),
        ("package/package.json", manifest.as_slice(), 0o644),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        archive.append_data(&mut header, path, bytes).unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap()
}

fn assert_api_error(result: Result<Value>, status: StatusCode, message: &str) {
    let error = result.unwrap_err();
    assert!(
        error.chain().any(|cause| {
            let text = cause.to_string();
            text.starts_with(&format!("API {}:", status.as_u16())) && text.contains(message)
        }),
        "expected {status} containing {message:?}, got {error:#}"
    );
}

#[tokio::test]
async fn owner_updates_and_rolls_back_over_noise() {
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        exercise_nodes(FsPath::new(&root)).await;
        return;
    }

    // Set the registry only in a child process: edition 2024 environment mutation
    // would be unsafe while the rest of the library tests run in parallel. The
    // setting disappears with the child, including on assertion failure.
    let parent = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let root = tempfile::tempdir_in(parent).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let package = npm::platform_package("@fliaping/cranibox").unwrap();
    let bytes = tarball(&package);
    let integrity = format!("sha512-{}", STANDARD.encode(Sha512::digest(&bytes)));
    let metadata = serde_json::to_vec(&json!({
        "name": package,
        "dist-tags": {"dev": "1.0.1"},
        "versions": {"1.0.1": {
            "name": package, "version": "1.0.1",
            "dist": {"tarball": format!("{base}/runtime.tgz"), "integrity": integrity}
        }}
    }))
    .unwrap();
    let package_path = format!("/{}", package.replace('/', "%2F"));
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let registry_calls = calls.clone();
    let app = Router::new().fallback(get(move |uri: axum::http::Uri| {
        let (bytes, metadata, package_path, calls) = (
            bytes.clone(),
            metadata.clone(),
            package_path.clone(),
            registry_calls.clone(),
        );
        async move {
            let body = if uri.path() == "/runtime.tgz" {
                bytes
            } else {
                assert_eq!(uri.path(), package_path);
                metadata
            };
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            body
        }
    }));
    let stop = CancellationToken::new();
    let registry_stop = stop.clone();
    let registry = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(registry_stop.cancelled_owned())
            .await
            .unwrap();
    });
    let output = tokio::time::timeout(
        Duration::from_secs(90),
        tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(CHILD_ROOT, root.path())
            .env("CRANIBOX_NPM_REGISTRY", base)
            .env_remove("CRANIBOX_NPM_PACKAGE")
            .env_remove("CRANIBOX_RUNTIME_ROOT")
            .env_remove("CRANIBOX_RUNTIME_MANAGED")
            .env_remove("CRANIBOX_RUNTIME_VERSION")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("remote update fixture timed out")
    .unwrap();
    stop.cancel();
    registry.await.unwrap();
    assert!(
        output.status.success(),
        "remote update fixture failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
}

async fn exercise_nodes(root: &FsPath) {
    let paths: Vec<_> = (0..3)
        .map(|n| root.join(format!("node-{n}/peers.json")))
        .collect();
    let mut identities = Vec::new();
    for path in &paths {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        Config::update(path, |config| {
            config.listener = Some(Listener {
                address: "127.0.0.1:0".parse().unwrap(),
                allow_insecure: false,
            });
            Ok(())
        })
        .unwrap();
        identities.push(Config::identity(path).unwrap());
    }
    let owner = OwnerKey::load_or_create(&root.join("owner.key")).unwrap();
    let members: Vec<_> = identities[..2]
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
                is_claude_node: false,
                name: format!("Device {n}"),
                added_at: 0,
                identity: None,
            }
        })
        .collect();
    for path in &paths[..2] {
        let roster_path = path.with_file_name("models").join("roster.json");
        fs::create_dir_all(roster_path.parent().unwrap()).unwrap();
        RosterStore::pin(&roster_path, &owner.public()).unwrap();
        let roster = RosterStore::load(roster_path).unwrap();
        roster.issue(&owner, members.clone()).unwrap();
        assert!(!roster.is_owner_node(&identities[2].public().node_id));
    }
    assert!(
        !paths[2]
            .with_file_name("models")
            .join("roster.json")
            .exists()
    );

    let layout = Layout::new(paths[1].with_file_name("runtime"));
    let old = Version::parse("1.0.0").unwrap();
    let stage = layout.begin_stage(&old).unwrap();
    fs::create_dir(stage.join("bin")).unwrap();
    let exe = stage.join(npm::executable_relative_path());
    fs::write(&exe, payload()).unwrap();
    crate::update::platform::verify_runtime(&exe).unwrap();
    layout.commit_stage(&old).unwrap();
    layout.activate(&old).unwrap();
    layout.confirm(&old).unwrap();

    let stop = CancellationToken::new();
    let mut running = Vec::new();
    let mut addresses = Vec::new();
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
        addresses.push(node.listener.as_ref().unwrap().address);
        running.push(node);
    }
    for (i, path) in paths.iter().enumerate() {
        Config::update(path, |config| {
            for (j, identity) in identities.iter().enumerate() {
                if i == j {
                    continue;
                }
                let public = identity.public().clone();
                let mut pinned = Pinned::new(public.clone());
                pinned.verified = true;
                let mut grant = Grant::issue(
                    &format!("{j:032x}{i:032x}"),
                    Permissions {
                        exec: true,
                        terminals: true,
                        ..Default::default()
                    },
                );
                grant.peer = Some(pinned.clone());
                config.grants.insert(public.node_id.clone(), grant);
                let token_file = path.with_file_name(format!("route-{j}.token"));
                agent_workspace_runtime::config::write_private(
                    &token_file,
                    format!("{i:032x}{j:032x}").as_bytes(),
                    false,
                )?;
                config.routes.insert(
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

    let a = Fleet::new(paths[0].clone());
    let c = Fleet::new(paths[2].clone());
    let b = &identities[1].public().node_id;
    let path = format!("/api/v1/nodes/{b}/update");
    let info = a.get(b, &path).await.unwrap();
    assert_eq!(info["node"], *b);
    assert_eq!(info["runtime"]["managed"], true);
    assert_eq!(info["runtime"]["can_update"], true);
    assert_eq!(info["runtime"]["state"], "confirmed");
    assert_eq!(info["runtime"]["version"], "1.0.0");

    let started = a.post(b, &path, json!({"spec": "dev"})).await.unwrap();
    let id = started["id"].as_str().unwrap();
    uuid::Uuid::parse_str(id).unwrap();
    let progress_path = format!("{path}/{id}");
    let activated = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match a.get(b, &progress_path).await {
                Ok(status) => {
                    assert_ne!(status["phase"], "failed", "{status}");
                    if status["phase"] == "activated" {
                        break status;
                    }
                }
                // The accepted task may not have persisted its initial status yet.
                Err(error) => {
                    assert_api_error(Err(error), StatusCode::NOT_FOUND, "unknown update id")
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("update did not activate within 30 seconds");
    assert_eq!(activated["from"], "1.0.0");
    assert_eq!(activated["to"], "1.0.1");
    let pending = layout.pointer().unwrap().unwrap();
    assert_eq!(pending.current, "1.0.1");
    assert_eq!(pending.previous.as_deref(), Some("1.0.0"));
    assert_eq!(pending.state, PointerState::Pending);
    let installed = layout.version_dir(&Version::parse("1.0.1").unwrap());
    assert!(installed.is_dir());
    let exe = installed.join(npm::executable_relative_path());
    assert_eq!(fs::read(&exe).unwrap(), payload());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&exe).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    assert_api_error(
        c.get(b, &path).await,
        StatusCode::FORBIDDEN,
        "owner roster membership required",
    );
    assert_api_error(
        c.post(b, &path, json!({"spec": "dev"})).await,
        StatusCode::FORBIDDEN,
        "owner roster membership required",
    );
    assert_api_error(
        c.get(b, &progress_path).await,
        StatusCode::FORBIDDEN,
        "owner roster membership required",
    );
    let rollback_path = format!("{path}/rollback");
    assert_api_error(
        c.post(b, &rollback_path, json!({})).await,
        StatusCode::FORBIDDEN,
        "owner roster membership required",
    );
    assert_api_error(
        a.post(b, &path, json!({"spec": "../x"})).await,
        StatusCode::BAD_REQUEST,
        "spec must be a version or a dist-tag",
    );
    let wrong_path = format!("/api/v1/nodes/{}/update", identities[0].public().node_id);
    assert_api_error(
        a.post(b, &wrong_path, json!({"spec": "dev"})).await,
        StatusCode::NOT_FOUND,
        "target is not this node",
    );
    assert_api_error(
        a.post(b, &path, json!({"spec": "dev"})).await,
        StatusCode::CONFLICT,
        "an update is already in progress",
    );
    assert_eq!(layout.pointer().unwrap().unwrap(), pending);
    assert_eq!(a.get(b, &progress_path).await.unwrap(), activated);

    let rolled_back = a.post(b, &rollback_path, json!({})).await.unwrap();
    assert_eq!(rolled_back["phase"], "rolled_back");
    assert_eq!(rolled_back["id"], id);
    assert_eq!(rolled_back["from"], "1.0.1");
    assert_eq!(rolled_back["to"], "1.0.0");
    let pointer = layout.pointer().unwrap().unwrap();
    assert_eq!(pointer.current, "1.0.0");
    assert_eq!(pointer.state, PointerState::Confirmed);
    assert_eq!(pointer.failed, ["1.0.1"]);
    assert_eq!(a.get(b, &progress_path).await.unwrap(), rolled_back);
    for node in running {
        node.shutdown().await.unwrap();
    }
}
