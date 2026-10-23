//! One program per device: `cranibox` runs this device's node and its web interface.
//! The Docker workspace build adds the container console and services.
use anyhow::Result;

#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().collect();
    if agent_workspace_runtime::peers::tasks::worker::maybe_serve(&args).await? {
        return Ok(());
    }
    let name = std::path::Path::new(&args[0])
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("cranibox")
        .to_owned();
    #[cfg(feature = "workspace")]
    {
        let mode = match name.as_str() {
            "workspacectl" => Some("ctl".to_owned()),
            "workspace-services" | "systemctl" => Some("service".to_owned()),
            "workspace-web" => Some("serve".to_owned()),
            _ if args.get(1).is_some_and(|s| s.starts_with("__")) => Some("service".to_owned()),
            _ if args
                .get(1)
                .is_some_and(|s| matches!(s.as_str(), "serve" | "ctl" | "service")) =>
            {
                Some(args.remove(1))
            }
            _ => None,
        };
        match mode.as_deref() {
            Some("serve") => return agent_workspace_web::run(args).await,
            Some("ctl") => std::process::exit(agent_workspacectl::run(args[1..].to_vec()).await?),
            Some("service") => {
                std::process::exit(agent_workspace_services::run(args[1..].to_vec())?)
            }
            _ => {}
        }
    }
    // `cranibox node …` is the long form; every other command belongs to the node.
    if args.get(1).is_some_and(|s| s == "node") {
        args.remove(1);
    }
    match args.get(1).map(String::as_str) {
        Some("-V" | "--version") => {
            println!("cranibox {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => cranibox_node::cli::run_from(args).await,
    }
}
