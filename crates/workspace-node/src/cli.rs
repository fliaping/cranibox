use crate::{agent, settings};
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "Cranibox device node for Windows, macOS and Linux")]
struct Args {
    #[arg(long, global = true, default_value_os_t = settings::default_path())]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<agent::Command>,
}

pub async fn run() -> anyhow::Result<()> {
    // The command tree contains large async states. Keep it off the native
    // entry thread's stack (particularly the smaller Windows main stack).
    Box::pin(run_from(std::env::args().collect())).await
}

/// The `cranibox` command line, shared by the device program and the desktop app so
/// both answer the same commands. `cranibox node …` is the long form.
pub async fn run_program(mut args: Vec<String>, version: &str) -> anyhow::Result<()> {
    crate::update::set_product_version(version);
    if args.get(1).is_some_and(|s| s == "node") {
        args.remove(1);
    }
    if matches!(args.get(1).map(String::as_str), Some("-V" | "--version")) {
        println!("cranibox {version}");
        return Ok(());
    }
    Box::pin(run_from(args)).await
}

pub async fn run_from(args: Vec<String>) -> anyhow::Result<()> {
    if agent_workspace_runtime::peers::tasks::worker::maybe_serve(
        &std::env::args().collect::<Vec<_>>(),
    )
    .await?
    {
        return Ok(());
    }
    let args = Args::parse_from(args);
    let peer_path = crate::peer::config::path_for(&args.config);
    // Every device runs the same way: node core plus its own interface.
    let service = match &args.command {
        None
        | Some(agent::Command::Peer {
            command: crate::peer::Command::Serve,
        }) => Some((None, Vec::new(), Default::default())),
        Some(agent::Command::Serve {
            listen,
            allow_host,
            relay,
            public_url,
            relay_listen,
        }) => Some((
            *listen,
            allow_host.clone(),
            crate::web::standalone::RelayFlags {
                relay: *relay,
                public_url: public_url.clone(),
                listen: *relay_listen,
            },
        )),
        _ => None,
    };
    if let Some((listen, allow_hosts, relay)) = service {
        let result = Box::pin(crate::web::standalone::run(
            peer_path,
            listen,
            allow_hosts,
            relay,
        ))
        .await;
        // An activated update asks the launcher to start the new version.
        if crate::update::restart_requested() {
            std::process::exit(crate::update::launcher::RESTART_EXIT_CODE);
        }
        return result;
    }
    let command = match args.command {
        Some(agent::Command::Access { args }) => {
            return crate::access::cli::run(&peer_path, args).await;
        }
        Some(agent::Command::TerminalHost { state, home }) => {
            return agent_workspace_pty::hosted::run(
                state,
                home.map(Ok).unwrap_or_else(crate::peer::home)?,
                crate::update::product_version(),
            )
            .await;
        }
        Some(agent::Command::Update { command }) => {
            return crate::update::cli::run(peer_path, args.config, command).await;
        }
        Some(agent::Command::UiUrl { listen }) => {
            println!(
                "{}",
                crate::web::endpoint::ui_url(&peer_path, listen).await?
            );
            return Ok(());
        }
        Some(agent::Command::AppMcp { data, service }) => {
            return crate::app_mcp::run(peer_path, data, &service).await;
        }
        Some(agent::Command::Space { command }) => {
            let result = crate::spaces::Store::new(peer_path)
                .manage(command, false)
                .await?;
            agent_workspace_runtime::client::print(result);
            return Ok(());
        }
        Some(agent::Command::Peer {
            command:
                crate::peer::Command::Messages {
                    action:
                        crate::peer::Messages::HookConfig {
                            harness,
                            wake_on_stop,
                        },
                },
        }) => {
            let config = crate::peer::hook::config(harness, wake_on_stop, &args.config)?;
            println!("{}", serde_json::to_string_pretty(&config)?);
            return Ok(());
        }
        Some(agent::Command::Peer { command }) => {
            return Box::pin(crate::peer::run(peer_path, command)).await;
        }
        command => command,
    };
    let command = command.expect("the device service is handled above");
    // The Agent reaches this device's own peers (paired and same-identity devices).
    agent::run_peers(peer_path, command).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{peer, spaces};

    #[test]
    fn terminal_host_is_a_hidden_local_command() {
        use clap::CommandFactory;
        let help = Args::command().render_long_help().to_string();
        assert!(!help.contains("terminal-host"));
        let args = Args::try_parse_from(["cranibox", "terminal-host", "--state", "private-state"])
            .unwrap();
        assert!(
            matches!(args.command, Some(agent::Command::TerminalHost { state, home: None }) if state == std::path::Path::new("private-state"))
        );
    }

    #[test]
    fn space_job_and_peer_arguments_keep_their_own_ids() {
        let id = uuid::Uuid::new_v4();
        for operation in ["stop", "resume", "remove", "credentials"] {
            let args =
                Args::try_parse_from(["cranibox", "space", operation, &id.to_string()]).unwrap();
            let parsed = match args.command.unwrap() {
                agent::Command::Space {
                    command:
                        spaces::Command::Stop { id }
                        | spaces::Command::Resume { id, .. }
                        | spaces::Command::Remove { id }
                        | spaces::Command::Credentials { id },
                } => id,
                other => panic!("unexpected command: {other:?}"),
            };
            assert_eq!(parsed, id);
        }
        for operation in ["job", "cancel"] {
            let args = Args::try_parse_from(["cranibox", operation, "job-123"]).unwrap();
            let parsed = match args.command.unwrap() {
                agent::Command::Job { id, .. } | agent::Command::Cancel { id, .. } => id,
                other => panic!("unexpected command: {other:?}"),
            };
            assert_eq!(parsed, "job-123");
        }
        let args = Args::try_parse_from([
            "cranibox",
            "peer",
            "grant",
            "--node",
            "peer-node",
            "--output-token",
            "peer.token",
        ])
        .unwrap();
        match args.command.unwrap() {
            agent::Command::Peer {
                command: peer::Command::Grant { token_file, .. },
            } => assert_eq!(token_file, PathBuf::from("peer.token")),
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
