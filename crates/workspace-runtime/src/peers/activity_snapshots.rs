//! Explicitly selected, route-bound collaboration observations. Never a task ledger.
use super::{Store, activity, config::Config, project_sync, projects::RefreshBusy};
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

const MAX_BYTES: u64 = activity::MAX_WIRE as u64 + 4096;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    binding: String,
    synced_at: u64,
    activity: activity::Snapshot,
}
impl Store {
    fn activity_snapshot_path(&self, node: &str) -> Result<PathBuf> {
        ensure!(valid_node_id(node), "invalid node ID");
        let mut directory = self
            .path
            .file_name()
            .context("peer configuration filename required")?
            .to_os_string();
        directory.push(".activity-snapshots");
        Ok(self
            .path
            .with_file_name(directory)
            .join(format!("{:x}.json", Sha256::digest(node.as_bytes()))))
    }
    /// Caller holds the peer lock. Request lock files are deliberately retained.
    pub(super) fn forget_activity_snapshot(&self, node: &str) -> Result<()> {
        match fs::remove_file(self.activity_snapshot_path(node)?) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("remove offline collaboration records"),
        }
    }
    fn read_activity_snapshot(&self, node: &str, binding: &str) -> Result<Option<Snapshot>> {
        let file = match fs::File::open(self.activity_snapshot_path(node)?) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            file.metadata()?.len() <= MAX_BYTES,
            "offline collaboration records too large"
        );
        let snapshot: Snapshot = serde_json::from_reader(file)?;
        ensure!(
            snapshot.version == 1
                && snapshot.synced_at > 0
                && snapshot.synced_at <= 253_402_300_799_000,
            "invalid offline collaboration records"
        );
        if snapshot.binding != binding {
            return Ok(None);
        }
        snapshot.activity.validate(node)?;
        Ok(Some(snapshot))
    }
    pub(super) async fn background_activity(&self, node: String, binding: String) -> Result<Value> {
        self.remote_activity(node, Some(binding)).await
    }
    pub(super) async fn remote_activity(
        &self,
        node: String,
        expected: Option<String>,
    ) -> Result<Value> {
        let store = self.clone();
        let target = node.clone();
        let (_request_guard, route, binding, selection) = tokio::task::spawn_blocking(move || {
            let path = store.activity_snapshot_path(&target)?;
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
            let selection = project_sync::activity_selection(
                &store.path,
                &configuration.node_id,
                &target,
                &binding,
            )?;
            ensure!(
                expected
                    .as_ref()
                    .is_none_or(|expected| expected == &binding && selection.is_some()),
                "activity sync connection changed; select the device again"
            );
            if selection.is_none() {
                store.forget_activity_snapshot(&target)?;
            }
            Ok::<_, anyhow::Error>((guard, route, binding, selection))
        })
        .await??;
        let result = async {
            let value = route
                .api(&node)?
                .activity(&format!("/api/v1/nodes/{node}/activity"))
                .await?;
            // A peer cannot supply local offline markers or an unchecked ledger.
            let snapshot: activity::Snapshot = serde_json::from_value(value)?;
            snapshot.validate(&node)?;
            Ok::<_, anyhow::Error>(snapshot)
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
                "direct connection changed; read collaboration records again"
            );
            ensure!(
                project_sync::activity_selection(
                    &store.path,
                    &configuration.node_id,
                    &node,
                    &binding
                )? == selection,
                "activity sync settings changed; read collaboration records again"
            );
            match result {
                Ok(activity) => {
                    let mut value = serde_json::to_value(&activity)?;
                    // Full message/task content is stored only after explicit selection.
                    if selection.is_some() {
                        let snapshot = Snapshot {
                            version: 1,
                            binding,
                            synced_at: SystemTime::now()
                                .duration_since(UNIX_EPOCH)?
                                .as_millis()
                                .try_into()?,
                            activity,
                        };
                        let saved = (|| {
                            let bytes = serde_json::to_vec(&snapshot)?;
                            ensure!(
                                bytes.len() as u64 <= MAX_BYTES,
                                "offline collaboration records too large"
                            );
                            config::write_private(
                                &store.activity_snapshot_path(&node)?,
                                &bytes,
                                true,
                            )
                        })();
                        value["sync"] = json!({"state":"online","synced_at":snapshot.synced_at});
                        if saved.is_err() {
                            value["sync"]["warning"] =
                                json!("Local collaboration snapshot could not be saved.");
                        }
                    }
                    Ok(value)
                }
                Err(error) => {
                    if outage
                        && selection.is_some()
                        && let Some(snapshot) = store.read_activity_snapshot(&node, &binding)?
                    {
                        let mut value = serde_json::to_value(snapshot.activity)?;
                        value["sync"] = json!({"state":"offline","synced_at":snapshot.synced_at});
                        return Ok(value);
                    }
                    if !outage {
                        store.forget_activity_snapshot(&node)?;
                    }
                    Err(error)
                }
            }
        })
        .await?
    }
}
