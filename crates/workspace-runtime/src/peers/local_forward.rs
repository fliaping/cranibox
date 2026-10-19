//! A persisted loopback route must be owned by a live node-managed forward.
use anyhow::{Context, Result, bail, ensure};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

pub fn path(peers: &Path, node: &str) -> Result<PathBuf> {
    ensure!(
        crate::protocol::valid_node_id(node),
        "invalid forward node ID"
    );
    let parent = peers
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(std::fs::canonicalize(parent)?.join(format!(".network-{node}.lock")))
}

pub struct Lease(File);
impl Lease {
    /// Acquire only after binding the port. Never wait for another owner.
    pub fn acquire(path: &Path) -> Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(path)
            .context("Open local network forward lease")?;
        fs2::FileExt::try_lock_exclusive(&file)
            .context("Local network forward is already owned")?;
        Ok(Self(file))
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

#[derive(Debug)]
pub(crate) struct Unavailable(String);
impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Unavailable {}

pub fn ensure_running(path: &Path) -> Result<()> {
    check_running(path).map_err(|error| Unavailable(error.to_string()).into())
}
fn check_running(path: &Path) -> Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .context("The local network forward is unavailable; start the node and reconnect")?;
    match fs2::FileExt::try_lock_exclusive(&file) {
        Ok(()) => bail!("The local network forward is stopped; start the node and reconnect"),
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => Ok(()),
        Err(error) => Err(error).context("Check local network forward ownership"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_or_missing_lease_never_authorizes_a_saved_loopback_route() {
        let root = tempfile::tempdir().unwrap();
        let path = path(&root.path().join("peers.json"), "remote").unwrap();
        assert!(ensure_running(&path).is_err());
        let lease = Lease::acquire(&path).unwrap();
        ensure_running(&path).unwrap();
        assert!(Lease::acquire(&path).is_err());
        drop(lease);
        assert!(ensure_running(&path).is_err());
        let restored = Lease::acquire(&path).unwrap();
        ensure_running(&path).unwrap();
        drop(restored);
        assert!(ensure_running(&path).is_err());
        assert!(super::path(root.path(), "../escape").is_err());
    }
    #[tokio::test]
    async fn existing_clients_recheck_ownership_before_sending_any_credential() {
        let root = tempfile::tempdir().unwrap();
        let lease_path = path(&root.path().join("peers.json"), "remote").unwrap();
        let lease = Lease::acquire(&lease_path).unwrap();
        let token = root.path().join("target.token");
        crate::config::write_private(&token, crate::config::token().as_bytes(), false).unwrap();
        let route = super::super::config::Route {
            target_identity: None,
            config_path: None,
            alternatives: vec![],
            url: "http://127.0.0.1:9".into(),
            token_file: token,
            allow_insecure: false,
            local_forward_lock: Some(lease_path),
        };
        let api = route.api("remote").unwrap();
        drop(lease);
        assert!(route.api("remote").is_err());
        assert!(
            api.get("/api/v1/nodes")
                .await
                .unwrap_err()
                .to_string()
                .contains("forward is stopped")
        );
        assert!(
            api.terminal("remote", &uuid::Uuid::new_v4().to_string(), 80, 24, false)
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("forward is stopped")
        );
        assert!(
            api.desktop("remote")
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("forward is stopped")
        );
    }

    #[tokio::test]
    async fn multiple_paths_skip_stopped_managed_ports_without_sending_credentials() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let root = tempfile::tempdir().unwrap();
        let lease_path = path(&root.path().join("peers.json"), "remote").unwrap();
        let lease = Lease::acquire(&lease_path).unwrap();
        drop(lease);
        let stale_port = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let available = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let alternative = format!("http://{}", available.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = available.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(stream.read_u8().await.unwrap());
                }
                assert!(
                    String::from_utf8(request)
                        .unwrap()
                        .starts_with("GET /api/v1/mesh/nodes ")
                );
                let body = r#"{"nodes":[{"id":"remote"}]}"#;
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let token = root.path().join("target.token");
        crate::config::write_private(&token, crate::config::token().as_bytes(), false).unwrap();
        let route = super::super::config::Route {
            url: format!("http://{}", stale_port.local_addr().unwrap()),
            token_file: token,
            allow_insecure: false,
            local_forward_lock: Some(lease_path),
            alternatives: vec![super::super::config::Endpoint {
                url: alternative.clone(),
                allow_insecure: false,
                local_forward_lock: None,
            }],
            target_identity: None,
            config_path: None,
        };
        let value = route.api("remote").unwrap().probe().await.unwrap();
        assert_eq!(value["connection"]["url"], alternative);
        assert_eq!(value["connection"]["fallback"], true);
        server.await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), stale_port.accept())
                .await
                .is_err()
        );
    }
}
