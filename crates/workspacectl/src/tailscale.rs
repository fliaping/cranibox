use crate::common::*;
use anyhow::{Result, ensure};
use serde_json::Value;
use std::{env, time::Duration};

pub fn serve_target(ctx: &Context) -> String {
    let path = env::var_os("CODE_SERVER_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| ctx.root.join(".config/code-server/config.yaml"));
    let config: Value = serde_yaml::from_str(&read(path)).unwrap_or(Value::Null);
    let port = config["bind-addr"]
        .as_str()
        .and_then(|bind| bind.rsplit(':').next())
        .and_then(|port| port.parse::<u16>().ok())
        .unwrap_or(8443);
    let tls = match &config["cert"] {
        Value::Null | Value::Bool(false) => false,
        Value::String(cert) => !cert.is_empty() && cert != "false",
        _ => true,
    };
    format!(
        "{}://127.0.0.1:{port}",
        if tls { "https+insecure" } else { "http" }
    )
}

pub async fn autoconnect(ctx: &Context) -> Result<i32> {
    let binary = command_path("tailscale");
    ensure!(
        !binary.is_empty(),
        "Tailscale is not installed; run: workspacectl install tailscale"
    );
    let wait = env::var("TAILSCALE_WAIT_SECONDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(30);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(wait);
    let state = loop {
        let result = ctx.probe(&[&binary, "status", "--json"], 5.).await;
        let status: Value = serde_json::from_str(&result.stdout).unwrap_or(Value::Null);
        let state = status["BackendState"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if !state.is_empty() || tokio::time::Instant::now() >= deadline {
            break state;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    };
    ensure!(
        !state.is_empty(),
        "[tailscale] daemon is not answering; is tailscaled-workspace running?"
    );
    let hostname = env::var("TAILSCALE_HOSTNAME").unwrap_or_else(|_| "agent-workspace".into());
    if state == "Running" {
        println!("[tailscale] already logged in; skipping auth key");
    } else {
        let key = env::var("TAILSCALE_AUTHKEY_FILE").unwrap_or_default();
        ensure!(
            !key.is_empty() && std::fs::File::open(&key).is_ok(),
            "[tailscale] no readable auth key file; set TAILSCALE_AUTHKEY or TAILSCALE_AUTHKEY_FILE"
        );
        let mut command = vec![
            binary.clone(),
            "up".into(),
            format!("--auth-key=file:{key}"),
            format!("--hostname={hostname}"),
            "--accept-dns=false".into(),
        ];
        if let Ok(tags) = env::var("TAILSCALE_ADVERTISE_TAGS")
            && !tags.is_empty()
        {
            command.push(format!("--advertise-tags={tags}"));
        }
        if let Ok(extra) = env::var("TAILSCALE_EXTRA_ARGS") {
            command.extend(extra.split_whitespace().map(str::to_owned));
        }
        let code = ctx
            .execute(&command.iter().map(String::as_str).collect::<Vec<_>>())
            .await?;
        if code != 0 {
            eprintln!("[tailscale] login with auth key failed");
            return Ok(code);
        }
        println!("[tailscale] logged in as {hostname}");
    }
    if env::var("TAILSCALE_SERVE").is_ok_and(|s| ["true", "1", "yes"].contains(&s.as_str())) {
        let timeout = env::var("TAILSCALE_SERVE_TIMEOUT")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60);
        let mut command = ctx.command(&binary);
        command
            .args(["serve", "--bg", "--yes", &serve_target(ctx)])
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        if !matches!(tokio::time::timeout(Duration::from_secs(timeout), child.wait()).await, Ok(Ok(status)) if status.success())
        {
            let _ = child.kill().await;
            eprintln!(
                "[tailscale] serve was not enabled; enable Serve/HTTPS for the tailnet, then run: workspacectl tailscale serve"
            );
        }
    }
    Ok(0)
}
