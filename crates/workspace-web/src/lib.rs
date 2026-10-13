mod actions;
mod metrics;
mod remote;
mod server;
mod terminal;

use anyhow::Result;
use clap::Parser;
use std::{net::IpAddr, path::PathBuf};

#[derive(Parser)]
#[command(
    version,
    about = "Agent Workspace browser control center (native Rust)"
)]
struct Args {
    #[arg(long, env = "AGENT_WORKSPACE_CONFIG_ROOT", default_value = "/config")]
    config_root: PathBuf,
    #[arg(long)]
    command: Option<PathBuf>,
    #[arg(long)]
    port: Option<u16>,
    #[arg(long, default_value = "127.0.0.1")]
    host: IpAddr,
    #[arg(long)]
    public_url: Option<String>,
    /// Compatibility listener in the same process; use 0 for an ephemeral port.
    #[arg(long, default_value_t = 9090)]
    gateway_port: u16,
}

pub async fn run(arguments: Vec<String>, integrated: bool) -> Result<()> {
    let args = Args::parse_from(arguments);
    anyhow::ensure!(
        args.host.is_loopback(),
        "workspace-web must bind to loopback; expose it through the authenticated gateway"
    );
    let mut config = server::Config::load(&args.config_root)?;
    if let Some(port) = args.port {
        config.port = port;
    }
    if let Some(url) = args.public_url {
        config.public_url = url;
    }
    config.validate()?;
    let command = args
        .command
        .unwrap_or_else(|| args.config_root.join("bin/workspacectl"));
    let mut state = server::State::new(args.config_root.clone(), command, config.clone())?;
    if integrated {
        let path = args
            .config_root
            .join(".config/agent-workspace/runtime.json");
        {
            let _lock = agent_workspace_runtime::config::lock_credentials(&path)?;
            if !path.exists() {
                let creds = agent_workspace_runtime::config::Credentials {
                    controller_token: agent_workspace_runtime::config::token(),
                    mesh_groups: Default::default(),
                    nodes: Default::default(),
                };
                agent_workspace_runtime::config::write_private(
                    &path,
                    &serde_json::to_vec_pretty(&creds)?,
                    false,
                )?;
            }
        }
        state.gateway = Some(agent_workspace_runtime::server::Gateway::new(path)?);
    }
    let listener = tokio::net::TcpListener::bind((args.host, config.port)).await?;
    println!(
        "Agent Workspace Web (Rust): http://{}",
        listener.local_addr()?
    );
    let compatibility = if integrated && args.gateway_port != config.port {
        let listener = tokio::net::TcpListener::bind((args.host, args.gateway_port)).await?;
        let port = listener.local_addr()?.port();
        state.allow_loopback_port(port);
        eprintln!(
            "Cranibox node compatibility listener: {}",
            listener.local_addr()?
        );
        Some(listener)
    } else {
        None
    };
    let cleanup_state = state.clone();
    let cleanup = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            interval.tick().await;
            cleanup_state.maintenance().await;
        }
    });
    let shutdown = state.clone();
    let router = if let Some(gateway) = &state.gateway {
        gateway
            .router()
            .fallback_service(server::router(state.clone()))
    } else {
        server::router(state.clone())
    };
    let mut closed = state.closing.subscribe();
    let mut compatibility_closed = state.closing.subscribe();
    let signal = tokio::spawn(async move {
        agent_workspace_runtime::shutdown_signal().await;
        shutdown.shutdown().await;
    });
    let main = axum::serve(listener, router.clone()).with_graceful_shutdown(async move {
        let _ = closed.changed().await;
    });
    let extra = async move {
        if let Some(listener) = compatibility {
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = compatibility_closed.changed().await;
                })
                .await
        } else {
            Ok(())
        }
    };
    let (first, second) = tokio::join!(main, extra);
    signal.abort();
    first?;
    second?;
    cleanup.abort();
    Ok(())
}
