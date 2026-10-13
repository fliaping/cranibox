use crate::{
    agent, node,
    settings::{self, Settings},
};
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    version,
    about = "Cranibox remote node client for Windows, macOS and Linux"
)]
struct Args {
    #[arg(long, global = true)]
    server: Option<String>,
    #[arg(long, global = true)]
    id: Option<String>,
    #[arg(long, global = true)]
    token_file: Option<PathBuf>,
    #[arg(long, global = true)]
    name: Option<String>,
    #[arg(long, global = true)]
    allow_insecure: bool,
    #[arg(long, global = true, default_value_os_t = settings::default_path())]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<agent::Command>,
}
pub async fn run() -> anyhow::Result<()> {
    let args = Args::parse();
    let settings = if args.server.is_some() || args.id.is_some() || args.token_file.is_some() {
        Settings {
            server: args
                .server
                .ok_or_else(|| anyhow::anyhow!("--server required"))?,
            id: args.id.ok_or_else(|| anyhow::anyhow!("--id required"))?,
            token_file: args
                .token_file
                .ok_or_else(|| anyhow::anyhow!("--token-file required"))?,
            name: args.name,
            allow_insecure: args.allow_insecure,
        }
    } else {
        Settings::read(&args.config)?
    };
    settings.validate()?;
    if let Some(command) = args.command {
        agent::run(settings, command).await
    } else {
        node::run(
            settings.server,
            settings.id,
            settings.token_file,
            settings.name,
            settings.allow_insecure,
        )
        .await
    }
}
