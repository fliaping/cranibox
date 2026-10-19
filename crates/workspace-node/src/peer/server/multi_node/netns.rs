//! Separate-process counterpart to the fast Noise test. The shell harness puts
//! each worker in its own network namespace; --local exercises the same RPCs.
use super::*;
use anyhow::{Context, Result, ensure};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};

const ACCOUNT: &str = "netns-account";
const KEY: &str = "fake-netns-credential-never-on-mesh";
const LABEL: &str = "private-netns-account-label";

async fn wait_file(path: &Path) -> Result<Vec<u8>> {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match std::fs::read(path) {
                Ok(bytes) => return Ok(bytes),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(e) => return Err(e.into()),
            }
        }
    })
    .await
    .context("fixture worker timed out")?
}
async fn rpc(dir: &Path, sequence: &mut u64, command: Value) -> Result<Value> {
    *sequence += 1;
    let request = dir.join(format!("request-{sequence}.json"));
    agent_workspace_runtime::config::write_private(
        &request,
        &serde_json::to_vec(&command)?,
        false,
    )?;
    let result: Value =
        serde_json::from_slice(&wait_file(&dir.join(format!("response-{sequence}.json"))).await?)?;
    ensure!(result.get("error").is_none(), "worker failed: {result}");
    Ok(result)
}
struct Children(Vec<Child>);
impl Drop for Children {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[tokio::test]
#[ignore = "fixture worker; launched only by tests/integration/vault_netns.sh"]
async fn worker() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("CRANIBOX_NETNS_NODE")?);
    let path = dir.join("peers.json");
    let stop = CancellationToken::new();
    let node = start_with_catalog(
        path.clone(),
        Engine::new(dir.join("home")),
        Arc::new(Semaphore::new(1)),
        stop.clone(),
        agent_workspace_runtime::projects::Store::new(dir.join("projects.json")),
    )
    .await?;
    let service =
        crate::models::Models::start_on(path, stop.clone(), Some(vec!["127.0.0.1:0".parse()?]))
            .await?;
    ensure!(
        node.app.models.set(service.clone()).is_ok(),
        "model service already set"
    );
    agent_workspace_runtime::config::write_private(&dir.join("ready"), b"ready", false)?;
    for sequence in 1u64.. {
        let request: Value = serde_json::from_slice(
            &wait_file(&dir.join(format!("request-{sequence}.json"))).await?,
        )?;
        let action = request["action"].as_str().context("missing action")?;
        let result: Result<Value> = async {
            match action {
                "add" => {
                    let credential = cranibox_llm::upstream::apikey::Credential {
                        access_token: KEY.into(),
                        endpoints: [(cranibox_contracts::llm::Wire::Chat, request["endpoint"].as_str().context("missing endpoint")?.into())].into(),
                        models: vec!["test".into()], ..Default::default()
                    };
                    service.vault.put(AccountSummary {
                        id: ACCOUNT.into(), provider: "netns".into(), plan: None, label: LABEL.into(),
                        kind: AccountKind::ApiKey, home_node: service.vault.node().into(), sync: true,
                        enabled_on: vec![service.vault.node().into()], all_nodes: false, state: AccountState::Ready,
                    }, Some(credential.secret()))?;
                    Ok(json!({"ok":true}))
                }
                "sync" => {
                    exchange(&service.vault, service.router.transport.as_ref(), request["node"].as_str().context("missing node")?).await?;
                    Ok(json!({"ok":true}))
                }
                "inspect" => Ok(json!({"accounts":service.vault.list(), "has_secret":service.vault.get_secret(ACCOUNT).is_ok()})),
                "enable" => {
                    let mut account = service.vault.get(ACCOUNT).context("account missing")?;
                    account.enabled_on.push(service.vault.node().into());
                    service.vault.put(account, None)?;
                    Ok(json!({"ok":true}))
                }
                "probe" => Ok(json!({"online":service.router.probe(request["node"].as_str().context("missing node")?).await})),
                "chat" => {
                    let response = reqwest::Client::new().post(format!("http://{}/v1/chat/completions", service.proxy.addresses[0]))
                        .bearer_auth("cbx-pi").json(&json!({"model":"netns/test", "messages":[{"role":"user", "content":"Hello"}]})).send().await?;
                    let status = response.status().as_u16();
                    Ok(json!({"status":status, "body":response.json::<Value>().await?}))
                }
                _ => anyhow::bail!("unknown fixture action"),
            }
        }.await;
        let response = result.unwrap_or_else(|error| json!({"error":error.to_string()}));
        agent_workspace_runtime::config::write_private(
            &dir.join(format!("response-{sequence}.json")),
            &serde_json::to_vec(&response)?,
            false,
        )?;
    }
    unreachable!()
}

#[tokio::test]
#[ignore = "requires explicit fixture harness; may create three child processes"]
async fn coordinator() -> Result<()> {
    ensure!(
        std::env::var("CRANIBOX_NETNS_RUN").as_deref() == Ok("1"),
        "use tests/integration/vault_netns.sh"
    );
    let prefix = std::env::var("CRANIBOX_NETNS_PREFIX").unwrap_or_default();
    let ips: Vec<_> = if prefix.is_empty() {
        vec!["127.0.0.1"; 3]
    } else {
        vec!["10.224.0.2", "10.224.0.3", "10.224.0.4"]
    };
    let root = tempfile::tempdir()?;
    let dirs: Vec<_> = (0..3).map(|i| root.path().join(format!("n{i}"))).collect();
    let mut identities = Vec::new();
    for (i, dir) in dirs.iter().enumerate() {
        std::fs::create_dir_all(dir)?;
        Config::update(&dir.join("peers.json"), |config| {
            config.listener = Some(Listener {
                address: format!("{}:{}", ips[i], 49400 + i).parse()?,
                allow_insecure: false,
            });
            Ok(())
        })?;
        identities.push(Config::identity(&dir.join("peers.json"))?);
    }
    let owner = OwnerKey::load_or_create(&root.path().join("owner.key"))?;
    let members: Vec<_> = identities
        .iter()
        .enumerate()
        .map(|(i, identity)| {
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
                role: if i == 0 { Role::Hub } else { Role::Node },
                priority: i as u8,
                is_claude_node: false,
                name: format!("Fixture {i}"),
                added_at: 0,
            }
        })
        .collect();
    for (i, dir) in dirs.iter().enumerate() {
        let model_dir = dir.join("models");
        std::fs::create_dir_all(&model_dir)?;
        agent_workspace_runtime::config::write_private(
            &model_dir.join("encryption.key"),
            identities[i].noise_private(),
            false,
        )?;
        let roster = model_dir.join("roster.json");
        RosterStore::pin(&roster, &owner.public())?;
        RosterStore::load(roster)?.issue(&owner, members.clone())?;
        Config::update(&dir.join("peers.json"), |config| {
            for (j, member) in identities.iter().enumerate() {
                if i == j {
                    continue;
                }
                let public = member.public().clone();
                let mut pinned = Pinned::new(public.clone());
                pinned.verified = true;
                let mut grant = Grant::issue(
                    &format!("{j:032x}{i:032x}"),
                    super::super::super::config::Permissions {
                        llm: true,
                        vault: true,
                        ..Default::default()
                    },
                );
                grant.peer = Some(pinned.clone());
                config.grants.insert(public.node_id.clone(), grant);
                let token_file = dir.join(format!("route-{j}.token"));
                agent_workspace_runtime::config::write_private(
                    &token_file,
                    format!("{i:032x}{j:032x}").as_bytes(),
                    false,
                )?;
                config.routes.insert(
                    public.node_id,
                    Route {
                        url: format!("http://{}:{}", ips[j], 49400 + j),
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
        })?;
    }
    let bind = if prefix.is_empty() {
        "127.0.0.1:0"
    } else {
        "10.224.0.1:0"
    };
    let upstream = tokio::net::TcpListener::bind(bind).await?;
    let endpoint = format!("http://{}/v1", upstream.local_addr()?);
    let server = tokio::spawn(async move {
        axum::serve(upstream, Router::new().route("/v1/chat/completions",post(|| async {Json(json!({"choices":[{"message":{"role":"assistant","content":"netns reply"},"finish_reason":"stop","index":0}]}))}))).await
    });
    let mut children = Children(Vec::new());
    for (i, dir) in dirs.iter().enumerate() {
        let mut command = if prefix.is_empty() {
            Command::new(std::env::current_exe()?)
        } else {
            let mut c = Command::new("ip");
            c.args(["netns", "exec", &format!("{prefix}-{i}")]);
            c.arg(std::env::current_exe()?);
            c
        };
        let log = std::fs::File::create(dir.join("worker.log"))?;
        command
            .args([
                "--ignored",
                "--exact",
                "peer::server::multi_node::netns::worker",
                "--nocapture",
            ])
            .env("CRANIBOX_NETNS_NODE", dir)
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log));
        children.0.push(command.spawn()?);
    }
    for dir in &dirs {
        wait_file(&dir.join("ready")).await?;
    }
    let mut seq = [0; 3];
    rpc(
        &dirs[0],
        &mut seq[0],
        json!({"action":"add","endpoint":endpoint}),
    )
    .await?;
    for i in [1, 2] {
        rpc(
            &dirs[0],
            &mut seq[0],
            json!({"action":"sync","node":members[i].node_id}),
        )
        .await?;
        let state = rpc(&dirs[i], &mut seq[i], json!({"action":"inspect"})).await?;
        ensure!(
            state["accounts"].as_array().is_some_and(|a| a.len() == 1)
                && state["has_secret"] == false,
            "unenabled device received a credential"
        );
    }
    rpc(
        &dirs[2],
        &mut seq[2],
        json!({"action":"probe","node":members[0].node_id}),
    )
    .await?;
    let response = rpc(&dirs[2], &mut seq[2], json!({"action":"chat"})).await?;
    ensure!(
        response["status"] == 200
            && response["body"]["choices"][0]["message"]["content"] == "netns reply",
        "forwarded chat failed: {response}"
    );
    rpc(&dirs[1], &mut seq[1], json!({"action":"enable"})).await?;
    rpc(
        &dirs[1],
        &mut seq[1],
        json!({"action":"sync","node":members[0].node_id}),
    )
    .await?;
    rpc(
        &dirs[0],
        &mut seq[0],
        json!({"action":"sync","node":members[1].node_id}),
    )
    .await?;
    let state = rpc(&dirs[1], &mut seq[1], json!({"action":"inspect"})).await?;
    ensure!(
        state["has_secret"] == true,
        "enabled device has no credential"
    );
    children.0[0].kill()?;
    children.0[0].wait()?;
    for _ in 0..3 {
        rpc(
            &dirs[1],
            &mut seq[1],
            json!({"action":"probe","node":members[0].node_id}),
        )
        .await?;
    }
    let response = rpc(&dirs[1], &mut seq[1], json!({"action":"chat"})).await?;
    ensure!(
        response["status"] == 200
            && response["body"]["choices"][0]["message"]["content"] == "netns reply",
        "local fallback failed: {response}"
    );
    drop(children);
    server.abort();
    println!(
        "PASS: three processes; encrypted sync; default disabled; remote chat; enable B; kill Hub; same local endpoint continues"
    );
    Ok(())
}
