//! Opt-in access to an owner-selected loopback Chromium CDP browser.
use super::{Store, config::Config};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio_tungstenite::tungstenite;

pub const MAX_MESSAGE: usize = 16 * 1024 * 1024;
/// Long enough for the owner to approve Chrome's "allow remote debugging" prompt,
/// which holds the handshake of a connection to their everyday profile.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
/// End-to-end encrypted when opened through a peer session; plain for local CDP.
pub type Socket = super::e2e::socket::SecureSocket;

pub mod manager;

/// The Chromium CDP endpoint shared with paired devices.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged, deny_unknown_fields)]
pub enum Provider {
    /// A browser started with `--remote-debugging-port` (e.g. the managed browser).
    Address { address: SocketAddr },
    /// The owner's everyday Chrome (144+), after they allow remote debugging at
    /// `chrome://inspect/#remote-debugging`. Chrome picks a new port on every launch
    /// and records it in `DevToolsActivePort`; it serves no `/json/version` in this
    /// mode and asks the owner to allow each new CDP connection.
    Profile { user_data_dir: PathBuf },
}
impl Provider {
    pub fn at(address: SocketAddr) -> Self {
        Self::Address { address }
    }
    /// The fixed loopback address, when the provider has one.
    pub fn address(&self) -> Option<SocketAddr> {
        match self {
            Self::Address { address } => Some(*address),
            Self::Profile { .. } => None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Address { address } => ensure!(
                address.ip().is_loopback() && address.port() != 0,
                "Browser sharing requires a loopback IP and nonzero port"
            ),
            Self::Profile { user_data_dir } => ensure!(
                user_data_dir.is_absolute()
                    && user_data_dir.as_os_str().len() <= 4096
                    && user_data_dir.is_dir(),
                "Browser sharing requires an existing absolute Chrome profile directory"
            ),
        }
        Ok(())
    }
    /// Never follow a provider redirect or dial the address advertised by CDP.
    pub async fn discover(&self) -> Result<(Value, String)> {
        self.validate()?;
        match self {
            Self::Address { address } => discover_address(*address).await,
            Self::Profile { user_data_dir } => discover_profile(user_data_dir).await,
        }
    }
}

fn valid_browser_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}

async fn discover_address(address: SocketAddr) -> Result<(Value, String)> {
    let mut response = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()?
        .get(format!("http://{address}/json/version"))
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
        valid_browser_id(id)
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
        format!("ws://{address}/devtools/browser/{id}"),
    ))
}

/// Read the live endpoint from the profile without opening a CDP session, which
/// would make Chrome prompt the owner. A TCP probe drops a file left by a crash.
async fn discover_profile(dir: &Path) -> Result<(Value, String)> {
    let (port, id) = active_port(dir)?;
    let address = SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port));
    tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect(address),
    )
    .await
    .context("Chrome remote debugging is not answering")?
    .context("Chrome remote debugging is not answering")?;
    let version = std::fs::read_to_string(dir.join("Last Version"))
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty() && v.len() <= 64 && v.bytes().all(|b| b.is_ascii_graphic()));
    Ok((
        json!({"version":version.map_or("Chrome".into(), |v| format!("Chrome/{v}")),"profile":true}),
        format!("ws://{address}/devtools/browser/{id}"),
    ))
}

/// `DevToolsActivePort`: line 1 is the port, line 2 `/devtools/browser/<id>`.
pub fn active_port(dir: &Path) -> Result<(u16, String)> {
    let path = dir.join("DevToolsActivePort");
    let file = std::fs::File::open(&path).context(
        "Chrome remote debugging is off; allow it at chrome://inspect/#remote-debugging",
    )?;
    ensure!(
        file.metadata()?.len() <= 1024,
        "DevToolsActivePort is too large"
    );
    let text = std::io::read_to_string(file)?;
    let mut lines = text.lines();
    let port: u16 = lines
        .next()
        .and_then(|l| l.trim().parse().ok())
        .filter(|p| *p != 0)
        .context("invalid DevToolsActivePort")?;
    let id = lines
        .next()
        .and_then(|l| l.trim().strip_prefix("/devtools/browser/"))
        .filter(|id| valid_browser_id(id))
        .context("invalid DevToolsActivePort")?;
    Ok((port, id.to_owned()))
}

/// Default profile directories of installed Chromium browsers that support the
/// remote-debugging permission, most common first.
pub fn profile_candidates() -> Vec<(&'static str, PathBuf)> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return vec![];
    };
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let list: Vec<(&'static str, PathBuf)> = if cfg!(target_os = "macos") {
        let base = home.join("Library/Application Support");
        vec![
            ("chrome", base.join("Google/Chrome")),
            ("chrome-beta", base.join("Google/Chrome Beta")),
            ("chrome-canary", base.join("Google/Chrome Canary")),
            ("edge", base.join("Microsoft Edge")),
            ("brave", base.join("BraveSoftware/Brave-Browser")),
            ("chromium", base.join("Chromium")),
        ]
    } else if cfg!(windows) {
        let Some(base) = local else { return vec![] };
        vec![
            ("chrome", base.join("Google/Chrome/User Data")),
            ("chrome-beta", base.join("Google/Chrome Beta/User Data")),
            ("edge", base.join("Microsoft/Edge/User Data")),
            ("brave", base.join("BraveSoftware/Brave-Browser/User Data")),
        ]
    } else {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        vec![
            ("chrome", base.join("google-chrome")),
            ("chrome-beta", base.join("google-chrome-beta")),
            ("edge", base.join("microsoft-edge")),
            ("brave", base.join("BraveSoftware/Brave-Browser")),
            ("chromium", base.join("chromium")),
        ]
    };
    list.into_iter().filter(|(_, p)| p.is_dir()).collect()
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
        CONNECT_TIMEOUT,
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
            Provider::at(address.parse().unwrap()).validate().unwrap();
        }
        for address in [
            "0.0.0.0:9222",
            "192.168.1.2:9222",
            "[::]:9222",
            "127.0.0.1:0",
        ] {
            assert!(Provider::at(address.parse().unwrap()).validate().is_err());
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
    async fn profile_provider_reads_the_live_port_without_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let provider = Provider::Profile {
            user_data_dir: dir.path().to_path_buf(),
        };
        let json = serde_json::to_value(&provider).unwrap();
        assert_eq!(json, json!({"user_data_dir": dir.path()}));
        assert_eq!(serde_json::from_value::<Provider>(json).unwrap(), provider);
        assert_eq!(
            serde_json::from_value::<Provider>(json!({"address":"127.0.0.1:9222"})).unwrap(),
            Provider::at("127.0.0.1:9222".parse().unwrap())
        );
        assert!(
            serde_json::from_value::<Provider>(
                json!({"address":"127.0.0.1:9222","user_data_dir":"/x"})
            )
            .is_err()
        );
        assert!(
            Provider::Profile {
                user_data_dir: "relative".into()
            }
            .validate()
            .is_err()
        );

        // Remote debugging off: no port file.
        let off = provider.discover().await.unwrap_err();
        assert!(format!("{off:#}").contains("chrome://inspect"), "{off:#}");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let file = dir.path().join("DevToolsActivePort");
        std::fs::write(&file, format!("{port}\n/devtools/browser/abc-123\n")).unwrap();
        std::fs::write(dir.path().join("Last Version"), "144.0.7559.3\n").unwrap();
        let (metadata, endpoint) = provider.discover().await.unwrap();
        assert_eq!(
            endpoint,
            format!("ws://127.0.0.1:{port}/devtools/browser/abc-123")
        );
        assert_eq!(metadata["version"], "Chrome/144.0.7559.3");

        for bad in [
            format!("{port}\n/devtools/page/abc\n"),
            format!("{port}\n/devtools/browser/a b\n"),
            "0\n/devtools/browser/abc\n".into(),
            "x\n".into(),
        ] {
            std::fs::write(&file, bad).unwrap();
            assert!(provider.discover().await.is_err());
        }
        // A file left behind by a crashed Chrome.
        std::fs::write(&file, format!("{port}\n/devtools/browser/abc\n")).unwrap();
        drop(listener);
        // A parallel test can be handed the freed port; only assert when nothing took it.
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err()
        {
            assert!(provider.discover().await.is_err());
        }
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
                browser: Some(Provider::at("127.0.0.1:9222".parse().unwrap())),
            })
            .await
            .unwrap();
        let current = Config::read(&store.path).unwrap();
        assert!(current.browser_paused);
        assert_eq!(current.browser_epoch, epoch + 1);
        assert!(
            store
                .manage(super::super::Command::BrowserSharing {
                    browser: Some(Provider::at("0.0.0.0:9222".parse().unwrap())),
                })
                .await
                .is_err()
        );
        let unchanged = Config::read(&store.path).unwrap();
        assert_eq!(unchanged.browser, current.browser);
        assert_eq!(unchanged.browser_epoch, current.browser_epoch);
    }
}
