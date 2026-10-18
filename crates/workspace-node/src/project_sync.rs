//! Background metadata synchronization belongs to the device, not a Web view.
use agent_workspace_runtime::peers::project_sync;
use std::{collections::BTreeSet, path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;

pub(crate) struct Service {
    task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl Service {
    pub fn start(path: PathBuf, stop: CancellationToken) -> Self {
        let task = tokio::spawn(async move {
            let mut timer = tokio::time::interval(Duration::from_secs(2));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut tasks = tokio::task::JoinSet::new();
            let mut active = BTreeSet::new();
            loop {
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => break,
                    result = tasks.join_next(), if !tasks.is_empty() => {
                        if let Some(Ok(node)) = result { active.remove(&node); }
                    },
                    _ = timer.tick() => {
                        if tasks.len() >= project_sync::MAX_PARALLEL { continue; }
                        let configuration = path.clone();
                        // Invalid private policy/state fails closed; the operator
                        // sees the error when opening its settings. No secrets log.
                        let Ok(Ok(pending)) = tokio::task::spawn_blocking(move || {
                                // Broken policy for one catalog must not stop the other.
                                let projects = project_sync::pending(&configuration).unwrap_or_default();
                                let agents = project_sync::pending_kind(&configuration, project_sync::Kind::Agents).unwrap_or_default();
                                let mut pending = Vec::new();
                                let mut projects = projects.into_iter();
                                let mut agents = agents.into_iter();
                                loop {
                                    let project = projects.next(); let agent = agents.next();
                                    if project.is_none() && agent.is_none() { break; }
                                    pending.extend(project); pending.extend(agent);
                                }
                                Ok::<_, anyhow::Error>(pending)
                            }).await else { continue; };
                        for job in pending {
                            if tasks.len() >= project_sync::MAX_PARALLEL { break; }
                            if !active.insert((job.kind, job.node.clone())) { continue; }
                            let path = path.clone();
                            tasks.spawn(async move {
                                let node = (job.kind, job.node.clone());
                                let _ = project_sync::run(&path, job).await;
                                node
                            });
                        }
                    },
                }
            }
            // Reads already in progress finish within the network deadline. A
            // pause/shutdown never sends a second operation or destroys a shell.
            while tasks.join_next().await.is_some() {}
        });
        Self {
            task: tokio::sync::Mutex::new(Some(task)),
        }
    }
    pub async fn shutdown(&self) {
        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        }
    }
}
