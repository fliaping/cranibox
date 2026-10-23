mod actions;
mod applications;
mod metrics;
mod projects;
mod relay;
mod server;

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
    /// Run the embedded iroh relay (default when --public-url is https).
    #[arg(long, conflicts_with = "no_relay")]
    relay: bool,
    /// Do not run the embedded iroh relay; devices use the public n0 relays.
    #[arg(long)]
    no_relay: bool,
    /// Accepted and ignored: the legacy gateway listener is gone, but service units
    /// written by earlier installs still pass it.
    #[arg(long = "gateway-port", hide = true)]
    _gateway_port: Option<u16>,
}

pub async fn run(mut arguments: Vec<String>) -> Result<()> {
    if arguments.get(1).is_some_and(|arg| arg == "node") {
        arguments.remove(1);
        return cranibox_node::cli::run_from(arguments).await;
    }
    if agent_workspace_runtime::peers::tasks::worker::maybe_serve(&arguments).await? {
        return Ok(());
    }
    let args = Args::parse_from(arguments);
    anyhow::ensure!(
        args.host.is_loopback(),
        "workspace-web must bind to loopback; expose it through an authenticated reverse proxy"
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
    let embedded = relay::setup(args.relay, args.no_relay, &config.public_url);
    state.local = Some(std::sync::Arc::new(
        cranibox_node::local::LocalNode::start_with(
            args.config_root.join(".config/cranibox/peers.json"),
            agent_workspace_pty::engine::Engine::Herdr(agent_workspace_pty::herdr::Engine::new(
                args.config_root.clone(),
                args.config_root.join(".local/state/cranibox/terminal"),
            )),
            agent_workspace_runtime::projects::Store::new(
                args.config_root
                    .join(".config/agent-workspace/projects.json"),
            ),
        )
        .await?,
    ));
    if std::env::var("CRANIBOX_APP_MOCK").as_deref() != Ok("1") {
        state.local.as_ref().expect("local node").models().await?;
    }
    state.attach_node();
    let listener = tokio::net::TcpListener::bind((args.host, config.port)).await?;
    println!(
        "Agent Workspace Web (Rust): http://{}",
        listener.local_addr()?
    );
    let cleanup_state = state.clone();
    let cleanup = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            interval.tick().await;
            cleanup_state.maintenance().await;
        }
    });
    let router = server::router(state.clone());
    let mut closed = state.closing.subscribe();
    embedded.spawn_self_check();
    let serving = embedded.serve(listener, router, async move {
        let _ = closed.changed().await;
    });
    tokio::pin!(serving);
    let result = tokio::select! {
        result = &mut serving => {
            state.shutdown().await;
            result
        }
        _ = agent_workspace_runtime::shutdown_signal() => {
            // Closing the listeners can finish before node cleanup. Await both
            // so private provider files and child processes are fully released.
            let ((), result) = tokio::join!(state.shutdown(), serving);
            result
        }
    };
    cleanup.abort();
    result?;
    Ok(())
}
