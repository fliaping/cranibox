//! Session-bound, one-use browser tickets. Node credentials never reach JavaScript.
use crate::server::State;
use agent_workspace_runtime::{
    peers::{
        Store,
        desktop::{Event, MAX_CHUNK, Stream},
    },
    protocol::valid_node_id,
};
use axum::{
    Json,
    extract::{Query, State as ExtractState, WebSocketUpgrade, ws::Message},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

#[derive(Clone, Default)]
pub(crate) struct Hub {
    tickets: Arc<Mutex<HashMap<String, Ticket>>>,
}
struct Ticket {
    session: String,
    expires: Instant,
    desktop: Stream,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    node: String,
    #[serde(default)]
    local: bool,
    #[serde(default)]
    relay: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cancel {
    ticket: String,
}
impl Hub {
    pub(crate) async fn cancel(&self, session: &str, body: Value) -> anyhow::Result<Value> {
        let options: Cancel = serde_json::from_value(body)?;
        anyhow::ensure!(
            options.ticket.len() == 64 && options.ticket.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid desktop ticket"
        );
        let mut tickets = self.tickets.lock().await;
        // A different login session cannot cancel another viewer's pending lease.
        let cancelled = tickets
            .get(&options.ticket)
            .is_some_and(|ticket| ticket.session == session);
        if cancelled {
            tickets.remove(&options.ticket);
        }
        Ok(json!({"cancelled":cancelled}))
    }
    pub(crate) async fn issue(
        &self,
        state: &State,
        session: String,
        body: Value,
    ) -> anyhow::Result<Value> {
        let options: Options = serde_json::from_value(body)?;
        anyhow::ensure!(valid_node_id(&options.node), "invalid node ID");
        anyhow::ensure!(
            !(options.local && options.relay),
            "choose one desktop route"
        );
        anyhow::ensure!(
            self.tickets.lock().await.len() < 64,
            "too many desktop requests"
        );
        let desktop = if options.local {
            anyhow::ensure!(
                state
                    .local
                    .as_ref()
                    .is_some_and(|node| node.id == options.node),
                "local node identity mismatch"
            );
            state.local.as_ref().unwrap().desktop().await?
        } else if options.relay {
            if let Some(gateway) = &state.gateway {
                gateway.desktop(&options.node).await?
            } else {
                agent_workspace_runtime::client::Api::new(
                    agent_workspace_runtime::client::ApiOptions {
                        url: state.config.runtime_url.clone(),
                        token_file: None,
                        credentials: state.root.join(".config/agent-workspace/runtime.json"),
                        allow_insecure: false,
                    },
                )?
                .desktop(&options.node)
                .await?
            }
        } else {
            store(state)
                .route(&options.node)?
                .api(&options.node)?
                .desktop(&options.node)
                .await?
        };
        let mut tickets = self.tickets.lock().await;
        tickets.retain(|_, ticket| ticket.expires > Instant::now());
        anyhow::ensure!(tickets.len() < 64, "too many desktop requests");
        let ticket = agent_workspace_runtime::config::token();
        tickets.insert(
            ticket.clone(),
            Ticket {
                session: session.clone(),
                expires: Instant::now() + Duration::from_secs(30),
                desktop,
            },
        );
        let tickets = self.tickets.clone();
        let expired = ticket.clone();
        let owner = state.clone();
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            let mut check = tokio::time::interval(Duration::from_secs(1));
            loop {
                check.tick().await;
                if tokio::time::Instant::now() >= deadline
                    || *owner.closing.borrow()
                    || !owner.has_session(&session).await
                    || !tickets.lock().await.contains_key(&expired)
                {
                    break;
                }
            }
            tickets.lock().await.remove(&expired);
        });
        Ok(json!({"ticket":ticket}))
    }
}
fn store(state: &State) -> Store {
    Store::new(state.root.join(".config/cranibox-node/peers.json"))
}
#[derive(Deserialize)]
pub(crate) struct Connection {
    ticket: String,
}
fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error":message}))).into_response()
}
pub(crate) async fn upgrade(
    ExtractState(state): ExtractState<State>,
    headers: HeaderMap,
    Query(query): Query<Connection>,
    ws: WebSocketUpgrade,
) -> Response {
    if !headers.contains_key("origin") {
        return error(StatusCode::FORBIDDEN, "Origin required");
    }
    let Some(session) = state.session(&headers).await else {
        return error(StatusCode::UNAUTHORIZED, "Session expired");
    };
    let session_id = session.lock().await.id.clone();
    let ticket = {
        let mut tickets = state.desktops.tickets.lock().await;
        if !tickets
            .get(&query.ticket)
            .is_some_and(|t| t.session == session_id && t.expires > Instant::now())
        {
            return error(StatusCode::UNAUTHORIZED, "Desktop ticket expired");
        }
        tickets.remove(&query.ticket).unwrap()
    };
    let mut desktop = ticket.desktop;
    let mut closing = state.closing.subscribe();
    ws.max_message_size(MAX_CHUNK).max_frame_size(MAX_CHUNK).on_upgrade(move |mut socket| async move {
        let _session = session;
        let run: anyhow::Result<()> = async {
            let mut check = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    _ = closing.changed() => break,
                    _ = check.tick() => { if *closing.borrow() || !state.has_session(&session_id).await { break; } }
                    message = socket.recv() => match message {
                        Some(Ok(Message::Binary(bytes))) => desktop.send(bytes.to_vec())?,
                        Some(Ok(Message::Ping(data))) => { tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Pong(data))).await??; }
                        Some(Ok(Message::Pong(_))) => {},
                        _ => break,
                    },
                    output = desktop.output.recv() => match output {
                        Some(Event::Data(bytes)) => { tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Binary(bytes.into()))).await??; }
                        _ => break,
                    },
                }
            }
            Ok(())
        }.await;
        let _ = run;
        drop(desktop);
        let _ = tokio::time::timeout(Duration::from_secs(1), socket.send(Message::Close(None))).await;
    }).into_response()
}
