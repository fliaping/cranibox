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
use anyhow::{Result, ensure};
use cranibox_net::{
    allow::Allowlist,
    relay::{Check, Relay},
};
use serde::{Deserialize, Serialize};
use std::{future::Future, net::SocketAddr, path::Path};

/// Persisted settings for this device's relay. The separate listener serves no UI.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub public_url: String,
    pub listen: SocketAddr,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            public_url: String::new(),
            listen: "127.0.0.1:7378".parse().expect("default relay listener"),
        }
    }
}

impl Settings {
    pub(crate) fn validated(mut self) -> Result<Self> {
        ensure!(
            self.listen.port() >= 1024,
            "Use a relay listening port between 1024 and 65535."
        );
        self.public_url = self.public_url.trim().to_owned();
        ensure!(
            !self.enabled || !self.public_url.is_empty(),
            "Enter this device's public HTTPS address before enabling its relay."
        );
        if !self.public_url.is_empty() {
            ensure!(
                self.public_url.len() <= 2048,
                "The relay public address is too long."
            );
            let url = url::Url::parse(&self.public_url)?;
            ensure!(
                url.scheme() == "https"
                    && url.host_str().is_some()
                    && url[..url::Position::BeforePath] == url.origin().ascii_serialization()
                    && matches!(url.path(), "" | "/")
                    && url.query().is_none()
                    && url.fragment().is_none(),
                "Use a public HTTPS origin without credentials, a path, query, or fragment."
            );
            self.public_url = url.origin().ascii_serialization();
        }
        Ok(self)
    }
}

/// Saved GUI settings override legacy relay defaults on the next start. A broken
/// network file must not prevent opening the local UI to repair it.
pub fn resolve(
    peers: &Path,
    mut flags: super::standalone::RelayFlags,
) -> super::standalone::RelayFlags {
    match crate::network::relay_settings(peers) {
        Ok(Some(settings)) => {
            flags.relay = settings.enabled;
            flags.no_relay |= !settings.enabled;
            flags.public_url = (settings.enabled && !settings.public_url.is_empty())
                .then_some(settings.public_url);
            flags.listen = Some(settings.listen);
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!(
                "relay settings: {error:#}; relay stays disabled until the network configuration is repaired"
            );
            flags.no_relay = true;
        }
    }
    flags
}

/// Resolve and register the relay before LocalNode starts, on every platform.
pub fn configured(
    peers: &Path,
    flags: super::standalone::RelayFlags,
) -> (super::standalone::RelayFlags, Embedded) {
    let flags = resolve(peers, flags);
    let embedded = setup_inner(
        flags.relay,
        flags.no_relay,
        flags.public_url.as_deref().unwrap_or(""),
        flags.listen,
    );
    (flags, embedded)
}

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
    setup_inner(relay, no_relay, public_url, None)
}

fn setup_inner(
    relay: bool,
    no_relay: bool,
    public_url: &str,
    listen: Option<SocketAddr>,
) -> Embedded {
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
        listen,
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

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn public_origin_and_listener_are_validated_before_save() {
        let mut settings = Settings {
            enabled: true,
            public_url: " https://Relay.Example:443/ ".into(),
            ..Settings::default()
        };
        assert_eq!(
            settings.clone().validated().unwrap().public_url,
            "https://relay.example"
        );
        for address in [
            "",
            "http://relay.example",
            "https://relay.example/path",
            "https://relay.example/?key=value",
            "https://relay.example/#fragment",
            "https://user@relay.example",
            "https://user:secret@relay.example",
        ] {
            settings.public_url = address.into();
            assert!(settings.clone().validated().is_err(), "{address}");
        }
        settings.public_url = "https://relay.example".into();
        settings.listen.set_port(443);
        assert!(settings.validated().is_err());
        assert!(Settings::default().validated().is_ok());
    }

    #[test]
    fn saved_enable_disable_and_corruption_resolve_safely() {
        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        let config = root.path().join("networks.json");
        let write = |enabled| {
            let document = serde_json::json!({
                "version": 1, "easytier": null, "forwards": {},
                "relay": {"enabled": enabled, "public_url": "https://relay.example", "listen": "127.0.0.1:7378"}
            });
            std::fs::write(&config, serde_json::to_vec(&document).unwrap()).unwrap();
        };
        assert!(!resolve(&peers, Default::default()).relay);
        write(true);
        let enabled = resolve(&peers, Default::default());
        assert!(enabled.relay && !enabled.no_relay);
        assert_eq!(enabled.listen, Some("127.0.0.1:7378".parse().unwrap()));
        assert_eq!(enabled.public_url.as_deref(), Some("https://relay.example"));
        assert!(
            resolve(
                &peers,
                super::super::standalone::RelayFlags {
                    no_relay: true,
                    ..Default::default()
                }
            )
            .no_relay
        );
        write(false);
        let disabled = resolve(
            &peers,
            super::super::standalone::RelayFlags {
                relay: true,
                ..Default::default()
            },
        );
        assert!(disabled.no_relay && !disabled.relay);
        assert!(disabled.public_url.is_none());
        std::fs::write(&config, "corrupt").unwrap();
        assert!(resolve(&peers, Default::default()).no_relay);
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "corrupt");
    }
}
