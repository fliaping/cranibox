//! Persistent project metadata. Working trees and shells stay on their owning device.
use crate::{config, protocol::valid_node_id};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf};
use uuid::Uuid;
#[cfg(feature = "client")]
pub mod copy;
pub mod handoff;
pub mod sync;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub replicas: Vec<Replica>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Replica {
    pub id: String,
    /// Legacy catalogs use None for the Workspace host. Embedded hosts claim
    /// those replicas with their persistent NodeID before serving the catalog.
    pub node: Option<String>,
    pub path: String,
    pub sessions: Vec<Session>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub terminal_id: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Catalog {
    pub revision: u64,
    pub projects: Vec<Project>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    List {},
    Register {
        #[serde(default)]
        project_id: Option<String>,
        name: String,
        node: Option<String>,
        path: String,
    },
    Bind {
        replica_id: String,
        terminal_id: String,
    },
    Unbind {
        replica_id: String,
        terminal_id: String,
    },
    Forget {
        replica_id: String,
    },
}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
}
impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    #[cfg(feature = "client")]
    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }
    pub fn ensure_node_available(&self, target: &str) -> Result<()> {
        let _lock = config::lock_credentials(&self.path)?;
        ensure!(
            !self
                .read()?
                .projects
                .iter()
                .flat_map(|p| &p.replicas)
                .any(|r| r.node.as_deref() == Some(target)),
            "target node ID already owns project records"
        );
        Ok(())
    }
    /// Apply administrator-approved identity aliases without changing project,
    /// replica or Shell binding IDs. Retrying after an interrupted write is safe.
    pub fn resolve_nodes(&self, aliases: &BTreeMap<String, String>) -> Result<()> {
        if aliases.is_empty() {
            return Ok(());
        }
        let _lock = config::lock_credentials(&self.path)?;
        let mut catalog = self.read()?;
        let mut changed = false;
        for replica in catalog.projects.iter_mut().flat_map(|p| &mut p.replicas) {
            if let Some(target) = replica.node.as_ref().and_then(|id| aliases.get(id)) {
                replica.node = Some(target.clone());
                changed = true;
            }
        }
        if changed {
            catalog.revision = catalog
                .revision
                .checked_add(1)
                .context("project revision overflow")?;
            let bytes = serde_json::to_vec(&catalog)?;
            ensure!(bytes.len() <= 1024 * 1024, "project catalog too large");
            config::write_private(&self.path, &bytes, true)?;
        }
        Ok(())
    }
    /// Preserve all stable IDs and bindings when upgrading legacy host replicas.
    /// The original file remains authoritative; there is no second catalog to merge.
    pub fn claim_host(&self, owner: &str) -> Result<()> {
        ensure!(valid_node_id(owner), "invalid owner node ID");
        let _lock = config::lock_credentials(&self.path)?;
        let mut catalog = self.read()?;
        let mut changed = false;
        for replica in catalog.projects.iter_mut().flat_map(|p| &mut p.replicas) {
            if replica.node.is_none() {
                replica.node = Some(owner.to_owned());
                changed = true;
            }
        }
        if changed {
            catalog.revision = catalog
                .revision
                .checked_add(1)
                .context("project revision overflow")?;
            let bytes = serde_json::to_vec(&catalog)?;
            ensure!(bytes.len() <= 1024 * 1024, "project catalog too large");
            config::write_private(&self.path, &bytes, true)?;
        }
        Ok(())
    }
    fn read(&self) -> Result<Catalog> {
        match fs::File::open(&self.path) {
            Ok(file) => {
                ensure!(
                    file.metadata()?.len() <= 1024 * 1024,
                    "project catalog too large"
                );
                serde_json::from_reader(file).context("read project catalog")
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Catalog::default()),
            Err(error) => Err(error.into()),
        }
    }
    pub fn replica(&self, id: &str, allowed: impl Fn(Option<&str>) -> bool) -> Result<Replica> {
        self.read()?
            .projects
            .into_iter()
            .flat_map(|p| p.replicas)
            .find(|r| r.id == id && allowed(r.node.as_deref()))
            .context("project replica unavailable")
    }
    pub fn apply(
        &self,
        command: Command,
        allowed: impl Fn(Option<&str>) -> bool,
    ) -> Result<Catalog> {
        self.apply_inner(command, allowed)
    }
    /// A node owns only its local replicas. An explicit ProjectID joins an existing
    /// logical project without importing another node's paths or permissions.
    pub fn apply_owned(&self, command: Command, owner: &str) -> Result<Catalog> {
        ensure!(valid_node_id(owner), "invalid owner node ID");
        self.apply_inner(command, |node| node == Some(owner))
    }
    fn apply_inner(
        &self,
        command: Command,
        allowed: impl Fn(Option<&str>) -> bool,
    ) -> Result<Catalog> {
        // Serialize read/modify/rename across both listeners and processes.
        let _lock = config::lock_credentials(&self.path)?;
        let mut catalog = self.read()?;
        let mut changed = false;
        let binding = matches!(&command, Command::Bind { .. });
        match command {
            Command::List {} => {}
            Command::Forget { replica_id } => {
                ensure!(
                    catalog
                        .projects
                        .iter()
                        .flat_map(|p| &p.replicas)
                        .any(|r| r.id == replica_id && allowed(r.node.as_deref())),
                    "project replica unavailable"
                );
                for project in &mut catalog.projects {
                    project.replicas.retain(|r| r.id != replica_id);
                }
                catalog.projects.retain(|p| !p.replicas.is_empty());
                changed = true;
            }
            Command::Register {
                project_id,
                name,
                node,
                path,
            } => {
                ensure!(
                    allowed(node.as_deref()),
                    "target is outside this device group"
                );
                ensure!(
                    node.as_ref().is_none_or(|n| valid_node_id(n)),
                    "invalid node ID"
                );
                let name = name.trim();
                ensure!(
                    !name.is_empty()
                        && name.chars().count() <= 80
                        && !name.chars().any(char::is_control),
                    "project name must be 1..80 characters"
                );
                ensure!(
                    absolute_path(&path)
                        && path.len() <= 4096
                        && !path.chars().any(char::is_control),
                    "use an absolute directory on the target device"
                );
                if let Some(id) = &project_id {
                    ensure!(
                        catalog.projects.iter().any(|p| &p.id == id
                            && p.replicas.iter().any(|r| allowed(r.node.as_deref())))
                            || Uuid::parse_str(id).is_ok()
                                && !catalog.projects.iter().any(|p| &p.id == id),
                        "project unavailable"
                    );
                }
                let existing = catalog.projects.iter().find(|p| {
                    p.replicas
                        .iter()
                        .any(|r| r.node == node && path_key(&r.path) == path_key(&path))
                });
                if let Some(existing) = existing {
                    ensure!(
                        project_id.as_ref().is_none_or(|id| id == &existing.id),
                        "directory already belongs to another project"
                    );
                } else {
                    ensure!(
                        catalog
                            .projects
                            .iter()
                            .map(|p| p.replicas.len())
                            .sum::<usize>()
                            < 512,
                        "project replica limit reached"
                    );
                    let replica = Replica {
                        id: Uuid::new_v4().to_string(),
                        node,
                        path,
                        sessions: vec![],
                    };
                    if let Some(project) = project_id
                        .as_ref()
                        .and_then(|id| catalog.projects.iter_mut().find(|p| &p.id == id))
                    {
                        project.replicas.push(replica);
                    } else {
                        catalog.projects.push(Project {
                            id: project_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
                            name: name.into(),
                            replicas: vec![replica],
                        });
                    }
                    changed = true;
                }
            }
            Command::Bind {
                replica_id,
                terminal_id,
            }
            | Command::Unbind {
                replica_id,
                terminal_id,
            } => {
                ensure!(Uuid::parse_str(&terminal_id).is_ok(), "invalid terminal ID");
                let replica = catalog
                    .projects
                    .iter_mut()
                    .flat_map(|p| &mut p.replicas)
                    .find(|r| r.id == replica_id && allowed(r.node.as_deref()))
                    .context("project replica unavailable")?;
                let present = replica
                    .sessions
                    .iter()
                    .any(|s| s.terminal_id == terminal_id);
                if binding && !present {
                    ensure!(replica.sessions.len() < 64, "project session limit reached");
                    replica.sessions.push(Session {
                        id: Uuid::new_v4().to_string(),
                        terminal_id,
                    });
                    changed = true;
                } else if !binding && present {
                    replica.sessions.retain(|s| s.terminal_id != terminal_id);
                    changed = true;
                }
            }
        }
        if changed {
            catalog.revision += 1;
            let bytes = serde_json::to_vec(&catalog)?;
            ensure!(bytes.len() <= 1024 * 1024, "project catalog too large");
            config::write_private(&self.path, &bytes, true)?;
        }
        for project in &mut catalog.projects {
            project.replicas.retain(|r| allowed(r.node.as_deref()));
        }
        catalog.projects.retain(|p| !p.replicas.is_empty());
        Ok(catalog)
    }
}
fn absolute_path(path: &str) -> bool {
    path.starts_with('/')
        || (path.len() >= 3
            && path.as_bytes()[0].is_ascii_alphabetic()
            && path.as_bytes()[1] == b':'
            && matches!(path.as_bytes()[2], b'/' | b'\\'))
        || path.starts_with("\\\\")
}
fn path_key(path: &str) -> String {
    if path.as_bytes().get(1) == Some(&b':') || path.starts_with("\\\\") {
        path.replace('\\', "/").trim_end_matches('/').to_lowercase()
    } else {
        path.trim_end_matches('/').to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn register(node: Option<&str>, path: &str, project_id: Option<String>) -> Command {
        Command::Register {
            project_id,
            name: "My project".into(),
            node: node.map(str::to_owned),
            path: path.into(),
        }
    }
    #[test]
    fn identity_aliases_preserve_project_replica_and_shell_ids_and_only_write_once() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("projects.json");
        let store = Store::new(path.clone());
        let first = store
            .apply(register(Some("old"), "/project", None), |_| true)
            .unwrap();
        let before = store
            .apply(
                Command::Bind {
                    replica_id: first.projects[0].replicas[0].id.clone(),
                    terminal_id: Uuid::new_v4().to_string(),
                },
                |_| true,
            )
            .unwrap();
        assert!(store.ensure_node_available("old").is_err());
        store.ensure_node_available("node-new").unwrap();
        let aliases = BTreeMap::from([("old".into(), "node-new".into())]);
        store.resolve_nodes(&aliases).unwrap();
        let after = store.read().unwrap();
        let mut expected = serde_json::to_value(&before).unwrap();
        expected["revision"] = (before.revision + 1).into();
        expected["projects"][0]["replicas"][0]["node"] = "node-new".into();
        assert_eq!(serde_json::to_value(after).unwrap(), expected);
        let bytes = fs::read(&path).unwrap();
        store.resolve_nodes(&aliases).unwrap();
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert!(store.ensure_node_available("node-new").is_err());
    }
    #[test]
    fn claiming_legacy_host_preserves_ids_bindings_and_other_devices() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("projects.json");
        let store = Store::new(path.clone());
        let first = store
            .apply(register(None, "/workspace/project", None), |_| true)
            .unwrap();
        let project = first.projects[0].id.clone();
        let replica = first.projects[0].replicas[0].id.clone();
        store
            .apply(
                register(Some("remote"), "C:/project", Some(project)),
                |_| true,
            )
            .unwrap();
        let before = store
            .apply(
                Command::Bind {
                    replica_id: replica,
                    terminal_id: Uuid::new_v4().to_string(),
                },
                |_| true,
            )
            .unwrap();
        assert!(store.claim_host("../invalid").is_err());
        store.claim_host("host").unwrap();
        let after = Store::new(path.clone()).read().unwrap();
        assert_eq!(after.revision, before.revision + 1);
        let mut expected = serde_json::to_value(before).unwrap();
        expected["revision"] = after.revision.into();
        expected["projects"][0]["replicas"][0]["node"] = "host".into();
        assert_eq!(serde_json::to_value(&after).unwrap(), expected);
        let bytes = fs::read(&path).unwrap();
        store.claim_host("host").unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let visible = store.apply_owned(Command::List {}, "host").unwrap();
        assert_eq!(visible.projects[0].replicas.len(), 1);
        assert_eq!(visible.projects[0].replicas[0].sessions.len(), 1);
    }
    #[test]
    fn identity_survives_reopen_and_retries_and_paths_are_device_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("projects.json");
        let store = Store::new(path.clone());
        let first = store
            .apply(register(Some("one"), "/project", None), |_| true)
            .unwrap();
        let id = first.projects[0].id.clone();
        let retry = Store::new(path)
            .apply(register(Some("one"), "/project/", None), |_| true)
            .unwrap();
        assert_eq!(retry.projects[0].id, id);
        assert_eq!(retry.revision, first.revision);
        let copies = store
            .apply(
                register(Some("two"), "C:/Projects/app", Some(id.clone())),
                |_| true,
            )
            .unwrap();
        assert_eq!(copies.projects.len(), 1);
        assert_eq!(copies.projects[0].replicas.len(), 2);
        assert_ne!(
            copies.projects[0].replicas[0].id,
            copies.projects[0].replicas[1].id
        );
        assert!(
            store
                .apply(
                    register(Some("one"), "/project", Some("unknown".into())),
                    |_| true
                )
                .is_err()
        );
    }
    #[test]
    fn filtering_applies_to_replicas_and_session_mutations() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(temp.path().join("projects.json"));
        let first = store
            .apply(register(Some("one"), "/private/one", None), |_| true)
            .unwrap();
        let id = first.projects[0].id.clone();
        let both = store
            .apply(register(Some("two"), "/private/two", Some(id)), |_| true)
            .unwrap();
        store
            .apply(register(None, "/workspace/secret", None), |_| true)
            .unwrap();
        let one = |node: Option<&str>| node == Some("one");
        let visible = store.apply(Command::List {}, one).unwrap();
        assert_eq!(visible.projects.len(), 1);
        assert_eq!(visible.projects[0].replicas.len(), 1);
        assert!(
            !serde_json::to_string(&visible)
                .unwrap()
                .contains("/private/two")
        );
        assert!(
            store
                .apply(register(None, "/unauthorized", None), one)
                .is_err()
        );
        assert!(
            store
                .apply(
                    Command::Bind {
                        replica_id: both.projects[0].replicas[1].id.clone(),
                        terminal_id: Uuid::new_v4().to_string()
                    },
                    one
                )
                .is_err()
        );
        let replica_id = visible.projects[0].replicas[0].id.clone();
        let terminal_id = Uuid::new_v4().to_string();
        let binding = Command::Bind {
            replica_id: replica_id.clone(),
            terminal_id: terminal_id.clone(),
        };
        let linked = store.apply(binding.clone(), one).unwrap();
        let retry = store.apply(binding, one).unwrap();
        assert_eq!(linked.revision, retry.revision);
        assert_eq!(
            linked.projects[0].replicas[0].sessions[0].id,
            retry.projects[0].replicas[0].sessions[0].id
        );
        assert!(
            store
                .apply(
                    Command::Unbind {
                        replica_id,
                        terminal_id
                    },
                    one
                )
                .unwrap()
                .projects[0]
                .replicas[0]
                .sessions
                .is_empty()
        );
    }
    #[test]
    fn nodes_share_project_identity_without_importing_each_others_paths() {
        let temp = tempfile::tempdir().unwrap();
        let one = Store::new(temp.path().join("one.json"));
        let two_path = temp.path().join("two.json");
        let two = Store::new(two_path.clone());
        let first = one
            .apply_owned(register(Some("one"), "/private/one", None), "one")
            .unwrap();
        let id = first.projects[0].id.clone();
        let second = two
            .apply_owned(
                register(Some("two"), "C:/Projects/app", Some(id.clone())),
                "two",
            )
            .unwrap();
        assert_eq!(second.projects[0].id, id);
        assert_ne!(
            first.projects[0].replicas[0].id,
            second.projects[0].replicas[0].id
        );
        assert!(
            !serde_json::to_string(&second)
                .unwrap()
                .contains("/private/one")
        );
        let reopened = Store::new(two_path)
            .apply_owned(Command::List {}, "two")
            .unwrap();
        assert_eq!(reopened.projects[0].id, id);
        assert_eq!(reopened.revision, second.revision);
        assert!(
            two.apply_owned(register(None, "/workspace/secret", None), "two")
                .is_err()
        );
        assert!(
            two.apply_owned(register(Some("one"), "/wrong/owner", None), "two")
                .is_err()
        );
        assert!(
            two.apply_owned(
                Command::Forget {
                    replica_id: first.projects[0].replicas[0].id.clone()
                },
                "two"
            )
            .is_err()
        );
        assert!(two.apply_owned(Command::List {}, "../invalid").is_err());
    }
    #[test]
    fn explicit_project_identity_does_not_bypass_existing_catalog_scope() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(temp.path().join("projects.json"));
        let first = store
            .apply_owned(register(Some("one"), "/private/one", None), "one")
            .unwrap();
        let id = first.projects[0].id.clone();
        assert!(
            store
                .apply_owned(
                    register(Some("two"), "/private/two", Some(id.clone())),
                    "two"
                )
                .is_err()
        );
        assert!(
            store
                .apply_owned(Command::List {}, "two")
                .unwrap()
                .projects
                .is_empty()
        );
        assert_eq!(
            store.apply_owned(Command::List {}, "one").unwrap().projects[0].id,
            id
        );
    }
    #[test]
    fn concurrent_writers_do_not_lose_projects_and_bad_files_are_not_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("projects.json");
        let workers: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    Store::new(path)
                        .apply(
                            register(Some("one"), &format!("/project/{i}"), None),
                            |_| true,
                        )
                        .unwrap()
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(
            Store::new(path.clone())
                .apply(Command::List {}, |_| true)
                .unwrap()
                .projects
                .len(),
            8
        );
        fs::write(&path, "broken").unwrap();
        assert!(
            Store::new(path.clone())
                .apply(register(Some("one"), "/new", None), |_| true)
                .is_err()
        );
        assert_eq!(fs::read_to_string(path).unwrap(), "broken");
    }
    #[test]
    fn relative_and_control_character_paths_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(temp.path().join("projects.json"));
        for path in [".", "relative/path", "C:relative", "/project\nother"] {
            assert!(
                store
                    .apply(register(Some("one"), path, None), |_| true)
                    .is_err()
            );
        }
    }
}
