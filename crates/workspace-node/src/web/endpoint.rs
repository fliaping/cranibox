//! The running interface publishes its actual address and authentication mode.
//! The desktop token is per-launch; never manufacture a different token in the CLI.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
enum Authentication {
    Device { token: String },
    Workspace,
}

#[derive(Serialize, Deserialize)]
struct Endpoint {
    address: SocketAddr,
    instance: String,
    #[serde(flatten)]
    authentication: Authentication,
}

/// Kept alive by the server. A stopping server cannot remove a newer record.
pub struct Registration {
    path: PathBuf,
    bytes: Vec<u8>,
}
impl Registration {
    pub fn device(peers: &Path, address: SocketAddr, token: &str) -> Result<Self> {
        Self::write(
            peers,
            address,
            Authentication::Device {
                token: token.into(),
            },
        )
    }

    pub fn workspace(peers: &Path, address: SocketAddr) -> Result<Self> {
        Self::write(peers, address, Authentication::Workspace)
    }

    fn write(peers: &Path, address: SocketAddr, authentication: Authentication) -> Result<Self> {
        ensure!(
            address.ip().is_loopback() && address.port() != 0,
            "Invalid interface address"
        );
        let path = peers.with_file_name("ui.endpoint.json");
        let bytes = serde_json::to_vec(&Endpoint {
            address,
            instance: uuid::Uuid::new_v4().to_string(),
            authentication,
        })?;
        agent_workspace_runtime::config::write_private(&path, &bytes, true)?;
        Ok(Self { path, bytes })
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        if std::fs::read(&self.path).is_ok_and(|bytes| bytes == self.bytes) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub async fn ui_url(peers: &Path, listen: Option<SocketAddr>) -> Result<String> {
    let path = peers.with_file_name("ui.endpoint.json");
    let endpoint = match std::fs::read(&path) {
        Ok(bytes) => {
            let endpoint: Endpoint = serde_json::from_slice(&bytes)
                .context("Invalid interface record; restart Cranibox")?;
            ensure!(
                listen.is_none_or(|address| address == endpoint.address),
                "--listen does not match the running interface ({})",
                endpoint.address
            );
            endpoint
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Older CLI servers recorded an address and a stable token separately.
            // An explicit address is also supported, but never guesses a port/token.
            let address = match listen {
                Some(address) => address,
                None => std::fs::read_to_string(peers.with_file_name("ui.address"))
                    .context("No running interface recorded. Start or restart Cranibox with this configuration, then retry ui-url")?
                    .trim().parse().context("Invalid recorded interface address")?,
            };
            let token = std::fs::read_to_string(peers.with_file_name("ui.token"))
                .context("No sign-in token recorded; restart Cranibox")?;
            Endpoint {
                address,
                instance: String::new(),
                authentication: Authentication::Device {
                    token: token.trim().into(),
                },
            }
        }
        Err(error) => return Err(error).context("Read the running interface record"),
    };
    ensure!(
        endpoint.address.ip().is_loopback() && endpoint.address.port() != 0,
        "The recorded interface must use a loopback address"
    );
    tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect(endpoint.address),
    )
    .await
    .context("The recorded interface is not responding; restart Cranibox")?
    .context("The recorded interface is not running; restart Cranibox")?;
    match endpoint.authentication {
        Authentication::Workspace => Ok(format!("http://{}/", endpoint.address)),
        Authentication::Device { token } => {
            ensure!(
                token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()),
                "Invalid sign-in token; restart Cranibox"
            );
            Ok(super::standalone::sign_in_url(endpoint.address, &token))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn actual_port_and_live_token_for_gui_cli_and_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let peers = dir.path().join("peers.json");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let token = "a".repeat(64);
        // A stale CLI token must never replace the GUI's current token.
        std::fs::write(peers.with_file_name("ui.token"), "b".repeat(64)).unwrap();
        let first = Registration::device(&peers, address, &token).unwrap();
        assert_eq!(
            ui_url(&peers, None).await.unwrap(),
            super::super::standalone::sign_in_url(address, &token)
        );
        assert!(
            ui_url(&peers, Some("127.0.0.1:1".parse().unwrap()))
                .await
                .is_err()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&first.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let workspace = Registration::workspace(&peers, address).unwrap();
        drop(first);
        assert_eq!(
            ui_url(&peers, None).await.unwrap(),
            format!("http://{address}/")
        );
        drop(listener);
        assert!(ui_url(&peers, None).await.is_err());
        drop(workspace);
        assert!(!peers.with_file_name("ui.endpoint.json").exists());
    }

    #[tokio::test]
    async fn no_guessed_port_or_created_token_and_legacy_is_supported() {
        let dir = tempfile::tempdir().unwrap();
        let peers = dir.path().join("peers.json");
        assert!(ui_url(&peers, None).await.is_err());
        assert!(!peers.with_file_name("ui.token").exists());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        std::fs::write(peers.with_file_name("ui.address"), address.to_string()).unwrap();
        std::fs::write(peers.with_file_name("ui.token"), "a".repeat(64)).unwrap();
        assert!(
            ui_url(&peers, None)
                .await
                .unwrap()
                .contains(&address.to_string())
        );
        std::fs::write(peers.with_file_name("ui.address"), "192.0.2.1:7377").unwrap();
        assert!(ui_url(&peers, None).await.is_err());
    }
}
