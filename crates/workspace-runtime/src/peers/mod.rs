//! Shared outbound routes for the node CLI, Web and Tauri.
pub mod agent_sessions;
pub mod browser;
pub mod computer;
pub mod config;
mod copies;
pub mod desktop;
pub mod e2e;
pub mod executions;
pub mod identity;
pub mod local_forward;
pub mod messages;
mod paths;
pub mod project_sync;
mod projects;
pub mod space_catalog;
pub mod tasks;
pub mod terminal;

use self::config::{Config, Grant, Permissions, Route};
use crate::{
    client::job_path,
    protocol::{ExecRequest, valid_node_id},
};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    List,
    Messages {
        command: messages::Command,
    },
    Tasks {
        command: tasks::Command,
    },
    AgentSessions {
        node: Option<String>,
        command: agent_sessions::Command,
    },
    Executions {
        node: Option<String>,
        command: executions::Command,
    },
    Identity,
    SpaceCatalog {
        node: String,
    },
    CopyRoot {
        path: Option<PathBuf>,
    },
    CopyPrepare {
        source: String,
        target: String,
        replica_id: String,
        handoff: Option<crate::projects::handoff::Handoff>,
    },
    CopyAgentPrepare {
        id: String,
        adapter: executions::Adapter,
        access: executions::Access,
    },
    CopyAgentStart {
        id: String,
        adapter: executions::Adapter,
        access: executions::Access,
        digest: String,
    },
    CopyAgentStatus {
        id: String,
        #[serde(default)]
        stdout_offset: u64,
        #[serde(default)]
        stderr_offset: u64,
    },
    CopyAgentStop {
        id: String,
    },
    CopyCommit {
        id: String,
        digest: String,
    },
    CopyPrepareResume {
        id: String,
    },
    CopyStatus {
        id: String,
    },
    CopyList,
    CopyActivity,
    CopyDiscard {
        id: String,
    },
    Listener {
        listener: Option<config::Listener>,
    },
    Desktop {
        desktop: Option<config::Desktop>,
    },
    ComputerSharing {
        computer: Option<computer::Provider>,
    },
    BrowserSharing {
        browser: Option<browser::Provider>,
    },
    BrowserOwner {
        command: browser::OwnerCommand,
    },
    Browser {
        node: String,
        command: browser::Request,
    },
    ComputerOwner {
        command: computer::OwnerCommand,
    },
    Computer {
        node: String,
        command: computer::Request,
    },
    Grant {
        node: String,
        permissions: Permissions,
    },
    Revoke {
        node: String,
    },
    /// Confirm a pinned peer key after both owners compared the pairing code.
    Verify {
        node: String,
        code: String,
    },
    Connect {
        node: String,
        url: String,
        #[serde(default)]
        token: String,
        #[serde(default)]
        allow_private_http: bool,
    },
    AddPath {
        node: String,
        url: String,
        token: String,
        #[serde(default)]
        allow_private_http: bool,
    },
    PreferPath {
        node: String,
        url: String,
    },
    RemovePath {
        node: String,
        url: String,
    },
    Forget {
        node: String,
    },
    Probe {
        node: String,
    },
    Exec {
        node: String,
        request: ExecRequest,
    },
    Job {
        node: String,
        id: String,
    },
    Cancel {
        node: String,
        id: String,
    },
    Sessions {
        node: String,
        command: Value,
    },
    Projects {
        node: String,
        command: crate::projects::Command,
    },
    ProjectSync {
        command: project_sync::Command,
    },
}

/// Mark pinned keys for `node` as verified when `code` equals their pairing code.
fn verify_pairing(path: &Path, node: &str, code: &str) -> Result<Value> {
    ensure!(valid_node_id(node), "invalid node ID");
    let code = identity::normalize_code(code);
    ensure!(code.len() == 12, "enter the 12-digit pairing code");
    let mut verified = vec![];
    let value = Config::update(path, |config| {
        let own = config
            .identity
            .clone()
            .context("node identity unavailable")?;
        let matches = |pinned: &config::Pinned| {
            identity::normalize_code(&identity::pairing_code(&own, &pinned.identity)) == code
        };
        if let Some(pinned) = config.grants.get_mut(node).and_then(|g| g.peer.as_mut())
            && matches(pinned)
        {
            pinned.verified = true;
            verified.push("grant");
        }
        if let Some(pinned) = config
            .routes
            .get_mut(node)
            .and_then(|r| r.target_identity.as_mut())
            && matches(pinned)
        {
            pinned.verified = true;
            verified.push("route");
        }
        ensure!(
            !verified.is_empty(),
            "pairing code does not match a key paired with this node; nothing was changed"
        );
        Ok(())
    })?;
    Ok(json!({"node":node,"verified":verified,"catalog":value.public_view()}))
}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
    local_projects: crate::projects::Store,
}
impl Store {
    pub fn new(path: PathBuf) -> Self {
        let local_projects = crate::projects::Store::new(path.with_file_name("node-projects.json"));
        Self {
            path,
            local_projects,
        }
    }
    /// Embedded hosts retain their existing catalog instead of opening a second one.
    pub fn with_local_projects(mut self, catalog: crate::projects::Store) -> Self {
        self.local_projects = catalog;
        self
    }
    pub fn list(&self) -> Result<Value> {
        match Config::read(&self.path) {
            Ok(value) => Ok(value.public_view()),
            Err(_) if !self.path.try_exists()? => {
                Ok(json!({"node_id":null,"listener":null,"routes":[],"grants":[]}))
            }
            Err(error) => Err(error),
        }
    }
    pub fn route(&self, node: &str) -> Result<Route> {
        ensure!(valid_node_id(node), "invalid node ID");
        Config::read(&self.path)?
            .routes
            .remove(node)
            .context("direct route is not configured")
    }
    fn connect(
        &self,
        node: String,
        url: String,
        token: String,
        allow_private_http: bool,
    ) -> Result<Value> {
        self.connect_route(node, url, token, allow_private_http, None, true)
    }
    /// Explicit discovery can add a path using the same target grant, never replace credentials.
    pub fn connect_discovered(&self, node: String, url: String, token: String) -> Result<Value> {
        if Config::read(&self.path).is_ok_and(|c| c.routes.contains_key(&node)) {
            return self.append_path(node, url, token, true, None);
        }
        self.connect_route(node, url, token, true, None, false)
    }
    /// Used only by the local network owner, after it has bound the forwarding port.
    pub fn connect_forward(
        &self,
        node: String,
        url: String,
        token: String,
        lease: PathBuf,
    ) -> Result<Value> {
        local_forward::ensure_running(&lease)?;
        if Config::read(&self.path).is_ok_and(|c| c.routes.contains_key(&node)) {
            return self.append_path(node, url, token, false, Some(lease));
        }
        self.connect_route(node, url, token, false, Some(lease), true)
    }
    fn connect_route(
        &self,
        node: String,
        url: String,
        token: String,
        allow_private_http: bool,
        local_forward_lock: Option<PathBuf>,
        replace_existing: bool,
    ) -> Result<Value> {
        ensure!(valid_node_id(&node), "invalid node ID");
        let origin = crate::config::check_url(url.trim(), false, allow_private_http)?;
        ensure!(origin.path() == "/", "use an HTTP(S) origin without a path");
        let url = origin.origin().ascii_serialization();
        let token = token.trim();
        ensure!(
            token.is_empty() || (token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())),
            "token must contain 64 hexadecimal characters"
        );
        let parent = self
            .path
            .parent()
            .context("peer configuration directory required")?;
        // New files keep a failed configuration save from breaking a working route.
        let mut created = None;
        let mut previous = None;
        let saved = Config::update(&self.path, |value| {
            ensure!(node != value.node_id, "cannot connect a node to itself");
            let existing = value.routes.get(&node);
            ensure!(
                replace_existing || existing.is_none(),
                "A connection for this node already exists. Edit or remove it in Direct devices before choosing another provider."
            );
            let token_file = if token.is_empty() {
                let existing = existing.context("token required for a new route")?;
                ensure!(
                    existing.url.trim_end_matches('/') == url,
                    "changing the endpoint requires its token"
                );
                crate::config::read_token(&existing.token_file)?;
                existing.token_file.clone()
            } else {
                std::fs::create_dir_all(parent)?;
                std::fs::canonicalize(parent)?
                    .join(format!(".route-{}.token", uuid::Uuid::new_v4()))
            };
            previous = existing
                .map(|r| r.token_file.clone())
                .filter(|p| p != &token_file);
            if !token.is_empty() {
                self.forget_project_snapshot(&node)?;
            }
            value.routes.insert(
                node.clone(),
                Route {
                    url: url.clone(),
                    token_file: token_file.clone(),
                    allow_insecure: allow_private_http,
                    alternatives: if token.is_empty() {
                        existing.map(|r| r.alternatives.clone()).unwrap_or_default()
                    } else {
                        vec![]
                    },
                    local_forward_lock: local_forward_lock.clone().or_else(|| {
                        token
                            .is_empty()
                            .then(|| existing.and_then(|route| route.local_forward_lock.clone()))
                            .flatten()
                    }),
                    target_identity: None,
                    config_path: None,
                },
            );
            value.validate()?;
            if !token.is_empty() {
                crate::config::write_private(&token_file, format!("{token}\n").as_bytes(), false)?;
                created = Some(token_file);
            }
            Ok(())
        });
        match saved {
            Ok(value) => {
                if let Some(path) = previous {
                    self.remove_managed_token(&path);
                }
                Ok(value.public_view())
            }
            Err(error) => {
                if let Some(path) = created {
                    let _ = std::fs::remove_file(path);
                }
                Err(error)
            }
        }
    }
    fn remove_managed_token(&self, path: &Path) {
        let owned_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix(".route-"))
            .and_then(|s| s.strip_suffix(".token"))
            .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok());
        let owned_directory = self
            .path
            .parent()
            .and_then(|p| std::fs::canonicalize(p).ok())
            .is_some_and(|parent| path.parent() == Some(parent.as_path()));
        if owned_name && owned_directory {
            let _ = std::fs::remove_file(path);
        }
    }
    pub async fn manage(&self, command: Command) -> Result<Value> {
        match command {
            Command::List => self.list(),
            Command::ProjectSync { command } => {
                let path = self.path.clone();
                tokio::task::spawn_blocking(move || project_sync::manage(&path, command)).await?
            }
            Command::SpaceCatalog { node } => self.space_catalog(&node).await,
            Command::Messages { command } => self.messages(command).await,
            Command::Tasks { command } => self.tasks(command).await,
            Command::AgentSessions { node, command } => self.agent_sessions(node, command).await,
            Command::Executions { node, command } => self.node_executions(node, command).await,
            Command::CopyRoot { path: root } => {
                let path = self.path.clone();
                tokio::task::spawn_blocking(move || {
                    let root = root.map(|p| crate::projects::copy::root(&p)).transpose()?;
                    Config::update(&path, |c| {
                        c.copy_root = root;
                        Ok(())
                    })
                    .map(|c| c.public_view())
                })
                .await?
            }
            Command::CopyPrepare {
                source,
                target,
                replica_id,
                handoff,
            } => self.copy_prepare(source, target, replica_id, handoff).await,
            Command::CopyAgentPrepare {
                id,
                adapter,
                access,
            } => self.copy_agent_prepare(id, adapter, access).await,
            Command::CopyAgentStart {
                id,
                adapter,
                access,
                digest,
            } => self.copy_agent_start(id, adapter, access, digest).await,
            Command::CopyAgentStatus {
                id,
                stdout_offset,
                stderr_offset,
            } => {
                self.copy_agent_status(id, stdout_offset, stderr_offset)
                    .await
            }
            Command::CopyAgentStop { id } => self.copy_agent_stop(id).await,
            Command::CopyCommit { id, digest } => self.copy_commit(id, digest).await,
            Command::CopyPrepareResume { id } => self.copy_prepare_resume(id).await,
            Command::CopyStatus { id } => self.copy_status(id).await,
            Command::CopyList => self.copy_list(),
            Command::CopyActivity => self.copy_activity(),
            Command::CopyDiscard { id } => self.copy_discard(id),
            Command::Listener { .. } => {
                anyhow::bail!("listener changes require a running local node")
            }
            Command::Identity => {
                let path = self.path.clone();
                tokio::task::spawn_blocking(move || {
                    Config::update(&path, |_| Ok(())).map(|c| c.public_view())
                })
                .await?
            }
            Command::Desktop { desktop } => {
                let path = self.path.clone();
                tokio::task::spawn_blocking(move || {
                    Config::update(&path, |config| {
                        config.desktop = desktop;
                        config.desktop_epoch = config
                            .desktop_epoch
                            .checked_add(1)
                            .context("desktop generation exhausted")?;
                        Ok(())
                    })
                    .map(|c| c.public_view())
                })
                .await?
            }
            Command::ComputerSharing { computer } => {
                let path = self.path.clone();
                tokio::task::spawn_blocking(move || {
                    Config::update(&path, |config| {
                        config.computer = computer;
                        config.computer_epoch = config
                            .computer_epoch
                            .checked_add(1)
                            .context("computer generation exhausted")?;
                        Ok(())
                    })
                    .map(|config| config.public_view())
                })
                .await?
            }
            Command::BrowserSharing { browser } => {
                let path = self.path.clone();
                tokio::task::spawn_blocking(move || {
                    Config::update(&path, |c| {
                        c.browser = browser;
                        c.browser_epoch = c
                            .browser_epoch
                            .checked_add(1)
                            .context("browser generation exhausted")?;
                        Ok(())
                    })
                    .map(|c| c.public_view())
                })
                .await?
            }
            Command::BrowserOwner { command } => browser::owner(&self.path, command).await,
            Command::Browser { node, command } => self.browser(&node, command).await,
            Command::ComputerOwner { command } => computer::owner(&self.path, command).await,
            Command::Computer { node, command } => self.computer(&node, command).await,
            Command::Grant { node, permissions } => {
                let path = self.path.clone();
                tokio::task::spawn_blocking(move || {
                    ensure!(valid_node_id(&node), "invalid caller node ID");
                    ensure!(
                        permissions.exec
                            || permissions.terminals
                            || permissions.projects
                            || permissions.desktop
                            || permissions.computer
                            || permissions.browser
                            || permissions.messages
                            || permissions.spaces,
                        "choose command, terminal, project, desktop, computer, browser, message and/or space catalog access"
                    );
                    let token = crate::config::token();
                    let value = Config::update(&path, |value| {
                        ensure!(node != value.node_id, "cannot grant this node to itself");
                        ensure!(
                            !value.grants.contains_key(&node),
                            "caller already granted; revoke before rotating its credential"
                        );
                        value
                            .grants
                            .insert(node.clone(), Grant::issue(&token, permissions));
                        Ok(())
                    })?;
                    // The credential is returned only to this authenticated issuance request.
                    Ok(json!({"catalog":value.public_view(),"node":node,"token":token}))
                })
                .await?
            }
            Command::Verify { node, code } => {
                let path = self.path.clone();
                tokio::task::spawn_blocking(move || verify_pairing(&path, &node, &code)).await?
            }
            Command::Revoke { node } => {
                let path = self.path.clone();
                tokio::task::spawn_blocking(move || {
                    ensure!(valid_node_id(&node), "invalid caller node ID");
                    Config::update(&path, |value| {
                        ensure!(
                            value.grants.remove(&node).is_some(),
                            "caller is not granted"
                        );
                        Ok(())
                    })
                    .map(|c| c.public_view())
                })
                .await?
            }
            Command::Connect {
                node,
                url,
                token,
                allow_private_http,
            } => {
                let store = self.clone();
                tokio::task::spawn_blocking(move || {
                    store.connect(node, url, token, allow_private_http)
                })
                .await?
            }
            Command::AddPath {
                node,
                url,
                token,
                allow_private_http,
            } => self.add_path(node, url, token, allow_private_http).await,
            Command::PreferPath { node, url } => self.change_path(node, url, true),
            Command::RemovePath { node, url } => self.change_path(node, url, false),
            Command::Forget { node } => {
                let store = self.clone();
                tokio::task::spawn_blocking(move || {
                    ensure!(valid_node_id(&node), "invalid node ID");
                    let mut removed = None;
                    let value = Config::update(&store.path, |value| {
                        store.forget_project_snapshot(&node)?;
                        removed = Some(
                            value
                                .routes
                                .remove(&node)
                                .context("direct route is not configured")?,
                        );
                        Ok(())
                    })?;
                    if let Some(route) = removed {
                        store.remove_managed_token(&route.token_file);
                    }
                    Ok(value.public_view())
                })
                .await?
            }
            Command::Probe { node } => self.route(&node)?.api(&node)?.probe().await,
            Command::Exec { node, request } => {
                self.route(&node)?
                    .api(&node)?
                    .execute(&node, request, true)
                    .await
            }
            Command::Job { node, id } => {
                let mut value = self.route(&node)?.api(&node)?.get(&job_path(&id)?).await?;
                if let Some(object) = value.as_object_mut() {
                    object.remove("stdout_base64");
                    object.remove("stderr_base64");
                }
                Ok(value)
            }
            Command::Cancel { node, id } => {
                self.route(&node)?
                    .api(&node)?
                    .post(&format!("{}/cancel", job_path(&id)?), json!({}))
                    .await
            }
            Command::Sessions { node, command } => {
                ensure!(command.is_object(), "session command must be an object");
                self.route(&node)?
                    .api(&node)?
                    .post(&format!("/api/v1/nodes/{node}/terminal/sessions"), command)
                    .await
            }
            Command::Projects { node, command } => self.projects(node, command).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn incoming_grants_require_scopes_and_only_issue_a_secret_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("peers.json"));
        let identity = store.manage(Command::Identity).await.unwrap();
        for node in ["caller", identity["node_id"].as_str().unwrap()] {
            assert!(
                store
                    .manage(Command::Grant {
                        node: node.into(),
                        permissions: Permissions::default(),
                    })
                    .await
                    .is_err()
            );
        }
        assert!(
            store
                .manage(Command::Grant {
                    node: identity["node_id"].as_str().unwrap().into(),
                    permissions: Permissions {
                        exec: true,
                        terminals: false,
                        ..Permissions::default()
                    },
                })
                .await
                .is_err()
        );
        let issued = store
            .manage(Command::Grant {
                node: "caller".into(),
                permissions: Permissions {
                    exec: false,
                    terminals: true,
                    ..Permissions::default()
                },
            })
            .await
            .unwrap();
        let secret = issued["token"].as_str().unwrap();
        assert_eq!(secret.len(), 64);
        assert!(secret.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(!issued["catalog"].to_string().contains(secret));
        assert!(!store.list().unwrap().to_string().contains(secret));
        assert!(
            store
                .manage(Command::Grant {
                    node: "caller".into(),
                    permissions: Permissions {
                        exec: true,
                        terminals: true,
                        ..Permissions::default()
                    },
                })
                .await
                .is_err()
        );
        let saved = Config::read(&store.path).unwrap();
        assert!(saved.grants["caller"].verifies(secret));
        assert!(!saved.grants["caller"].permissions.exec);
        assert!(saved.grants["caller"].permissions.terminals);
        store
            .manage(Command::Revoke {
                node: "caller".into(),
            })
            .await
            .unwrap();
        assert_eq!(store.list().unwrap()["grants"], json!([]));
        assert!(
            store
                .manage(Command::Revoke {
                    node: "caller".into()
                })
                .await
                .is_err()
        );
        let rotated = store
            .manage(Command::Grant {
                node: "caller".into(),
                permissions: Permissions {
                    exec: true,
                    terminals: false,
                    ..Permissions::default()
                },
            })
            .await
            .unwrap();
        assert_ne!(rotated["token"], issued["token"]);
    }
    #[tokio::test]
    async fn forgetting_a_cli_route_preserves_its_external_credential() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("peers.json"));
        let credential = dir.path().join("cli.token");
        crate::config::write_private(&credential, crate::config::token().as_bytes(), false)
            .unwrap();
        Config::update(&store.path, |config| {
            config.routes.insert(
                "remote".into(),
                Route {
                    target_identity: None,
                    config_path: None,
                    alternatives: vec![],
                    url: "https://remote.example.com".into(),
                    token_file: credential.clone(),
                    allow_insecure: false,
                    local_forward_lock: None,
                },
            );
            Ok(())
        })
        .unwrap();
        let view = store
            .manage(Command::Forget {
                node: "remote".into(),
            })
            .await
            .unwrap();
        assert_eq!(view["routes"], json!([]));
        assert!(credential.exists());
    }
    #[test]
    fn discovered_connections_cannot_replace_existing_routes_or_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("peers.json"));
        let secret = crate::config::token();
        let view = store
            .connect_discovered(
                "remote".into(),
                "http://100.64.0.2:1234".into(),
                secret.clone(),
            )
            .unwrap();
        assert!(!view.to_string().contains(&secret));
        let before = std::fs::read(&store.path).unwrap();
        let credential = store.route("remote").unwrap().token_file;
        for url in ["http://100.64.0.2:1234", "http://100.64.0.3:4321"] {
            assert!(
                store
                    .connect_discovered("remote".into(), url.into(), crate::config::token())
                    .is_err()
            );
            assert_eq!(std::fs::read(&store.path).unwrap(), before);
            assert_eq!(crate::config::read_token(&credential).unwrap(), secret);
        }
        assert_eq!(
            std::fs::read_dir(dir.path())
                .unwrap()
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.file_name().to_string_lossy().ends_with(".token"))
                .count(),
            1
        );
        let own = Config::read(&store.path).unwrap().node_id;
        assert!(
            store
                .connect_discovered(own, "http://100.64.0.4:1234".into(), crate::config::token())
                .is_err()
        );
    }
    #[test]
    fn credentials_and_endpoint_updates_are_safe() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("peers.json"));
        assert!(store.list().unwrap()["node_id"].is_null());
        assert!(!store.path.exists());
        let secret = crate::config::token();
        let view = store
            .connect(
                "remote".into(),
                "http://localhost:9020".into(),
                secret.clone(),
                false,
            )
            .unwrap();
        assert!(!view.to_string().contains(&secret));
        let path = store.route("remote").unwrap().token_file;
        assert_eq!(crate::config::read_token(&path).unwrap(), secret);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(
            store
                .connect(
                    "remote".into(),
                    "http://localhost:9021".into(),
                    String::new(),
                    false
                )
                .is_err()
        );
        assert_eq!(store.route("remote").unwrap().url, "http://localhost:9020");
        assert!(
            store
                .connect(
                    "remote".into(),
                    "http://localhost:9020/".into(),
                    String::new(),
                    false
                )
                .is_ok()
        );
        store
            .connect(
                "remote".into(),
                "http://localhost:9021".into(),
                crate::config::token(),
                false,
            )
            .unwrap();
        assert!(!path.exists());
        assert!(
            store
                .connect(
                    "bad".into(),
                    "http://192.0.2.1".into(),
                    crate::config::token(),
                    false
                )
                .is_err()
        );
        assert!(
            store
                .connect(
                    "bad".into(),
                    "https://example.com/path".into(),
                    crate::config::token(),
                    false
                )
                .is_err()
        );
        std::fs::write(&store.path, b"broken").unwrap();
        assert!(
            store
                .connect("remote".into(), "https://example.com".into(), secret, false)
                .is_err()
        );
        assert_eq!(std::fs::read(&store.path).unwrap(), b"broken");
    }
}
