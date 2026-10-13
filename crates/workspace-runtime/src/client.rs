use crate::{config, protocol::*};
use clap::Args;
use std::path::PathBuf;
#[derive(Args, Clone)]
pub struct ApiOptions {
    #[arg(
        long,
        env = "AGENT_WORKSPACE_RUNTIME_URL",
        default_value = "http://127.0.0.1:9090"
    )]
    pub url: String,
    #[arg(long, env = "AGENT_WORKSPACE_RUNTIME_TOKEN_FILE")]
    pub token_file: Option<PathBuf>,
    #[arg(long, default_value_os_t = config::default_credentials())]
    pub credentials: PathBuf,
    #[arg(long)]
    pub allow_insecure: bool,
}

use anyhow::Result;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub struct Api {
    options: ApiOptions,
    client: reqwest::Client,
    mesh: bool,
}
impl Api {
    pub fn new(options: ApiOptions) -> Result<Self> {
        let url = config::check_url(&options.url, false, options.allow_insecure)?;
        anyhow::ensure!(
            url.path() == "/",
            "API URL must be an origin without a path"
        );
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self {
            options,
            client,
            mesh: false,
        })
    }
    pub fn mesh(options: ApiOptions) -> Result<Self> {
        let mut api = Self::new(options)?;
        api.mesh = true;
        Ok(api)
    }
    fn token(&self) -> Result<String> {
        match &self.options.token_file {
            Some(path) => config::read_token(path),
            None => Ok(config::Credentials::read(&self.options.credentials)?.controller_token),
        }
    }
    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let mut request = self
            .client
            .request(
                method,
                format!(
                    "{}{}",
                    self.options.url.trim_end_matches('/'),
                    if self.mesh {
                        path.replacen("/api/v1", "/api/v1/mesh", 1)
                    } else {
                        path.into()
                    }
                ),
            )
            .bearer_auth(self.token()?);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request.send().await?;
        let status = response.status();
        let mut bytes = vec![];
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(
                bytes.len() + chunk.len() <= 1024 * 1024,
                "response exceeds 1 MiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            status.is_success(),
            "API {}: {}",
            status.as_u16(),
            value["error"].as_str().unwrap_or("request failed")
        );
        Ok(value)
    }
    pub async fn get(&self, path: &str) -> Result<Value> {
        self.request(reqwest::Method::GET, path, None).await
    }
    pub async fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.request(reqwest::Method::POST, path, Some(body)).await
    }
    pub async fn execute(&self, node: &str, request: ExecRequest, detach: bool) -> Result<Value> {
        anyhow::ensure!(valid_node_id(node), "invalid node ID");
        request.validate()?;
        let timeout = request.timeout_secs;
        let job = self
            .post(
                &format!("/api/v1/nodes/{node}/exec"),
                serde_json::to_value(request)?,
            )
            .await?;
        if detach {
            return Ok(job);
        }
        let id = job["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing job ID"))?;
        let path = job_path(id)?;
        tokio::time::timeout(Duration::from_secs(timeout + DEAD_SECS + 15), async {
            loop {
                let value = self.get(&path).await?;
                if value["outcome"] != "running" {
                    return Ok::<_, anyhow::Error>(value);
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("job {id} still unresolved; query it with 'job {id}'"))?
    }
}
pub fn job_path(id: &str) -> Result<String> {
    anyhow::ensure!(uuid::Uuid::parse_str(id).is_ok(), "invalid job ID");
    Ok(format!("/api/v1/jobs/{id}"))
}
pub fn print(value: Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("JSON value")
    );
}

fn tools() -> Value {
    let job = json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"],"additionalProperties":false});
    json!([
        {"name":"workspace_nodes","description":"List enrolled Windows, macOS and Linux nodes with connection status.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
        {"name":"workspace_exec","description":"Start a command on a connected node and return a job ID. No implicit shell. Use workspace_job to poll output and completion. Commands run with the node client's OS account privileges.",
         "inputSchema":{"type":"object","properties":{
            "node":{"type":"string"},"program":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},
            "cwd":{"type":"string"},"env":{"type":"object","additionalProperties":{"type":"string"}},
            "timeout_secs":{"type":"integer","minimum":1,"maximum":3600,"default":60}},"required":["node","program"],"additionalProperties":false}},
        {"name":"workspace_job","description":"Read a job's current stdout/stderr, exit code and outcome. Output is bounded to 64 KiB; truncated indicates loss. Jobs are in memory and expire after ten minutes.","inputSchema":job},
        {"name":"workspace_cancel","description":"Cancel a running remote command and terminate its process tree. Poll workspace_job for confirmation.","inputSchema":job}
    ])
}
async fn call(api: &Api, params: &Value) -> Result<Value> {
    let name = params["name"].as_str().unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    match name {
        "workspace_nodes" => api.get("/api/v1/nodes").await,
        "workspace_exec" => {
            let mut args = args
                .as_object()
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("arguments must be an object"))?;
            let node = args
                .remove("node")
                .and_then(|v| v.as_str().map(str::to_owned))
                .ok_or_else(|| anyhow::anyhow!("node required"))?;
            api.execute(&node, serde_json::from_value(Value::Object(args))?, true)
                .await
        }
        "workspace_job" | "workspace_cancel" => {
            let path = job_path(args["id"].as_str().unwrap_or_default())?;
            if name == "workspace_cancel" {
                api.post(&format!("{path}/cancel"), json!({})).await
            } else {
                api.get(&path).await
            }
        }
        _ => anyhow::bail!("unknown tool"),
    }
}
pub async fn mcp(api: Api) -> Result<()> {
    let mut input = BufReader::new(tokio::io::stdin());
    let mut output = tokio::io::stdout();
    loop {
        // Stdio frames are newline-delimited JSON, with a bound before allocation can grow.
        let mut line = vec![];
        loop {
            let available = input.fill_buf().await?;
            if available.is_empty() {
                break;
            }
            let count = available
                .iter()
                .position(|b| *b == b'\n')
                .map_or(available.len(), |at| at + 1);
            anyhow::ensure!(
                line.len() + count <= MAX_FRAME,
                "MCP request exceeds 64 KiB"
            );
            let done = available[count - 1] == b'\n';
            line.extend_from_slice(&available[..count]);
            input.consume(count);
            if done {
                break;
            }
        }
        if line.is_empty() {
            break;
        }
        let response = match serde_json::from_slice::<Value>(&line) {
            Err(_) => Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Invalid JSON"}}),
            ),
            Ok(req) if req.get("id").is_none() => None,
            Ok(req) => {
                let id = req["id"].clone();
                let result = match req["method"].as_str().unwrap_or_default() {
                    "initialize" => {
                        let requested = req["params"]["protocolVersion"]
                            .as_str()
                            .unwrap_or("2024-11-05");
                        let protocol =
                            if ["2024-11-05", "2025-03-26", "2025-06-18"].contains(&requested) {
                                requested
                            } else {
                                "2024-11-05"
                            };
                        json!({"protocolVersion":protocol,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"agent-workspace-nodes","version":env!("CARGO_PKG_VERSION")}})
                    }
                    "ping" => json!({}),
                    "tools/list" => json!({"tools":tools()}),
                    "tools/call" => match call(&api, &req["params"]).await {
                        Ok(value) => {
                            json!({"content":[{"type":"text","text":serde_json::to_string(&value)?}]})
                        }
                        Err(error) => {
                            json!({"isError":true,"content":[{"type":"text","text":error.to_string()}]})
                        }
                    },
                    _ => {
                        let response = json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}});
                        output.write_all(format!("{response}\n").as_bytes()).await?;
                        output.flush().await?;
                        continue;
                    }
                };
                Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
            }
        };
        if let Some(response) = response {
            output.write_all(format!("{response}\n").as_bytes()).await?;
            output.flush().await?;
        }
    }
    Ok(())
}
