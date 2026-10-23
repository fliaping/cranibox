mod common;
mod inspect;
mod openclaw;
mod operations;
mod proxy;
mod resources;
mod scaling;
mod settings;
mod tailscale;

use anyhow::{Result, bail, ensure};
use clap::Parser;
use common::*;
use std::{env, path::PathBuf};

const HELP: &str = "Agent Workspace control plane (Rust)
Usage: workspacectl <command> [arguments]

Inspect: info | status | agents | skills | mcp | locale | services | ports | routes | doctor
         browser status | desktop status | tailscale status | timezone status
         Add --json for the stable Control Center JSON contract.
Operate: service <status|start|stop|restart|enable|disable> <name>
         s6 <status|start|stop|restart> <name> | logs <name> [lines]
         install [agent] <name> | update | agent update openclaw
         skill add|update|remove ... | mcp add|remove ...
         locale en|zh-cn [--restart] | timezone set <IANA-zone> [--config-only]
         timezone rollback | browser setup|open | desktop setup|start|stop|restart
         desktop emergency-stop|resume|screenshot [--output <path>]
         desktop scaling status|repair|restore | network domain <domain>
         tailscale login|up|serve|autoconnect|logout | proxy init|add|remove|list|check|render|domain|rollback
         web install|start|stop|restart|status|url|configure
Devices: nodes | node exec|job|cancel|mcp [arguments]   (paired and same-identity devices)
         messages held | release <message-id> | reject <message-id>
         messages inbound <node-id> accept|hold|refuse
";

fn node_binary(ctx: &Context) -> PathBuf {
    env::var_os("CRANIBOX_NODE_BIN")
        .map(PathBuf::from)
        .filter(|path| executable(path))
        .or_else(|| {
            let path = ctx.root.join("bin/cranibox");
            executable(&path).then_some(path)
        })
        .unwrap_or_else(|| PathBuf::from("cranibox"))
}
/// Owner approval queue for agent-to-agent messages held by inbound policy or the loop
/// guard. Uses the same peer store as the Control Center (`/api/peers`).
fn messages_args(ctx: &Context, args: &[String]) -> Result<Vec<String>> {
    const USAGE: &str = "Usage: workspacectl messages held | release <message-id> | reject <message-id> | inbound <node-id> accept|hold|refuse";
    let uuid = |value: &str| {
        value.len() == 36
            && value.char_indices().all(|(i, c)| {
                if [8, 13, 18, 23].contains(&i) {
                    c == '-'
                } else {
                    c.is_ascii_hexdigit()
                }
            })
    };
    let config = ctx.root.join(".config/cranibox/settings.json");
    let mut command = vec![
        "--config".to_owned(),
        config.to_string_lossy().into_owned(),
        "peer".to_owned(),
    ];
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["held"] | ["list"] | [] => command.extend(["messages".into(), "held".into()]),
        [action @ ("release" | "reject"), id] => {
            ensure!(uuid(id), "invalid message ID; {USAGE}");
            command.extend([
                "messages".into(),
                (*action).to_owned(),
                "--message-id".into(),
                (*id).to_owned(),
            ]);
        }
        ["inbound", node, policy @ ("accept" | "hold" | "refuse")] => {
            ensure!(
                node.starts_with("node-") && node.len() <= 64,
                "invalid node ID; {USAGE}"
            );
            command.extend([
                "inbound".into(),
                "--node".into(),
                (*node).to_owned(),
                "--policy".into(),
                (*policy).to_owned(),
            ]);
        }
        _ => bail!("{USAGE}"),
    }
    Ok(command)
}
/// Agent device commands run in the device node, over its own peers.
async fn node(ctx: &Context, args: &[String]) -> Result<i32> {
    let mut command = ctx.command(&node_binary(ctx).to_string_lossy());
    command.args(args);
    Ok(command.status().await?.code().unwrap_or(1))
}
async fn messages(ctx: &Context, args: &[String]) -> Result<i32> {
    let arguments = messages_args(ctx, args)?;
    let mut command = ctx.command(&node_binary(ctx).to_string_lossy());
    command.args(arguments);
    Ok(command.status().await?.code().unwrap_or(1))
}

pub async fn run(mut args: Vec<String>) -> Result<i32> {
    let ctx = Context::new()?;
    if args.first().is_some_and(|value| value == "messages") {
        return messages(&ctx, &args[1..]).await;
    }
    if args.first().is_some_and(|value| value == "node") {
        ensure!(
            args.get(1)
                .is_some_and(|value| ["exec", "job", "cancel", "mcp"].contains(&value.as_str())),
            "Usage: workspacectl node exec|job|cancel|mcp [arguments]"
        );
        return node(&ctx, &args[1..]).await;
    }
    if args.first().is_some_and(|value| value == "nodes") {
        let mut after_separator = false;
        args.retain(|value| {
            if value == "--" {
                after_separator = true;
            }
            after_separator || value != "--json"
        });
        return node(&ctx, &args).await;
    }
    let json_output = args.iter().any(|value| value == "--json");
    args.retain(|value| value != "--json");
    let name =
        args.first()
            .map(String::as_str)
            .unwrap_or(if json_output { "status" } else { "help" });
    let rest = args.get(1..).unwrap_or_default();
    if ["help", "-h", "--help"].contains(&name) {
        println!("{HELP}");
        return Ok(0);
    }
    if ["--version", "-V"].contains(&name) {
        println!("workspacectl {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    if name == "__restart-code-server" {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        return ctx
            .execute(&["systemctl", "--user", "restart", "code-server.service"])
            .await;
    }
    let inspect = [
        "info", "status", "agents", "services", "ports", "routes", "doctor",
    ]
    .contains(&name)
        || (["skills", "mcp"].contains(&name) && (rest.is_empty() || rest == ["list"]))
        || (["browser", "desktop", "tailscale"].contains(&name)
            && (rest.is_empty() || rest == ["status"]));
    if inspect {
        ensure!(
            rest.is_empty() || rest == ["list"] || rest == ["status"],
            "Unexpected inspect arguments"
        );
        if name == "routes" && !json_output {
            return proxy::run(&ctx, proxy::Args::parse_from(["proxy", "list"])).await;
        }
        let value = inspect::payload(&ctx, name).await;
        if !json_output && inspect::print_text(name, &value) {
            return Ok(0);
        }
        if json_output || ["browser", "desktop", "tailscale"].contains(&name) {
            print_json(&value);
        } else {
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        return Ok(0);
    }
    let parser_args = |label: &str| {
        let mut values = vec![format!("workspacectl {label}")];
        values.extend_from_slice(rest);
        values
    };
    match name {
        "locale" => settings::locale(
            &ctx,
            settings::LocaleArgs::parse_from(parser_args(name)),
            json_output,
        ),
        "timezone" => {
            settings::timezone(
                &ctx,
                settings::TimezoneArgs::parse_from(parser_args(name)),
                json_output,
            )
            .await
        }
        "web" => {
            ensure!(
                !json_output,
                "--json is only available for inspect commands"
            );
            settings::web(&ctx, settings::WebArgs::parse_from(parser_args(name))).await
        }
        "skills" | "skill" | "mcp" => {
            ensure!(
                !json_output,
                "--json is only available for inspect commands"
            );
            resources::run(
                &ctx,
                resources::parse(if name == "skills" { "skill" } else { name }, rest),
            )
            .await
        }
        "proxy" => {
            ensure!(
                !json_output,
                "--json is only available for inspect commands"
            );
            proxy::run(&ctx, proxy::Args::parse_from(parser_args(name))).await
        }
        "desktop" if rest.first().is_some_and(|value| value == "scaling") => {
            ensure!(
                rest.len() <= 2,
                "Usage: workspacectl desktop scaling status|repair|restore"
            );
            scaling::run(&ctx, rest.get(1).map(String::as_str).unwrap_or("status")).await
        }
        "agent" if rest == ["update", "openclaw"] => {
            ensure!(
                !json_output,
                "--json is only available for inspect commands"
            );
            openclaw::update(&ctx).await
        }
        "browser" | "desktop" | "network" | "tailscale" | "service" | "s6" | "logs" | "install"
        | "update" => {
            ensure!(
                !json_output,
                "--json is only available for inspect commands"
            );
            operations::run(&ctx, name, rest).await
        }
        _ => bail!("Unknown command: {name}\n{HELP}"),
    }
}
