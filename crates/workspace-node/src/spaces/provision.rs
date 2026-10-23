use super::{PullPolicy, Space, Store, engine::Runner};
use agent_workspace_runtime::{
    config as secrets,
    peers::config::{Config, Route},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use uuid::Uuid;

const INSTALLATION: &str = "io.cranibox.installation";
const SPACE: &str = "io.cranibox.space";
#[derive(Clone, Default, Deserialize, Serialize)]
pub(super) struct Ports {
    pub peer: Option<u16>,
    pub desktop: Option<u16>,
    pub code: Option<u16>,
}
impl Ports {
    pub(super) fn from_container(container: &Value) -> Result<Self> {
        fn port(container: &Value, internal: &str) -> Result<Option<u16>> {
            let Some(bindings) = container["NetworkSettings"]["Ports"][internal].as_array() else {
                return Ok(None);
            };
            ensure!(
                bindings.len() == 1 && bindings[0]["HostIp"] == "127.0.0.1",
                "space port is not restricted to the engine host's loopback address"
            );
            let port: u16 = bindings[0]["HostPort"]
                .as_str()
                .context("missing published port")?
                .parse()?;
            ensure!(port != 0, "invalid published port");
            Ok(Some(port))
        }
        Ok(Self {
            peer: port(container, "9020/tcp")?,
            desktop: port(container, "3001/tcp")?,
            code: port(container, "8443/tcp")?,
        })
    }
}
/// Point the space's node at its in-container desktop bridge and allow the managing
/// device to use it, keeping that device's existing credential.
async fn give_agent_computer(runner: &Runner, space: &Space) -> Result<()> {
    const NODE: &str = "/usr/local/lib/agent-workspace/cranibox";
    const SETTINGS: &str = "/config/.config/cranibox/settings.json";
    let container = space.container();
    for command in [
        vec!["computer", "--address", SPACE_COMPUTER],
        vec!["allow", "--node", &space.owner, "--computer"],
    ] {
        let mut line = vec![
            "exec",
            "-u",
            "1000:1000",
            &container,
            NODE,
            "--config",
            SETTINGS,
            "peer",
        ];
        line.extend(command);
        runner.run(&args(&line), None, 30).await?;
    }
    Ok(())
}
fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}
pub(super) fn check_labels(labels: &Value, installation: Uuid, space: Uuid) -> Result<()> {
    ensure!(
        labels[INSTALLATION].as_str() == Some(&installation.to_string())
            && labels[SPACE].as_str() == Some(&space.to_string()),
        "refusing to modify a container or volume owned by another installation"
    );
    Ok(())
}
pub(super) async fn inspect(runner: &Runner, kind: &str, name: &str) -> Result<Option<Value>> {
    // A failed daemon request is never interpreted as a missing resource.
    let filter = if kind == "container" {
        format!("name=^/{name}$")
    } else {
        format!("name=^{name}$")
    };
    let format = if kind == "container" {
        "{{.Names}}"
    } else {
        "{{.Name}}"
    };
    let mut request = args(&[kind, "ls"]);
    if kind == "container" {
        request.push("--all".into());
    }
    request.extend(args(&["--filter", &filter, "--format", format]));
    let names = runner.run(&request, None, 10).await?;
    if !names.lines().any(|candidate| candidate.trim() == name) {
        return Ok(None);
    }
    let value = runner
        .run(
            &args(&[kind, "inspect", "--format", "{{json .}}", name]),
            None,
            10,
        )
        .await?;
    Ok(Some(
        serde_json::from_str(value.trim()).context("invalid container inspection response")?,
    ))
}
fn create_args(
    space: &Space,
    installation: Uuid,
    env: &str,
    network: Option<&str>,
    platform: &str,
) -> Vec<String> {
    let mut result = args(&[
        "container",
        "create",
        "--pull=never",
        "--name",
        &space.container(),
        "--hostname",
        &space.container(),
        "--label",
        &format!("{INSTALLATION}={installation}"),
        "--label",
        &format!("{SPACE}={}", space.id),
        "--restart",
        "unless-stopped",
        "--cpus",
        "2",
        "--memory",
        "4g",
        "--shm-size",
        "1g",
        "--env-file",
        env,
        "--mount",
        &format!("type=volume,source={},target=/config", space.volume()),
        "--platform",
        platform,
    ]);
    for port in ["9020", "3001", "8443"] {
        result.extend(args(&["--publish", &format!("127.0.0.1::{port}")]));
    }
    if let Some(network) = network {
        result.extend(args(&["--network", network]));
    }
    result.push(space.image.clone());
    result
}
/// `agent-desktop-bridge` inside the space image.
const SPACE_COMPUTER: &str = "127.0.0.1:8765";
const SEED: &str = r#"set -eu
directory=/config/.config/cranibox
install -d -m 700 -o 1000 -g 1000 "$directory"
if test -e "$directory/peers.json"; then cat >/dev/null; exit 0; fi
umask 077
cat > "$directory/peers.json.seed"
chown 1000:1000 "$directory/peers.json.seed"
mv "$directory/peers.json.seed" "$directory/peers.json"
"#;
impl Store {
    pub(super) async fn provision(&self, id: Uuid) -> Result<()> {
        let catalog = self.read()?;
        let space = catalog
            .spaces
            .iter()
            .find(|space| space.id == id)
            .context("space not found")?
            .clone();
        ensure!(
            Config::read(&self.peers)?.node_id == space.owner,
            "the creating device's identity changed; restore its peer configuration first"
        );
        let installation = catalog.installation;
        let runner = space.provider.runner()?;
        let info = runner
            .run(&args(&["info", "--format", "{{json .}}"]), None, 10)
            .await
            .context("container engine unavailable; complete environment setup first")?;
        let info: Value = serde_json::from_str(&info)?;
        ensure!(
            info["OSType"] == "linux",
            "Switch Docker to Linux containers."
        );
        // The published manifests carry amd64 and arm64; pin the engine's own one.
        let platform = super::images::platform(info["Architecture"].as_str().unwrap_or_default())?;
        let access = runner.access(space.provider).await?;
        let current = inspect(&runner, "container", &space.container()).await?;
        if let Some(container) = &current {
            check_labels(&container["Config"]["Labels"], installation, id)?;
        } else {
            let inspect_image = args(&[
                "image",
                "inspect",
                "--format",
                "{{json .Config.Labels}}",
                &space.image,
            ]);
            let local = if matches!(space.pull_policy, PullPolicy::Always) {
                None
            } else {
                let output = runner.output(&inspect_image, None, 10).await?;
                if output.success {
                    Some(output.stdout)
                } else {
                    ensure!(
                        !matches!(space.pull_policy, PullPolicy::Never),
                        "Local image unavailable: {}. Build or load the image on this Docker engine before retrying. No image was downloaded and no space data was changed. Docker: {}",
                        space.image,
                        output.stderr.trim()
                    );
                    None
                }
            };
            let labels = match local {
                Some(labels) => labels,
                None => {
                    self.stage(id, "pulling")?;
                    runner
                        .run(
                            &args(&["image", "pull", "--platform", platform, &space.image]),
                            None,
                            1800,
                        )
                        .await?;
                    runner.run(&inspect_image, None, 10).await?
                }
            };
            ensure!(
                serde_json::from_str::<Value>(labels.trim())?["io.cranibox.space-protocol"] == "1",
                "This image does not support Cranibox virtual-space enrollment (io.cranibox.space-protocol=1). Select an image built from the current Cranibox Dockerfile, then retry. No space data was changed."
            );
            match inspect(&runner, "volume", &space.volume()).await? {
                Some(volume) => check_labels(&volume["Labels"], installation, id)?,
                None => {
                    runner
                        .run(
                            &args(&[
                                "volume",
                                "create",
                                "--label",
                                &format!("{INSTALLATION}={installation}"),
                                "--label",
                                &format!("{SPACE}={id}"),
                                &space.volume(),
                            ]),
                            None,
                            15,
                        )
                        .await?;
                }
            }
            self.stage(id, "creating")?;
            // The space is the Agent's own computer: its desktop bridge (loopback inside
            // the container) serves Computer Use to the managing device only.
            let peer_config = json!({"version":1,"node_id":space.node,"listener":{"address":"0.0.0.0:9020","allow_insecure":true},"desktop":null,"desktop_epoch":0,
                "computer":{"address":SPACE_COMPUTER},
                "grants":{(space.owner.clone()):{"token":space.token,"permissions":{"exec":true,"terminals":true,"projects":true,"desktop":false,"computer":true}}},"routes":{}});
            runner
                .run(
                    &args(&[
                        "run",
                        "--pull=never",
                        "--platform",
                        platform,
                        "--rm",
                        "--interactive",
                        "--network",
                        "none",
                        "--label",
                        &format!("{INSTALLATION}={installation}"),
                        "--label",
                        &format!("{SPACE}={id}"),
                        "--mount",
                        &format!("type=volume,source={},target=/config", space.volume()),
                        "--entrypoint",
                        "/bin/sh",
                        &space.image,
                        "-c",
                        SEED,
                    ]),
                    Some(&serde_json::to_vec(&peer_config)?),
                    30,
                )
                .await?;
            let environment = format!(
                "PUID=1000\nPGID=1000\nCUSTOM_USER=agent\nPASSWORD={}\nAGENT_WORKSPACE_BOOTSTRAP=remote\nAGENT_WORKSPACE_AGENT=deepseek-harness\nCODE_SERVER_BIND=0.0.0.0:8443\nCODE_SERVER_AUTH=password\nCODE_SERVER_CERT=true\nCODE_SERVER_RECONFIGURE=true\nSTART_DOCKER=false\nTZ=Etc/UTC\n{}",
                space.password,
                space.display.environment()
            );
            let env_file = self.directory.join(format!("{id}.env"));
            secrets::write_private(&env_file, environment.as_bytes(), true)?;
            // Docker reads this file on the client, including when the CLI runs inside WSL.
            let env_path = if space.provider == super::Provider::Wsl {
                // Windows paths are not paths in Ubuntu. Send env lines over stdin instead.
                None
            } else {
                Some(env_file.to_string_lossy().into_owned())
            };
            let result = if let Some(env_path) = env_path {
                runner
                    .run(
                        &create_args(
                            &space,
                            installation,
                            &env_path,
                            access.network.as_deref(),
                            platform,
                        ),
                        None,
                        30,
                    )
                    .await
            } else {
                // /dev/stdin is consumed by Docker before it creates the container.
                runner
                    .run(
                        &create_args(
                            &space,
                            installation,
                            "/dev/stdin",
                            access.network.as_deref(),
                            platform,
                        ),
                        Some(environment.as_bytes()),
                        30,
                    )
                    .await
            };
            let _ = std::fs::remove_file(env_file);
            result?;
        }
        self.stage(id, "starting")?;
        runner
            .run(&args(&["container", "start", &space.container()]), None, 30)
            .await?;
        let container = inspect(&runner, "container", &space.container())
            .await?
            .context("container disappeared during startup")?;
        check_labels(&container["Config"]["Labels"], installation, id)?;
        let ports = Ports::from_container(&container)?;
        let peer_port = ports.peer.context("peer port is not published")?;
        let mut catalog = self.read()?;
        let saved = catalog
            .spaces
            .iter_mut()
            .find(|space| space.id == id)
            .context("space not found")?;
        saved.ports = ports;
        saved.phase = "enrolling".into();
        self.save(&catalog)?;
        let credential = std::fs::canonicalize(&self.directory)?.join(format!("{id}.token"));
        secrets::write_private(&credential, format!("{}\n", space.token).as_bytes(), true)?;
        let route = if let Some(network) = access.network {
            let address = container["NetworkSettings"]["Networks"][&network]["IPAddress"].as_str().context("space is not attached to the manager's Docker network; restore that network before resuming")?;
            let ip: std::net::Ipv4Addr = address.parse().context("invalid space Docker address")?;
            ensure!(
                ip.is_private() || ip.is_loopback(),
                "space Docker address is not private"
            );
            Route {
                target_identity: None,
                config_path: None,
                alternatives: vec![],
                url: format!("http://{ip}:9020"),
                token_file: credential,
                allow_insecure: true,
                local_forward_lock: None,
            }
        } else {
            Route {
                target_identity: None,
                config_path: None,
                alternatives: vec![],
                url: format!("http://127.0.0.1:{peer_port}"),
                token_file: credential,
                allow_insecure: false,
                local_forward_lock: None,
            }
        };
        let api = route.api(&space.node)?;
        // A running container is not enough: verify the actual node identity and grant.
        let nodes = tokio::time::timeout(Duration::from_secs(600), async {
            loop {
                if let Ok(value) = api.get("/api/v1/nodes").await {
                    ensure!(value["nodes"].as_array().is_some_and(|nodes| nodes.len() == 1 && nodes[0]["id"].as_str() == Some(&space.node)), "the space reports an unexpected node identity");
                    return Ok::<_,anyhow::Error>(value);
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }).await.context("container started but its Cranibox node is not reachable yet; resume after bootstrap finishes. With a remote engine or mounted Docker socket, run the desktop client on that engine's host.")??;
        Config::update(&self.peers, |config| {
            ensure!(config.node_id == space.owner, "creator identity changed");
            config.routes.insert(space.node.clone(), route);
            Ok(())
        })?;
        // Spaces created before they became the Agent's own computer get it on their next
        // start. Best effort: an older image without `peer allow` keeps working as before.
        if nodes["nodes"][0]["computer"] != true
            && let Err(error) = give_agent_computer(&runner, &space).await
        {
            eprintln!("space {id}: Computer Use not enabled: {error:#}");
        }
        self.stage(id, "ready")?;
        Ok(())
    }
    pub(super) async fn stop(&self, id: Uuid, remove: bool) -> Result<()> {
        let catalog = self.read()?;
        let space = catalog
            .spaces
            .iter()
            .find(|space| space.id == id)
            .context("space not found")?;
        let runner = space.provider.runner()?;
        if let Some(container) = inspect(&runner, "container", &space.container()).await? {
            check_labels(&container["Config"]["Labels"], catalog.installation, id)?;
            if container["State"]["Running"] == true {
                runner
                    .run(
                        &args(&["container", "stop", "--time", "20", &space.container()]),
                        None,
                        30,
                    )
                    .await?;
            }
            if remove {
                runner
                    .run(&args(&["container", "rm", &space.container()]), None, 15)
                    .await?;
            }
        }
        if remove {
            // Preserve enrollment credentials, so re-creation uses the same identity and volume.
            Config::update(&self.peers, |config| {
                if config.routes.get(&space.node).is_some_and(|route| {
                    route.token_file == self.directory.join(format!("{id}.token"))
                }) {
                    config.routes.remove(&space.node);
                }
                Ok(())
            })?;
        }
        let mut catalog = self.read()?;
        let space = catalog
            .spaces
            .iter_mut()
            .find(|space| space.id == id)
            .context("space not found")?;
        space.ports = Ports::default();
        space.phase = if remove { "archived" } else { "stopped" }.into();
        space.error = None;
        self.save(&catalog)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn externally_exposed_ports_and_foreign_ownership_are_rejected() {
        let exposed = json!({"NetworkSettings":{"Ports":{"9020/tcp":[{"HostIp":"0.0.0.0","HostPort":"42000"}]}}});
        assert!(Ports::from_container(&exposed).is_err());
        assert!(check_labels(&json!({}), Uuid::new_v4(), Uuid::new_v4()).is_err());
    }
}
