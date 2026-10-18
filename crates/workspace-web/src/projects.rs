use crate::server::State;
use agent_workspace_runtime::projects::{Command, Store};
use anyhow::Result;
use serde_json::{Value, json};

pub async fn manage(state: &State, body: Value) -> Result<Value> {
    let mut command: Command = serde_json::from_value(body)?;
    if let Command::Register { node, .. } = &mut command
        && node.is_none()
    {
        *node = state.local.as_ref().map(|local| local.id.clone());
    }
    let store = Store::new(state.root.join(".config/agent-workspace/projects.json"));
    // The host's terminal engine belongs to workspace-web, not the gateway.
    if let Command::Bind {
        replica_id,
        terminal_id,
    } = &command
    {
        let replica = store.replica(replica_id, |_| true)?;
        if replica.node.is_none()
            || state
                .local
                .as_ref()
                .is_some_and(|local| replica.node.as_deref() == Some(&local.id))
        {
            let catalog = crate::terminal::manage(state, json!({"operation":"list"})).await?;
            anyhow::ensure!(
                catalog["sessions"]
                    .as_array()
                    .is_some_and(|sessions| sessions
                        .iter()
                        .any(|s| s["id"].as_str() == Some(terminal_id))),
                "terminal session unavailable"
            );
        }
    }
    if let Some(gateway) = &state.gateway {
        gateway
            .api(
                axum::http::Method::POST,
                "/projects",
                Some(serde_json::to_value(command)?),
            )
            .await
    } else {
        let owner = state.local.as_ref().map(|local| local.id.clone());
        Ok(serde_json::to_value(
            tokio::task::spawn_blocking(move || {
                store.apply(command, |node| node == owner.as_deref())
            })
            .await??,
        )?)
    }
}
