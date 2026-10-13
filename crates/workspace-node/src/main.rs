#[tokio::main(worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    cranibox_node::cli::run().await
}
