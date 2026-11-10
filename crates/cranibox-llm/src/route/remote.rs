//! Owner-roster routing and authenticated, streaming mesh forwarding.
use super::{RouteDecision, Router, health::HealthBook};
use cranibox_contracts::{
    error::ErrorCode,
    llm::*,
    net::{BoxBody, PeerTransport, routes},
    roster::{Role, RosterView},
    vault::AccountStore,
};
use futures_util::{StreamExt, stream};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
/// How long background probes leave a refusing node alone before asking again.
const DENIED_PROBE_PAUSE: Duration = Duration::from_secs(600);
/// A request is only replayed before any response byte has been exposed to its caller.
pub struct RemoteRouter {
    pub node: String,
    pub roster: Arc<dyn RosterView>,
    pub store: Arc<dyn AccountStore>,
    pub transport: Arc<dyn PeerTransport>,
    pub health: Arc<HealthBook>,
    // Pause background requests after an explicit capability refusal. Probing
    // resumes after DENIED_PROBE_PAUSE, an owner-roster change, a successful user
    // request, or a restart.
    denied_probes: Mutex<HashMap<String, (Option<u64>, Instant)>>,
}
impl RemoteRouter {
    pub fn new(
        node: String,
        roster: Arc<dyn RosterView>,
        store: Arc<dyn AccountStore>,
        transport: Arc<dyn PeerTransport>,
    ) -> Self {
        Self {
            node,
            roster,
            store,
            transport,
            health: Arc::default(),
            denied_probes: Mutex::default(),
        }
    }
    pub fn candidates(&self, provider: &str) -> Vec<String> {
        let Some(roster) = self.roster.current() else {
            return vec![];
        };
        if provider == "claude" {
            return self
                .roster
                .claude_node()
                .into_iter()
                .map(|m| m.node_id)
                .collect();
        }
        let mut accounts = self
            .store
            .list()
            .into_iter()
            .filter(|a| {
                a.provider == provider
                    && a.kind != AccountKind::ClaudeOAuth
                    && matches!(a.state, AccountState::Ready)
            })
            .collect::<Vec<_>>();
        accounts.sort_by(|a, b| a.id.cmp(&b.id));
        // A usable local account always wins, independent of Hub health.
        if accounts
            .iter()
            .any(|a| a.all_nodes || a.enabled_on.contains(&self.node))
        {
            return vec![self.node.clone()];
        }
        let mut nodes = Vec::new();
        for account in accounts {
            let mut enabled = roster
                .members
                .iter()
                .filter(|m| {
                    (account.all_nodes || account.enabled_on.contains(&m.node_id))
                        && self.health.get(&m.node_id).available()
                })
                .collect::<Vec<_>>();
            enabled.sort_by_key(|m| {
                (
                    if m.role == Role::Hub {
                        0
                    } else if m.node_id == account.home_node {
                        1
                    } else {
                        2
                    },
                    m.priority,
                    m.node_id.clone(),
                )
            });
            for m in enabled {
                if !nodes.contains(&m.node_id) {
                    nodes.push(m.node_id.clone());
                }
            }
        }
        nodes
    }
    pub async fn probe(&self, node: &str) -> bool {
        if !self.roster.is_owner_node(node) {
            return false;
        }
        let epoch = self.roster.current().map(|roster| roster.epoch);
        {
            let mut denied = self.denied_probes.lock().expect("probe denials");
            if denied.get(node).is_some_and(|(at_epoch, at)| {
                *at_epoch == epoch && at.elapsed() < DENIED_PROBE_PAUSE
            }) {
                return false;
            }
            denied.remove(node);
        }
        let req = http::Request::get(routes::llm(node, routes::LLM_HEALTH))
            .body(BoxBody::empty())
            .expect("valid node path");
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            self.transport.request(&node.to_owned(), req),
        )
        .await;
        let ok = match response {
            Ok(Ok(response)) if response.status().is_success() => true,
            Ok(Ok(response)) if response.status() == http::StatusCode::FORBIDDEN => {
                if tokio::time::timeout(
                    Duration::from_secs(1),
                    capability_off(response.into_body()),
                )
                .await
                .unwrap_or(false)
                {
                    self.denied_probes
                        .lock()
                        .expect("probe denials")
                        .insert(node.to_owned(), (epoch, Instant::now()));
                }
                false
            }
            _ => false,
        };
        if ok {
            self.health.succeeded(node, crate::auth::now());
        } else {
            self.health.failed(node);
        }
        ok
    }
    async fn send_one(
        &self,
        node: &str,
        provider: &str,
        request: CanonicalRequest,
        count: bool,
    ) -> Result<UpstreamResponse, UpstreamError> {
        if !self.roster.is_owner_node(node) {
            return Err(UpstreamError::Network("peer is not in owner roster".into()));
        }
        let path = if count {
            if request.wire != Wire::Messages {
                return Err(UpstreamError::Unsupported);
            }
            routes::LLM_COUNT_TOKENS
        } else {
            match request.wire {
                Wire::Chat => routes::LLM_CHAT,
                Wire::Responses => routes::LLM_RESPONSES,
                Wire::Messages => routes::LLM_MESSAGES,
            }
        };
        let mut body = request.body;
        let object = body.as_object_mut().ok_or(UpstreamError::Unsupported)?;
        object.insert(
            "model".into(),
            format!("{provider}/{}", request.model).into(),
        );
        if count {
            object.remove("stream");
        } else {
            object.insert("stream".into(), request.stream.into());
        }
        let mut builder =
            http::Request::post(routes::llm(node, path)).header("content-type", "application/json");
        if let Some(session) = request.session_id {
            builder = builder.header("x-session-id", session);
        }
        let outbound = builder
            .body(BoxBody::full(
                serde_json::to_vec(&body).map_err(|_| UpstreamError::Unsupported)?,
            ))
            .map_err(|_| UpstreamError::Unsupported)?;
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            self.transport.request(&node.to_owned(), outbound),
        )
        .await
        .map_err(|_| UpstreamError::Network("peer response headers timed out".into()))?
        .map_err(|_| UpstreamError::Network("peer unavailable".into()))?;
        let (parts, body) = response.into_parts();
        if count && parts.status == http::StatusCode::NOT_IMPLEMENTED {
            return Err(UpstreamError::Unsupported);
        }
        if !parts.status.is_success() {
            return Err(UpstreamError::Network("peer proxy unavailable".into()));
        }
        let mut body = body.into_stream();
        let first = tokio::time::timeout(Duration::from_secs(90), async {
            loop {
                match body.next().await {
                    Some(Ok(b)) if b.is_empty() => continue,
                    Some(Ok(b)) => return Ok(b),
                    _ => {
                        return Err(UpstreamError::Network(
                            "peer ended before first byte".into(),
                        ));
                    }
                }
            }
        })
        .await
        .map_err(|_| UpstreamError::Network("peer first byte timed out".into()))??;
        Ok(UpstreamResponse {
            status: parts.status.as_u16(),
            headers: parts
                .headers
                .iter()
                .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str().into(), v.into())))
                .collect(),
            wire: request.wire,
            body: BoxBody::new(stream::once(async move { Ok(first) }).chain(body)),
        })
    }
    async fn forward_operation(
        &self,
        node: &str,
        provider: &str,
        request: CanonicalRequest,
        count: bool,
    ) -> Result<UpstreamResponse, UpstreamError> {
        let mut nodes = vec![node.to_owned()];
        for candidate in self.candidates(provider) {
            if !nodes.contains(&candidate) {
                nodes.push(candidate);
            }
        }
        for candidate in nodes {
            if candidate == self.node {
                continue;
            }
            match self
                .send_one(&candidate, provider, request.clone(), count)
                .await
            {
                Ok(response) => {
                    self.denied_probes
                        .lock()
                        .expect("probe denials")
                        .remove(&candidate);
                    self.health.succeeded(&candidate, crate::auth::now());
                    return Ok(response);
                }
                // A missing count capability does not make a reachable peer unhealthy.
                Err(UpstreamError::Unsupported) if count => return Err(UpstreamError::Unsupported),
                Err(_) => self.health.failed(&candidate),
            }
            if provider == "claude" {
                break;
            }
        }
        Err(UpstreamError::Network(
            if provider == "claude" {
                "claude_node_offline"
            } else {
                "home_offline"
            }
            .into(),
        ))
    }
}
#[async_trait::async_trait]
impl Router for RemoteRouter {
    async fn decide(&self, provider: &str, _request: &CanonicalRequest) -> RouteDecision {
        match self.candidates(provider).into_iter().next() {
            Some(node) if node == self.node => RouteDecision::Local,
            Some(node) => RouteDecision::Remote { node },
            None => RouteDecision::Unavailable {
                code: if provider == "claude" {
                    ErrorCode::ClaudeNodeOffline
                } else if self.store.list().iter().any(|a| a.provider == provider) {
                    ErrorCode::HomeOffline
                } else {
                    ErrorCode::AllAccountsExhausted
                },
            },
        }
    }
    async fn forward(
        &self,
        node: &str,
        provider: &str,
        request: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        self.forward_operation(node, provider, request, false).await
    }
    async fn forward_count_tokens(
        &self,
        node: &str,
        provider: &str,
        request: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        self.forward_operation(node, provider, request, true).await
    }
}

/// Only the machine-readable capability refusal stops polling, never approval or
/// transient errors. Limit untrusted error responses before parsing them.
async fn capability_off(body: BoxBody) -> bool {
    let mut stream = body.into_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return false;
        };
        if bytes.len() + chunk.len() > 8192 {
            return false;
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| {
                    error
                        .as_str()
                        .or_else(|| error.get("code").and_then(|code| code.as_str()))
                })
                .or_else(|| value.get("code").and_then(|code| code.as_str()))
                .map(str::to_owned)
        })
        .is_some_and(|code| code == "capability_off")
}
