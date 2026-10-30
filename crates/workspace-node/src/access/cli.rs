//! Local store operations: no daemon or network is needed to approve a request.
use super::*;
use clap::{Args as ClapArgs, Subcommand};
use serde_json::{Value, json};

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Print structured JSON instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List peers and their modes, pending requests, approvals and active sessions.
    List,
    /// Change capabilities or groups (messages, terminal, screen, browser, update,
    /// spaces, activity, models). Multiple assignments are applied together.
    Set {
        node: String,
        #[arg(required = true, value_parser = assignment)]
        changes: Vec<Assignment>,
    },
    /// Approve a pending request, bound to its original operation.
    Approve {
        request_id: String,
        #[arg(long = "for", default_value = "session", value_parser = scope)]
        duration: DurationScope,
    },
    Deny {
        request_id: String,
    },
    /// Revoke approvals and disconnect their sessions. Defaults to all.
    Revoke {
        #[arg(long, conflicts_with = "approval_id")]
        all: bool,
        approval_id: Option<String>,
    },
    /// Requests waiting on your other devices, and deciding them from here.
    Remote {
        #[command(subcommand)]
        command: Remote,
    },
    /// Show recent audit entries; integrity is always checked.
    Log {
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        node: Option<String>,
        #[arg(long, value_parser = capability)]
        capability: Option<Capability>,
        /// Exit with an error when the hash chain is broken.
        #[arg(long)]
        verify: bool,
    },
}
#[derive(Subcommand, Debug)]
enum Remote {
    /// List requests waiting on your other devices.
    List,
    /// Approve or deny a request waiting on another of your devices. Reads your
    /// recovery code (or recovery passphrase) from standard input; it is used on this
    /// device only to sign the decision and is never sent or kept.
    Approve {
        node: String,
        request_id: String,
        #[arg(long = "for", default_value = "session", value_parser = scope)]
        duration: DurationScope,
        #[arg(long)]
        deny: bool,
    },
    /// Send an already signed decision file to the device it names, unchanged. The
    /// device accepts each decision once.
    Submit { file: std::path::PathBuf },
    /// Whether your other devices may decide requests waiting on this one.
    Allow {
        #[arg(value_parser = clap::builder::BoolishValueParser::new())]
        enabled: bool,
    },
}
fn read_line() -> Result<String> {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}
fn app_error(e: cranibox_contracts::app::AppError) -> anyhow::Error {
    match e.body {
        Some(body) => anyhow::anyhow!("{}: {} {}", e.code, e.message, body),
        None => anyhow::anyhow!("{}: {}", e.code, e.message),
    }
}
async fn remote(peers: &Path, store: &Store, command: Remote) -> Result<Value> {
    match command {
        Remote::List => Ok(super::api::remote_list(peers, store).await),
        Remote::Approve {
            node,
            request_id,
            duration,
            deny,
        } => {
            let node = store
                .owner_devices()
                .into_iter()
                .find(|(id, name)| *id == node || name.eq_ignore_ascii_case(&node))
                .map(|(id, _)| id)
                .context("not one of your other devices")?;
            let list = super::api::remote_list(peers, store).await;
            let shown = list["devices"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|d| d["node"] == node.as_str())
                .flat_map(|d| d["requests"].as_array().cloned().unwrap_or_default())
                .find(|p| p["request"]["id"] == request_id.as_str())
                .context("no such request waiting on that device")?;
            eprintln!(
                "{} wants {} on {}",
                shown["request"]["name"].as_str().unwrap_or(""),
                operation_line(&shown["request"]["operation"])
                    .unwrap_or_else(|| shown["request"]["capability"].to_string()),
                store.name(&node)
            );
            eprintln!("Recovery code or passphrase:");
            let auth = json!({"code": *zeroize::Zeroizing::new(read_line()?)});
            let body = json!({
                "node": node,
                "request": request_id,
                "op_hash": shown["op_hash"],
                "decision": if deny { "deny" } else { "approve" },
                "scope": (!deny).then_some(duration),
                "auth": auth,
            });
            super::api::remote_decide(peers, store, body)
                .await
                .map_err(app_error)
        }
        Remote::Submit { file } => {
            let assertion: cranibox_contracts::access::RemoteApproval =
                serde_json::from_slice(&std::fs::read(&file)?).context("not a signed decision")?;
            agent_workspace_runtime::client::Fleet::new(peers.to_owned())
                .post(
                    &assertion.target,
                    "/api/v1/access/remote/decide",
                    serde_json::to_value(&assertion)?,
                )
                .await
        }
        Remote::Allow { enabled } => {
            store.set_remote_enabled(enabled)?;
            Ok(json!({"remote_enabled": store.remote_enabled()}))
        }
    }
}
#[derive(Clone, Debug)]
struct Assignment(Vec<Capability>, Mode);
fn capability(value: &str) -> std::result::Result<Capability, String> {
    serde_json::from_value(json!(value)).map_err(|_| format!("unknown capability: {value}"))
}
fn scope(value: &str) -> std::result::Result<DurationScope, String> {
    serde_json::from_value(json!(value))
        .map_err(|_| "use session, 30m, 1h, 8h, today, 7d or always".into())
}
fn assignment(value: &str) -> std::result::Result<Assignment, String> {
    let (key, value) = value.split_once('=').ok_or("expected capability=mode")?;
    let mode = serde_json::from_value(json!(value)).map_err(|_| "use off, ask or allow")?;
    use Capability::*;
    let caps = match key {
        "terminal" | "commands" | "终端与命令" => vec![Terminals, Exec, Projects],
        "screen" | "屏幕" => vec![Desktop, Computer],
        "models" | "模型与账号" => vec![Llm, Vault],
        "消息与任务" => vec![Messages],
        "浏览器" => vec![Browser],
        "远程更新" => vec![Update],
        "虚拟空间" => vec![Spaces],
        "活动与同步" => vec![Activity],
        _ => vec![capability(key)?],
    };
    Ok(Assignment(caps, mode))
}
fn resolve(store: &Store, value: &str) -> Result<String> {
    let peers = store.view()?.peers;
    if peers.iter().any(|peer| peer.node == value) {
        return Ok(value.into());
    }
    let matches: Vec<_> = peers
        .into_iter()
        .filter(|peer| peer.name.eq_ignore_ascii_case(value))
        .collect();
    if matches.is_empty() && agent_workspace_runtime::protocol::valid_node_id(value) {
        return Ok(value.into());
    }
    ensure!(
        matches.len() == 1,
        "device name is unknown or ambiguous: {value}; use its node ID"
    );
    Ok(matches[0].node.clone())
}
fn execute(store: &Store, command: Command) -> Result<(Value, bool)> {
    match command {
        Command::Remote { .. } => anyhow::bail!("remote commands run asynchronously"),
        Command::List => Ok((serde_json::to_value(store.view()?)?, false)),
        Command::Set { node, changes } => {
            let node = resolve(store, &node)?;
            let mut modes = BTreeMap::new();
            for Assignment(caps, mode) in changes {
                for cap in caps {
                    modes.insert(cap, mode);
                }
            }
            store.change(&node, modes)?;
            Ok((json!({"node":node,"result":"saved"}), false))
        }
        Command::Approve {
            request_id,
            duration,
        } => Ok((
            serde_json::to_value(store.decide(
                &request_id,
                DecideRequest {
                    decision: ApprovalDecision::Approve,
                    scope: Some(duration),
                },
            )?)?,
            false,
        )),
        Command::Deny { request_id } => Ok((
            serde_json::to_value(store.decide(
                &request_id,
                DecideRequest {
                    decision: ApprovalDecision::Deny,
                    scope: None,
                },
            )?)?,
            false,
        )),
        Command::Revoke { approval_id, .. } => {
            store.revoke(approval_id.as_deref(), false)?;
            Ok((json!({"result":"revoked", "id":approval_id}), false))
        }
        Command::Log {
            limit,
            node,
            capability,
            verify,
        } => {
            let node = node.map(|node| resolve(store, &node)).transpose()?;
            let audit = store.audit(limit, None, node.as_deref(), capability)?;
            let failed = verify && !audit.verified;
            Ok((serde_json::to_value(audit)?, failed))
        }
    }
}
pub async fn run(peers: &Path, args: Args) -> Result<()> {
    let store = Store::new(peers);
    let (value, failed) = match args.command {
        Command::Remote { command } => (remote(peers, &store, command).await?, false),
        command => execute(&store, command)?,
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("{}", human(&value));
    }
    ensure!(!failed, "access audit integrity check failed");
    Ok(())
}
/// `exec.run [danger, asks every time]: rm -rf /tmp/x` for an operation, if any.
fn operation_line(operation: &Value) -> Option<String> {
    let kind = operation["kind"].as_str()?;
    let mut tags = vec![operation["risk"].as_str().unwrap_or("").to_owned()];
    if operation["confirm"] == true {
        tags.push("asks every time".into());
    }
    let summary = operation["summary"].as_str().unwrap_or("");
    Some(format!(
        "{kind} [{}]{}",
        tags.join(", "),
        if summary.is_empty() {
            String::new()
        } else {
            format!(": {summary}")
        }
    ))
}
fn human(value: &Value) -> String {
    if let Some(peers) = value["peers"].as_array() {
        let mut output = String::from("Peers (what they may do on this device):\n");
        for peer in peers {
            output.push_str(&format!(
                "{} ({})\n",
                peer["name"].as_str().unwrap_or(""),
                peer["node"].as_str().unwrap_or("")
            ));
            if let Some(caps) = peer["capabilities"].as_object() {
                for (cap, mode) in caps {
                    output.push_str(&format!("  {cap}={}\n", mode.as_str().unwrap_or("")));
                }
            }
        }
        for (key, title) in [
            ("pending", "Pending requests"),
            ("approvals", "Approvals"),
            ("sessions", "Active sessions"),
        ] {
            output.push_str(&format!("{title}:\n"));
            if let Some(items) = value[key].as_array() {
                for item in items {
                    let from = item["from"].as_str().unwrap_or("");
                    let name = peers
                        .iter()
                        .find(|peer| peer["node"] == from)
                        .and_then(|peer| peer["name"].as_str())
                        .unwrap_or(from);
                    output.push_str(&format!(
                        "  {}  {} ({})  {}  scope={}  expires={}\n",
                        item["id"].as_str().unwrap_or(""),
                        name,
                        from,
                        item["capability"].as_str().unwrap_or(""),
                        item["scope"].as_str().unwrap_or("session"),
                        item["expires_at"]
                            .as_u64()
                            .map(|at| at.to_string())
                            .unwrap_or_else(|| "session end".into())
                    ));
                    if let Some(line) = operation_line(&item["operation"]) {
                        output.push_str(&format!("    wants to: {line}\n"));
                    }
                }
                if items.is_empty() {
                    output.push_str("  none\n");
                }
            }
        }
        output
    } else if let Some(entries) = value["entries"].as_array() {
        let mut output = if value["verified"] == true {
            "Log verified\n".into()
        } else {
            format!(
                "Log may have been tampered with (entry {})\n",
                value["broken_index"]
            )
        };
        for entry in entries {
            output.push_str(&format!(
                "#{} {}  {} ({})  {}  {}  {}\n",
                entry["index"],
                entry["at"],
                entry["name"].as_str().unwrap_or("local"),
                entry["from"].as_str().unwrap_or(""),
                entry["capability"].as_str().unwrap_or("—"),
                entry["event"].as_str().unwrap_or(""),
                entry["result"].as_str().unwrap_or("")
            ));
            if let Some(line) = operation_line(&entry["operation"]) {
                let status = entry["status"]
                    .as_u64()
                    .map(|s| format!("  status={s}"))
                    .unwrap_or_default();
                output.push_str(&format!("    {line}{status}\n"));
            }
        }
        output
    } else if value["result"] == "saved" {
        format!(
            "Saved access modes for {}.",
            value["node"].as_str().unwrap_or("")
        )
    } else if value["result"] == "revoked" {
        match value["id"].as_str() {
            Some(id) => format!("Revoked approval {id} and disconnected its sessions."),
            None => "Revoked all approvals and disconnected all sessions.".into(),
        }
    } else if let Some(state) = value["state"].as_str() {
        format!(
            "Request {}: {state} ({} from {}).",
            value["id"].as_str().unwrap_or(""),
            value["capability"].as_str().unwrap_or(""),
            value["from"].as_str().unwrap_or("")
        )
    } else {
        serde_json::to_string_pretty(value).unwrap_or_default()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[derive(Parser)]
    struct TestArgs {
        #[command(flatten)]
        args: Args,
    }
    fn parse(args: &[&str]) -> Args {
        TestArgs::try_parse_from(args).unwrap().args
    }
    #[test]
    fn parsing_validates_assignments_scopes_and_conflicts() {
        for duration in ["session", "30m", "8h"] {
            parse(&["access", "approve", "request", "--for", duration, "--json"]);
        }
        parse(&[
            "access",
            "set",
            "Desk PC",
            "terminal=ask",
            "exec=off",
            "screen=allow",
            "models=ask",
        ]);
        parse(&[
            "access",
            "log",
            "--limit",
            "20",
            "--node",
            "Desk PC",
            "--capability",
            "update",
            "--verify",
        ]);
        for args in [
            vec!["access", "set", "Desk", "exec=always"],
            vec!["access", "set", "Desk", "admin=allow"],
            vec!["access", "approve", "id", "--for", "forever"],
            vec!["access", "revoke", "id", "--all"],
        ] {
            assert!(TestArgs::try_parse_from(args).is_err());
        }
    }
    #[test]
    fn local_set_approve_use_log_cycle() {
        let (_root, store) = super::super::tests::fixture();
        let node = super::super::tests::NODE;
        execute(
            &store,
            parse(&["access", "set", node, "terminal=ask"]).command,
        )
        .unwrap();
        let Denied::ApprovalRequired { request_id, .. } = store
            .authorize(node, Capability::Exec, "cycle")
            .unwrap_err()
        else {
            panic!("approval required")
        };
        execute(
            &store,
            parse(&["access", "approve", &request_id, "--for", "30m"]).command,
        )
        .unwrap();
        let session = store.start(node, Capability::Exec, "cycle").unwrap();
        assert!(session.valid());
        drop(session);
        let (log, failed) = execute(
            &store,
            parse(&["access", "log", "--verify", "--node", node]).command,
        )
        .unwrap();
        assert!(!failed);
        assert!(human(&log).contains("Log verified"));
        for kind in [
            "policy_changed",
            "approved",
            "session_started",
            "session_ended",
        ] {
            assert!(
                log["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|entry| entry["event"] == kind)
            );
        }
        execute(&store, parse(&["access", "revoke", "--all"]).command).unwrap();
        assert!(store.view().unwrap().approvals.is_empty());
    }
    #[test]
    fn names_resolve_only_when_unambiguous() {
        let (_root, store) = super::super::tests::fixture();
        let node = super::super::tests::NODE;
        crate::app::known::update(&store.peers, node, |known| {
            known.alias = Some("Desk PC".into())
        })
        .unwrap();
        assert_eq!(resolve(&store, "desk pc").unwrap(), node);
        execute(
            &store,
            parse(&["access", "set", "Desk PC", "screen=ask"]).command,
        )
        .unwrap();
        assert_eq!(
            store.view().unwrap().peers[0].capabilities[&Capability::Desktop],
            Mode::Ask
        );
        crate::app::known::update(
            &store.peers,
            "node-abcdef0123456789abcdef0123456789",
            |known| known.name = Some("Desk PC".into()),
        )
        .unwrap();
        assert!(resolve(&store, "Desk PC").is_err());
        assert!(resolve(&store, "Missing device").is_err());
        crate::app::known::update(&store.peers, node, |known| {
            known.alias = Some("Laptop".into())
        })
        .unwrap();
        assert_eq!(resolve(&store, "laptop").unwrap(), node);
        assert_eq!(resolve(&store, node).unwrap(), node);
    }
    #[tokio::test]
    async fn log_reports_tampering_and_verify_fails() {
        let (root, store) = super::super::tests::fixture();
        let node = super::super::tests::NODE;
        execute(
            &store,
            parse(&["access", "set", node, "terminal=ask"]).command,
        )
        .unwrap();
        let path = root.path().join("access-audit.jsonl");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replacen("policy_changed", "session_started", 1)).unwrap();
        let (log, failed) = execute(&store, parse(&["access", "log"]).command).unwrap();
        assert!(!failed);
        assert_eq!(log["verified"], false);
        assert_eq!(log["broken_index"], 1);
        assert!(human(&log).contains("tampered with (entry 1)"));
        assert!(
            run(
                &store.peers,
                parse(&["access", "log", "--verify", "--json"])
            )
            .await
            .is_err()
        );
    }
}
