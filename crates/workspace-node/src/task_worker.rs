//! Automatic acceptance belongs to the node service, with a receiver-owned policy.
use agent_workspace_runtime::peers::tasks::automatic;
use std::{path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;

pub(crate) struct Service {
    task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl Service {
    pub fn start(path: PathBuf, stop: CancellationToken) -> Self {
        let task = tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(2));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => break,
                    _ = tick.tick() => {
                        let path = path.clone();
                        // Await the bounded disk operation before shutdown; do not detach a launch.
                        // Operator reads surface invalid stores without logging task text or secrets.
                        let _ = tokio::task::spawn_blocking(move || automatic::tick(&path)).await;
                    },
                }
            }
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
