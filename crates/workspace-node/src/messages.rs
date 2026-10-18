//! Delivery belongs to the node service; closing a UI never drops its outbox.
use agent_workspace_runtime::peers::Store;
use std::{path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;

pub(crate) struct Service {
    task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl Service {
    pub fn start(path: PathBuf, stop: CancellationToken) -> Self {
        let task = tokio::spawn(async move {
            let store = Store::new(path);
            let mut tick = tokio::time::interval(Duration::from_secs(2));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => break,
                    _ = tick.tick() => tokio::select! {
                        _ = stop.cancelled() => break,
                        // Local UI/CLI reads report corrupt stores; no message text or credentials in logs.
                        _ = async { let _ = tokio::join!(store.deliver_messages(),store.deliver_tasks()); } => {},
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
