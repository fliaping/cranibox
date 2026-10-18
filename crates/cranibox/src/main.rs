use anyhow::{Result, bail};

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
    let mode = match name.as_str() {
        "workspacectl" => "ctl".to_owned(),
        "workspace-services" | "systemctl" => "service".to_owned(),
        "workspace-runtime" => "runtime".to_owned(),
        "workspace-web" => "serve".to_owned(),
        _ => {
            if args.get(1).is_some_and(|s| s.starts_with("__")) {
                "service".to_owned()
            } else if args.len() > 1 {
                args.remove(1)
            } else {
                "help".to_owned()
            }
        }
    };
    match mode.as_str() {
        "serve" => agent_workspace_web::run(args, true).await?,
        "ctl" => std::process::exit(agent_workspacectl::run(args[1..].to_vec()).await?),
        "service" => std::process::exit(agent_workspace_services::run(args[1..].to_vec())?),
        "runtime" => agent_workspace_runtime::cli::run(args).await?,
        "-V" | "--version" => println!("cranibox {}", env!("CARGO_PKG_VERSION")),
        "help" | "-h" | "--help" => println!(
            "Cranibox server\nUsage: cranibox serve [options]\n       cranibox ctl <command>\n       cranibox service --user <command>\n       cranibox runtime <command>\nRemote machines use the separate cranibox-node application."
        ),
        _ => bail!("unknown command: {mode}; use cranibox --help"),
    }
    Ok(())
}
