use crate::{
    agent,
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
    // Local commands also have IDs and token files; keep their Clap keys separate.
    #[arg(long, global = true, id = "connection-id")]
    id: Option<String>,
    #[arg(long, global = true, id = "connection-token-file")]
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
    // The command tree contains large async states. Keep it off the native
    // entry thread's stack (particularly the smaller Windows main stack).
    Box::pin(run_from(std::env::args().collect())).await
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
    // Every device runs the same way: node core plus its own interface. A saved legacy
    // gateway connection is kept alongside, never required; `peer serve` keeps its
    // meaning of running without any Workspace.
    let service = match &args.command {
        None => Some((None, Vec::new(), true, Default::default())),
        Some(agent::Command::Peer {
            command: crate::peer::Command::Serve,
        }) => Some((None, Vec::new(), false, Default::default())),
        Some(agent::Command::Serve {
            listen,
            allow_host,
            relay,
            public_url,
            relay_listen,
        }) => Some((
            *listen,
            allow_host.clone(),
            true,
            crate::web::standalone::RelayFlags {
                relay: *relay,
                public_url: public_url.clone(),
                listen: *relay_listen,
            },
        )),
        _ => None,
    };
    if let Some((listen, allow_hosts, with_gateway, relay)) = service {
        let gateway = if !with_gateway {
            None
        } else if args.server.is_some() || args.id.is_some() || args.token_file.is_some() {
            Some(Settings {
                server: args
                    .server
                    .ok_or_else(|| anyhow::anyhow!("--server required"))?,
                id: args.id.ok_or_else(|| anyhow::anyhow!("--id required"))?,
                token_file: args
                    .token_file
                    .ok_or_else(|| anyhow::anyhow!("--token-file required"))?,
                name: args.name,
                allow_insecure: args.allow_insecure,
            })
        } else if args.config.exists() {
            Some(Settings::read(&args.config)?)
        } else {
            None
        };
        return Box::pin(crate::web::standalone::run(
            peer_path,
            listen,
            allow_hosts,
            gateway,
            relay,
        ))
        .await;
    }
    let command = match args.command {
        Some(agent::Command::UiUrl { listen }) => {
            let token = crate::web::standalone::access_token(&peer_path)?;
            let listen =
                listen.unwrap_or_else(|| crate::web::standalone::recorded_address(&peer_path));
            println!("{}", crate::web::standalone::sign_in_url(listen, &token));
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
    let command = command.expect("the device service is handled above");
    agent::run(settings, command).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{peer, spaces};

    #[test]
    fn space_ids_are_distinct_from_the_gateway_node_id() {
        let id = uuid::Uuid::new_v4();
        for operation in ["stop", "resume", "remove", "credentials"] {
            let args = Args::try_parse_from([
                "cranibox-node",
                "--id",
                "gateway-node",
                "space",
                operation,
                &id.to_string(),
            ])
            .unwrap();
            assert_eq!(args.id.as_deref(), Some("gateway-node"));
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
    }

    #[test]
    fn job_ids_do_not_become_gateway_credentials() {
        for operation in ["job", "cancel"] {
            let args = Args::try_parse_from(["cranibox-node", operation, "job-123"]).unwrap();
            assert!(args.id.is_none());
            let parsed = match args.command.unwrap() {
                agent::Command::Job { id } | agent::Command::Cancel { id } => id,
                other => panic!("unexpected command: {other:?}"),
            };
            assert_eq!(parsed, "job-123");
        }
    }

    #[test]
    fn peer_tokens_are_distinct_from_the_gateway_token() {
        for (operation, flag) in [
            ("grant", "--output-token"),
            ("connect", "--credential-file"),
        ] {
            let mut command = vec![
                "cranibox-node",
                "--token-file",
                "gateway.token",
                "peer",
                operation,
                "--node",
                "peer-node",
                flag,
                "peer.token",
            ];
            if operation == "connect" {
                command.extend(["--url", "http://127.0.0.1:9020"]);
            }
            let args = Args::try_parse_from(command).unwrap();
            assert_eq!(args.token_file, Some(PathBuf::from("gateway.token")));
            let parsed = match args.command.unwrap() {
                agent::Command::Peer {
                    command:
                        peer::Command::Grant { token_file, .. }
                        | peer::Command::Connect { token_file, .. },
                } => token_file,
                other => panic!("unexpected command: {other:?}"),
            };
            assert_eq!(parsed, PathBuf::from("peer.token"));
        }
    }
}
