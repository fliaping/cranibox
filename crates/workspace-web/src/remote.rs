use crate::server::State;
use agent_workspace_runtime::{config, protocol};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

fn credentials(state: &State) -> std::path::PathBuf {
    state.root.join(".config/agent-workspace/runtime.json")
}
pub async fn api(
    state: &State,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    if let Some(gateway) = &state.gateway {
        return gateway.api(method, path, body).await;
    }
    let creds = config::Credentials::read(&credentials(state))
        .context("Remote node gateway is not installed")?;
    let mut request = state
        .remote_client
        .request(
            method,
            format!(
                "{}/api/v1{path}",
                state.config.runtime_url.trim_end_matches('/')
            ),
        )
        .bearer_auth(creds.controller_token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .context("Remote node gateway unavailable")?;
    let status = response.status();
    ensure!(
        response.content_length().unwrap_or(0) <= 2 * 1024 * 1024,
        "node response exceeds limit"
    );
    let bytes = response.bytes().await?;
    ensure!(
        bytes.len() <= 2 * 1024 * 1024,
        "node response exceeds limit"
    );
    let value: Value = serde_json::from_slice(&bytes)?;
    ensure!(
        status.is_success(),
        "{}",
        value["error"].as_str().unwrap_or("Node operation failed")
    );
    Ok(value)
}
pub async fn snapshot(state: &State) -> Value {
    if !credentials(state).exists() {
        return json!({"installed":false,"available":false,"nodes":[]});
    }
    match api(state, reqwest::Method::GET, "/nodes", None).await {
        Ok(mut value) => {
            value["installed"] = json!(true);
            value["available"] = json!(true);
            value
        }
        Err(_) => {
            json!({"installed":true,"available":false,"nodes":[],"error":"Remote node gateway unavailable"})
        }
    }
}
pub async fn manage(state: &State, action: &str, body: &Value) -> Result<Value> {
    match action {
        "group" => {
            let id = body["id"].as_str().context("Node ID required")?.to_owned();
            let group = body["group"]
                .as_str()
                .context("Device group required")?
                .to_owned();
            ensure!(
                protocol::valid_node_id(&id)
                    && (group.is_empty() || protocol::valid_node_id(&group)),
                "Invalid device group"
            );
            let path = credentials(state);
            tokio::task::spawn_blocking(move || {
                let _lock = config::lock_credentials(&path)?;
                let mut creds = config::Credentials::read(&path)?;
                let id = creds.resolve_node(&id).to_owned();
                ensure!(creds.nodes.contains_key(&id), "Unknown node");
                if group.is_empty() {
                    creds.mesh_groups.remove(&id);
                } else {
                    creds.mesh_groups.insert(id, group);
                }
                config::write_private(&path, &serde_json::to_vec_pretty(&creds)?, true)?;
                Ok(json!({"ok":true}))
            })
            .await?
        }
        "enroll" | "revoke" => {
            let id = body["id"].as_str().context("Node ID required")?.to_owned();
            ensure!(
                protocol::valid_node_id(&id),
                "Node ID must use letters, numbers, _ or - (1–64 characters)"
            );
            let path = credentials(state);
            let rotate = body["rotate"] == true;
            let revoke = action == "revoke";
            tokio::task::spawn_blocking(move || {
                let _lock = config::lock_credentials(&path)?;
                let mut creds = config::Credentials::read(&path)?;
                if revoke {
                    creds.revoke_node(&id)?;
                    config::write_private(&path, &serde_json::to_vec_pretty(&creds)?, true)?;
                    Ok(json!({"ok":true}))
                } else {
                    ensure!(
                        rotate || !creds.node_aliases.contains_key(&id),
                        "Node ID is reserved by an identity migration; use the canonical NodeID"
                    );
                    let id = creds.resolve_node(&id).to_owned();
                    ensure!(
                        rotate || !creds.nodes.contains_key(&id),
                        "Node already registered; explicitly rotate its credential"
                    );
                    ensure!(
                        creds.nodes.len() < 128 || creds.nodes.contains_key(&id),
                        "Maximum 128 nodes"
                    );
                    let token = config::token();
                    creds.nodes.insert(id.clone(), token.clone());
                    config::write_private(&path, &serde_json::to_vec_pretty(&creds)?, true)?;
                    // Returned once, never included in snapshots, operation history or logs.
                    Ok(json!({"id":id,"token":token}))
                }
            })
            .await?
        }
        "identity" => {
            let id = body["id"].as_str().context("Node ID required")?;
            let target = body["target"].as_str().context("Target NodeID required")?;
            ensure!(
                protocol::valid_node_id(id) && protocol::valid_node_id(target),
                "Invalid node ID"
            );
            api(
                state,
                reqwest::Method::POST,
                &format!("/nodes/{id}/identity"),
                Some(json!({"target":target})),
            )
            .await
        }
        "exec" => {
            let id = body["id"].as_str().context("Node ID required")?;
            ensure!(protocol::valid_node_id(id), "Invalid node ID");
            let request: protocol::ExecRequest = serde_json::from_value(body["request"].clone())?;
            request.validate()?;
            api(
                state,
                reqwest::Method::POST,
                &format!("/nodes/{id}/exec"),
                Some(serde_json::to_value(request)?),
            )
            .await
        }
        "jobs" => api(state, reqwest::Method::GET, "/jobs", None).await,
        "job" | "cancel" => {
            let id = body["id"].as_str().context("Job ID required")?;
            ensure!(
                id.len() == 36 && id.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-'),
                "Invalid job ID"
            );
            let path = format!(
                "/jobs/{id}{}",
                if action == "cancel" { "/cancel" } else { "" }
            );
            let mut value = api(
                state,
                if action == "cancel" {
                    reqwest::Method::POST
                } else {
                    reqwest::Method::GET
                },
                &path,
                (action == "cancel").then(|| json!({})),
            )
            .await?;
            if let Some(obj) = value.as_object_mut() {
                obj.remove("stdout_base64");
                obj.remove("stderr_base64");
            }
            Ok(value)
        }
        _ => anyhow::bail!("Unsupported node operation"),
    }
}
