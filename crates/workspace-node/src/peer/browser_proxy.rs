//! A process-scoped local CDP endpoint. The remote bearer never reaches the MCP child.
use agent_workspace_runtime::peers::{
    Store,
    browser::{self, Socket},
};
use anyhow::{Context, Result, ensure};
use axum::{
    Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use std::{
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio_tungstenite::tungstenite::Message as Upstream;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct App {
    store: Store,
    node: String,
    host: String,
    stop: CancellationToken,
}
async fn upgrade(State(app): State<App>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    if headers.contains_key("origin")
        || headers.contains_key("sec-fetch-site")
        || headers.get("host").and_then(|h| h.to_str().ok()) != Some(&app.host)
    {
        return (StatusCode::FORBIDDEN, "Local automation clients only").into_response();
    }
    let upstream = match async {
        app.store
            .route(&app.node)?
            .api(&app.node)?
            .browser(&app.node)
            .await
    }
    .await
    {
        Ok(socket) => socket,
        Err(error) => return (StatusCode::BAD_GATEWAY, error.to_string()).into_response(),
    };
    ws.max_message_size(browser::MAX_MESSAGE)
        .max_frame_size(browser::MAX_MESSAGE)
        .on_upgrade(move |socket| async move {
            let _ = relay(socket, upstream, move || !app.stop.is_cancelled()).await;
        })
}

pub(crate) async fn relay(
    mut local: WebSocket,
    mut upstream: Socket,
    valid: impl Fn() -> bool + Send + Sync,
) -> Result<()> {
    let run = async {
        let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
        let mut seen = Instant::now();
        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    ensure!(seen.elapsed() < Duration::from_secs(60), "Browser connection timed out");
                    upstream.send(Upstream::Ping(vec![].into())).await?;
                }
                message = local.recv() => match message {
                    Some(Ok(Message::Text(text))) => {
                        ensure!(valid(), "Browser sharing stopped or access revoked");
                        upstream.send(Upstream::Text(text.to_string().into())).await?;
                    }
                    Some(Ok(Message::Ping(bytes))) => local.send(Message::Pong(bytes)).await?,
                    Some(Ok(Message::Pong(_))) => {},
                    _ => break,
                },
                message = upstream.next() => {
                    seen = Instant::now();
                    match message {
                        Some(Ok(Upstream::Text(text))) => {
                            ensure!(valid(), "Browser sharing stopped or access revoked");
                            local.send(Message::Text(text.to_string().into())).await?;
                        }
                        Some(Ok(Upstream::Ping(bytes))) => upstream.send(Upstream::Pong(bytes)).await?,
                        Some(Ok(Upstream::Pong(_))) => {},
                        _ => break,
                    }
                },
            }
        }
        Ok(())
    };
    // This independent guard also closes a connection blocked by a slow reader/writer.
    let guard = async {
        let mut check = tokio::time::interval(Duration::from_millis(250));
        loop {
            check.tick().await;
            ensure!(valid(), "Browser sharing stopped or access revoked");
        }
    };
    let result = tokio::select! { result = run => result, result = guard => result };
    let _ = tokio::time::timeout(Duration::from_secs(1), local.send(Message::Close(None))).await;
    let _ = tokio::time::timeout(Duration::from_secs(1), upstream.close(None)).await;
    result
}

pub(super) async fn run(path: PathBuf, node: String, mcp: bool) -> Result<()> {
    let store = Store::new(path);
    let status = store.browser(&node, browser::Request::Status {}).await?;
    ensure!(
        status["result"]["paused"] == false && status["result"]["backend_available"] == true,
        "Target browser is paused or unavailable; check it on the target device"
    );
    let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let host = tcp.local_addr()?.to_string();
    let route = format!("/{}", uuid::Uuid::new_v4().simple());
    let endpoint = format!("ws://{host}{route}");
    let stop = CancellationToken::new();
    let app = App {
        store,
        node: node.clone(),
        host,
        stop: stop.clone(),
    };
    let router = Router::new().route(&route, get(upgrade)).with_state(app);
    let shutdown = stop.clone();
    let mut server = tokio::spawn(async move {
        axum::serve(tcp, router)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
    });
    let result = if mcp {
        // Keep MCP stdout exclusively owned by the official upstream process.
        let program = if cfg!(windows) { "npx.cmd" } else { "npx" };
        let mut command = tokio::process::Command::new(program);
        command
            .args([
                "--yes",
                "chrome-devtools-mcp@1.11.0",
                "--no-usage-statistics",
                "--no-performance-crux",
                &format!("--wsEndpoint={endpoint}"),
            ])
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
        let child = command.spawn();
        match child {
            Ok(mut child) => {
                let result = match crate::node::ProcessTree::new(&child) {
                    Ok(tree) => {
                        let result = tokio::select! {
                            status = child.wait() => status.context("Wait for Browser MCP").and_then(|s| { ensure!(s.success(), "Browser MCP exited with {s}"); Ok(()) }),
                            _ = tokio::signal::ctrl_c() => Ok(()),
                        };
                        tree.kill();
                        result
                    }
                    Err(error) => Err(error),
                };
                let _ = child.kill().await;
                result
            }
            Err(error) => {
                Err(error).context("Browser MCP needs Node.js and npx on the calling device")
            }
        }
    } else {
        println!("{}", serde_json::json!({"node":node,"websocket":endpoint}));
        tokio::signal::ctrl_c().await.map_err(Into::into)
    };
    stop.cancel();
    if tokio::time::timeout(Duration::from_secs(3), &mut server)
        .await
        .is_err()
    {
        server.abort();
        let _ = server.await;
    }
    result
}
