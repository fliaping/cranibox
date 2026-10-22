//! Opt-in access to an owner-selected loopback Chromium CDP browser.
use super::{Store, config::Config};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{net::SocketAddr, path::Path, time::Duration};
use tokio_tungstenite::tungstenite;

pub const MAX_MESSAGE: usize = 16 * 1024 * 1024;
/// End-to-end encrypted when opened through a peer session; plain for local CDP.
pub type Socket = super::e2e::socket::SecureSocket;

pub mod manager;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub address: SocketAddr,
}
impl Provider {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.address.ip().is_loopback() && self.address.port() != 0,
            "Browser sharing requires a loopback IP and nonzero port"
        );
        Ok(())
    }
    /// Never follow a provider redirect or dial the address advertised by CDP.
    pub async fn discover(&self) -> Result<(Value, String)> {
        self.validate()?;
        let mut response = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()?
            .get(format!("http://{}/json/version", self.address))
            .send()
            .await?
            .error_for_status()?;
        let mut data = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                data.len() + chunk.len() <= 64 * 1024,
                "CDP metadata too large"
            );
            data.extend_from_slice(&chunk);
        }
        let metadata: Value = serde_json::from_slice(&data).context("invalid CDP metadata")?;
        let advertised = metadata["webSocketDebuggerUrl"]
            .as_str()
            .context("browser CDP endpoint missing")?;
        let url = url::Url::parse(advertised)?;
        let id = url
            .path()
            .strip_prefix("/devtools/browser/")
            .context("browser CDP endpoint required")?;
        ensure!(
            !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
                && url.scheme() == "ws"
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "invalid browser CDP endpoint"
        );
        let version = metadata["Browser"]
            .as_str()
            .context("CDP browser version missing")?;
        ensure!(version.len() <= 256, "CDP version too large");
        Ok((
            json!({"version":version}),
            format!("ws://{}/devtools/browser/{id}", self.address),
        ))
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum OwnerCommand {
    Status {},
    EmergencyStop {},
    Resume {},
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Status {},
    EmergencyStop {},
}

pub async fn owner(path: &Path, command: OwnerCommand) -> Result<Value> {
    if matches!(command, OwnerCommand::EmergencyStop {}) {
        let configuration = Config::update(path, |c| {
            c.browser_paused = true;
            c.browser_epoch = c
                .browser_epoch
                .checked_add(1)
                .context("browser generation exhausted")?;
            Ok(())
        })?;
        return Ok(json!({"paused":true,"configured":configuration.browser.is_some()}));
    }
    let configuration = Config::read(path)?;
    let found = if let Some(provider) = &configuration.browser {
        provider.discover().await.ok()
    } else {
        None
    };
    if matches!(command, OwnerCommand::Resume {}) {
        ensure!(
            found.is_some(),
            "Start the configured Chromium CDP browser before resuming sharing"
        );
        Config::update(path, |c| {
            ensure!(
                c.browser == configuration.browser
                    && c.browser_epoch == configuration.browser_epoch,
                "Browser sharing changed; inspect status before resuming"
            );
            c.browser_paused = false;
            c.browser_epoch = c
                .browser_epoch
                .checked_add(1)
                .context("browser generation exhausted")?;
            Ok(())
        })?;
    }
    let fresh = Config::read(path)?;
    let found = if fresh.browser == configuration.browser {
        found
    } else {
        None
    };
    Ok(
        json!({"paused":fresh.browser_paused,"configured":fresh.browser.is_some(),
        "backend_available":found.is_some(),"browser":found.map(|(metadata, _)| metadata)}),
    )
}

pub async fn connect(
    request: tungstenite::http::Request<()>,
    cipher: Option<super::e2e::stream::Cipher>,
) -> Result<Socket> {
    let config = tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE + super::e2e::stream::OVERHEAD))
        .max_frame_size(Some(MAX_MESSAGE + super::e2e::stream::OVERHEAD));
    let (socket, _) = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async_with_config(request, Some(config), true),
    )
    .await?
    .map_err(|error| match error {
        tungstenite::Error::Http(response) => anyhow::anyhow!(
            "Browser connection rejected (HTTP {}); check target sharing, grant and controller",
            response.status()
        ),
        _ => anyhow::anyhow!("Browser connection unavailable"),
    })?;
    Ok(Socket::new(socket, cipher))
}

impl Store {
    pub async fn browser(&self, node: &str, command: Request) -> Result<Value> {
        self.route(node)?
            .api(node)?
            .post(
                &format!("/api/v1/nodes/{node}/browser"),
                serde_json::to_value(command)?,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_accepts_only_loopback_nonzero_ports_and_typed_commands() {
        for address in ["127.0.0.1:9222", "[::1]:9222"] {
            Provider {
                address: address.parse().unwrap(),
            }
            .validate()
            .unwrap();
        }
        for address in [
            "0.0.0.0:9222",
            "192.168.1.2:9222",
            "[::]:9222",
            "127.0.0.1:0",
        ] {
            assert!(
                Provider {
                    address: address.parse().unwrap()
                }
                .validate()
                .is_err()
            );
        }
        assert!(serde_json::from_value::<Request>(json!({"operation":"resume"})).is_err());
        assert!(
            serde_json::from_value::<Request>(
                json!({"operation":"status","url":"http://example.com"})
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn stop_needs_no_browser_and_provider_changes_do_not_resume() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("peers.json"));
        store.manage(super::super::Command::Identity).await.unwrap();
        let stopped = owner(&store.path, OwnerCommand::EmergencyStop {})
            .await
            .unwrap();
        assert_eq!(stopped["paused"], true);
        let epoch = Config::read(&store.path).unwrap().browser_epoch;
        store
            .manage(super::super::Command::BrowserSharing {
                browser: Some(Provider {
                    address: "127.0.0.1:9222".parse().unwrap(),
                }),
            })
            .await
            .unwrap();
        let current = Config::read(&store.path).unwrap();
        assert!(current.browser_paused);
        assert_eq!(current.browser_epoch, epoch + 1);
        assert!(
            store
                .manage(super::super::Command::BrowserSharing {
                    browser: Some(Provider {
                        address: "0.0.0.0:9222".parse().unwrap()
                    }),
                })
                .await
                .is_err()
        );
        let unchanged = Config::read(&store.path).unwrap();
        assert_eq!(unchanged.browser, current.browser);
        assert_eq!(unchanged.browser_epoch, current.browser_epoch);
    }
}
