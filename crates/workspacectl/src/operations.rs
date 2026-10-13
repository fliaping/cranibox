use crate::{common::*, inspect, proxy, resources, settings};
use anyhow::{Context as _, Result, bail, ensure};
use base64::Engine;
use serde_json::json;
use std::{env, path::PathBuf, time::Duration};

fn refs(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}
async fn manager(ctx: &Context, args: &[&str]) -> Result<i32> {
    ctx.execute(&[&["agent-workspace-manager"][..], args].concat())
        .await
}
async fn browser(ctx: &Context, action: &str) -> Result<i32> {
    if action == "setup" {
        let status = inspect::browser(ctx).await;
        ensure!(
            status["installed"] == true && status["supported"] == true,
            "Chrome/Chromium 136+ is required for managed browser debugging"
        );
        let agents = inspect::agents(ctx).await;
        let ids = agents
            .as_array()
            .unwrap()
            .iter()
            .filter(|agent| agent["installed"] == true && agent["id"] != "openclaw")
            .map(|agent| inspect::text(&agent["id"]))
            .collect::<Vec<_>>()
            .join(",");
        ensure!(
            !ids.is_empty(),
            "Install an Agent before configuring browser MCP"
        );
        let port = inspect::debug_port();
        let code = resources::add_mcp(
            ctx,
            "chrome-devtools",
            "stdio".into(),
            String::new(),
            vec![
                "npx".into(),
                "--yes".into(),
                "chrome-devtools-mcp@latest".into(),
                format!("--browser-url=http://127.0.0.1:{port}"),
                "--no-usage-statistics".into(),
            ],
            &ids,
        )
        .await?;
        if code != 0 {
            return Ok(code);
        }
    } else {
        ensure!(
            action == "open",
            "Usage: workspacectl browser status|setup|open"
        );
    }
    let path = command_path("agent-workspace-browser");
    let path = if path.is_empty() {
        ctx.root
            .join("bin/agent-workspace-browser")
            .to_string_lossy()
            .into_owned()
    } else {
        path
    };
    ensure!(
        executable(std::path::Path::new(&path)),
        "Managed browser launcher is missing"
    );
    let display = env::var("AGENT_WORKSPACE_BROWSER_DISPLAY")
        .ok()
        .or_else(|| {
            env::var("DISPLAY").ok().filter(|display| {
                std::path::Path::new(&format!(
                    "/tmp/.X11-unix/X{}",
                    display
                        .trim_start_matches(':')
                        .split('.')
                        .next()
                        .unwrap_or_default()
                ))
                .exists()
            })
        })
        .or_else(|| {
            directories("/tmp/.X11-unix")
                .first()
                .and_then(|path| path.file_name())
                .map(|name| format!(":{}", name.to_string_lossy().trim_start_matches('X')))
        })
        .unwrap_or_else(|| ":1".into());
    let log = ctx.root.join(".local/log/agent-workspace-browser.log");
    let mut command = ctx.command(&path);
    command.arg("about:blank").env("DISPLAY", display);
    let mut child = detach(&mut command, &log)?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    if let Some(status) = child.try_wait()? {
        ensure!(
            status.success(),
            "Browser launcher failed; see {}",
            log.display()
        );
    }
    println!("Managed Agent browser opened");
    Ok(0)
}
async fn desktop(ctx: &Context, args: &[String]) -> Result<i32> {
    let action = args.first().map(String::as_str).unwrap_or("status");
    if action == "setup" {
        return manager(ctx, &["install", "computer-use"]).await;
    }
    if ["start", "stop", "restart"].contains(&action) {
        ensure!(args.len() == 1, "Unexpected desktop arguments");
        return ctx
            .execute(&[
                "systemctl",
                "--user",
                action,
                "agent-desktop-bridge.service",
            ])
            .await;
    }
    ensure!(
        ["emergency-stop", "resume", "screenshot"].contains(&action),
        "Usage: workspacectl desktop setup|start|stop|restart|emergency-stop|resume|screenshot"
    );
    let output = if action == "screenshot" && args.len() == 3 && args[1] == "--output" {
        PathBuf::from(&args[2])
    } else {
        ensure!(args.len() == 1, "Unexpected desktop arguments");
        ctx.root.join("Downloads/agent-desktop.png")
    };
    let payload = if action == "emergency-stop" {
        json!({"action":"emergency_stop","reason":"stopped from workspacectl"})
    } else {
        json!({"action":action})
    };
    let mut value: serde_json::Value = ctx
        .http
        .post(format!("{}/v1/action", inspect::bridge_url(ctx)))
        .header("X-Agent-Session", "workspacectl")
        .json(&payload)
        .timeout(Duration::from_secs(40))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if action == "screenshot" {
        let bytes = base64::engine::general_purpose::STANDARD.decode(
            value["data"]
                .as_str()
                .context("screenshot data is missing")?,
        )?;
        atomic_write(&output, bytes, 0o600)?;
        value = json!({"result":"ok","output":output});
    }
    print_json(&value);
    Ok(0)
}
pub async fn run(ctx: &Context, name: &str, args: &[String]) -> Result<i32> {
    let action = args.first().map(String::as_str).unwrap_or("status");
    match name {
        "browser" => {
            ensure!(
                args.len() == 1,
                "Usage: workspacectl browser status|setup|open"
            );
            browser(ctx, action).await
        }
        "desktop" => desktop(ctx, args).await,
        "service" | "s6" => {
            ensure!(
                args.len() == 2,
                "Usage: workspacectl {name} <action> <name>"
            );
            validate_name(&args[1])?;
            ensure!(
                ["status", "start", "stop", "restart"].contains(&action)
                    || (name == "service" && ["enable", "disable"].contains(&action)),
                "Unsupported {name} action: {action}"
            );
            if name == "service" {
                return ctx
                    .execute(&["systemctl", "--user", action, &args[1]])
                    .await;
            }
            let path = format!("/run/service/{}", args[1]);
            ensure!(
                std::path::Path::new(&path).is_dir(),
                "Unknown s6 service: {}",
                args[1]
            );
            let mut command = if ctx.sudo().await {
                vec!["sudo", "-n"]
            } else {
                Vec::new()
            };
            command.extend(if action == "status" {
                vec!["s6-svstat", &path]
            } else {
                vec![
                    "s6-svc",
                    match action {
                        "start" => "-u",
                        "stop" => "-d",
                        _ => "-r",
                    },
                    &path,
                ]
            });
            ctx.execute(&command).await
        }
        "logs" => {
            ensure!(
                !args.is_empty() && args.len() <= 2,
                "Usage: workspacectl logs <name> [lines]"
            );
            validate_name(&args[0])?;
            let lines = args
                .get(1)
                .map(|value| value.parse::<usize>())
                .transpose()?
                .unwrap_or(100);
            let path = ctx
                .logs()
                .join(format!("{}.log", args[0].trim_end_matches(".service")));
            ensure!(
                path.is_file(),
                "No persistent log found at {}",
                path.display()
            );
            // tail bounds memory for large service logs and preserves the existing CLI output.
            ctx.execute(&["tail", "-n", &lines.to_string(), &path.to_string_lossy()])
                .await
        }
        "install" => {
            if action == "agent" {
                ensure!(args.len() == 2, "Missing Agent name");
                validate_name(&args[1])?;
                manager(ctx, &["install", "agents", &args[1]]).await
            } else {
                ensure!(args.len() == 1, "Missing capability");
                validate_name(action)?;
                manager(ctx, &["install", action]).await
            }
        }
        "update" => {
            ensure!(args.is_empty(), "Unexpected update arguments");
            manager(ctx, &["update"]).await
        }
        "runtime" => {
            if action == "install" {
                let installer = ctx.implementation_script("install-workspace-runtime.sh")?;
                let mut command = ctx.command("bash");
                command.arg(installer).args(&args[1..]);
                return Ok(command.status().await?.code().unwrap_or(1));
            }
            ensure!(
                ["status", "start", "stop", "restart"].contains(&action) && args.len() <= 1,
                "Usage: workspacectl runtime install|start|stop|restart|status"
            );
            ctx.execute(&[
                "systemctl",
                "--user",
                action,
                if ctx
                    .root
                    .join(".config/agent-workspace/unified-server")
                    .exists()
                {
                    "agent-workspace-web.service"
                } else {
                    "agent-workspace-runtime.service"
                },
            ])
            .await
        }
        "tailscale" => {
            if action == "autoconnect" {
                ensure!(args.len() == 1, "Unexpected tailscale arguments");
                return crate::tailscale::autoconnect(ctx).await;
            }
            ensure!(
                ["login", "up", "serve", "logout"].contains(&action),
                "Usage: workspacectl tailscale status|login|serve|autoconnect|logout"
            );
            let mut command = if ctx.sudo().await {
                vec!["sudo".into(), "-n".into(), "tailscale".into()]
            } else {
                vec!["tailscale".into()]
            };
            if ["login", "up"].contains(&action) {
                command.extend([
                    "up".into(),
                    format!(
                        "--hostname={}",
                        env::var("TAILSCALE_HOSTNAME").unwrap_or_else(|_| "agent-workspace".into())
                    ),
                    "--accept-dns=false".into(),
                ]);
                command.extend_from_slice(&args[1..]);
            } else {
                ensure!(args.len() == 1, "Unexpected tailscale arguments");
                command.extend(if action == "serve" {
                    vec![
                        "serve".into(),
                        "--bg".into(),
                        crate::tailscale::serve_target(ctx),
                    ]
                } else {
                    vec!["logout".into()]
                });
            }
            ctx.execute(&refs(&command)).await
        }
        "network" => {
            ensure!(
                args.len() == 2 && action == "domain",
                "Usage: workspacectl network domain <domain>"
            );
            let domain = proxy::normalize_host(&args[1])?;
            ensure!(
                proxy::valid_host(&domain),
                "domain must be a bare hostname such as workspace.example.com"
            );
            let root = ctx.root.join("proxyctl");
            if root.join("bin/caddy").is_file() && root.join("env").is_file() {
                let code = proxy::run(
                    ctx,
                    proxy::Args {
                        action: proxy::Action::Domain {
                            domain: domain.clone(),
                            dry_run: false,
                        },
                    },
                )
                .await?;
                if code != 0 {
                    return Ok(code);
                }
            } else {
                let mut command = ctx.command("agent-workspace-manager");
                command
                    .args(["install", "proxyctl"])
                    .env("PROXY_ROOT_DOMAIN", &domain)
                    .env("PROXY_RECONFIGURE", "true");
                if env::var_os("CODE_SERVER_PROXY_DOMAIN").is_none() {
                    command.env("CODE_SERVER_PROXY_DOMAIN", format!("{{{{port}}}}.{domain}"));
                }
                let status = command.status().await?;
                if !status.success() {
                    return Ok(status.code().unwrap_or(1));
                }
            }
            let values = env_file(&root.join("env"));
            let proxy_domain = values
                .get("CODE_SERVER_PROXY_DOMAIN")
                .cloned()
                .unwrap_or_else(|| format!("{{{{port}}}}.{domain}"));
            settings::update_yaml(
                &ctx.root.join(".config/code-server/config.yaml"),
                "proxy-domain",
                &proxy_domain,
            )?;
            settings::schedule_restart(ctx)?;
            println!("custom domain configured: {domain}\ncode-server restart scheduled");
            Ok(0)
        }
        _ => bail!("Unsupported command: {name}"),
    }
}
