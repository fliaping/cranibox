mod node;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    version,
    about = "Cranibox remote node client for Windows, macOS and Linux"
)]
struct Args {
    #[arg(long)]
    server: String,
    #[arg(long)]
    id: String,
    #[arg(long)]
    token_file: PathBuf,
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    allow_insecure: bool,
}
#[tokio::main(worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    node::run(
        args.server,
        args.id,
        args.token_file,
        args.name,
        args.allow_insecure,
    )
    .await
}
