#[tokio::main(worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    agent_workspace_runtime::cli::run(std::env::args().collect()).await
}
