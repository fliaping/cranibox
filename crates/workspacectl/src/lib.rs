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
use std::env;

const HELP: &str = "Cranibox control plane (Rust)
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
         desktop on|off|power [--json]   (graphical desktop; off frees its memory)
         tailscale login|up|serve|autoconnect|logout | proxy init|add|remove|list|check|render|domain|rollback
         web install|start|stop|restart|status|url|configure
";

pub async fn run(mut args: Vec<String>) -> Result<i32> {
    let ctx = Context::new()?;
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
        "desktop"
            if rest
                .first()
                .is_some_and(|value| ["on", "off", "power"].contains(&value.as_str())) =>
        {
            ensure!(
                rest.len() == 1,
                "Usage: workspacectl desktop on|off|power [--json]"
            );
            let action = if rest[0] == "power" {
                "status"
            } else {
                rest[0].as_str()
            };
            let mut command = vec!["cranibox-desktop", action];
            if json_output {
                command.push("--json");
            }
            ctx.execute(&command).await
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
