//! Read-only device/approval MCP. Decisions remain exclusive to the human App API.
use agent_workspace_runtime::peers::{Store, config::Config, roster::RosterStore};
use anyhow::{Result, ensure};
use cranibox_contracts::roster::RosterView;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

pub async fn run(path: PathBuf, data: PathBuf, service: &str) -> Result<()> {
    let config = Config::read(&path)?;
    match service {
        "computer" => {
            agent_workspace_runtime::client::computer_mcp(Store::new(path), config.node_id).await
        }
        "browser" => super::peer::browser_proxy::run(path, config.node_id, true).await,
        "devices" => {
            let env = cranibox_agents::environment(crate::peer::home()?, data);
            serve(
                BufReader::new(tokio::io::stdin()),
                tokio::io::stdout(),
                &path,
                &env,
            )
            .await
        }
        _ => anyhow::bail!("unknown Cranibox MCP service"),
    }
}

async fn frame(input: &mut (impl AsyncBufRead + Unpin)) -> Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let chunk = input.fill_buf().await?;
        if chunk.is_empty() {
            return Ok((!line.is_empty()).then_some(line));
        }
        let end = chunk.iter().position(|b| *b == b'\n').map(|i| i + 1);
        let size = end.unwrap_or(chunk.len());
        ensure!(line.len() + size <= 64 * 1024, "MCP request exceeds 64 KiB");
        line.extend_from_slice(&chunk[..size]);
        input.consume(size);
        if end.is_some() {
            return Ok(Some(line));
        }
    }
}

fn tools() -> Value {
    json!([
        {"name":"cranibox_devices","description":"Read exact device IDs in the owner-signed roster and saved peer routes. Does not grant access or prove a device is online.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
        {"name":"cranibox_approvals","description":"Read pending local Agent approval IDs and expiry times. Only the human can approve or deny in the Cranibox inbox. This tool cannot make decisions.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}}
    ])
}

fn call(path: &Path, env: &cranibox_contracts::agents::Env, params: &Value) -> Result<Value> {
    ensure!(
        params["arguments"]
            .as_object()
            .is_some_and(|a| a.is_empty()),
        "expected empty arguments"
    );
    let config = Config::read(path)?;
    let value = match params["name"].as_str() {
        Some("cranibox_devices") => {
            let roster = RosterStore::load(path.with_file_name("models").join("roster.json"))
                .ok()
                .and_then(|r| r.current());
            let mut devices = std::collections::BTreeMap::new();
            devices.insert(
                config.node_id.clone(),
                json!({"node_id":config.node_id,"local":true}),
            );
            for node in config.routes.keys() {
                devices.insert(
                    node.clone(),
                    json!({"node_id":node,"local":node == &config.node_id}),
                );
            }
            for member in roster.into_iter().flat_map(|r| r.members) {
                devices.insert(member.node_id.clone(), json!({"node_id":member.node_id,"name":member.name,"role":member.role,"local":member.node_id == config.node_id}));
            }
            json!({"devices":devices.into_values().collect::<Vec<_>>()})
        }
        Some("cranibox_approvals") => {
            json!({"approvals":cranibox_agents::approvals::pending(env, &config.node_id)?.into_iter().map(|a|json!({"id":a.id,"agent":a.agent,"expires_at":a.expires_at})).collect::<Vec<_>>()})
        }
        _ => anyhow::bail!("unknown tool; approval decisions require the human App API"),
    };
    Ok(json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value}))
}

async fn serve(
    mut input: impl AsyncBufRead + Unpin,
    mut output: impl AsyncWrite + Unpin,
    path: &Path,
    env: &cranibox_contracts::agents::Env,
) -> Result<()> {
    while let Some(line) = frame(&mut input).await? {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let message: Value = match serde_json::from_slice(&line) {
            Ok(message) => message,
            Err(_) => {
                output.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32700,\"message\":\"Invalid JSON\"}}\n").await?;
                output.flush().await?;
                continue;
            }
        };
        let Some(id) = message.get("id") else {
            continue;
        };
        let result = match message["method"].as_str() {
            Some("initialize") => Ok(
                json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"cranibox","version":env!("CARGO_PKG_VERSION")}}),
            ),
            Some("ping") => Ok(json!({})),
            Some("tools/list") => Ok(json!({"tools":tools()})),
            Some("tools/call") => call(path, env, &message["params"]),
            _ => Err(anyhow::anyhow!("method not supported")),
        };
        let response = match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err(e) => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":e.to_string()}})
            }
        };
        let mut bytes = serde_json::to_vec(&response)?;
        bytes.push(b'\n');
        output.write_all(&bytes).await?;
        output.flush().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn stdio_lists_devices_and_never_accepts_agent_approval_decisions() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("peers.json");
        let config = Config::update(&path, |_| Ok(())).unwrap();
        let env =
            cranibox_agents::environment(root.path().join("home"), root.path().join("models"));
        let requests = [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"cranibox_devices","arguments":{}}}),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"cranibox_approvals","arguments":{"decision":"allow_once"}}}),
            json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"approve","arguments":{}}}),
        ];
        let input = requests
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let mut output = Vec::new();
        serve(input.as_bytes(), &mut output, &path, &env)
            .await
            .unwrap();
        let replies: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(replies.len(), 5);
        assert_eq!(replies[1]["result"]["tools"].as_array().unwrap().len(), 2);
        assert_eq!(
            replies[2]["result"]["structuredContent"]["devices"][0]["node_id"],
            config.node_id
        );
        assert!(replies[3]["error"].is_object());
        assert!(replies[4]["error"].is_object());
    }
}
