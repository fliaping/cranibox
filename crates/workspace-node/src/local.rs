//! The device owns its shells and catalog; peer connections are only transports.
use crate::peer::{self, config::Config};
use agent_workspace_pty::engine::{Engine, Manage};
use agent_workspace_runtime::{
    peers::terminal::{Event, Input},
    projects::{Catalog, Command, Store},
    protocol::MAX_PARALLEL,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

pub struct LocalNode {
    pub id: String,
    app: tokio::sync::OnceCell<Arc<crate::app::Service>>,
    path: PathBuf,
    catalog: Store,
    pub(crate) engine: Engine,
    stop: CancellationToken,
    peer: Mutex<Option<peer::Running>>,
    desktop_service: crate::desktop_service::Service,
    browser_service: agent_workspace_runtime::peers::browser::manager::Service,
    network: crate::network::Service,
    iroh: Mutex<Option<peer::transport_iroh::Service>>,
    messages: crate::messages::Service,
    supervisor: crate::supervisor::Service,
    task_worker: crate::task_worker::Service,
    project_sync: crate::project_sync::Service,
    paths_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    identity_lock: Mutex<Option<std::fs::File>>,
    /// Pairing requests received on this device's public relay address.
    pub pair_requests: Arc<crate::pair_url::Requests>,
}
impl LocalNode {
    pub async fn start(path: PathBuf) -> Result<Self> {
        let catalog = Store::new(path.with_file_name("node-projects.json"));
        let state = std::path::absolute(path.parent().unwrap_or(std::path::Path::new(".")))?;
        let engine = Engine::hosted(peer::home()?, state).await;
        Self::start_with(path, engine, catalog).await
    }
    /// Embed a node in an existing host without starting another terminal engine.
    pub async fn start_with(path: PathBuf, engine: Engine, catalog: Store) -> Result<Self> {
        let lock = peer::service_lock(&path)?;
        let config = Config::update(&path, |_| Ok(()))?;
        let migration = catalog.clone();
        let owner = config.node_id.clone();
        tokio::task::spawn_blocking(move || migration.claim_host(&owner)).await??;
        let slots = Arc::new(Semaphore::new(MAX_PARALLEL));
        let stop = CancellationToken::new();
        let peer = peer::start_with_catalog(
            path.clone(),
            engine.clone(),
            slots.clone(),
            stop.clone(),
            catalog.clone(),
        )
        .await?;
        let supervisor = match crate::supervisor::Service::start(
            path.clone(),
            config.node_id.clone(),
            stop.clone(),
        ) {
            Ok(service) => service,
            Err(error) => {
                stop.cancel();
                let _ = peer.shutdown().await;
                return Err(error);
            }
        };
        if let Some(dir) = path.parent() {
            agent_workspace_runtime::peers::computer::cua::embedded::configure(dir);
        }
        let desktop_service = crate::desktop_service::Service::start(path.clone(), stop.clone());
        let browser_service = agent_workspace_runtime::peers::browser::manager::Service::start(
            path.clone(),
            stop.clone().cancelled_owned(),
        );
        let task_worker = crate::task_worker::Service::start(path.clone(), stop.clone());
        let project_sync = crate::project_sync::Service::start(path.clone(), stop.clone());
        let messages = crate::messages::Service::start(path.clone(), stop.clone());
        let network =
            crate::network::Service::start(path.clone(), peer.network_endpoint(), stop.clone());
        let iroh = peer::transport_iroh::Service::start(
            path.clone(),
            peer.network_endpoint(),
            peer.mesh_router(),
            stop.clone(),
        )
        .await;
        let paths_task = crate::app::paths::start(path.clone(), iroh.handle(), stop.clone());
        Ok(Self {
            app: tokio::sync::OnceCell::new(),
            id: config.node_id,
            path,
            catalog,
            engine,
            desktop_service,
            browser_service,
            network,
            iroh: Mutex::new(Some(iroh)),
            messages,
            supervisor,
            task_worker,
            project_sync,
            paths_task: Mutex::new(Some(paths_task)),
            stop,
            peer: Mutex::new(Some(peer)),
            identity_lock: Mutex::new(Some(lock)),
            pair_requests: Arc::default(),
        })
    }
    pub async fn app(self: &Arc<Self>) -> Result<Arc<crate::app::Service>> {
        self.app
            .get_or_try_init(|| async {
                Ok(Arc::new(crate::app::Service::new(
                    Arc::downgrade(self),
                    self.models().await?,
                )))
            })
            .await
            .cloned()
    }
    pub fn config_path(&self) -> &std::path::Path {
        &self.path
    }
    pub async fn models(&self) -> Result<Arc<crate::models::Models>> {
        self.peer
            .lock()
            .await
            .as_ref()
            .context("local node is stopping")?
            .models()
            .await
    }
    pub async fn manage_peers(
        &self,
        command: agent_workspace_runtime::peers::Command,
    ) -> Result<Value> {
        use agent_workspace_runtime::peers::{Command, Store};
        let store = Store::new(self.path.clone()).with_local_projects(self.catalog.clone());
        let mut value = if let Command::ComputerOwner { command } = command {
            let action = {
                let peer = self.peer.lock().await;
                peer.as_ref()
                    .context("local node is stopping")?
                    .computer_owner(command)
            };
            // A slow provider request must not hold the host lock and delay emergency stop.
            action.await?
        } else if let Command::Listener { listener } = command {
            let mut peer = self.peer.lock().await;
            peer.as_mut()
                .context("local node is stopping")?
                .configure(listener)
                .await?;
            store.list()?
        } else {
            store.manage(command).await?
        };
        let catalog = if value.get("catalog").is_some() {
            &mut value["catalog"]
        } else {
            &mut value
        };
        if catalog.get("node_id").is_some() && catalog.get("routes").is_some() {
            let peer = self.peer.lock().await;
            catalog["listener_runtime"] = peer.as_ref().map_or_else(
                || json!({"active":false,"address":null,"applied":false}),
                |peer| peer.status(&catalog["listener"]),
            );
        }
        Ok(value)
    }
    pub async fn manage_desktop_service(
        &self,
        command: crate::desktop_service::Command,
    ) -> Result<Value> {
        self.desktop_service.manage(command).await
    }
    pub async fn manage_browser_service(
        &self,
        command: agent_workspace_runtime::peers::browser::manager::Command,
    ) -> Result<Value> {
        self.browser_service.manage(command).await
    }
    /// iroh network and pairing back end (App API `pair/*`), if iroh is running.
    pub async fn iroh(&self) -> Option<peer::transport_iroh::Handle> {
        self.iroh.lock().await.as_ref().and_then(|s| s.handle())
    }
    pub async fn manage_network(&self, command: crate::network::Command) -> Result<Value> {
        self.network.manage(command).await
    }
    /// Used only by the narrowly scoped invitation endpoint, without a browser session.
    pub async fn redeem_network_invitation(
        &self,
        token: String,
        recovery: Option<String>,
    ) -> Result<Value> {
        self.network.redeem(token, recovery).await
    }
    pub async fn projects(&self, mut command: Command) -> Result<Value> {
        if let Command::Register { node, .. } = &mut command
            && node.is_none()
        {
            *node = Some(self.id.clone());
        }
        let catalog = projects_in(self.catalog.clone(), &self.id, &self.engine, command).await?;
        Ok(
            json!({"owner":self.id,"revision":catalog.revision,"projects":catalog.projects,
            "permissions":{"projects":true,"terminals":true}}),
        )
    }
    pub async fn sessions(&self, command: Manage) -> Result<Value> {
        self.engine.manage(command).await
    }
    pub async fn attach(
        &self,
        session: &str,
        cols: u16,
        rows: u16,
        takeover: bool,
    ) -> Result<Terminal> {
        let mut pty = self.engine.attach(session, cols, rows, takeover).await?;
        let (input, mut incoming) = mpsc::channel::<Input>(256);
        let (outgoing, output) = mpsc::channel(32);
        let task = tokio::spawn(async move {
            for data in pty.take_replay() {
                if outgoing.send(Event::Output(data)).await.is_err() {
                    return;
                }
            }
            loop {
                tokio::select! {
                    value = incoming.recv() => {
                        let Some(value) = value else { break; };
                        let result = match value {
                            Input::Input { data } => pty.write(data.into_bytes()),
                            Input::Resize { cols, rows } => pty.resize(cols, rows),
                        };
                        if let Err(error) = result { let _ = outgoing.send(Event::Error(error.to_string())).await; break; }
                    }
                    value = pty.recv() => {
                        let Some(value) = value else { break; };
                        let ended = matches!(value, agent_workspace_pty::Event::Exit(_));
                        let value = match value {
                            agent_workspace_pty::Event::Output(data) => Event::Output(data),
                            agent_workspace_pty::Event::Exit(code) => Event::Exit(code),
                        };
                        if outgoing.send(value).await.is_err() || ended { break; }
                    }
                }
            }
        });
        Ok(Terminal {
            input,
            output,
            task,
        })
    }
    pub async fn shutdown(&self) -> Result<()> {
        self.stop.cancel();
        if let Some(task) = self.paths_task.lock().await.take() {
            let _ = task.await;
        }
        // The embedded Computer Use driver is this app's child; it ends with the node.
        agent_workspace_runtime::peers::computer::cua::embedded::stop_daemon();
        let iroh = self.iroh.lock().await.take();
        tokio::join!(
            async {
                if let Some(iroh) = iroh {
                    iroh.shutdown().await;
                }
            },
            self.network.shutdown(),
            self.desktop_service.shutdown(),
            self.browser_service.shutdown(),
            self.messages.shutdown(),
            self.task_worker.shutdown(),
            self.project_sync.shutdown(),
            self.supervisor.shutdown()
        );
        let result = if let Some(peer) = self.peer.lock().await.take() {
            peer.shutdown().await
        } else {
            Ok(())
        };
        self.engine.shutdown().await;
        // Ownership ends after shutdown, even when callers retain an Arc or
        // a concurrently spawned child briefly inherits the file descriptor.
        if let Some(lock) = self.identity_lock.lock().await.take() {
            fs2::FileExt::unlock(&lock)
                .context("release the stopped local node's identity lock")?;
        }
        result
    }
    pub async fn desktop(&self) -> Result<agent_workspace_runtime::peers::desktop::Stream> {
        self.peer
            .lock()
            .await
            .as_ref()
            .context("local node is stopping")?
            .desktop()
            .await
    }
    pub async fn desktop_page(&self) -> Result<agent_workspace_runtime::peers::desktop::Stream> {
        self.peer
            .lock()
            .await
            .as_ref()
            .context("local node is stopping")?
            .desktop_page()
            .await
    }
}

pub(crate) async fn projects_in(
    store: Store,
    owner: &str,
    engine: &Engine,
    command: Command,
) -> Result<Catalog> {
    if let Command::Bind { terminal_id, .. } = &command {
        let sessions = engine.manage(Manage::List).await?;
        ensure!(
            sessions["sessions"]
                .as_array()
                .is_some_and(|values| values.iter().any(|s| s["id"].as_str() == Some(terminal_id))),
            "terminal session unavailable"
        );
    }
    let owner = owner.to_owned();
    tokio::task::spawn_blocking(move || store.apply_owned(command, &owner)).await?
}

pub struct Terminal {
    input: mpsc::Sender<Input>,
    pub output: mpsc::Receiver<Event>,
    task: tokio::task::JoinHandle<()>,
}
impl Terminal {
    pub fn send(&self, value: Input) -> Result<()> {
        value.validate()?;
        self.input
            .try_send(value)
            .map_err(|_| anyhow::anyhow!("terminal input unavailable"))
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Drop for LocalNode {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Duration;

    async fn expect(terminal: &mut Terminal, needle: &str) {
        let mut output = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = terminal.output.recv().await {
                if let Event::Output(data) = event {
                    output.extend(data);
                }
                if String::from_utf8_lossy(&output).contains(needle) {
                    return;
                }
            }
            panic!("terminal closed before expected output");
        })
        .await
        .expect("terminal output timed out");
    }

    #[tokio::test]
    async fn local_catalog_and_shell_survive_detach_and_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("peers.json");
        // This test exercises the embedded native engine's shutdown contract.
        async fn start(path: PathBuf) -> Result<LocalNode> {
            let catalog = Store::new(path.with_file_name("node-projects.json"));
            let home = path.parent().unwrap().to_owned();
            LocalNode::start_with(path, Engine::new(home), catalog).await
        }
        let local = Arc::new(start(path.clone()).await.unwrap());
        assert!(
            start(path.clone()).await.is_err(),
            "one Shell owner per identity"
        );
        assert!(!root.path().join("settings.json").exists());
        let catalog = local
            .projects(Command::Register {
                project_id: None,
                name: "Local project".into(),
                node: None,
                path: root.path().to_string_lossy().into(),
            })
            .await
            .unwrap();
        let replica = catalog["projects"][0]["replicas"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(catalog["projects"][0]["replicas"][0]["node"], local.id);
        let shell = local
            .sessions(Manage::Create {
                name: "Device Shell".into(),
                cwd: Some(root.path().to_string_lossy().into()),
            })
            .await
            .unwrap();
        let id = shell["session"]["id"].as_str().unwrap().to_owned();
        assert!(
            local
                .projects(Command::Bind {
                    replica_id: replica.clone(),
                    terminal_id: uuid::Uuid::new_v4().to_string()
                })
                .await
                .is_err()
        );
        let saved = local
            .projects(Command::Bind {
                replica_id: replica,
                terminal_id: id.clone(),
            })
            .await
            .unwrap();
        let remote = projects_in(
            local.catalog.clone(),
            &local.id,
            &local.engine,
            Command::List {},
        )
        .await
        .unwrap();
        assert_eq!(remote.projects[0].replicas[0].sessions[0].terminal_id, id);
        let mut first = local.attach(&id, 80, 24, false).await.unwrap();
        first
            .send(Input::Input {
                data: "export CRANIBOX_LOCAL=kept; printf '%s%s\n' LOCAL _READY\n".into(),
            })
            .unwrap();
        expect(&mut first, "LOCAL_READY").await;
        drop(first);

        let mut second = local.attach(&id, 100, 30, true).await.unwrap();
        second
            .send(Input::Input {
                data: "printf '%s:%s\n' LOCAL_VALUE \"$CRANIBOX_LOCAL\"\n".into(),
            })
            .unwrap();
        expect(&mut second, "LOCAL_VALUE:kept").await;
        assert_eq!(local.projects(Command::List {}).await.unwrap(), saved);
        assert_eq!(
            local.sessions(Manage::List).await.unwrap()["sessions"][0]["id"],
            id
        );
        drop(second);
        let identity = local.id.clone();
        local.shutdown().await.unwrap();
        assert!(
            local.sessions(Manage::List).await.unwrap()["sessions"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let reopened = start(path).await.unwrap();
        drop(local);
        assert_eq!(reopened.id, identity);
        assert_eq!(reopened.projects(Command::List {}).await.unwrap(), saved);
        assert!(
            reopened.sessions(Manage::List).await.unwrap()["sessions"]
                .as_array()
                .unwrap()
                .is_empty(),
            "restart must not silently recreate a bound Shell"
        );
        reopened.shutdown().await.unwrap();
    }
}
