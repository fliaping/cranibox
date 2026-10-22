#[tokio::main(worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    agent_workspace_web::run(std::env::args().collect()).await
}
