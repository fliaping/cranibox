use agent_workspace_runtime::{client, protocol::ExecRequest};
use anyhow::Result;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Cranibox MCP services; this command never starts another node service.
    AppMcp {
        #[arg(long)]
        data: std::path::PathBuf,
        #[arg(long, value_parser = ["devices", "computer", "browser"])]
        service: String,
    },
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
    /// Update this device or other devices of your identity to a new version.
    Update {
        #[command(subcommand)]
        command: crate::update::cli::Command,
    },
    /// Execute on a paired or same-identity device.
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
    /// Copy a file to a paired or same-identity device (needs its exec grant).
    Push {
        #[arg(long)]
        node: String,
        /// Local file.
        local: std::path::PathBuf,
        /// Absolute destination path on the device.
        remote: String,
        /// Replace an existing file.
        #[arg(long)]
        overwrite: bool,
    },
    /// Copy a file from a paired or same-identity device (needs its exec grant).
    Pull {
        #[arg(long)]
        node: String,
        /// Absolute file path on the device.
        remote: String,
        /// Local destination.
        local: std::path::PathBuf,
        #[arg(long)]
        overwrite: bool,
    },
    Job {
        id: String,
        /// Device running the job.
        #[arg(long)]
        node: Option<String>,
    },
    Cancel {
        id: String,
        #[arg(long)]
        node: Option<String>,
    },
    /// MCP stdio server using this device's own credentials.
    Mcp,
    /// Run this device and serve its interface on loopback (the default command).
    Serve {
        /// Loopback address; default 127.0.0.1:7377, or a free port when that is taken.
        #[arg(long)]
        listen: Option<std::net::SocketAddr>,
        /// Also accept this Host (host[:port]) from a trusted reverse proxy.
        #[arg(long = "allow-host")]
        allow_host: Vec<String>,
        /// Run an iroh relay for your devices (on by default with an https --public-url).
        #[arg(long)]
        relay: bool,
        /// Public https origin of this device's relay, e.g. https://cranibox.example.com.
        #[arg(long)]
        public_url: Option<String>,
        /// Public listener for the relay and read-only identity, e.g. 0.0.0.0:7378;
        /// the interface stays on loopback.
        #[arg(long)]
        relay_listen: Option<std::net::SocketAddr>,
    },
    /// Print the sign-in link of this device's interface.
    UiUrl {
        /// Defaults to the address the running device service recorded.
        #[arg(long)]
        listen: Option<std::net::SocketAddr>,
    },
}
/// Agent commands over this device's own peers (paired and same-identity devices).
pub async fn run_peers(peers: std::path::PathBuf, command: Command) -> Result<()> {
    let fleet = client::Fleet::new(peers.clone());
    match command {
        Command::Nodes => client::print(fleet.nodes().await?),
        // Needs the settings path, which only the command line entry point has.
        Command::Update { .. } => {
            anyhow::bail!("update is handled by the command line entry point")
        }
        Command::Job { id, node } => client::print(fleet.job(&id, node.as_deref(), false).await?),
        Command::Cancel { id, node } => client::print(fleet.job(&id, node.as_deref(), true).await?),
        Command::Mcp => client::fleet_mcp(peers).await?,
        Command::Push {
            node,
            local,
            remote,
            overwrite,
        } => client::print(fleet.push(&node, &local, &remote, overwrite).await?),
        Command::Pull {
            node,
            remote,
            local,
            overwrite,
        } => client::print(fleet.pull(&node, &remote, &local, overwrite).await?),
        Command::Exec {
            node,
            cwd,
            timeout_secs,
            detach,
            mut command,
        } => {
            let result = fleet
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
        _ => anyhow::bail!("device commands use their local configuration"),
    }
    Ok(())
}
