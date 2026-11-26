use agent_workspace_runtime::{client, protocol::ExecRequest};
use anyhow::Result;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Manage access to this device, including approvals on headless devices.
    Access {
        #[command(flatten)]
        args: crate::access::cli::Args,
    },
    #[command(hide = true)]
    TerminalHost {
        #[arg(long)]
        state: std::path::PathBuf,
        #[arg(long, hide = true)]
        home: Option<std::path::PathBuf>,
    },
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
    Nodes {
        /// Print JSON (the default when stdout is not a terminal).
        #[arg(long)]
        json: bool,
    },
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
    /// Copy a file or folder to a paired or same-identity device (needs its exec grant).
    Push {
        #[arg(long)]
        node: String,
        /// Local file or folder.
        local: std::path::PathBuf,
        /// Absolute destination path on the device; a folder's files land under it.
        remote: String,
        /// Replace files that already exist.
        #[arg(long)]
        overwrite: bool,
    },
    /// Copy a file or folder from a paired or same-identity device (needs its exec grant).
    Pull {
        #[arg(long)]
        node: String,
        /// Absolute path of a file or folder on the device.
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
        Command::Nodes { json } => {
            let value = fleet.nodes().await?;
            if json || !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
                client::print(value);
            } else {
                print!("{}", format_nodes_table(&value));
            }
        }
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

/// Human-readable table for `cranibox nodes` when stdout is a terminal.
pub fn format_nodes_table(value: &serde_json::Value) -> String {
    let nodes = value["nodes"].as_array().cloned().unwrap_or_default();
    let mut rows: Vec<[String; 5]> = vec![[
        "NAME".into(),
        "OS".into(),
        "ONLINE".into(),
        "PATH".into(),
        "NODE ID".into(),
    ]];
    for node in &nodes {
        let id = node["id"].as_str().unwrap_or("").to_owned();
        let this = node["this"].as_bool().unwrap_or(false);
        let name = node["name"]
            .as_str()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if this {
                    "this device".into()
                } else {
                    id.clone()
                }
            });
        let os = node["os"]
            .as_str()
            .map(str::trim)
            .filter(|os| !os.is_empty())
            .unwrap_or("-")
            .to_owned();
        let online = if node["online"].as_bool().unwrap_or(false) {
            "yes"
        } else {
            "no"
        }
        .to_owned();
        let path = node["path"]
            .as_str()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| if this { "local".into() } else { "-".into() });
        rows.push([name, os, online, path, id]);
    }
    let widths: [usize; 5] = std::array::from_fn(|column| {
        rows.iter()
            .map(|row| row[column].chars().count())
            .max()
            .unwrap_or(0)
    });
    let mut out = String::new();
    for row in rows {
        for (column, cell) in row.iter().enumerate() {
            if column > 0 {
                out.push_str("  ");
            }
            out.push_str(cell);
            out.extend(std::iter::repeat_n(
                ' ',
                widths[column].saturating_sub(cell.chars().count()),
            ));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nodes_table_lists_name_os_online_path_and_id() {
        let value = json!({"nodes":[
            {"id":"node-local","this":true,"online":true},
            {"id":"node-mac","name":"Studio","os":"macOS","online":true,"path":"direct"},
            {"id":"node-win","name":"Windows PC","os":"Windows","online":false}
        ]});
        let table = format_nodes_table(&value);
        let lines: Vec<_> = table.lines().collect();
        assert_eq!(
            lines[0].split_whitespace().collect::<Vec<_>>(),
            ["NAME", "OS", "ONLINE", "PATH", "NODE", "ID"]
        );
        assert!(table.contains("this device"));
        assert!(table.contains("local"));
        assert!(table.contains("Studio"));
        assert!(table.contains("macOS"));
        assert!(table.contains("direct"));
        assert!(table.contains("Windows PC"));
        assert!(table.contains("node-win"));
        // Offline devices with no path show a dash, not a blank path column.
        let win = lines
            .iter()
            .find(|line| line.contains("Windows PC"))
            .unwrap();
        assert!(win.contains("no"));
        assert!(win.split_whitespace().any(|cell| cell == "-"));
    }

    #[test]
    fn nodes_table_for_missing_list_is_just_the_header() {
        assert_eq!(
            format_nodes_table(&json!({})),
            "NAME  OS  ONLINE  PATH  NODE ID\n"
        );
    }
}
