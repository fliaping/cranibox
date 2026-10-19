//! Route-bound public Agent reports. Cached reports never renew an Agent lease.
use super::{Store, agent_sessions::Catalog, config::Config, project_sync, projects::RefreshBusy};
use crate::{config, protocol::valid_node_id};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_BYTES: u64 = 128 * 1024 + 4096;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    binding: String,
    synced_at: u64,
    catalog: Catalog,
}

impl Store {
    fn agent_snapshot_path(&self, node: &str) -> Result<PathBuf> {
        ensure!(valid_node_id(node), "invalid node ID");
        let mut directory = self
            .path
            .file_name()
            .context("peer configuration filename required")?
            .to_os_string();
        directory.push(".agent-snapshots");
        Ok(self
            .path
            .with_file_name(directory)
            .join(format!("{:x}.json", Sha256::digest(node.as_bytes()))))
    }

    /// Configuration mutations hold the peer lock before removing a snapshot.
    pub(super) fn forget_agent_snapshot(&self, node: &str) -> Result<()> {
        match fs::remove_file(self.agent_snapshot_path(node)?) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("remove offline Agent directory"),
        }
    }

    fn read_agent_snapshot(&self, node: &str, binding: &str) -> Result<Option<Snapshot>> {
        let file = match fs::File::open(self.agent_snapshot_path(node)?) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            file.metadata()?.len() <= MAX_BYTES,
            "offline Agent directory too large"
        );
        let snapshot: Snapshot = serde_json::from_reader(file)?;
        ensure!(
            snapshot.version == 1
                && snapshot.synced_at > 0
                && snapshot.synced_at <= 253_402_300_799_000,
            "invalid offline Agent directory"
        );
        if snapshot.binding != binding {
            return Ok(None);
        }
        snapshot.catalog.validate(node)?;
        Ok(Some(snapshot))
    }

    pub(super) async fn background_agents(&self, node: String, binding: String) -> Result<Value> {
        self.remote_agents(node, Some(binding)).await
    }

    pub(super) async fn remote_agents(
        &self,
        node: String,
        expected: Option<String>,
    ) -> Result<Value> {
        let store = self.clone();
        let target = node.clone();
        let (_request_guard, route, binding) = tokio::task::spawn_blocking(move || {
            // Keep one request per source across interactive and background reads.
            // The separate lock file is never unlinked while a request holds it.
            let path = store.agent_snapshot_path(&target)?;
            let guard = if expected.is_some() {
                config::try_lock_credentials(&path)?.ok_or(RefreshBusy)?
            } else {
                config::lock_credentials(&path)?
            };
            let _peer = config::lock_credentials(&store.path)?;
            let configuration = Config::read(&store.path)?;
            let route = configuration
                .routes
                .get(&target)
                .context("direct route is not configured")?
                .clone();
            let binding = project_sync::binding(&configuration.node_id, &target, &route)?;
            ensure!(
                expected
                    .as_ref()
                    .is_none_or(|expected| expected == &binding),
                "Agent sync connection changed; select the device again"
            );
            Ok::<_, anyhow::Error>((guard, route, binding))
        })
        .await??;
        let result = async {
            let value = route
                .api(&node)?
                .get(&format!("/api/v1/nodes/{node}/agent-sessions"))
                .await?;
            // Ignore no fields: a peer cannot inject a local cached-state marker.
            let catalog: Catalog = serde_json::from_value(value)?;
            catalog.validate(&node)?;
            Ok::<_, anyhow::Error>(catalog)
        }
        .await;
        let outage = result
            .as_ref()
            .err()
            .is_some_and(crate::client::unavailable);
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let _peer = config::lock_credentials(&store.path)?;
            let configuration = Config::read(&store.path)?;
            let current = configuration
                .routes
                .get(&node)
                .context("direct route was removed")?;
            ensure!(
                project_sync::binding(&configuration.node_id, &node, current)? == binding,
                "direct connection changed; refresh the Agent directory"
            );
            match result {
                Ok(catalog) => {
                    let mut value = serde_json::to_value(&catalog)?;
                    let snapshot = Snapshot {
                        version: 1,
                        binding,
                        synced_at: SystemTime::now()
                            .duration_since(UNIX_EPOCH)?
                            .as_millis()
                            .try_into()?,
                        catalog,
                    };
                    let saved = (|| {
                        let bytes = serde_json::to_vec(&snapshot)?;
                        ensure!(
                            bytes.len() as u64 <= MAX_BYTES,
                            "offline Agent directory too large"
                        );
                        config::write_private(&store.agent_snapshot_path(&node)?, &bytes, true)
                    })();
                    value["sync"] = json!({"state":"online", "synced_at":snapshot.synced_at});
                    if saved.is_err() {
                        value["sync"]["warning"] =
                            json!("Offline Agent directory could not be saved on this device.");
                    }
                    Ok(value)
                }
                Err(error) if outage => {
                    if let Some(snapshot) =
                        store.read_agent_snapshot(&node, &binding).ok().flatten()
                    {
                        let mut value = serde_json::to_value(snapshot.catalog)?;
                        value["sync"] = json!({"state":"offline", "synced_at":snapshot.synced_at});
                        return Ok(value);
                    }
                    Err(error)
                }
                Err(error) => {
                    store.forget_agent_snapshot(&node)?;
                    Err(error)
                }
            }
        })
        .await?
    }
}
