//! The iroh relay embedded in `cranibox serve` (Hub).
//!
//! HTTP(S)/WebSocket only: the relay shares the control-center port and answers
//! `GET /relay` (WebSocket upgrade), `GET /ping` (HTTPS latency probe) and
//! `GET /generate_204` (captive-portal probe). Everything else goes to the
//! application router. No QUIC address discovery is offered, so it works behind
//! a TLS-terminating proxy or Cloudflare Tunnel as long as `Upgrade` is forwarded.
//!
//! Access: only allowlisted (paired) EndpointIds, plus one-time pairing tokens
//! derived from a pending ticket secret, may register with the relay.
use crate::allow::Allowlist;
use anyhow::Result;
use iroh_relay::server::{
    Access, AccessControl, ClientRequest, Metrics,
    http_server::{Handlers, RelayService, RelayServiceWithNotify},
    streams::MaybeTlsStream,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::net::TcpListener;

pub use iroh_relay::http::{RELAY_PATH, RELAY_PROBE_PATH};
pub const CAPTIVE_PORTAL_PATH: &str = "/generate_204";

/// Relay access token for a ticket holder that is not paired yet.
pub fn pairing_token(secret: &[u8; 16]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"cranibox-relay-pair-v1\0");
    hasher.update(secret);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Debug)]
struct Gate {
    allow: Allowlist,
    pairing: Mutex<HashMap<String, Instant>>,
}
impl AccessControl for Gate {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        if self.allow.contains(&request.endpoint_id()) {
            return Access::Allow;
        }
        if let Some(token) = request.auth_token() {
            let mut pairing = self.pairing.lock().unwrap();
            pairing.retain(|_, expires| *expires > Instant::now());
            if pairing.keys().any(|known| {
                bool::from(subtle::ConstantTimeEq::ct_eq(
                    known.as_bytes(),
                    token.as_bytes(),
                ))
            }) {
                return Access::Allow;
            }
        }
        Access::Deny {
            reason: Some("this relay only serves paired Cranibox devices".into()),
        }
    }
}

#[derive(Clone)]
pub struct Relay {
    service: RelayService,
    gate: Arc<Gate>,
}
impl Relay {
    /// A relay admitting `allow`. Ids removed from the allowlist are disconnected.
    pub fn new(allow: Allowlist) -> Self {
        let gate = Arc::new(Gate {
            allow: allow.clone(),
            pairing: Mutex::default(),
        });
        let service = RelayService::new(
            Handlers::default(),
            http::HeaderMap::new(),
            None,
            iroh_relay::KeyCache::new(4096),
            gate.clone(),
            Arc::new(Metrics::default()),
        );
        let relay = Self { service, gate };
        tokio::spawn(relay.clone().enforce(allow));
        relay
    }
    /// Admit a not-yet-paired ticket holder until `ttl` passes.
    pub fn allow_pairing(&self, secret: &[u8; 16], ttl: Duration) {
        self.gate
            .pairing
            .lock()
            .unwrap()
            .insert(pairing_token(secret), Instant::now() + ttl);
    }
    async fn enforce(self, allow: Allowlist) {
        let mut changes = allow.subscribe();
        let mut previous = allow.snapshot();
        while changes.changed().await.is_ok() {
            let current = allow.snapshot();
            for id in previous.difference(&current) {
                // Unpaired: end its relay session now, not at its next reconnect.
                self.service.clients().disconnect(*id, None);
            }
            previous = current;
        }
    }
    pub async fn shutdown(&self) {
        self.service.shutdown().await;
    }

    /// Serve `listener` (plain HTTP; TLS is terminated by the proxy in front):
    /// relay paths here, every other request to `app`.
    pub async fn serve(
        self,
        listener: TcpListener,
        app: axum::Router,
        shutdown: impl Future<Output = ()> + Send,
    ) -> std::io::Result<()> {
        let mut connections = tokio::task::JoinSet::new();
        tokio::pin!(shutdown);
        loop {
            let (tcp, _) = tokio::select! {
                _ = &mut shutdown => break,
                accepted = listener.accept() => match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        tracing::warn!("accept: {error}");
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                },
            };
            let relay = self.service.clone();
            let app = app.clone();
            connections.spawn(serve_connection(tcp, relay, app));
            while connections.try_join_next().is_some() {}
        }
        // Let in-flight HTTP requests finish briefly; relay sessions end with the service.
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            while connections.join_next().await.is_some() {}
        })
        .await;
        connections.abort_all();
        self.service.shutdown().await;
        Ok(())
    }
}

async fn serve_connection(tcp: tokio::net::TcpStream, relay: RelayService, app: axum::Router) {
    use hyper::service::Service as _;
    let _ = tcp.set_nodelay(true);
    let notify = Arc::new(tokio::sync::Notify::new());
    let service =
        hyper::service::service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
            let relay = RelayServiceWithNotify::new(relay.clone(), notify.clone());
            let app = app.clone();
            async move {
                let path = request.uri().path().to_owned();
                if path == RELAY_PATH {
                    return relay
                        .call(request)
                        .await
                        .map(|r| r.map(axum::body::Body::new))
                        .map_err(|e| std::io::Error::other(e.to_string()));
                }
                if request.method() == http::Method::GET && path == RELAY_PROBE_PATH {
                    return Ok(http::Response::new(axum::body::Body::empty()));
                }
                if request.method() == http::Method::GET && path == CAPTIVE_PORTAL_PATH {
                    return Ok(captive_portal(&request));
                }
                let mut app = app;
                Ok(
                    tower_service::Service::call(&mut app, request.map(axum::body::Body::new))
                        .await
                        .unwrap_or_else(|never| match never {}),
                )
            }
        });
    let io = hyper_util::rt::TokioIo::new(MaybeTlsStream::Plain(tcp));
    if let Err(error) = hyper::server::conn::http1::Builder::new()
        .serve_connection(io, service)
        .with_upgrades()
        .await
    {
        tracing::debug!("http connection: {error}");
    }
}

fn captive_portal(
    request: &hyper::Request<hyper::body::Incoming>,
) -> http::Response<axum::body::Body> {
    let mut response = http::Response::new(axum::body::Body::empty());
    *response.status_mut() = http::StatusCode::NO_CONTENT;
    if let Some(challenge) = request
        .headers()
        .get("x-iroh-challenge")
        .and_then(|v| v.to_str().ok())
        .filter(|c| {
            c.len() <= 64
                && c.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-_:".contains(&b))
        })
        && let Ok(value) = http::HeaderValue::from_str(&format!("response {challenge}"))
    {
        response.headers_mut().insert("x-iroh-response", value);
    }
    response
}

/// Result of [`self_check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// `GET /relay` answered `101 Switching Protocols`.
    Ok,
    /// Something in front of the relay is wrong; the text says what to fix.
    Problem(String),
}

/// Probe `<public_url>/relay` the way an iroh client does and explain failures.
pub async fn self_check(public_url: &str, extra_roots: &[Vec<u8>]) -> Check {
    match probe(public_url, extra_roots).await {
        Ok(check) => check,
        Err(error) => Check::Problem(format!(
            "{public_url} is unreachable from this server ({error:#}). Check DNS, the tunnel or proxy, and the TLS certificate."
        )),
    }
}

/// `GET <relay>/ping` answers 200: the relay is reachable over HTTPS.
pub async fn ping(relay: &str, extra_roots: &[Vec<u8>]) -> bool {
    async {
        let url = format!(
            "{}{}",
            crate::addr::normalize_relay(relay)?
                .to_string()
                .trim_end_matches('/'),
            RELAY_PROBE_PATH
        );
        let response = client(extra_roots)?.get(url).send().await?;
        anyhow::Ok(response.status().is_success())
    }
    .await
    .unwrap_or(false)
}

fn client(extra_roots: &[Vec<u8>]) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .http1_only();
    for der in extra_roots {
        builder = builder.add_root_certificate(reqwest::Certificate::from_der(der)?);
    }
    Ok(builder.build()?)
}

async fn probe(public_url: &str, extra_roots: &[Vec<u8>]) -> Result<Check> {
    let mut url = crate::addr::normalize_relay(public_url)?.to_string();
    url = format!("{}{}", url.trim_end_matches('/'), RELAY_PATH);
    let client = client(extra_roots)?;
    let mut key = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rng(), &mut key);
    let response = client
        .get(&url)
        .header("upgrade", "websocket")
        .header("connection", "Upgrade")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", data_encoding::BASE64.encode(&key))
        .header(
            "sec-websocket-protocol",
            iroh_relay::http::ProtocolVersion::all_joined(),
        )
        .send()
        .await?;
    let status = response.status();
    let body = if status == http::StatusCode::SWITCHING_PROTOCOLS {
        String::new()
    } else {
        response.text().await.unwrap_or_default()
    };
    Ok(diagnose(status.as_u16(), &body))
}

/// Map a probe response to a readable diagnosis.
pub fn diagnose(status: u16, body: &str) -> Check {
    let lower = body.to_ascii_lowercase();
    match status {
        101 => Check::Ok,
        400 if lower.contains("upgrade") || lower.contains("websocket") => Check::Problem(
            "The proxy in front of cranibox serve does not forward the WebSocket Upgrade header (expected HTTP 101, got 400). nginx: add `proxy_http_version 1.1; proxy_set_header Upgrade $http_upgrade; proxy_set_header Connection \"upgrade\";`. Cloudflare: keep WebSockets enabled for the zone.".into(),
        ),
        404 => Check::Problem(
            "The public URL does not reach the embedded relay at /relay (HTTP 404). Point the proxy or tunnel at the cranibox serve port without a path prefix, and do not start serve with --no-relay.".into(),
        ),
        502..=504 | 530 => Check::Problem(format!(
            "The proxy or tunnel cannot reach cranibox serve (HTTP {status}). Check that the tunnel connector is running and targets the serve port."
        )),
        200 => Check::Problem(
            "The proxy answered 200 instead of upgrading the connection; it is buffering or rewriting WebSocket requests.".into(),
        ),
        _ => Check::Problem(format!(
            "Unexpected HTTP {status} from /relay: {}",
            body.chars().take(200).collect::<String>()
        )),
    }
}
