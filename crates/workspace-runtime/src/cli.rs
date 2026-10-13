use crate::{client, config, protocol, server};
use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use std::{net::SocketAddr, path::PathBuf};

#[derive(Parser)]
#[command(
    version,
    about = "Agent Workspace Rust gateway and remote node runtime"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Args, Clone)]
pub struct ApiOptions {
    #[arg(
        long,
        env = "AGENT_WORKSPACE_RUNTIME_URL",
        default_value = "http://127.0.0.1:9090"
    )]
    pub(crate) url: String,
    #[arg(long, env = "AGENT_WORKSPACE_RUNTIME_TOKEN_FILE")]
    pub(crate) token_file: Option<PathBuf>,
    #[arg(long, default_value_os_t = config::default_credentials())]
    pub(crate) credentials: PathBuf,
    #[arg(long)]
    pub(crate) allow_insecure: bool,
}
#[derive(Subcommand)]
enum Command {
    /// Generate separate controller and per-node credentials (never print tokens).
    Init {
        #[arg(long, default_value_os_t = config::default_credentials())]
        credentials: PathBuf,
    },
    /// Enroll/rotate a node. Transfer its token file privately to that machine.
    Enroll {
        id: String,
        #[arg(long, default_value_os_t = config::default_credentials())]
        credentials: PathBuf,
        #[arg(long)]
        token_file: PathBuf,
    },
    /// Revoke a node, including its active connection on the next heartbeat.
    Revoke {
        id: String,
        #[arg(long, default_value_os_t = config::default_credentials())]
        credentials: PathBuf,
    },
    /// HTTP + WebSocket gateway. Optionally forward existing Web UI to --web-upstream.
    Serve {
        #[arg(long, default_value = "127.0.0.1:9090")]
        listen: SocketAddr,
        #[arg(long, default_value_os_t = config::default_credentials())]
        credentials: PathBuf,
        #[arg(long)]
        web_upstream: Option<String>,
    },
    /// List enrolled nodes and their online/offline state as JSON.
    Nodes {
        #[command(flatten)]
        api: ApiOptions,
    },
    /// Execute directly without an implicit shell. Print result JSON, or return a job ID.
    Exec {
        #[command(flatten)]
        api: ApiOptions,
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
        #[command(flatten)]
        api: ApiOptions,
        id: String,
    },
    Cancel {
        #[command(flatten)]
        api: ApiOptions,
        id: String,
    },
    /// MCP stdio server: workspace_nodes, workspace_exec, workspace_job, workspace_cancel.
    Mcp {
        #[command(flatten)]
        api: ApiOptions,
    },
}

pub async fn run(args: Vec<String>) -> Result<()> {
    match Cli::parse_from(args).command {
        Command::Init { credentials } => {
            let _lock = config::lock_credentials(&credentials)?;
            let value = config::Credentials {
                controller_token: config::token(),
                nodes: Default::default(),
            };
            config::write_private(&credentials, &serde_json::to_vec_pretty(&value)?, false)?;
            eprintln!("Created {}", credentials.display());
        }
        Command::Enroll {
            id,
            credentials,
            token_file,
        } => {
            let _lock = config::lock_credentials(&credentials)?;
            anyhow::ensure!(
                protocol::valid_node_id(&id),
                "node ID must be 1..64 ASCII letters/digits/_/-"
            );
            anyhow::ensure!(
                token_file != credentials,
                "token file must differ from credentials file"
            );
            let mut value = config::Credentials::read(&credentials)?;
            anyhow::ensure!(
                value.nodes.len() < 128 || value.nodes.contains_key(&id),
                "maximum 128 enrolled nodes"
            );
            let token = config::token();
            config::write_private(&token_file, format!("{token}\n").as_bytes(), false)?;
            value.nodes.insert(id.clone(), token);
            config::write_private(&credentials, &serde_json::to_vec_pretty(&value)?, true)?;
            eprintln!(
                "Enrolled {id}; transfer {} privately to the node",
                token_file.display()
            );
        }
        Command::Revoke { id, credentials } => {
            let _lock = config::lock_credentials(&credentials)?;
            let mut value = config::Credentials::read(&credentials)?;
            anyhow::ensure!(value.nodes.remove(&id).is_some(), "unknown node");
            config::write_private(&credentials, &serde_json::to_vec_pretty(&value)?, true)?;
            eprintln!("Revoked {id}");
        }
        Command::Serve {
            listen,
            credentials,
            web_upstream,
        } => server::serve(listen, credentials, web_upstream).await?,
        Command::Nodes { api } => client::print(client::Api::new(api)?.get("/api/v1/nodes").await?),
        Command::Job { api, id } => {
            client::print(client::Api::new(api)?.get(&client::job_path(&id)?).await?)
        }
        Command::Cancel { api, id } => client::print(
            client::Api::new(api)?
                .post(
                    &format!("{}/cancel", client::job_path(&id)?),
                    serde_json::json!({}),
                )
                .await?,
        ),
        Command::Exec {
            api,
            node,
            cwd,
            timeout_secs,
            detach,
            mut command,
        } => {
            let request = protocol::ExecRequest {
                program: command.remove(0),
                args: command,
                cwd,
                env: Default::default(),
                timeout_secs,
            };
            request.validate()?;
            let result = client::Api::new(api)?
                .execute(&node, request, detach)
                .await?;
            let success = detach || (result["outcome"] == "exited" && result["exit_code"] == 0);
            client::print(result);
            if !success {
                std::process::exit(1);
            }
        }
        Command::Mcp { api } => client::mcp(client::Api::new(api)?).await?,
    }
    Ok(())
}
