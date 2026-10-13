#[tokio::main(worker_threads = 2)]
async fn main() {
    match agent_workspacectl::run(std::env::args().skip(1).collect()).await {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("workspacectl: {error:#}");
            std::process::exit(1);
        }
    }
}
