use crate::server::{SessionRef, State, TextKind, Validation};
use anyhow::{Result, bail, ensure};
use regex::Regex;
use serde_json::{Value, json};
use std::sync::OnceLock;

fn agent_ids() -> Vec<&'static str> {
    cranibox_agents::registry()
        .iter()
        .map(|a| a.id().as_str())
        .collect()
}
const ACTIONS: &[&str] = &[
    "installAgent",
    "updateOpenClaw",
    "installMcpm",
    "installDesktop",
    "setupComputerUse",
    "desktopControl",
    "desktopEmergencyStop",
    "setupBrowser",
    "openManagedBrowser",
    "installTailscale",
    "serveTailscale",
    "configureDomain",
    "addSkill",
    "updateSkills",
    "updateSkill",
    "removeSkill",
    "addMcp",
    "removeMcp",
    "serviceAction",
    "showLogs",
    "installProxy",
    "checkProxy",
    "addRoute",
    "removeRoute",
    "completeOnboarding",
    "updateSource",
    "switchLanguage",
];
pub fn valid_name(text: &str) -> bool {
    let mut chars = text.bytes();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || b"_@".contains(&c))
        && chars.all(|c| c.is_ascii_alphanumeric() || b"_.@-".contains(&c))
}
pub fn valid_domain(text: &str) -> bool {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(||Regex::new(r"^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+$").unwrap()).is_match(text.to_lowercase().trim_end_matches('.'))
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}
pub fn validate(body: &Value) -> Result<()> {
    let action = text(body, "type");
    ensure!(ACTIONS.contains(&action), "Unsupported action");
    for field in ["id", "name"] {
        if let Some(value) = body.get(field) {
            ensure!(value.as_str().is_some_and(valid_name), "Invalid {field}");
        }
    }
    match action {
        "installAgent" => ensure!(agent_ids().contains(&text(body, "id")), "Unknown Agent"),
        "serviceAction" => {
            ensure!(valid_name(text(body, "id")), "Invalid service");
            ensure!(
                ["start", "stop", "restart"].contains(&text(body, "action")),
                "Unsupported service action"
            );
            ensure!(
                ["s6", "systemd", "user", ""].contains(&text(body, "kind")),
                "Unsupported service kind"
            );
        }
        "desktopControl" => ensure!(
            ["start", "stop", "restart", "resume"].contains(&text(body, "action")),
            "Unsupported desktop action"
        ),
        "showLogs" | "updateSkill" | "removeSkill" | "removeRoute" | "removeMcp" => ensure!(
            valid_name(text(
                body,
                if action == "removeRoute" {
                    "name"
                } else {
                    "id"
                }
            )),
            "Invalid name"
        ),
        _ => {}
    }
    Ok(())
}
async fn input(
    state: &State,
    session: &SessionRef,
    title: &str,
    prompt: &str,
    kind: TextKind,
    value: &str,
) -> Result<Option<String>> {
    Ok(state
        .ask(
            session,
            json!({"kind":"input","title":title,"prompt":prompt,"value":value}),
            Validation::Text { kind },
        )
        .await?
        .and_then(|v| v.as_str().map(str::to_owned)))
}
async fn confirm(state: &State, session: &SessionRef, title: &str, choice: &str) -> Result<bool> {
    Ok(state
        .ask(
            session,
            json!({"kind":"confirm","title":title,"choices":[choice]}),
            Validation::Confirm {
                choices: vec![choice.into()],
            },
        )
        .await?
        .is_some())
}
async fn pick(
    state: &State,
    session: &SessionRef,
    title: &str,
    items: Vec<Value>,
    many: bool,
) -> Result<Option<Value>> {
    state
        .ask(
            session,
            json!({"kind":"pick","title":title,"items":items,"canPickMany":many}),
            Validation::Pick { items, many },
        )
        .await
}
async fn operation(
    state: &State,
    session: &SessionRef,
    title: &str,
    args: &[&str],
    timeout: u64,
) -> Result<bool> {
    state
        .operation(
            session,
            title,
            args.iter().map(|v| v.to_string()).collect(),
            timeout,
        )
        .await
}
pub async fn execute(state: &State, session: &SessionRef, body: Value) -> Result<()> {
    let snapshot = state.snapshot().await.unwrap_or_default();
    let zh = snapshot["locale"]["current"] == "zh-cn" || state.locale() == "zh-cn";
    let action = text(&body, "type");
    let id = text(&body, "id");
    match action {
        "installAgent" => {
            operation(
                state,
                session,
                &format!("Install {id}"),
                &["install", "agent", id],
                900,
            )
            .await?;
        }
        "updateOpenClaw" => {
            let choice = if zh {
                "更新 Gateway"
            } else {
                "Update Gateway"
            };
            let title = if zh {
                "现在更新 OpenClaw Gateway？服务会短暂重启；验证失败会恢复原服务。"
            } else {
                "Update OpenClaw Gateway now? The service will restart briefly; verification failure restores the previous service."
            };
            if confirm(state, session, title, choice).await? {
                operation(
                    state,
                    session,
                    choice,
                    &["agent", "update", "openclaw"],
                    2400,
                )
                .await?;
            }
        }
        "installMcpm" => {
            operation(state, session, "Install MCPM", &["install", "mcpm"], 900).await?;
        }
        "installDesktop" => {
            operation(
                state,
                session,
                "Install Selkies Desktop integration",
                &["install", "desktop"],
                900,
            )
            .await?;
        }
        "setupComputerUse" => {
            operation(
                state,
                session,
                "Set up Computer Use",
                &["desktop", "setup"],
                180,
            )
            .await?;
        }
        "desktopControl" => {
            let control = text(&body, "action");
            operation(
                state,
                session,
                &format!("Computer Use: {control}"),
                &["desktop", control],
                60,
            )
            .await?;
        }
        "desktopEmergencyStop" => {
            operation(
                state,
                session,
                "Computer Use: input stopped",
                &["desktop", "emergency-stop"],
                30,
            )
            .await?;
        }
        "setupBrowser" | "openManagedBrowser" => {
            let browser = if action == "setupBrowser" {
                "setup"
            } else {
                "open"
            };
            if operation(
                state,
                session,
                "Managed browser",
                &["browser", browser],
                if browser == "setup" { 120 } else { 30 },
            )
            .await?
            {
                session.lock().await.emit(json!({"type":"notice","level":"info","message":if zh{"需要时请在 Chromium 中确认控制权限。"}else{"Approve browser control in Chromium when requested."}}));
                session
                    .lock()
                    .await
                    .emit(json!({"type":"openUrl","url":state.config.desktop_url}));
            }
        }
        "installTailscale" => {
            operation(
                state,
                session,
                "Install Tailscale",
                &["install", "tailscale"],
                900,
            )
            .await?;
        }
        "serveTailscale" => {
            operation(
                state,
                session,
                "Share workspace inside the tailnet",
                &["tailscale", "serve"],
                60,
            )
            .await?;
        }
        "serviceAction" => {
            let verb = text(&body, "action");
            let scope = if text(&body, "kind") == "s6" {
                "s6"
            } else {
                "service"
            };
            operation(
                state,
                session,
                &format!("{verb} {id}"),
                &[scope, verb, id],
                60,
            )
            .await?;
        }
        "showLogs" => {
            operation(
                state,
                session,
                &format!("Logs: {id}"),
                &["logs", id, "200"],
                10,
            )
            .await?;
        }
        "checkProxy" => {
            operation(state, session, "Check routing", &["proxy", "check"], 60).await?;
        }
        "updateSource" => {
            operation(state, session, "Update Agent Workspace", &["update"], 900).await?;
        }
        "updateSkills" => {
            operation(
                state,
                session,
                "Update global Skills",
                &["skill", "update"],
                900,
            )
            .await?;
        }
        "updateSkill" => {
            operation(
                state,
                session,
                &format!("Update Skill {id}"),
                &["skill", "update", id],
                900,
            )
            .await?;
        }
        "removeSkill" => {
            if confirm(
                state,
                session,
                &format!("Remove global Skill {id} from supported Agents?"),
                "Remove",
            )
            .await?
            {
                operation(
                    state,
                    session,
                    &format!("Remove Skill {id}"),
                    &["skill", "remove", id],
                    900,
                )
                .await?;
            }
        }
        "addSkill" => {
            if let Some(source) = input(
                state,
                session,
                "Agent Workspace: Install global Skill",
                "Enter owner/repository, a Git URL, or a local Skill package path.",
                TextKind::Source,
                "",
            )
            .await?
            {
                operation(
                    state,
                    session,
                    "Install global Skill",
                    &["skill", "add", &source],
                    900,
                )
                .await?;
            }
        }
        "addRoute" => {
            let Some(name) = input(
                state,
                session,
                "Agent Workspace: Route name",
                "Short route label, for example api or dashboard.",
                TextKind::Name,
                "",
            )
            .await?
            else {
                return Ok(());
            };
            let Some(target) = input(
                state,
                session,
                "Agent Workspace: Route target",
                "Upstream host:port reachable from this container.",
                TextKind::Target,
                "",
            )
            .await?
            else {
                return Ok(());
            };
            operation(
                state,
                session,
                &format!("Add route {name}"),
                &["proxy", "add", &name, &target],
                60,
            )
            .await?;
        }
        "removeRoute" => {
            let name = text(&body, "name");
            let host = text(&body, "host");
            if confirm(
                state,
                session,
                &format!(
                    "Remove route {}?",
                    if host.is_empty() { name } else { host }
                ),
                "Remove",
            )
            .await?
            {
                operation(
                    state,
                    session,
                    &format!("Remove route {name}"),
                    &["proxy", "remove", name],
                    900,
                )
                .await?;
            }
        }
        "installProxy" | "configureDomain" => {
            let current = snapshot["network"]["root_domain"]
                .as_str()
                .unwrap_or_default();
            let Some(domain) = input(
                state,
                session,
                "Agent Workspace: Custom domain",
                "Enter a domain pointing to this workspace.",
                TextKind::Domain,
                current,
            )
            .await?
            else {
                return Ok(());
            };
            let domain = domain.to_lowercase().trim_end_matches('.').to_owned();
            if !current.is_empty()
                && current != domain
                && !confirm(
                    state,
                    session,
                    &format!("Change domain from {current} to {domain}?"),
                    "Configure domain",
                )
                .await?
            {
                return Ok(());
            }
            operation(
                state,
                session,
                &format!("Configure domain {domain}"),
                &["network", "domain", &domain],
                900,
            )
            .await?;
        }
        "completeOnboarding" => {
            let path = state
                .root
                .join(".local/state/agent-workspace/onboarding-v1.complete");
            std::fs::create_dir_all(path.parent().unwrap())?;
            #[cfg(unix)]
            {
                use std::io::Write;
                use std::os::unix::fs::OpenOptionsExt;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .truncate(true)
                    .write(true)
                    .mode(0o600)
                    .open(path)?;
                file.write_all(b"complete\n")?;
            }
            #[cfg(not(unix))]
            std::fs::write(path, "complete\n")?;
            session.lock().await.force_welcome = false;
        }
        "switchLanguage" => {
            let target = if zh { "en" } else { "zh-cn" };
            let choice = if zh {
                "切换并重启"
            } else {
                "Switch and restart"
            };
            if confirm(
                state,
                session,
                if zh {
                    "切换到英文界面？"
                } else {
                    "Switch to Chinese?"
                },
                choice,
            )
            .await?
            {
                operation(
                    state,
                    session,
                    "Switch language",
                    &["locale", target, "--restart"],
                    10,
                )
                .await?;
            }
        }
        "addMcp" => {
            let Some(name) = input(
                state,
                session,
                "Agent Workspace: MCP server name",
                "Use one stable name across all selected Agents.",
                TextKind::Name,
                "",
            )
            .await?
            else {
                return Ok(());
            };
            let Some(transport)=pick(state,session,"Agent Workspace: MCP transport",vec![json!({"label":"Streamable HTTP","id":"http","description":"A remote HTTP(S) MCP endpoint"}),json!({"label":"STDIO command","id":"stdio","description":"A local executable and arguments"})],false).await?else{return Ok(())};
            let transport = transport["id"].as_str().unwrap();
            let Some(value) = input(
                state,
                session,
                if transport == "http" {
                    "Agent Workspace: MCP URL"
                } else {
                    "Agent Workspace: MCP command"
                },
                "Enter the MCP endpoint or command.",
                if transport == "http" {
                    TextKind::Url
                } else {
                    TextKind::Source
                },
                "",
            )
            .await?
            else {
                return Ok(());
            };
            let items:Vec<_>=snapshot["mcp"]["agents"].as_array().into_iter().flatten().filter(|a|a["installed"]==true && a["id"].as_str().is_some_and(|id|agent_ids().contains(&id))).map(|a|json!({"id":a["id"],"label":a["label"],"picked":true,"description":format!("{} configured",a["configured"])})).collect();
            ensure!(
                !items.is_empty(),
                "Install at least one Agent before adding an MCP server"
            );
            let Some(selected) = pick(
                state,
                session,
                "Agent Workspace: Make MCP available to",
                items,
                true,
            )
            .await?
            else {
                return Ok(());
            };
            let agents = selected
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|v| v["id"].as_str())
                .collect::<Vec<_>>()
                .join(",");
            if agents.is_empty() {
                return Ok(());
            }
            operation(
                state,
                session,
                &format!("Add MCP {name}"),
                &[
                    "mcp",
                    "add",
                    &name,
                    "--transport",
                    transport,
                    "--agents",
                    &agents,
                    if transport == "http" {
                        "--url"
                    } else {
                        "--command-line"
                    },
                    &value,
                ],
                900,
            )
            .await?;
        }
        "removeMcp" => {
            let agents = text(&body, "agents")
                .split(',')
                .filter(|id| agent_ids().contains(id))
                .collect::<Vec<_>>()
                .join(",");
            if agents.is_empty() {
                return Ok(());
            }
            if confirm(
                state,
                session,
                &format!("Remove MCP {id} from selected Agents?"),
                "Remove",
            )
            .await?
            {
                let mut args = vec!["mcp", "remove", id, "--agents", &agents];
                if body["managed"] == true {
                    args.push("--purge-global");
                }
                operation(state, session, &format!("Remove MCP {id}"), &args, 900).await?;
            }
        }
        _ => bail!("Unsupported action"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dispatch_rejects_arbitrary_commands() {
        assert!(validate(&json!({"type":"terminal","command":"touch /tmp/no"})).is_err());
        assert!(validate(&json!({"type":"serviceAction","id":"ok","action":"enable"})).is_err());
        assert!(validate(&json!({"type":"installAgent","id":"other"})).is_err());
    }
}
