use crate::{common::*, inspect};
use anyhow::{Result, ensure};
use clap::{Parser, Subcommand};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;

#[derive(Parser)]
pub struct ResourceCli {
    #[command(subcommand)]
    pub command: Group,
}
#[derive(Subcommand)]
pub enum Group {
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
}
#[derive(Subcommand)]
pub enum SkillAction {
    Add { source: String },
    Update { name: Option<String> },
    Remove { name: String },
}
#[derive(Subcommand)]
pub enum McpAction {
    Add {
        name: String,
        #[arg(long, value_parser = ["stdio", "http"])]
        transport: String,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        command_line: Option<String>,
        #[arg(long, default_value = "all")]
        agents: String,
    },
    Remove {
        name: String,
        #[arg(long, default_value = "all")]
        agents: String,
        #[arg(long)]
        purge_global: bool,
    },
}
pub fn parse_agents(value: &str) -> Result<Vec<&str>> {
    let mut ids = Vec::new();
    for id in if value == "all" {
        inspect::agent_catalog()
            .iter()
            .map(|(id, _, _)| *id)
            .collect()
    } else {
        value.split(',').map(str::trim).collect::<Vec<_>>()
    } {
        ensure!(
            inspect::agent_catalog()
                .iter()
                .any(|(known, _, _)| *known == id),
            "agents must be all or a comma-separated subset of deepseek-harness,codex,claude-code,pi,hermes,openclaw"
        );
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ensure!(!ids.is_empty(), "at least one Agent is required");
    Ok(ids)
}
fn emit(label: &str, result: &Probe) {
    println!(
        "[{label}] {}",
        if result.code == 0 {
            "complete"
        } else {
            "failed"
        }
    );
    if !result.stdout.trim().is_empty() {
        println!("{}", result.stdout.trim());
    }
    if !result.stderr.trim().is_empty() {
        eprintln!("{}", result.stderr.trim());
    }
}
fn digest(name: &str) -> String {
    format!("{:x}", Sha256::digest(name.as_bytes()))
}
fn server_name(name: &str) -> String {
    let normalized = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    if !normalized.is_empty() && normalized.len() <= 32 {
        normalized
    } else {
        format!(
            "{}_{}",
            normalized.chars().take(23).collect::<String>(),
            &digest(name)[..8]
        )
    }
}
pub fn deepseek_patch(ctx: &Context, servers: &Map<String, Value>) -> Result<()> {
    atomic_write(
        &ctx.root
            .join(".local/share/agent-workspace/deepseek-mcp.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({"version":1,"servers":servers}))?
        ),
        0o600,
    )?;
    let path = ctx.root.join(".dsh/agent-workspace-mcp.cordis.yml");
    // Native Cranibox MCP and approval hooks share this overlay. Replace only
    // the legacy resource-manager rows; retain unrelated blocks byte for byte.
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let mut content = String::new();
    let mut block = String::new();
    let retain = |block: &str| -> Result<bool> {
        if block.trim() == "[]" {
            return Ok(false);
        }
        if block
            .lines()
            .all(|l| l.trim().is_empty() || l.trim_start().starts_with('#'))
        {
            return Ok(true);
        }
        let rows: Vec<Value> = serde_yaml::from_str(block)?;
        let inserts = rows
            .iter()
            .filter_map(|r| r["insert"].as_array())
            .flatten()
            .collect::<Vec<_>>();
        let managed = inserts
            .iter()
            .filter(|r| {
                r["id"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("agent-workspace-mcp-"))
            })
            .count();
        ensure!(
            managed == 0 || managed == inserts.len(),
            "Mixed user and managed DSH insertions; review the overlay first"
        );
        Ok(managed == 0)
    };
    for line in existing.split_inclusive('\n') {
        if line.starts_with("- ") && !block.is_empty() {
            if retain(&block)? {
                content.push_str(&block);
            }
            block.clear();
        }
        block.push_str(line);
    }
    if !block.trim().is_empty() && retain(&block)? {
        content.push_str(&block);
    }
    if !servers.is_empty() {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str("- insert:\n");
    }
    for (name, config) in servers {
        content += &format!(
            "    - id: {}\n      name: \"@deepseek-ai/dsh-mcp-client\"\n      config:\n        serverName: {}\n",
            json!(format!("agent-workspace-mcp-{}", &digest(name)[..12])),
            json!(server_name(name))
        );
        if config["transport"] == "streamable-http" {
            content += &format!(
                "        transport: \"streamable-http\"\n        url: {}\n",
                json!(inspect::text(&config["url"]))
            );
        } else {
            content += &format!(
                "        transport: \"stdio\"\n        command: {}\n        args: {}\n        cwd: {}\n",
                json!(inspect::text(&config["command"])),
                config["args"],
                json!(ctx.root.join("Workspace"))
            );
        }
    }
    if content.trim().is_empty() {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    } else {
        atomic_write(&path, content, 0o600)
    }
}
async fn deepseek(ctx: &Context, name: &str, config: Option<Value>) -> Result<()> {
    let state = ctx
        .root
        .join(".local/share/agent-workspace/deepseek-mcp.json");
    let loaded = json_file(state);
    let mut servers = loaded["servers"].as_object().cloned().unwrap_or_default();
    servers.retain(|name, value| valid_name(name) && value.is_object());
    if let Some(config) = config {
        servers.insert(name.into(), config);
    } else {
        servers.remove(name);
    }
    deepseek_patch(ctx, &servers)?;
    if ctx
        .probe(
            &[
                "systemctl",
                "--user",
                "show",
                "deepseek-harness.service",
                "--property",
                "ActiveState",
            ],
            10.,
        )
        .await
        .stdout
        .contains("ActiveState=active")
    {
        let mut command = ctx.command("systemctl");
        command
            .args(["--user", "restart", "deepseek-harness.service"])
            .kill_on_drop(true);
        let mut child = detach(&mut command, &ctx.logs().join("deepseek-mcp-restart.log"))?;
        let status = tokio::time::timeout(Duration::from_secs(60), child.wait()).await??;
        ensure!(status.success(), "DeepSeek Harness restart failed");
    }
    Ok(())
}
async fn agent_mcp(
    ctx: &Context,
    id: &str,
    name: &str,
    config: Option<(&str, &str, &[String])>,
) -> Probe {
    let binary = inspect::agent_catalog()
        .iter()
        .find(|(known, _, _)| *known == id)
        .map(|(_, _, binary)| command_path(binary))
        .unwrap_or_default();
    if binary.is_empty() {
        return Probe::failed("Agent CLI is not installed");
    }
    if id == "pi" {
        let env =
            cranibox_agents::environment(ctx.root.clone(), ctx.state().join("agent-adapters"));
        let result = match config {
            Some(("stdio", _, args)) if !args.is_empty() => {
                cranibox_agents::adapter(cranibox_agents::AgentId::Pi)
                    .unwrap()
                    .register_mcp(
                        &env,
                        &cranibox_agents::McpSpec {
                            name: name.into(),
                            command: args[0].clone(),
                            args: args[1..].to_vec(),
                            env: Default::default(),
                        },
                    )
                    .map(|_| ())
            }
            Some(_) => Err(cranibox_agents::AgentError::Io(
                "Pi MCP requires a stdio command; use an HTTP-to-stdio bridge".into(),
            )),
            None => cranibox_agents::unregister_mcp(cranibox_agents::AgentId::Pi, &env, name)
                .map(|_| ()),
        };
        return match result {
            Ok(()) => Probe {
                code: 0,
                stdout: "Pi MCP extension updated".into(),
                stderr: String::new(),
            },
            Err(e) => Probe::failed(e),
        };
    }
    if id == "deepseek-harness" {
        let config = config.map(|(transport, url, args)| {
            if transport == "http" {
                json!({"transport":"streamable-http","url":url,"enabled":true})
            } else {
                json!({"transport":"stdio","command":args[0],"args":args[1..],"enabled":true})
            }
        });
        return match deepseek(ctx, name, config).await {
            Ok(()) => Probe {
                code: 0,
                stdout: "DeepSeek Harness MCP patch updated".into(),
                stderr: String::new(),
            },
            Err(error) => Probe::failed(error),
        };
    }
    let mut args = vec![
        binary,
        "mcp".into(),
        if config.is_some() { "add" } else { "remove" }.into(),
    ];
    if id == "claude-code" {
        args.extend(["--scope".into(), "user".into()]);
    }
    if let Some((transport, url, command)) = config {
        if id == "claude-code" && transport == "http" {
            args.extend(["--transport".into(), "http".into()]);
        }
        args.push(name.into());
        if transport == "http" {
            if id != "claude-code" {
                args.push("--url".into());
            }
            args.push(url.into());
        } else if id == "openclaw" {
            args.extend(["--command".into(), command[0].clone()]);
            for arg in &command[1..] {
                args.extend(["--arg".into(), arg.clone()]);
            }
        } else if id == "hermes" {
            args.extend(["--command".into(), command[0].clone(), "--args".into()]);
            args.extend_from_slice(&command[1..]);
        } else {
            args.push("--".into());
            args.extend_from_slice(command);
        }
    } else {
        args.push(name.into());
    }
    ctx.probe(&args.iter().map(String::as_str).collect::<Vec<_>>(), 180.)
        .await
}
pub async fn add_mcp(
    ctx: &Context,
    name: &str,
    mut transport: String,
    mut url: String,
    mut command: Vec<String>,
    agents: &str,
) -> Result<i32> {
    validate_name(name)?;
    let agents = parse_agents(agents)?;
    if transport == "http" {
        let parsed = reqwest::Url::parse(&url)?;
        ensure!(
            ["http", "https"].contains(&parsed.scheme())
                && parsed.host_str().is_some()
                && !url.chars().any(char::is_whitespace),
            "HTTP MCP URLs must begin with http:// or https://"
        );
    } else {
        ensure!(
            !command.is_empty() && !command[0].starts_with('-'),
            "a valid STDIO command line is required"
        );
    }
    let mcpm = command_path("mcpm");
    if !mcpm.is_empty() {
        let mut args = vec!["new".to_owned(), name.to_owned(), "--type".to_owned()];
        if transport == "http" {
            args.extend(["remote".into(), "--url".into(), url.clone()]);
        } else {
            args.extend([
                "stdio".into(),
                "--command".into(),
                command[0].clone(),
                "--args".into(),
                shell_words::join(&command[1..]),
            ]);
        }
        args.push("--force".into());
        let mut process = ctx.command(&mcpm);
        process
            .args(args)
            .env("MCPM_NON_INTERACTIVE", "true")
            .env("MCPM_FORCE", "true");
        let result = capture(process, Duration::from_secs(900)).await;
        emit("mcpm", &result);
        if result.code != 0 {
            return Ok(result.code);
        }
        command = vec![mcpm, "run".into(), name.into()];
        url.clear();
        transport = "stdio".into();
    }
    let mut failures = false;
    for id in agents {
        let result = agent_mcp(ctx, id, name, Some((&transport, &url, &command))).await;
        failures |= result.code != 0;
        emit(id, &result);
    }
    Ok(i32::from(failures))
}
pub async fn run(ctx: &Context, group: Group) -> Result<i32> {
    match group {
        Group::Skill { action } => {
            let npx = command_path("npx");
            ensure!(
                !npx.is_empty(),
                "npx is required for global Skill management"
            );
            let mut args = vec![npx, "--yes".into(), "skills".into()];
            let label = match action {
                SkillAction::Add { source } => {
                    let source = source.trim();
                    ensure!(
                        !source.is_empty() && !source.starts_with('-') && !source.contains('\0'),
                        "a valid Skill package, repository URL, or local path is required"
                    );
                    args.extend([
                        "add".into(),
                        source.into(),
                        "--global".into(),
                        "--agent".into(),
                        "*".into(),
                        "--yes".into(),
                    ]);
                    "add"
                }
                SkillAction::Update { name } => {
                    args.push("update".into());
                    if let Some(name) = name {
                        validate_name(&name)?;
                        args.push(name);
                    }
                    args.extend(["--global".into(), "--yes".into()]);
                    "update"
                }
                SkillAction::Remove { name } => {
                    validate_name(&name)?;
                    args.extend(["remove".into(), name, "--global".into(), "--yes".into()]);
                    "remove"
                }
            };
            let result = ctx
                .probe(&args.iter().map(String::as_str).collect::<Vec<_>>(), 900.)
                .await;
            emit(&format!("skills:{label}"), &result);
            Ok(result.code)
        }
        Group::Mcp {
            action:
                McpAction::Add {
                    name,
                    transport,
                    url,
                    command_line,
                    agents,
                },
        } => {
            let command = shell_words::split(command_line.as_deref().unwrap_or_default())?;
            add_mcp(
                ctx,
                &name,
                transport,
                url.unwrap_or_default().trim().to_owned(),
                command,
                &agents,
            )
            .await
        }
        Group::Mcp {
            action:
                McpAction::Remove {
                    name,
                    agents,
                    purge_global,
                },
        } => {
            validate_name(&name)?;
            let mut failed = false;
            for id in parse_agents(&agents)? {
                let result = agent_mcp(ctx, id, &name, None).await;
                failed |= result.code != 0;
                emit(id, &result);
            }
            let mcpm = command_path("mcpm");
            if purge_global && !mcpm.is_empty() {
                let mut command = ctx.command(&mcpm);
                command
                    .args(["uninstall", &name, "--force"])
                    .env("MCPM_NON_INTERACTIVE", "true")
                    .env("MCPM_FORCE", "true");
                let result = capture(command, Duration::from_secs(900)).await;
                failed |= result.code != 0;
                emit("mcpm", &result);
            }
            Ok(i32::from(failed))
        }
    }
}
pub fn parse(group: &str, args: &[String]) -> Group {
    let mut argv = vec!["workspacectl".to_owned(), group.to_owned()];
    argv.extend_from_slice(args);
    ResourceCli::parse_from(argv).command
}
