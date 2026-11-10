//! One program per device: `cranibox` runs this device's node and its web interface.
//! The Linux build also carries the Agent Workspace container console and services.
use anyhow::Result;

/// Inside the Agent Workspace image `serve` is the container console and the image
/// manages updates; on any other device `serve` is the node.
#[cfg(feature = "workspace")]
fn in_workspace(args: &[String]) -> bool {
    args.iter()
        .any(|a| a == "--config-root" || a.starts_with("--config-root="))
        || std::env::var_os("AGENT_WORKSPACE_CONFIG_ROOT").is_some()
        || std::path::Path::new("/usr/local/lib/agent-workspace").is_dir()
}

#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    #[cfg_attr(not(feature = "workspace"), allow(unused_mut))]
    let mut args: Vec<String> = std::env::args().collect();
    if agent_workspace_runtime::peers::tasks::worker::maybe_serve(&args).await? {
        return Ok(());
    }
    #[cfg(feature = "workspace")]
    {
        let name = std::path::Path::new(&args[0])
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("cranibox")
            .to_owned();
        let mode = match name.as_str() {
            "workspacectl" => Some("ctl".to_owned()),
            "workspace-services" | "systemctl" => Some("service".to_owned()),
            "workspace-web" => Some("serve".to_owned()),
            _ if args.get(1).is_some_and(|s| s.starts_with("__")) => Some("service".to_owned()),
            _ if args
                .get(1)
                .is_some_and(|s| matches!(s.as_str(), "ctl" | "service")) =>
            {
                Some(args.remove(1))
            }
            _ if args.get(1).is_some_and(|s| s == "serve") && in_workspace(&args) => {
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
    // Devices update themselves; inside the Agent Workspace image the image does.
    // Development builds with the workspace console leave management off.
    #[cfg(any(not(feature = "workspace"), feature = "managed"))]
    {
        #[cfg(feature = "workspace")]
        let managed = !in_workspace(&args);
        #[cfg(not(feature = "workspace"))]
        let managed = true;
        if managed {
            cranibox_node::update::set_product_version(env!("CARGO_PKG_VERSION"));
            if let Some(code) = cranibox_node::update::cli::auto_launch(&args).await? {
                std::process::exit(code);
            }
        }
    }
    cranibox_node::cli::run_program(args, env!("CARGO_PKG_VERSION")).await
}
