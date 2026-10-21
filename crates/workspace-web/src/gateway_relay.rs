//! Legacy Workspace gateway routes for devices that are enrolled but not paired directly.
use agent_workspace_runtime::{
    peers::{
        desktop::Stream,
        terminal::{Event, Input},
    },
    protocol::Wire,
    server::{Gateway, RemoteTerminal},
};
use anyhow::Result;
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use cranibox_node::web::{Relay, RelayTerminal};
use serde_json::Value;
use std::path::PathBuf;

pub(crate) struct GatewayRelay {
    pub gateway: Option<Gateway>,
    pub runtime_url: String,
    pub root: PathBuf,
}
#[async_trait]
impl Relay for GatewayRelay {
    async fn desktop(&self, node: &str) -> Result<Stream> {
        if let Some(gateway) = &self.gateway {
            return gateway.desktop(node).await;
        }
        agent_workspace_runtime::client::Api::new(agent_workspace_runtime::client::ApiOptions {
            url: self.runtime_url.clone(),
            token_file: None,
            credentials: self.root.join(".config/agent-workspace/runtime.json"),
            allow_insecure: false,
        })?
        .desktop(node)
        .await
    }
    async fn terminal(
        &self,
        node: &str,
        cols: u16,
        rows: u16,
        session_id: String,
        takeover: bool,
    ) -> Result<Box<dyn RelayTerminal>> {
        let gateway = self.gateway.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Remote terminals require the unified Cranibox server")
        })?;
        Ok(Box::new(GatewayTerminal(
            gateway
                .terminal(node, cols, rows, session_id, takeover)
                .await?,
        )))
    }
    async fn terminal_manage(&self, node: &str, body: Value) -> Result<Value> {
        self.gateway
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("remote sessions require the unified server"))?
            .terminal_manage(node, body)
            .await
    }
}

struct GatewayTerminal(RemoteTerminal);
#[async_trait]
impl RelayTerminal for GatewayTerminal {
    fn send(&self, input: Input) -> Result<()> {
        let remote = &self.0;
        match input {
            Input::Input { data } => remote.send(Wire::TerminalInput {
                id: remote.id.clone(),
                data: STANDARD.encode(data),
            }),
            Input::Resize { cols, rows } => {
                agent_workspace_pty::validate_size(cols, rows)?;
                remote.send(Wire::TerminalResize {
                    id: remote.id.clone(),
                    cols,
                    rows,
                })
            }
        }
    }
    async fn output(&mut self) -> Option<Event> {
        match self.0.output.recv().await? {
            Wire::TerminalOutput { data, .. } => STANDARD.decode(data).ok().map(Event::Output),
            Wire::TerminalExit { code, .. } => Some(Event::Exit(code)),
            _ => None,
        }
    }
}
