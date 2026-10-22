//! Opt-in target-authorized direct transport, independent of the Workspace gateway.
pub(crate) mod browser_proxy;
pub mod config;
pub(crate) mod hook;
pub(crate) mod secure_ws;
mod server;
pub mod transport_iroh;
pub(crate) use server::start_with_catalog;
pub(crate) use server::{NetworkEndpoint, NetworkListener};
pub use server::{Running, start};

use agent_workspace_pty::engine::Engine;
use agent_workspace_runtime::{
    client, config as secrets,
    protocol::{ExecRequest, MAX_PARALLEL, valid_node_id},
};
use anyhow::{Context, Result, ensure};
use clap::Subcommand;
use config::{Config, Grant, Listener, Permissions, Route};
use serde_json::json;
use std::{
    io::Read,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create or show this device's persistent direct identity, without secrets.
    Identity,
    /// Read active tasks, messages and collaborations; remote nodes require a separate activity grant.
    Activity {
        #[arg(long)]
        node: Option<String>,
    },
    /// Durable device text messages. Receiving a message never runs a command.
    Messages {
        #[command(subcommand)]
        action: Messages,
    },
    /// Read/update collaboration tasks; local run commands require explicit launch review.
    Tasks {
        /// A typed task command as JSON; omitted means {"operation":"list"}.
        #[arg(long)]
        command: Option<String>,
    },
    /// Configure this node's background project catalog pulls; no remote changes.
    ProjectSync {
        /// Typed JSON command; omitted means {"operation":"status"}.
        #[arg(long)]
        command: Option<String>,
    },
    /// Running Codex and Claude Code sessions on this device, found passively from their
    /// transcript files (repo, branch, activity). Local only; nothing is registered.
    DiscoverSessions,
    /// Configure background public Agent directory pulls; separate from project sync.
    AgentSync {
        #[arg(long)]
        command: Option<String>,
    },
    /// Opt in to background read-only collaboration snapshots; activity access is required.
    ActivitySync {
        #[arg(long)]
        command: Option<String>,
    },
    /// Explicit Agent session registrations; remote targets support list only.
    AgentSessions {
        #[arg(long)]
        node: Option<String>,
        #[arg(long)]
        command: Option<String>,
    },
    /// Durable noninteractive runs on this device or an authorized direct node.
    Runs {
        #[command(subcommand)]
        action: Option<Runs>,
        #[arg(long)]
        node: Option<String>,
        #[arg(long)]
        command: Option<String>,
    },
    /// Allow copied code snapshots under an existing local directory. Omit --path to disable.
    CopyRoot {
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Prepare, review and copy project files between this device and configured direct nodes.
    Copy {
        #[command(subcommand)]
        action: Copies,
    },
    /// Grant a caller access on THIS target; writes a new private token file.
    Grant {
        #[arg(long)]
        node: String,
        #[arg(long = "output-token")]
        token_file: PathBuf,
        #[arg(long)]
        exec: bool,
        #[arg(long)]
        terminals: bool,
        #[arg(long)]
        projects: bool,
        #[arg(long)]
        desktop: bool,
        /// Agent screenshot/input/window actions, with a short input lease.
        #[arg(long)]
        computer: bool,
        /// Full control of all tabs in the target's explicitly shared Chromium browser.
        #[arg(long)]
        browser: bool,
        #[arg(long)]
        messages: bool,
        /// Read all active task/message/collaboration content, including other senders. No writes.
        #[arg(long)]
        activity: bool,
        /// Read only the virtual-space directory. Does not grant Docker or space credentials.
        #[arg(long)]
        spaces: bool,
        /// Strict mode (the default): accept only end-to-end sessions from the pinned key and
        /// refuse every capability until the owner compared the pairing code (peer verify).
        /// Kept for scripts written before it became the default.
        #[arg(long, conflicts_with = "allow_unverified")]
        require_verified: bool,
        /// Less safe opt-out: allow the caller before its pairing code is verified, and allow
        /// plaintext bearer requests (older nodes, gateways). A key pinned on first use is
        /// still enforced once seen.
        #[arg(long)]
        allow_unverified: bool,
        /// Agent-to-agent content from this caller: hold (default, human approval queue),
        /// accept, or refuse.
        #[arg(long, value_enum, default_value = "hold")]
        inbound: InboundPolicy,
    },
    /// Change how agent-to-agent content from an authorized caller is handled.
    Inbound {
        #[arg(long)]
        node: String,
        #[arg(long, value_enum)]
        policy: InboundPolicy,
    },
    /// Mark a paired key as verified after comparing the pairing code shown on both devices.
    Verify {
        #[arg(long)]
        node: String,
        #[arg(long)]
        code: String,
    },
    /// Revoke a caller on THIS target; running direct commands and attachments are cancelled.
    Revoke {
        #[arg(long)]
        node: String,
    },
    /// Save an opt-in listener used by both the normal service and standalone peer serve.
    Listen {
        #[arg(long)]
        address: SocketAddr,
        #[arg(long)]
        allow_private_http: bool,
    },
    /// Disable the listener for the next service start.
    Disable,
    /// Share an existing loopback desktop server. Omit --address to stop sharing.
    Desktop {
        #[arg(long)]
        address: Option<SocketAddr>,
        #[arg(long, default_value = "vnc", value_parser = ["vnc", "selkies"])]
        protocol: String,
    },
    /// Share an existing local desktop bridge. Omit --address to disable Computer Use.
    Computer {
        #[arg(long)]
        address: Option<SocketAddr>,
    },
    /// Owner-only Computer Use status, emergency stop or resume as typed JSON.
    ComputerOwner {
        #[arg(long)]
        command: String,
    },
    /// Share an existing loopback Chromium CDP browser. Omit --address to disable.
    Browser {
        #[arg(long)]
        address: Option<SocketAddr>,
    },
    /// Owner-only browser status, emergency stop or resume as typed JSON.
    BrowserOwner {
        #[arg(long)]
        command: String,
    },
    /// Register an outbound direct route using a token issued by that target.
    Connect {
        #[arg(long)]
        node: String,
        #[arg(long)]
        url: String,
        #[arg(long = "credential-file")]
        token_file: PathBuf,
        #[arg(long)]
        allow_private_http: bool,
    },
    /// Add, prefer or remove explicit paths for one authorized device.
    Paths {
        #[command(subcommand)]
        action: Paths,
    },
    /// Keep this node's configured peer, desktop and network services running without a Workspace.
    Serve,
    /// Pair devices with a one-time link or QR code (iroh network).
    Pair {
        #[command(subcommand)]
        action: transport_iroh::PairCommand,
    },
    /// CLI/MCP calls go to the configured target; never automatically retry through the gateway.
    Call {
        #[arg(long)]
        node: String,
        #[command(subcommand)]
        action: Action,
    },
}
#[derive(Debug, Subcommand)]
pub enum Runs {
    /// Opt-in MCP for explicit process execution. Does not start a node service.
    Mcp,
}
#[derive(Debug, Subcommand)]
pub enum Paths {
    /// Verify a new address with the same target grant before saving it.
    Add {
        #[arg(long)]
        node: String,
        #[arg(long)]
        url: String,
        #[arg(long = "credential-file")]
        token_file: PathBuf,
        #[arg(long)]
        allow_private_http: bool,
    },
    /// Use this path first for new calls and attachments.
    Prefer {
        #[arg(long)]
        node: String,
        #[arg(long)]
        url: String,
    },
    /// Remove one path, keeping at least one connection to the device.
    Remove {
        #[arg(long)]
        node: String,
        #[arg(long)]
        url: String,
    },
}
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum Harness {
    Claude,
    Codex,
}
#[derive(Debug, Subcommand)]
pub enum Messages {
    /// Owner approval queue: messages held by inbound policy or the Agent loop guard.
    Held,
    /// Deliver a held message to this device's mailbox and Agents.
    Release {
        #[arg(long = "message-id")]
        message_id: uuid::Uuid,
    },
    /// Discard a held message without delivering it.
    Reject {
        #[arg(long = "message-id")]
        message_id: uuid::Uuid,
    },
    List {
        /// Route by this Agent UUID: addressed inbox and self-reported outbox.
        #[arg(long)]
        session: Option<uuid::Uuid>,
    },
    /// MCP stdio server for this device's mailbox; no Workspace relay required.
    Mcp,
    /// Claude Code / Codex hook: reads the hook event JSON on stdin and injects released,
    /// unread messages addressed to that session as untrusted context. Never fails the turn.
    Hook,
    /// Print hook configuration that runs `messages hook` for Claude Code or Codex.
    HookConfig {
        #[arg(long, value_enum)]
        harness: Harness,
        /// Also continue a session that is stopping when a message is waiting. The message
        /// then arrives as the next prompt of an unattended turn; off by default.
        #[arg(long)]
        wake_on_stop: bool,
    },
    Send {
        /// Reuse this UUID when retrying an ambiguous local submission.
        #[arg(long = "message-id")]
        message_id: Option<uuid::Uuid>,
        #[arg(long)]
        node: String,
        #[arg(long)]
        text: String,
        #[arg(long)]
        thread: Option<uuid::Uuid>,
        #[arg(long)]
        in_reply_to: Option<uuid::Uuid>,
        #[arg(long)]
        sender_session: Option<uuid::Uuid>,
        #[arg(long)]
        target_session: Option<uuid::Uuid>,
    },
    Acknowledge {
        message_id: uuid::Uuid,
    },
    Retry {
        message_id: uuid::Uuid,
    },
    /// Hide an incoming message or stop retries and delete a local outgoing record.
    Archive {
        message_id: uuid::Uuid,
    },
}
#[derive(Debug, Subcommand)]
pub enum Copies {
    List,
    /// Continue downloading a snapshot after an interrupted preparation.
    PrepareResume {
        id: String,
    },
    /// Review, explicitly start, or inspect the new Agent run for a completed copy.
    Agent {
        #[arg(long)]
        command: String,
    },
    Prepare {
        #[arg(long)]
        source: String,
        #[arg(long)]
        target: String,
        #[arg(long)]
        replica_id: String,
        /// JSON containing goal, progress, next_steps and verification. No credentials.
        #[arg(long)]
        handoff_file: Option<PathBuf>,
    },
    Commit {
        id: String,
        #[arg(long)]
        digest: String,
    },
    Status {
        id: String,
    },
    Discard {
        id: String,
    },
}
#[derive(Debug, Subcommand)]
pub enum Action {
    Nodes,
    /// Typed Computer Use command; omitted means status. Input requires an acquired lease.
    Computer {
        #[arg(long)]
        command: Option<String>,
    },
    /// Computer Use MCP bound to this explicitly selected device.
    ComputerMcp,
    /// Browser status or emergency stop as typed JSON; omitted means status.
    Browser {
        #[arg(long)]
        command: Option<String>,
    },
    /// Official Chrome DevTools MCP bound to this explicitly selected device.
    BrowserMcp,
    /// Print an ephemeral loopback CDP WebSocket URL; keep running until Ctrl+C.
    BrowserProxy,
    /// Read the target's explicitly shared virtual-space directory.
    Spaces,
    Exec {
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long, default_value_t = 60)]
        timeout_secs: u64,
        #[arg(long)]
        detach: bool,
        #[arg(required = true, trailing_var_arg = true)]
        command: Vec<String>,
    },
    Job {
        id: String,
    },
    Cancel {
        id: String,
    },
    Mcp,
    Sessions {
        #[command(subcommand)]
        action: Sessions,
    },
    Projects {
        #[command(subcommand)]
        action: Projects,
    },
}
#[derive(Debug, Subcommand)]
pub enum Projects {
    List,
    Register {
        #[arg(long)]
        name: String,
        #[arg(long)]
        path: String,
        #[arg(long)]
        project_id: Option<String>,
    },
    Bind {
        replica_id: String,
        terminal_id: String,
    },
    Unbind {
        replica_id: String,
        terminal_id: String,
    },
    Forget {
        replica_id: String,
    },
}
#[derive(Debug, Subcommand)]
pub enum Sessions {
    List,
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        cwd: Option<String>,
    },
    End {
        id: String,
    },
    Remove {
        id: String,
    },
}

pub async fn run(path: PathBuf, command: Command) -> Result<()> {
    match command {
        Command::Activity { node } => client::print(
            agent_workspace_runtime::peers::Store::new(path)
                .activity(node)
                .await?,
        ),
        Command::Identity => client::print(Config::update(&path, |_| Ok(()))?.public_view()),
        Command::Tasks { command } => {
            let command =
                serde_json::from_str(command.as_deref().unwrap_or(r#"{"operation":"list"}"#))?;
            client::print(
                agent_workspace_runtime::peers::Store::new(path)
                    .tasks(command)
                    .await?,
            );
        }
        Command::ProjectSync { command } => {
            let command =
                serde_json::from_str(command.as_deref().unwrap_or(r#"{"operation":"status"}"#))?;
            client::print(agent_workspace_runtime::peers::project_sync::manage(
                &path, command,
            )?);
        }
        Command::DiscoverSessions => {
            use agent_workspace_runtime::peers::discovery;
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .context("HOME is not set")?;
            let sessions = discovery::discover(
                &discovery::Roots::from_env(&home),
                std::time::SystemTime::now(),
            );
            client::print(json!({"sessions": sessions}));
        }
        Command::AgentSync { command } => {
            let command =
                serde_json::from_str(command.as_deref().unwrap_or(r#"{"operation":"status"}"#))?;
            client::print(agent_workspace_runtime::peers::project_sync::manage_kind(
                &path,
                command,
                agent_workspace_runtime::peers::project_sync::Kind::Agents,
            )?);
        }
        Command::ActivitySync { command } => {
            let command =
                serde_json::from_str(command.as_deref().unwrap_or(r#"{"operation":"status"}"#))?;
            client::print(agent_workspace_runtime::peers::project_sync::manage_kind(
                &path,
                command,
                agent_workspace_runtime::peers::project_sync::Kind::Activity,
            )?);
        }
        Command::AgentSessions { node, command } => {
            let command =
                serde_json::from_str(command.as_deref().unwrap_or(r#"{"operation":"list"}"#))?;
            client::print(
                agent_workspace_runtime::peers::Store::new(path)
                    .agent_sessions(node, command)
                    .await?,
            );
        }
        Command::Runs {
            action,
            node,
            command,
        } => {
            if matches!(action, Some(Runs::Mcp)) {
                ensure!(
                    node.is_none() && command.is_none(),
                    "runs mcp does not accept --node or --command; choose an explicit node in each tool call"
                );
                Config::read(&path)?;
                return client::executions_mcp(agent_workspace_runtime::peers::Store::new(path))
                    .await;
            }
            let command =
                serde_json::from_str(command.as_deref().unwrap_or(r#"{"operation":"list"}"#))?;
            client::print(
                agent_workspace_runtime::peers::Store::new(path)
                    .node_executions(node, command)
                    .await?,
            );
        }
        Command::Messages { action } => {
            use agent_workspace_runtime::peers::{Store, messages::Command as Message};
            let store = Store::new(path.clone());
            if matches!(action, Messages::Hook) {
                return crate::peer::hook::run(&store).await;
            }
            if matches!(action, Messages::Mcp) {
                Config::read(&path)?;
                let stop = CancellationToken::new();
                let delivery = crate::messages::Service::start(path, stop.clone());
                let result = client::messages_mcp(store).await;
                stop.cancel();
                delivery.shutdown().await;
                return result;
            }
            let command = match action {
                Messages::Mcp | Messages::Hook | Messages::HookConfig { .. } => {
                    unreachable!("handled above")
                }
                Messages::List { session } => Message::List { session },
                Messages::Send {
                    message_id,
                    node,
                    text,
                    thread,
                    in_reply_to,
                    sender_session,
                    target_session,
                } => Message::Send {
                    id: message_id,
                    node,
                    text,
                    thread,
                    in_reply_to,
                    sender_session,
                    target_session,
                },
                Messages::Acknowledge { message_id } => Message::Acknowledge { id: message_id },
                Messages::Retry { message_id } => Message::Retry { id: message_id },
                Messages::Archive { message_id } => Message::Archive { id: message_id },
                Messages::Held => Message::Held,
                Messages::Release { message_id } => Message::Release { id: message_id },
                Messages::Reject { message_id } => Message::Reject { id: message_id },
            };
            client::print(store.messages(command).await?);
        }
        Command::Copy { action } => {
            use agent_workspace_runtime::peers::{Command as CopyCommand, Store};
            let command = match action {
                Copies::List => CopyCommand::CopyList,
                Copies::PrepareResume { id } => CopyCommand::CopyPrepareResume { id },
                Copies::Agent { command } => {
                    let value: CopyCommand = serde_json::from_str(&command)?;
                    ensure!(
                        matches!(
                            value,
                            CopyCommand::CopyAgentPrepare { .. }
                                | CopyCommand::CopyAgentStart { .. }
                                | CopyCommand::CopyAgentStatus { .. }
                                | CopyCommand::CopyAgentStop { .. }
                        ),
                        "expected a copy_agent_prepare, copy_agent_start, copy_agent_status or copy_agent_stop command"
                    );
                    value
                }
                Copies::Prepare {
                    source,
                    target,
                    replica_id,
                    handoff_file,
                } => CopyCommand::CopyPrepare {
                    source,
                    target,
                    replica_id,
                    handoff: handoff_file
                        .map(|path| {
                            let mut bytes = vec![];
                            std::fs::File::open(path)?
                                .take(65537)
                                .read_to_end(&mut bytes)?;
                            ensure!(bytes.len() <= 65536, "handoff JSON file exceeds 64 KiB");
                            Ok::<_, anyhow::Error>(serde_json::from_slice(&bytes)?)
                        })
                        .transpose()?,
                },
                Copies::Commit { id, digest } => CopyCommand::CopyCommit { id, digest },
                Copies::Status { id } => CopyCommand::CopyStatus { id },
                Copies::Discard { id } => CopyCommand::CopyDiscard { id },
            };
            client::print(Store::new(path).manage(command).await?);
        }
        Command::CopyRoot { path: root } => {
            let root = root
                .map(|p| agent_workspace_runtime::projects::copy::root(&p))
                .transpose()?;
            client::print(
                Config::update(&path, |c| {
                    c.copy_root = root;
                    Ok(())
                })?
                .public_view(),
            );
        }
        Command::Grant {
            node,
            token_file,
            exec,
            terminals,
            projects,
            desktop,
            computer,
            browser,
            messages,
            activity,
            spaces,
            require_verified: _,
            allow_unverified,
            inbound,
        } => {
            ensure!(valid_node_id(&node), "invalid caller node ID");
            ensure!(
                exec || terminals
                    || projects
                    || desktop
                    || computer
                    || browser
                    || messages
                    || activity
                    || spaces,
                "choose --exec, --terminals, --projects, --desktop, --computer, --browser, --messages, --activity and/or --spaces"
            );
            let token = secrets::token();
            let value = Config::update(&path, |value| {
                let parent = |p: &Path| {
                    p.parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or(Path::new("."))
                        .to_path_buf()
                };
                std::fs::create_dir_all(parent(&token_file))?;
                ensure!(
                    std::fs::canonicalize(parent(&path))?
                        != std::fs::canonicalize(parent(&token_file))?
                        || path.file_name() != token_file.file_name(),
                    "credential output must not replace the peer configuration"
                );
                ensure!(node != value.node_id, "cannot grant this node to itself");
                ensure!(
                    !value.grants.contains_key(&node),
                    "caller already granted; revoke before rotating its credential"
                );
                ensure!(value.grants.len() < 128, "peer limit reached");
                secrets::write_private(&token_file, format!("{token}\n").as_bytes(), false)?;
                let mut grant = Grant::issue(
                    &token,
                    Permissions {
                        exec,
                        terminals,
                        projects,
                        desktop,
                        computer,
                        browser,
                        messages,
                        activity,
                        spaces,
                        // Granted by owner-device pairing (WS1/WS6), not by this CLI.
                        llm: false,
                        vault: false,
                    },
                );
                grant.require_verified = !allow_unverified;
                grant.inbound = inbound.into();
                value.grants.insert(node, grant);
                Ok(())
            })?;
            client::print(value.public_view());
        }
        Command::Inbound { node, policy } => client::print(
            agent_workspace_runtime::peers::Store::new(path.clone())
                .manage(agent_workspace_runtime::peers::Command::Inbound {
                    node,
                    policy: policy.into(),
                })
                .await?,
        ),
        Command::Verify { node, code } => client::print(
            agent_workspace_runtime::peers::Store::new(path.clone())
                .manage(agent_workspace_runtime::peers::Command::Verify { node, code })
                .await?,
        ),
        Command::Revoke { node } => client::print(
            Config::update(&path, |value| {
                ensure!(
                    value.grants.remove(&node).is_some(),
                    "caller is not granted"
                );
                Ok(())
            })?
            .public_view(),
        ),
        Command::Computer { address } => client::print(
            agent_workspace_runtime::peers::Store::new(path.clone())
                .manage(agent_workspace_runtime::peers::Command::ComputerSharing {
                    computer: address.map(|address| {
                        agent_workspace_runtime::peers::computer::Provider::http(address)
                    }),
                })
                .await?,
        ),
        Command::ComputerOwner { command } => client::print(
            agent_workspace_runtime::peers::Store::new(path.clone())
                .manage(agent_workspace_runtime::peers::Command::ComputerOwner {
                    command: serde_json::from_str(&command)
                        .context("invalid Computer Use owner command")?,
                })
                .await?,
        ),
        Command::Browser { address } => client::print(
            agent_workspace_runtime::peers::Store::new(path.clone())
                .manage(agent_workspace_runtime::peers::Command::BrowserSharing {
                    browser: address.map(|address| {
                        agent_workspace_runtime::peers::browser::Provider { address }
                    }),
                })
                .await?,
        ),
        Command::BrowserOwner { command } => client::print(
            agent_workspace_runtime::peers::Store::new(path.clone())
                .manage(agent_workspace_runtime::peers::Command::BrowserOwner {
                    command: serde_json::from_str(&command)
                        .context("invalid browser owner command")?,
                })
                .await?,
        ),
        Command::Desktop { address, protocol } => client::print(
            Config::update(&path, |config| {
                config.desktop =
                    address.map(|address| agent_workspace_runtime::peers::config::Desktop {
                        address,
                        protocol: if protocol == "selkies" {
                            agent_workspace_runtime::peers::config::DesktopProtocol::Selkies
                        } else {
                            agent_workspace_runtime::peers::config::DesktopProtocol::Vnc
                        },
                    });
                config.desktop_epoch = config
                    .desktop_epoch
                    .checked_add(1)
                    .context("desktop generation exhausted")?;
                Ok(())
            })?
            .public_view(),
        ),
        Command::Listen {
            address,
            allow_private_http,
        } => client::print(
            Config::update(&path, |value| {
                let listener = Listener {
                    address,
                    allow_insecure: allow_private_http,
                };
                listener.validate()?;
                value.listener = Some(listener);
                Ok(())
            })?
            .public_view(),
        ),
        Command::Disable => client::print(
            Config::update(&path, |value| {
                value.listener = None;
                Ok(())
            })?
            .public_view(),
        ),
        Command::Connect {
            node,
            url,
            token_file,
            allow_private_http,
        } => {
            ensure!(valid_node_id(&node), "invalid target node ID");
            secrets::read_token(&token_file)?;
            let route = Route {
                target_identity: None,
                config_path: None,
                alternatives: vec![],
                url,
                token_file: std::fs::canonicalize(token_file)?,
                allow_insecure: allow_private_http,
                local_forward_lock: None,
            };
            route.api(&node)?;
            client::print(
                Config::update(&path, |value| {
                    // Reconnecting keeps a pinned target key: a changed key must be re-paired.
                    let pinned = value
                        .routes
                        .get(&node)
                        .and_then(|old| old.target_identity.clone());
                    value.routes.insert(
                        node,
                        Route {
                            target_identity: pinned,
                            ..route
                        },
                    );
                    Ok(())
                })?
                .public_view(),
            );
        }
        Command::Paths { action } => {
            use agent_workspace_runtime::peers::{Command as PeerCommand, Store};
            let command = match action {
                Paths::Add {
                    node,
                    url,
                    token_file,
                    allow_private_http,
                } => PeerCommand::AddPath {
                    node,
                    url,
                    token: secrets::read_token(&token_file)?,
                    allow_private_http,
                },
                Paths::Prefer { node, url } => PeerCommand::PreferPath { node, url },
                Paths::Remove { node, url } => PeerCommand::RemovePath { node, url },
            };
            client::print(Store::new(path).manage(command).await?);
        }
        Command::Serve => serve(&path).await?,
        Command::Pair { action } => transport_iroh::run(path.clone(), action).await?,
        Command::Call { node, action } => call(&path, &node, action).await?,
    }
    Ok(())
}

async fn serve(path: &Path) -> Result<()> {
    let _lock = service_lock(path)?;
    let stop = CancellationToken::new();
    let signal = stop.clone();
    tokio::spawn(async move {
        agent_workspace_runtime::shutdown_signal().await;
        signal.cancel();
    });
    let engine = Engine::new(home()?);
    let slots = Arc::new(tokio::sync::Semaphore::new(MAX_PARALLEL));
    let started = async {
        Config::update(path, |_| Ok(()))?;
        if let Some(running) =
            start(path.into(), engine.clone(), slots.clone(), stop.clone()).await?
        {
            Ok(running)
        } else {
            // An EasyTier-only node does not need a separate plaintext LAN listener.
            start_with_catalog(
                path.into(),
                engine.clone(),
                slots,
                stop.clone(),
                agent_workspace_runtime::projects::Store::new(
                    path.with_file_name("node-projects.json"),
                ),
            )
            .await
        }
    }
    .await;
    let result = match started {
        Ok(running) => {
            let supervisor = match Config::read(path).and_then(|configuration| {
                crate::supervisor::Service::start(path.into(), configuration.node_id, stop.clone())
            }) {
                Ok(service) => service,
                Err(error) => {
                    stop.cancel();
                    let _ = running.shutdown().await;
                    engine.shutdown().await;
                    return Err(error);
                }
            };
            let desktop_service = crate::desktop_service::Service::start(path.into(), stop.clone());
            let browser_service = agent_workspace_runtime::peers::browser::manager::Service::start(
                path.into(),
                stop.clone().cancelled_owned(),
            );
            let task_worker = crate::task_worker::Service::start(path.into(), stop.clone());
            let project_sync = crate::project_sync::Service::start(path.into(), stop.clone());
            let messages = crate::messages::Service::start(path.into(), stop.clone());
            let network = crate::network::Service::start(
                path.into(),
                running.network_endpoint(),
                stop.clone(),
            );
            let iroh = transport_iroh::Service::start(
                path.into(),
                running.network_endpoint(),
                running.mesh_router(),
                stop.clone(),
            )
            .await;
            let result = running.wait().await;
            stop.cancel();
            tokio::join!(
                iroh.shutdown(),
                network.shutdown(),
                desktop_service.shutdown(),
                browser_service.shutdown(),
                messages.shutdown(),
                task_worker.shutdown(),
                project_sync.shutdown(),
                supervisor.shutdown()
            );
            result
        }
        Err(error) => Err(error),
    };
    stop.cancel();
    engine.shutdown().await;
    result
}
pub(crate) fn home() -> Result<PathBuf> {
    Ok(std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?))
}
pub(crate) fn service_lock(path: &Path) -> Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("service.lock"))?;
    fs2::FileExt::try_lock_exclusive(&file).map_err(|_| {
        anyhow::anyhow!("a direct node service is already running with this identity")
    })?;
    Ok(file)
}
async fn call(path: &Path, node: &str, action: Action) -> Result<()> {
    ensure!(valid_node_id(node), "invalid target node ID");
    let configuration = Config::read(path)?;
    let route = configuration
        .routes
        .get(node)
        .ok_or_else(|| anyhow::anyhow!("configure this target with 'peer connect'"))?;
    let api = route.api(node)?;
    let value = match action {
        Action::Nodes => api.probe().await?,
        Action::Computer { command } => {
            let request =
                serde_json::from_str(command.as_deref().unwrap_or("{\"operation\":\"status\"}"))
                    .context("invalid Computer Use command")?;
            agent_workspace_runtime::peers::Store::new(path.into())
                .computer(node, request)
                .await?
        }
        Action::ComputerMcp => {
            client::computer_mcp(
                agent_workspace_runtime::peers::Store::new(path.into()),
                node.into(),
            )
            .await?;
            return Ok(());
        }
        Action::Browser { command } => {
            let request =
                serde_json::from_str(command.as_deref().unwrap_or("{\"operation\":\"status\"}"))
                    .context("invalid browser command")?;
            agent_workspace_runtime::peers::Store::new(path.into())
                .browser(node, request)
                .await?
        }
        Action::BrowserMcp | Action::BrowserProxy => {
            browser_proxy::run(
                path.into(),
                node.into(),
                matches!(action, Action::BrowserMcp),
            )
            .await?;
            return Ok(());
        }
        Action::Spaces => {
            agent_workspace_runtime::peers::Store::new(path.into())
                .space_catalog(node)
                .await?
        }
        Action::Job { id } => api.get(&client::job_path(&id)?).await?,
        Action::Cancel { id } => {
            api.post(&format!("{}/cancel", client::job_path(&id)?), json!({}))
                .await?
        }
        Action::Mcp => {
            client::mcp(api).await?;
            return Ok(());
        }
        Action::Exec {
            cwd,
            timeout_secs,
            detach,
            mut command,
        } => {
            let result = api
                .execute(
                    node,
                    ExecRequest {
                        program: command.remove(0),
                        args: command,
                        cwd,
                        env: Default::default(),
                        timeout_secs,
                    },
                    detach,
                )
                .await?;
            let success = detach || (result["outcome"] == "exited" && result["exit_code"] == 0);
            client::print(result);
            ensure!(success, "direct command failed");
            return Ok(());
        }
        Action::Sessions { action } => {
            let command = match action {
                Sessions::List => json!({"operation":"list"}),
                Sessions::Create { name, cwd } => {
                    json!({"operation":"create","name":name,"cwd":cwd})
                }
                Sessions::End { id } => json!({"operation":"end","id":id}),
                Sessions::Remove { id } => json!({"operation":"remove","id":id}),
            };
            api.post(&format!("/api/v1/nodes/{node}/terminal/sessions"), command)
                .await?
        }
        Action::Projects { action } => {
            use agent_workspace_runtime::projects::Command;
            let command = match action {
                Projects::List => Command::List {},
                Projects::Register {
                    name,
                    path,
                    project_id,
                } => Command::Register {
                    name,
                    path,
                    project_id,
                    node: Some(node.into()),
                },
                Projects::Bind {
                    replica_id,
                    terminal_id,
                } => Command::Bind {
                    replica_id,
                    terminal_id,
                },
                Projects::Unbind {
                    replica_id,
                    terminal_id,
                } => Command::Unbind {
                    replica_id,
                    terminal_id,
                },
                Projects::Forget { replica_id } => Command::Forget { replica_id },
            };
            api.post(
                &format!("/api/v1/nodes/{node}/projects"),
                serde_json::to_value(command)?,
            )
            .await?
        }
    };
    client::print(value);
    Ok(())
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum InboundPolicy {
    Accept,
    Hold,
    Refuse,
}
impl From<InboundPolicy> for agent_workspace_runtime::peers::config::Inbound {
    fn from(value: InboundPolicy) -> Self {
        match value {
            InboundPolicy::Accept => Self::Accept,
            InboundPolicy::Hold => Self::Hold,
            InboundPolicy::Refuse => Self::Refuse,
        }
    }
}
