//! Use the user's existing daemon. Cranibox never owns or stops tailscaled.
use super::process;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{path::PathBuf, process::Stdio};
use tokio::process::{Child, Command};

pub(super) fn executable() -> Option<PathBuf> {
    let name = format!("tailscale{}", std::env::consts::EXE_SUFFIX);
    let paths = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .map(|p| p.join(&name))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    paths
        .into_iter()
        .chain([
            PathBuf::from("/opt/homebrew/bin/tailscale"),
            PathBuf::from("/usr/local/bin/tailscale"),
            PathBuf::from("/Applications/Tailscale.app/Contents/MacOS/Tailscale"),
            PathBuf::from(r"C:\Program Files\Tailscale\tailscale.exe"),
        ])
        .find(|p| p.is_file())
}
pub(super) async fn status() -> Result<Value> {
    let Some(exe) = executable() else {
        return Ok(json!({"installed":false,"state":"NotInstalled","devices":[]}));
    };
    let bytes = process::output(&exe, &["status", "--json"]).await?;
    let raw: Value = serde_json::from_slice(&bytes).context("Read Tailscale status")?;
    Ok(sanitize(&raw))
}
fn sanitize(raw: &Value) -> Value {
    let state = raw["BackendState"]
        .as_str()
        .filter(|s| s.len() <= 64)
        .unwrap_or("Unknown");
    let auth_url = raw["AuthURL"].as_str().filter(|s| {
        s.len() <= 2048
            && url::Url::parse(s).is_ok_and(|u| {
                u.scheme() == "https"
                    && u.host_str() == Some("login.tailscale.com")
                    && u.port().is_none()
                    && u.username().is_empty()
                    && u.password().is_none()
                    && u.fragment().is_none()
            })
    });
    let devices: Vec<_> = raw["Peer"].as_object().into_iter().flat_map(|p|p.values()).take(512).map(|p| {
        let ips: Vec<_> = p["TailscaleIPs"].as_array().into_iter().flatten().filter_map(|ip|ip.as_str()?.parse::<std::net::IpAddr>().ok()).take(4).collect();
        json!({"name":p["HostName"].as_str().filter(|s|s.len() <= 253).unwrap_or(""),"addresses":ips,
            "online":p["Online"] == true,"path":if p["CurAddr"].as_str().is_some_and(|s|!s.is_empty()){"direct"}else{"relay"}})
    }).collect();
    json!({"installed":true,"state":state,"login_url":auth_url,"devices":devices})
}
pub(super) async fn login() -> Result<Child> {
    let state = status().await?;
    ensure!(
        state["state"] == "NeedsLogin",
        "Tailscale login is available when the existing system daemon reports NeedsLogin. Start Tailscale in the system first."
    );
    Ok(
        Command::new(executable().context("Install and start Tailscale first.")?)
            .args(["login", "--accept-dns=false", "--timeout=5m"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_never_returns_credentials_or_untrusted_login_links() {
        let raw = json!({"BackendState":"NeedsLogin","AuthURL":"https://evil.example/login","PrivateKey":"SECRET","Peer":{"a":{"HostName":"mac","Online":true,"TailscaleIPs":["100.64.0.2","invalid"],"PrivateKey":"SECRET"}}});
        let safe = sanitize(&raw);
        assert!(safe["login_url"].is_null());
        assert!(!safe.to_string().contains("SECRET"));
        assert_eq!(safe["devices"][0]["addresses"][0], "100.64.0.2");
        let mut raw = raw;
        raw["AuthURL"] = json!("https://login.tailscale.com/a/fixture");
        assert!(sanitize(&raw)["login_url"].is_string());
    }
}
