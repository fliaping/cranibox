//! Private, route-bound snapshots. They are read-only hints while an owner is
//! unreachable, never another authority for project records or permissions.
use super::{
    Store,
    config::{Config, Permissions, Route},
};
use crate::{
    config,
    projects::{
        Catalog, Command,
        sync::{Delta, Index, PROTOCOL},
    },
    protocol::valid_node_id,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_SNAPSHOT_BYTES: u64 = 1024 * 1024 + 4096;

#[derive(Debug)]
pub(super) struct RefreshBusy;
impl std::fmt::Display for RefreshBusy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("project catalog is being refreshed by another request")
    }
}
impl std::error::Error for RefreshBusy {}

#[derive(Serialize, Deserialize)]
struct RemoteCatalog {
    owner: String,
    permissions: Permissions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sync_protocol: Option<String>,
    #[serde(flatten)]
    catalog: Catalog,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SyncReply {
    owner: String,
    permissions: Permissions,
    sync_protocol: String,
    delta: Delta,
}
impl SyncReply {
    fn apply(self, node: &str, previous: &Catalog) -> Result<RemoteCatalog> {
        ensure!(
            self.owner == node && self.permissions.projects && self.sync_protocol == PROTOCOL,
            "unexpected project sync authority or protocol"
        );
        let catalog = RemoteCatalog {
            owner: self.owner,
            permissions: self.permissions,
            sync_protocol: Some(self.sync_protocol),
            catalog: self.delta.apply(previous)?,
        };
        catalog.validate(node)?;
        Ok(catalog)
    }
}
impl RemoteCatalog {
    fn validate(&self, owner: &str) -> Result<()> {
        ensure!(self.owner == owner, "unexpected catalog owner");
        ensure!(
            self.permissions.projects,
            "project catalog permission missing"
        );
        let mut projects = BTreeSet::new();
        let mut replicas = BTreeSet::new();
        let mut bindings = BTreeSet::new();
        ensure!(
            self.catalog.projects.iter().all(|p| {
                uuid::Uuid::parse_str(&p.id).is_ok()
                    && projects.insert(&p.id)
                    && p.replicas.iter().all(|r| {
                        r.node.as_deref() == Some(owner)
                            && uuid::Uuid::parse_str(&r.id).is_ok()
                            && replicas.insert(&r.id)
                            && r.sessions.iter().all(|s| {
                                uuid::Uuid::parse_str(&s.id).is_ok()
                                    && bindings.insert(&s.id)
                                    && uuid::Uuid::parse_str(&s.terminal_id).is_ok()
                            })
                    })
            }),
            "unexpected catalog replica owner or ID"
        );
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    binding: String,
    synced_at: u64,
    catalog: RemoteCatalog,
}

pub(super) fn fingerprint(caller: &str, node: &str, route: &Route) -> Result<String> {
    // Bind to the actual secret, not only its filename. Changing the endpoint,
    // caller identity or credential cannot reuse another connection's metadata.
    let token = config::read_token(&route.token_file)?;
    let bytes = serde_json::to_vec(&(caller, node, &route.url, route.allow_insecure, token))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

impl Store {
    fn project_snapshot_path(&self, node: &str) -> Result<PathBuf> {
        ensure!(valid_node_id(node), "invalid node ID");
        let mut directory = self
            .path
            .file_name()
            .context("peer configuration filename required")?
            .to_os_string();
        directory.push(".project-snapshots");
        // Hash the name to avoid case-insensitive filename collisions on macOS
        // and Windows. NodeIDs themselves remain case-sensitive.
        Ok(self
            .path
            .with_file_name(directory)
            .join(format!("{:x}.json", Sha256::digest(node.as_bytes()))))
    }

    /// Caller holds the peer configuration lock. Do not remove the separate
    /// request lock: in-flight requests must keep sharing the same lock inode.
    pub(super) fn forget_project_snapshot(&self, node: &str) -> Result<()> {
        match fs::remove_file(self.project_snapshot_path(node)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).context("remove offline project catalog"),
        }
    }

    fn read_project_snapshot(&self, node: &str, binding: &str) -> Result<Option<Snapshot>> {
        let file = match fs::File::open(self.project_snapshot_path(node)?) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        ensure!(
            file.metadata()?.len() <= MAX_SNAPSHOT_BYTES,
            "offline project catalog too large"
        );
        let snapshot: Snapshot = serde_json::from_reader(file)?;
        ensure!(
            snapshot.version == 1
                && snapshot.synced_at > 0
                && snapshot.synced_at <= 253_402_300_799_000,
            "invalid offline project catalog"
        );
        if snapshot.binding != binding {
            return Ok(None);
        }
        snapshot.catalog.validate(node)?;
        Ok(Some(snapshot))
    }

    pub(super) async fn projects(&self, node: String, command: Command) -> Result<Value> {
        self.projects_inner(node, command, None).await
    }

    pub(super) async fn background_projects(&self, node: String, binding: String) -> Result<Value> {
        self.projects_inner(node, Command::List {}, Some(binding))
            .await
    }

    async fn projects_inner(
        &self,
        node: String,
        command: Command,
        expected: Option<String>,
    ) -> Result<Value> {
        let list = matches!(command, Command::List {});
        let store = self.clone();
        let target = node.clone();
        let (_request_guard, route, binding, previous) = tokio::task::spawn_blocking(move || {
            // Serialize this connection's reads and writes, including across Web
            // restarts/processes. A slow list cannot overwrite a later mutation.
            let path = store.project_snapshot_path(&target)?;
            let guard = if expected.is_some() {
                config::try_lock_credentials(&path)?.ok_or(RefreshBusy)?
            } else {
                config::lock_credentials(&path)?
            };
            let config = Config::read(&store.path)?;
            let route = config
                .routes
                .get(&target)
                .context("direct route is not configured")?
                .clone();
            let binding = fingerprint(&config.node_id, &target, &route)?;
            ensure!(
                expected
                    .as_ref()
                    .is_none_or(|expected| super::project_sync::binding(
                        &config.node_id,
                        &target,
                        &route
                    )
                    .is_ok_and(|actual| actual == *expected)),
                "project sync connection changed; review the selected devices again"
            );
            // A corrupt, missing or credential-mismatched derived snapshot is
            // repaired from a full owner read, never sent back as remote edits.
            let previous = if list {
                store
                    .read_project_snapshot(&target, &binding)
                    .ok()
                    .flatten()
            } else {
                None
            };
            Ok::<_, anyhow::Error>((guard, route, binding, previous))
        })
        .await??;
        let result = async {
            let api = route.api(&node)?;
            if let Some(snapshot) = &previous
                && snapshot.catalog.sync_protocol.as_deref() == Some(PROTOCOL)
            {
                let value = api
                    .post(
                        &format!("/api/v1/nodes/{node}/projects/sync"),
                        serde_json::to_value(Index::from_catalog(&snapshot.catalog.catalog)?)?,
                    )
                    .await;
                match value {
                    Ok(value) => {
                        return serde_json::from_value::<SyncReply>(value)?
                            .apply(&node, &snapshot.catalog.catalog);
                    }
                    // Rolling back an owner's binary can remove the optional
                    // endpoint. Only an explicit 404 permits a fresh full read.
                    Err(error) if crate::client::not_found(&error) => {}
                    Err(error) => return Err(error),
                }
            }
            let value = api
                .post(
                    &format!("/api/v1/nodes/{node}/projects"),
                    serde_json::to_value(command)?,
                )
                .await?;
            // Deserialize and reserialize only the authoritative fields. A peer
            // cannot supply local sync state or claim that its answer is cached.
            let catalog: RemoteCatalog = serde_json::from_value(value)?;
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
            // Config mutations take this same lock before erasing snapshots.
            // Recheck after I/O so a forgotten or rotated route cannot repopulate
            // its cache with the response from an older connection.
            let _guard = config::lock_credentials(&store.path)?;
            let config = Config::read(&store.path)?;
            let current = config.routes.get(&node).context("direct route was removed")?;
            ensure!(fingerprint(&config.node_id, &node, current)? == binding,
                "direct connection changed; refresh the project catalog");
            match result {
                Ok(catalog) => {
                    let mut value = serde_json::to_value(&catalog)?;
                    let snapshot = Snapshot {
                        version: 1, binding,
                        synced_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().try_into()?,
                        catalog,
                    };
                    let saved = (|| {
                        let bytes = serde_json::to_vec(&snapshot)?;
                        ensure!(bytes.len() as u64 <= MAX_SNAPSHOT_BYTES, "offline project catalog too large");
                        config::write_private(&store.project_snapshot_path(&node)?, &bytes, true)
                    })();
                    if saved.is_err() {
                        // A completed remote mutation must not appear to fail and
                        // invite a duplicate retry because this local cache failed.
                        value["sync"] = json!({"state":"online", "warning":"Offline catalog could not be saved on this device."});
                    }
                    Ok(value)
                }
                Err(error) if outage && list => {
                    if let Some(snapshot) = store.read_project_snapshot(&node, &binding).ok().flatten() {
                        let mut value = serde_json::to_value(snapshot.catalog)?;
                        value["permissions"] = json!({"projects":true, "exec":false, "terminals":false, "desktop":false});
                        value["sync"] = json!({"state":"offline", "synced_at":snapshot.synced_at});
                        return Ok(value);
                    }
                    Err(error)
                }
                Err(error) => {
                    if !outage { store.forget_project_snapshot(&node)?; }
                    Err(error)
                }
            }
        }).await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshots_are_bound_to_identity_endpoint_and_credential() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("peers.json"));
        let token = crate::config::token();
        store
            .connect(
                "target".into(),
                "http://localhost:9123".into(),
                token.clone(),
                false,
            )
            .unwrap();
        let config = Config::read(&store.path).unwrap();
        let route = config.routes["target"].clone();
        let binding = fingerprint(&config.node_id, "target", &route).unwrap();
        assert_ne!(
            binding,
            fingerprint("another-caller", "target", &route).unwrap()
        );
        let mut other = route.clone();
        other.url = "http://localhost:9124".into();
        assert_ne!(
            binding,
            fingerprint(&config.node_id, "target", &other).unwrap()
        );
        let snapshot = Snapshot {
            version: 1,
            binding: binding.clone(),
            synced_at: 123,
            catalog: RemoteCatalog {
                owner: "target".into(),
                sync_protocol: None,
                permissions: Permissions {
                    projects: true,
                    ..Permissions::default()
                },
                catalog: Catalog::default(),
            },
        };
        let path = store.project_snapshot_path("target").unwrap();
        config::write_private(&path, &serde_json::to_vec(&snapshot).unwrap(), true).unwrap();
        assert!(
            store
                .read_project_snapshot("target", &binding)
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .read_project_snapshot("target", "different")
                .unwrap()
                .is_none()
        );
        assert!(!fs::read_to_string(&path).unwrap().contains(&token));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        config::write_private(&route.token_file, crate::config::token().as_bytes(), true).unwrap();
        assert!(
            store
                .read_project_snapshot(
                    "target",
                    &fingerprint(&config.node_id, "target", &route).unwrap()
                )
                .unwrap()
                .is_none()
        );
        assert_ne!(path, store.project_snapshot_path("TARGET").unwrap());
        fs::write(&path, b"broken").unwrap();
        assert!(store.read_project_snapshot("target", &binding).is_err());
        store.forget_project_snapshot("target").unwrap();
        assert!(!path.exists());
    }
}
