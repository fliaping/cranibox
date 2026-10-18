//! Local container workspaces. A space owns its data and joins the normal peer catalog.
pub mod applications;
mod engine;
mod provision;

use agent_workspace_runtime::{config as secrets, peers::config::Config};
use anyhow::{Context, Result, ensure};
pub use engine::{Provider, SetupAction};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use uuid::Uuid;

pub const DEFAULT_IMAGE: &str = "xuping/agent-workspace:ubuntu-xfce";

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum PullPolicy {
    #[default]
    Always,
    Missing,
    Never,
}

#[derive(Debug, Deserialize, clap::Subcommand)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    List,
    Preflight {
        #[arg(long, default_value = "docker")]
        provider: Provider,
    },
    Plan {
        action: SetupAction,
    },
    Install {
        action: SetupAction,
    },
    Create {
        name: String,
        #[arg(long, default_value = "docker")]
        provider: Provider,
        #[serde(default = "default_image")]
        #[arg(long, default_value = DEFAULT_IMAGE)]
        image: String,
        #[serde(default)]
        #[arg(long, value_enum, default_value = "always")]
        pull_policy: PullPolicy,
    },
    Resume {
        id: Uuid,
    },
    Stop {
        id: Uuid,
    },
    /// Remove only this managed container; its volume and enrollment are retained.
    Remove {
        id: Uuid,
    },
    Credentials {
        id: Uuid,
    },
}
fn default_image() -> String {
    DEFAULT_IMAGE.into()
}

#[derive(Clone, Deserialize, Serialize)]
struct Space {
    id: Uuid,
    name: String,
    provider: Provider,
    image: String,
    #[serde(default)]
    pull_policy: PullPolicy,
    node: String,
    owner: String,
    password: String,
    token: String,
    phase: String,
    error: Option<String>,
    #[serde(default)]
    ports: provision::Ports,
}
impl Space {
    fn container(&self) -> String {
        format!("cranibox-space-{}", self.id)
    }
    fn volume(&self) -> String {
        format!("cranibox-space-{}-data", self.id)
    }
    fn public(&self) -> Value {
        json!({"id":self.id,"name":self.name,"provider":self.provider,"image":self.image,"pull_policy":self.pull_policy,
            "node":self.node,"phase":self.phase,"error":self.error,"ports":self.ports,
            "container":self.container(),"volume":self.volume()})
    }
}
#[derive(Deserialize, Serialize)]
struct Catalog {
    version: u32,
    installation: Uuid,
    spaces: Vec<Space>,
    #[serde(default)]
    setup: Option<Setup>,
}
#[derive(Deserialize, Serialize)]
struct Setup {
    action: SetupAction,
    phase: String,
    error: Option<String>,
}

struct OperationLock(fs::File);
impl Drop for OperationLock {
    fn drop(&mut self) {
        // Explicitly release ownership even if a concurrent process spawn
        // briefly inherited the open file description before exec.
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

#[derive(Clone)]
pub struct Store {
    directory: PathBuf,
    peers: PathBuf,
}
impl Store {
    /// A peer may read metadata only. Do not recover operations, inspect Docker,
    /// expose local addresses, or return the private catalog's error strings.
    pub fn shared_catalog(
        &self,
        owner: &str,
    ) -> Result<agent_workspace_runtime::peers::space_catalog::Catalog> {
        use agent_workspace_runtime::peers::space_catalog::{Catalog, Summary};
        let catalog = self.read()?;
        ensure!(
            catalog.spaces.iter().all(|space| space.owner == owner),
            "space catalog owner changed"
        );
        let result = Catalog {
            owner: owner.into(),
            spaces: catalog
                .spaces
                .into_iter()
                .map(|space| Summary {
                    id: space.id,
                    name: space.name,
                    node: space.node,
                    provider: match space.provider {
                        Provider::Docker => "docker",
                        Provider::Colima => "colima",
                        Provider::Orbstack => "orbstack",
                        Provider::Wsl => "wsl",
                    }
                    .into(),
                    phase: space.phase,
                })
                .collect(),
        };
        result.validate(owner)?;
        Ok(result)
    }
    pub fn new(peers: PathBuf) -> Self {
        Self {
            directory: peers.with_file_name("spaces"),
            peers,
        }
    }
    fn read(&self) -> Result<Catalog> {
        let file = self.directory.join("catalog.json");
        if !file.try_exists()? {
            return Ok(Catalog {
                version: 1,
                installation: Uuid::new_v4(),
                spaces: vec![],
                setup: None,
            });
        }
        let value: Catalog = serde_json::from_slice(&fs::read(file)?)?;
        ensure!(
            value.version == 1 && value.spaces.len() <= 32,
            "unsupported space catalog"
        );
        Ok(value)
    }
    fn save(&self, catalog: &Catalog) -> Result<()> {
        secrets::write_private(
            &self.directory.join("catalog.json"),
            &serde_json::to_vec_pretty(catalog)?,
            true,
        )
    }
    fn lock(&self) -> Result<OperationLock> {
        fs::create_dir_all(&self.directory)?;
        let mut options = fs::OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(self.directory.join("operation.lock"))?;
        fs2::FileExt::try_lock_exclusive(&file)
            .context("another space operation is running; wait for it to finish")?;
        Ok(OperationLock(file))
    }
    pub fn list(&self) -> Result<Value> {
        let mut catalog = self.read()?;
        // A crash/reboot never leaves a wizard permanently busy. Only the lock owner mutates.
        if let Ok(_lock) = self.lock() {
            catalog = self.read()?;
            let mut changed = false;
            for space in &mut catalog.spaces {
                if busy(&space.phase) {
                    space.phase = "interrupted".into();
                    space.error = Some("Operation interrupted. Resume to reconcile the existing container and data.".into());
                    changed = true;
                }
            }
            if let Some(setup) = &mut catalog.setup
                && setup.phase == "installing"
            {
                setup.phase = "interrupted".into();
                setup.error =
                    Some("Setup interrupted. Check the environment before retrying.".into());
                changed = true;
            }
            if changed {
                self.save(&catalog)?;
            }
        }
        Ok(
            json!({"spaces":catalog.spaces.iter().map(Space::public).collect::<Vec<_>>(),"setup":catalog.setup,
            "platform":std::env::consts::OS,"default_image":DEFAULT_IMAGE}),
        )
    }
    /// Web/Tauri use background jobs; the CLI awaits the same operation to keep it alive.
    pub async fn manage(&self, command: Command, background: bool) -> Result<Value> {
        match command {
            Command::List => return self.list(),
            Command::Preflight { provider } => return engine::preflight(provider).await,
            Command::Plan { action } => return engine::plan(action),
            Command::Credentials { id } => {
                let space = self
                    .read()?
                    .spaces
                    .into_iter()
                    .find(|space| space.id == id)
                    .context("space not found")?;
                return Ok(json!({"username":"agent","password":space.password}));
            }
            _ => {}
        }
        let lock = self.lock()?;
        let mut catalog = self.read()?;
        let id = match &command {
            Command::Create {
                name,
                provider,
                image,
                pull_policy,
            } => {
                validate_name(name)?;
                validate_image(image)?;
                provider.validate_platform()?;
                ensure!(catalog.spaces.len() < 32, "maximum of 32 spaces reached");
                ensure!(
                    !catalog.spaces.iter().any(|space| space.name == name.trim()),
                    "a space with this name already exists"
                );
                let owner = Config::update(&self.peers, |_| Ok(()))?.node_id;
                let id = Uuid::new_v4();
                catalog.spaces.push(Space {
                    id,
                    name: name.trim().into(),
                    provider: *provider,
                    image: image.clone(),
                    pull_policy: *pull_policy,
                    node: format!("node-{}", Uuid::new_v4().simple()),
                    owner,
                    password: secrets::token(),
                    token: secrets::token(),
                    phase: "preparing".into(),
                    error: None,
                    ports: Default::default(),
                });
                Some(id)
            }
            Command::Install { action } => {
                engine::plan(*action)?;
                catalog.setup = Some(Setup {
                    action: *action,
                    phase: "installing".into(),
                    error: None,
                });
                None
            }
            Command::Resume { id } | Command::Stop { id } | Command::Remove { id } => {
                let space = catalog
                    .spaces
                    .iter_mut()
                    .find(|space| space.id == *id)
                    .context("space not found")?;
                space.phase = if matches!(command, Command::Stop { .. }) {
                    "stopping"
                } else if matches!(command, Command::Remove { .. }) {
                    "removing"
                } else {
                    "preparing"
                }
                .into();
                space.error = None;
                Some(*id)
            }
            _ => unreachable!(),
        };
        self.save(&catalog)?;
        let store = self.clone();
        let operation = async move {
            let _lock = lock;
            let result = match command {
                Command::Install { action } => engine::install(action).await,
                Command::Create { .. } | Command::Resume { .. } => {
                    store.provision(id.expect("space operation")).await
                }
                Command::Stop { .. } => store.stop(id.expect("space operation"), false).await,
                Command::Remove { .. } => store.stop(id.expect("space operation"), true).await,
                _ => unreachable!(),
            };
            let mut catalog = store.read()?;
            if let Some(id) = id {
                let space = catalog
                    .spaces
                    .iter_mut()
                    .find(|space| space.id == id)
                    .context("space not found")?;
                if let Err(error) = &result {
                    space.phase = "failed".into();
                    space.error = Some(sanitize_error(error, space));
                }
            } else if let Some(setup) = &mut catalog.setup {
                setup.phase = if result.is_ok() {
                    "check_required"
                } else {
                    "failed"
                }
                .into();
                setup.error = result.as_ref().err().map(|error| format!("{error:#}"));
            }
            store.save(&catalog)?;
            result
        };
        if background {
            tokio::spawn(async move {
                let _ = operation.await;
            });
        } else {
            operation.await?;
        }
        self.list()
    }
    fn stage(&self, id: Uuid, phase: &str) -> Result<Space> {
        let mut catalog = self.read()?;
        let space = catalog
            .spaces
            .iter_mut()
            .find(|space| space.id == id)
            .context("space not found")?;
        space.phase = phase.into();
        let result = space.clone();
        self.save(&catalog)?;
        Ok(result)
    }
}
fn busy(phase: &str) -> bool {
    matches!(
        phase,
        "preparing" | "pulling" | "creating" | "starting" | "enrolling" | "stopping" | "removing"
    )
}
fn validate_name(value: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= 120 && !value.chars().any(char::is_control),
        "space name must contain 1–120 characters without control characters"
    );
    Ok(())
}
fn validate_image(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 240
            && !value.starts_with('-')
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-/:@".contains(&c)),
        "invalid container image reference"
    );
    Ok(())
}
fn sanitize_error(error: &anyhow::Error, space: &Space) -> String {
    format!("{error:#}")
        .replace(&space.password, "[redacted]")
        .replace(&space.token, "[redacted]")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_creation_recovers_without_losing_identity_or_credentials() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().join("peers.json"));
        let space = Space {
            id: Uuid::new_v4(),
            name: "Project space".into(),
            provider: Provider::Docker,
            image: DEFAULT_IMAGE.into(),
            pull_policy: PullPolicy::default(),
            node: "node-persistent".into(),
            owner: "owner".into(),
            password: "private-password".into(),
            token: "private-token".into(),
            phase: "pulling".into(),
            error: None,
            ports: Default::default(),
        };
        store
            .save(&Catalog {
                version: 1,
                installation: Uuid::new_v4(),
                spaces: vec![space.clone()],
                setup: None,
            })
            .unwrap();
        let private = fs::read(store.directory.join("catalog.json")).unwrap();
        let mut old: Value = serde_json::from_slice(&private).unwrap();
        old["spaces"][0]
            .as_object_mut()
            .unwrap()
            .remove("pull_policy");
        let old: Catalog = serde_json::from_value(old).unwrap();
        assert!(matches!(old.spaces[0].pull_policy, PullPolicy::Always));
        let shared = store.shared_catalog("owner").unwrap();
        assert_eq!(shared.spaces[0].phase, "pulling");
        assert_eq!(shared.spaces[0].node, space.node);
        let shared = serde_json::to_value(shared).unwrap();
        assert_eq!(shared["spaces"][0].as_object().unwrap().len(), 5);
        assert!(!shared.to_string().contains("private-"));
        assert!(store.shared_catalog("another-owner").is_err());
        assert_eq!(
            fs::read(store.directory.join("catalog.json")).unwrap(),
            private
        );
        let public = store.list().unwrap();
        assert_eq!(public["spaces"][0]["phase"], "interrupted");
        assert!(!public.to_string().contains("private-"));
        let recovered = store.read().unwrap().spaces.remove(0);
        assert_eq!(recovered.node, space.node);
        assert_eq!(recovered.token, space.token);
        let lock = store.lock().unwrap();
        assert!(store.lock().is_err());
        let inherited = lock.0.try_clone().unwrap();
        drop(lock);
        assert!(store.lock().is_ok());
        drop(inherited);
    }
    #[test]
    fn rejects_invalid_commands_before_touching_an_engine() {
        for image in ["--privileged", "ubuntu;echo bad", "", "ubuntu\n"] {
            assert!(validate_image(image).is_err());
        }
        assert!(validate_image("ghcr.io/owner/workspace:stable").is_ok());
        assert!(validate_name("\n").is_err());
        assert!(
            serde_json::from_value::<Command>(json!({"operation":"remove","id":"other-container"}))
                .is_err()
        );
        assert!(
            serde_json::from_value::<Command>(
                json!({"operation":"create","name":"hi","provider":"docker","privileged":true})
            )
            .is_err()
        );
    }
}
