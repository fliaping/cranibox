use crate::settings::Settings;
use agent_workspace_runtime::{client, protocol::ExecRequest};
use anyhow::Result;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Create and manage isolated container workspaces on this device.
    Space {
        #[command(subcommand)]
        command: crate::spaces::Command,
    },
    /// Manage target-owned direct connectivity without a Workspace connection.
    Peer {
        #[command(subcommand)]
        command: crate::peer::Command,
    },
    /// List devices accessible to this device's Agent.
    Nodes,
    /// Execute on a device in the same trust group.
    Exec {
        #[arg(long)]
        node: String,
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long, default_value_t = 60)]
        timeout_secs: u64,
        #[arg(long)]
        detach: bool,
        #[arg(required = true, trailing_var_arg = true)]
        command: Vec<String>,
    },
    Job {
        id: String,
    },
    Cancel {
        id: String,
    },
    /// MCP stdio server using this device's own credentials.
    Mcp,
}
pub async fn run(settings: Settings, command: Command) -> Result<()> {
    let api = settings.api()?;
    match command {
        Command::Peer { .. } | Command::Space { .. } => {
            anyhow::bail!("device commands use their local configuration")
        }
        Command::Nodes => client::print(api.get("/api/v1/nodes").await?),
        Command::Job { id } => client::print(api.get(&client::job_path(&id)?).await?),
        Command::Cancel { id } => client::print(
            api.post(
                &format!("{}/cancel", client::job_path(&id)?),
                serde_json::json!({}),
            )
            .await?,
        ),
        Command::Mcp => client::mcp(api).await?,
        Command::Exec {
            node,
            cwd,
            timeout_secs,
            detach,
            mut command,
        } => {
            let result = api
                .execute(
                    &node,
                    ExecRequest {
                        program: command.remove(0),
                        args: command,
                        cwd,
                        env: Default::default(),
                        timeout_secs,
                    },
                    detach,
                )
                .await?;
            let success = detach || (result["outcome"] == "exited" && result["exit_code"] == 0);
            client::print(result);
            anyhow::ensure!(success, "remote command failed");
        }
    }
    Ok(())
}
