//! Optional iroh relay of a device: `/relay`, `/ping` and `/generate_204` (C5), shared by
//! the CLI node, the desktop app and the Workspace container.
//!
//! `iroh_relay::RelayService` needs the raw TCP stream to upgrade, so these paths are
//! dispatched in front of axum by [`cranibox_net::relay::Relay::serve`]. Only paired
//! EndpointIds (grants and routes in `peers.json`, kept current by the node's iroh
//! service) and holders of an open pairing ticket may use it. It speaks plain HTTP:
//! TLS is terminated by the proxy or tunnel in front (Caddy, nginx, Cloudflare Tunnel),
//! which must forward `Upgrade`.
use crate::peer::transport_iroh::{HubOptions, configure_hub};
use cranibox_net::{
    allow::Allowlist,
    relay::{Check, Relay},
};
use std::future::Future;

/// `--relay` / `--no-relay` / `--public-url`, resolved before the node starts.
#[derive(Clone)]
pub struct Embedded {
    relay: Option<Relay>,
    public_url: Option<String>,
}

/// The relay runs when `relay` is set, or by default when there is an `https://` public
/// URL; `no_relay` always wins. Must run before the local node starts, because the
/// node's iroh service picks these settings up.
pub fn setup(relay: bool, no_relay: bool, public_url: &str) -> Embedded {
    let public_url = Some(public_url.trim_end_matches('/').to_owned()).filter(|u| !u.is_empty());
    let https = public_url
        .as_deref()
        .is_some_and(|u| u.starts_with("https://"));
    let enabled = !no_relay && (relay || https);
    let allow = Allowlist::default();
    let relay = enabled.then(|| Relay::new(allow.clone()));
    if enabled && !https {
        eprintln!(
            "iroh relay: running, but without an https --public-url devices cannot reach it; \
             they use the public n0 relays instead"
        );
    }
    configure_hub(HubOptions {
        allow,
        relay: relay.clone(),
        public_url: public_url.clone(),
    });
    Embedded { relay, public_url }
}

impl Embedded {
    pub fn enabled(&self) -> bool {
        self.relay.is_some()
    }
    pub fn public_url(&self) -> Option<&str> {
        self.public_url.as_deref()
    }
    /// Serve `router`, with the relay paths in front when the relay is enabled.
    pub async fn serve(
        &self,
        listener: tokio::net::TcpListener,
        router: axum::Router,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<()> {
        match &self.relay {
            Some(relay) => relay.clone().serve(listener, router, shutdown).await,
            None => {
                axum::serve(listener, router)
                    .with_graceful_shutdown(shutdown)
                    .await
            }
        }
    }

    /// Startup self-check: `GET <public-url>/relay` must reach us as `101`.
    pub fn spawn_self_check(&self) {
        let (Some(_), Some(url)) = (&self.relay, self.public_url.clone()) else {
            return;
        };
        if !url.starts_with("https://") {
            return;
        }
        if url::Url::parse(&url).is_ok_and(|u| u.path() != "/") {
            eprintln!(
                "iroh relay: --public-url {url} has a path; iroh always dials <origin>/relay, \
                 so the relay must be reachable at the root of that origin"
            );
        }
        tokio::spawn(async move {
            // Give the listener and any tunnel a moment to come up.
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            match cranibox_net::relay::self_check(&url, &extra_roots()).await {
                Check::Ok => eprintln!("iroh relay self-check: {url}/relay OK (101)"),
                Check::Problem(problem) => eprintln!("iroh relay self-check FAILED: {problem}"),
            }
        });
    }
}

/// Test/private-CA override shared with the node (`CRANIBOX_IROH_EXTRA_ROOT`, DER).
fn extra_roots() -> Vec<Vec<u8>> {
    std::env::var_os("CRANIBOX_IROH_EXTRA_ROOT")
        .and_then(|file| std::fs::read(file).ok())
        .into_iter()
        .collect()
}
