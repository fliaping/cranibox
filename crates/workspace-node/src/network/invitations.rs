//! Short-lived, single-use network invitations. Capability grants are deliberately separate.
use super::config::EasyTier;
use agent_workspace_runtime::config as secrets;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const LIFETIME: u64 = 15 * 60;
const LIMIT: usize = 16;
const FILE_LIMIT: u64 = 512 * 1024;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}
fn binding(spec: &EasyTier) -> String {
    digest(&serde_json::to_vec(&(&spec.network, &spec.token)).expect("serialize strings"))
}
pub(super) fn valid_token(token: &str) -> bool {
    token.len() == 68
        && token.starts_with("cbi_")
        && token[4..].bytes().all(|c| c.is_ascii_hexdigit())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Offer {
    version: u8,
    network: String,
    token: String,
    peers: Vec<String>,
}
impl Offer {
    fn into_spec(self, listen_port: Option<u16>) -> Result<EasyTier> {
        ensure!(
            self.version == 1 && !self.peers.is_empty(),
            "Unsupported network invitation."
        );
        let spec = EasyTier {
            network: self.network,
            token: self.token,
            peer: String::new(),
            peers: self.peers,
            listen_port,
        };
        spec.validate()?;
        Ok(spec)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    hash: String,
    network: String,
    expires_at: u64,
    peers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    redemption: Option<Redemption>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Redemption {
    recovery_hash: String,
    redeemed_at: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u8,
    entries: BTreeMap<String, Entry>,
}
#[derive(Serialize)]
pub(super) struct View {
    pub id: String,
    pub expires_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redeemed_at: Option<u64>,
}
#[derive(Serialize)]
pub(super) struct Issued {
    pub id: String,
    pub token: String,
    pub expires_at: u64,
}
pub(super) struct Store(PathBuf);
impl Store {
    pub fn new(spec_path: &Path) -> Self {
        Self(spec_path.with_file_name("network-invitations.json"))
    }
    fn read(&self) -> Result<Saved> {
        if !self.0.try_exists()? {
            return Ok(Saved {
                version: 1,
                entries: BTreeMap::new(),
            });
        }
        let file = std::fs::File::open(&self.0)?;
        ensure!(
            file.metadata()?.len() <= FILE_LIMIT,
            "Network invitation state is too large."
        );
        let saved: Saved = serde_json::from_reader(file)
            .context("Invalid network-invitations.json; restore it before managing invitations.")?;
        ensure!(
            saved.version == 1 && saved.entries.len() <= LIMIT,
            "Invalid network invitation state."
        );
        for (id, entry) in &saved.entries {
            ensure!(
                id.len() == 32
                    && id.bytes().all(|c| c.is_ascii_hexdigit())
                    && entry.hash.len() == 64
                    && entry.hash.bytes().all(|c| c.is_ascii_hexdigit())
                    && entry.network.len() == 64
                    && entry.network.bytes().all(|c| c.is_ascii_hexdigit())
                    && !entry.peers.is_empty()
                    && entry.redemption.as_ref().is_none_or(|redemption| {
                        valid_digest(&redemption.recovery_hash)
                            && redemption.redeemed_at < entry.expires_at
                    }),
                "Invalid network invitation state."
            );
            EasyTier {
                network: "validation".into(),
                token: "x".repeat(32),
                peer: String::new(),
                peers: entry.peers.clone(),
                listen_port: None,
            }
            .validate()?;
        }
        Ok(saved)
    }
    fn change<T>(&self, action: impl FnOnce(&mut Saved) -> Result<T>) -> Result<T> {
        let _lock = secrets::lock_credentials(&self.0)?;
        let mut saved = self.read()?;
        let result = action(&mut saved)?;
        let bytes = serde_json::to_vec(&saved)?;
        ensure!(
            bytes.len() as u64 <= FILE_LIMIT,
            "Network invitation state is too large."
        );
        secrets::write_private(&self.0, &bytes, true)?;
        Ok(result)
    }
    pub fn list(&self, spec: Option<&EasyTier>) -> Result<Vec<View>> {
        let saved = self.read()?;
        let current = spec.map(binding);
        Ok(saved
            .entries
            .into_iter()
            .filter(|(_, entry)| {
                entry.expires_at > now() && current.as_ref() == Some(&entry.network)
            })
            .map(|(id, entry)| View {
                id,
                expires_at: entry.expires_at,
                redeemed_at: entry.redemption.map(|redemption| redemption.redeemed_at),
            })
            .collect())
    }
    pub fn create(&self, spec: &EasyTier, peers: Vec<String>) -> Result<Issued> {
        let offer = Offer {
            version: 1,
            network: spec.network.clone(),
            token: spec.token.clone(),
            peers: peers.clone(),
        };
        offer.into_spec(None)?;
        let token = format!("cbi_{}", secrets::token());
        let id = uuid::Uuid::new_v4().simple().to_string();
        let expires_at = now() + LIFETIME;
        let network = binding(spec);
        self.change(|saved| {
            saved
                .entries
                .retain(|_, entry| entry.expires_at > now() && entry.network == network);
            ensure!(
                saved.entries.len() < LIMIT,
                "Revoke an invitation before creating another (limit 16, including recoverable invitations)."
            );
            saved.entries.insert(
                id.clone(),
                Entry {
                    hash: digest(token.as_bytes()),
                    network,
                    expires_at,
                    peers,
                    redemption: None,
                },
            );
            Ok(())
        })?;
        Ok(Issued {
            id,
            token,
            expires_at,
        })
    }
    pub fn revoke(&self, id: &str) -> Result<()> {
        self.change(|saved| {
            saved
                .entries
                .remove(id)
                .context("Invitation is no longer active.")?;
            Ok(())
        })
    }
    pub fn clear(&self) -> Result<()> {
        if self.0.try_exists()? {
            self.change(|saved| {
                saved.entries.clear();
                Ok(())
            })?;
        }
        Ok(())
    }
    pub fn redeem(&self, spec: &EasyTier, token: &str, recovery: Option<&str>) -> Result<Offer> {
        ensure!(
            valid_token(token) && recovery.is_none_or(valid_digest),
            "Invitation is invalid, expired or already used."
        );
        let hash = digest(token.as_bytes());
        let network = binding(spec);
        self.change(|saved| {
            let observed_at = now();
            let id = saved
                .entries
                .iter()
                .find(|(_, e)| e.hash == hash && e.network == network && e.expires_at > observed_at)
                .map(|(id, _)| id.clone())
                .context("Invitation is invalid, expired or already used.")?;
            let entry = saved.entries.get_mut(&id).expect("entry found under lock");
            match (&entry.redemption, recovery) {
                (Some(redemption), Some(recovery)) => ensure!(
                    redemption.recovery_hash == digest(recovery.as_bytes()),
                    "Invitation is invalid, expired or already used."
                ),
                (None, Some(recovery)) => {
                    entry.redemption = Some(Redemption {
                        recovery_hash: digest(recovery.as_bytes()),
                        redeemed_at: observed_at,
                    });
                }
                (None, None) => {}
                (Some(_), None) => anyhow::bail!("Invitation is invalid, expired or already used."),
            }
            let peers = entry.peers.clone();
            // Legacy clients still consume their invitation in one step. A new client
            // must present both the invitation and its private recovery secret to retry.
            if recovery.is_none() {
                saved.entries.remove(&id);
            }
            Ok(Offer {
                version: 1,
                network: spec.network.clone(),
                token: spec.token.clone(),
                peers,
            })
        })
    }
}

fn endpoint(address: &str) -> Result<url::Url> {
    let mut url = url::Url::parse(address)
        .map_err(|_| anyhow::anyhow!("Use the Workspace HTTPS address."))?;
    let loopback = url.host_str().is_some_and(|host| {
        host.trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
    });
    ensure!(
        address.len() <= 2048
            && (url.scheme() == "https" || (url.scheme() == "http" && loopback))
            && url.host_str().is_some()
            && url.port_or_known_default().is_some_and(|p| p != 0)
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none(),
        "Use the Workspace HTTPS origin without credentials, a path, a query or a fragment. HTTP is only allowed for a literal loopback address."
    );
    url.set_path("/api/network/bootstrap");
    Ok(url)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Enrollment {
    version: u8,
    binding: String,
    recovery: String,
}
impl Enrollment {
    fn prepare(spec_path: &Path, endpoint: &url::Url, token: &str) -> Result<Self> {
        let path = spec_path.with_file_name("network-enrollment.json");
        let _lock = secrets::lock_credentials(&path)?;
        let binding = digest(&serde_json::to_vec(&(endpoint.as_str(), token))?);
        if path.try_exists()? {
            let file = std::fs::File::open(&path)?;
            ensure!(
                file.metadata()?.len() <= 1024,
                "Network enrollment state is too large."
            );
            let saved: Self = serde_json::from_reader(file).context(
                "Invalid network-enrollment.json; restore it before retrying enrollment.",
            )?;
            ensure!(
                saved.version == 1 && valid_digest(&saved.binding) && valid_digest(&saved.recovery),
                "Invalid network-enrollment.json; existing state was preserved."
            );
            if saved.binding == binding {
                return Ok(saved);
            }
        }
        let saved = Self {
            version: 1,
            binding,
            recovery: secrets::token(),
        };
        secrets::write_private(&path, &serde_json::to_vec(&saved)?, true)?;
        Ok(saved)
    }
}

pub(super) fn clear_enrollment(spec_path: &Path) {
    // The network is already saved; an optional cleanup failure must not report
    // enrollment as failed or undo the working configuration.
    let _ = std::fs::remove_file(spec_path.with_file_name("network-enrollment.json"));
}
pub(super) async fn enroll(
    spec_path: &Path,
    address: &str,
    token: &str,
    listen_port: Option<u16>,
) -> Result<EasyTier> {
    ensure!(
        valid_token(token),
        "Supply a network invitation token beginning with cbi_."
    );
    ensure!(
        listen_port.is_none_or(|p| p >= 1024),
        "Listen port must be between 1024 and 65535."
    );
    let url = endpoint(address)?;
    let enrollment = Enrollment::prepare(spec_path, &url, token)?;
    let mut response = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(10)).build()?.post(url).bearer_auth(token).json(&serde_json::json!({"recovery":enrollment.recovery})).send().await.map_err(|_| anyhow::anyhow!("Could not reach the Workspace invitation endpoint. Retry with the same address and invitation; this device keeps its recovery information."))?;
    ensure!(
        response.status() != reqwest::StatusCode::UNAUTHORIZED,
        "Invitation is invalid, expired or already used. Request a new invitation."
    );
    ensure!(
        response.status() != reqwest::StatusCode::BAD_REQUEST,
        "Workspace rejected recoverable enrollment. Upgrade the inviting Workspace and retry with the same invitation."
    );
    ensure!(
        response.status().is_success(),
        "Workspace invitation endpoint returned HTTP {}.",
        response.status().as_u16()
    );
    ensure!(
        response.content_length().is_none_or(|len| len <= 32768),
        "Invitation response is too large."
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("Read network invitation response")?
    {
        ensure!(
            bytes.len() + chunk.len() <= 32768,
            "Invitation response is too large."
        );
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice::<Offer>(&bytes)
        .map_err(|_| anyhow::anyhow!("Workspace returned an invalid network invitation."))?
        .into_spec(listen_port)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> EasyTier {
        EasyTier {
            network: "home".into(),
            token: "a".repeat(64),
            peer: String::new(),
            peers: vec!["tcp://entry:11010".into()],
            listen_port: None,
        }
    }
    #[test]
    fn invitations_are_private_single_use_expiring_revocable_and_network_bound() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("networks.json");
        let store = Store::new(&path);
        let mut spec = spec();
        let first = store.create(&spec, spec.peers.clone()).unwrap();
        let saved = std::fs::read_to_string(&store.0).unwrap();
        assert!(!saved.contains(&first.token) && !saved.contains(&spec.token));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&store.0).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(Store::new(&path).list(Some(&spec)).unwrap().len(), 1);
        spec.token = "b".repeat(64);
        assert!(store.redeem(&spec, &first.token, None).is_err());
        spec.token = "a".repeat(64);
        assert_eq!(
            store
                .redeem(&spec, &first.token, None)
                .unwrap()
                .into_spec(None)
                .unwrap()
                .peers,
            spec.peers
        );
        assert!(Store::new(&path).redeem(&spec, &first.token, None).is_err());
        let revoked = store.create(&spec, spec.peers.clone()).unwrap();
        store.revoke(&revoked.id).unwrap();
        assert!(store.redeem(&spec, &revoked.token, None).is_err());
        let expired = store.create(&spec, spec.peers.clone()).unwrap();
        store
            .change(|saved| {
                saved.entries.get_mut(&expired.id).unwrap().expires_at = now();
                Ok(())
            })
            .unwrap();
        assert!(store.redeem(&spec, &expired.token, None).is_err());
        assert!(store.list(Some(&spec)).unwrap().is_empty());
        for _ in 0..LIMIT {
            store.create(&spec, spec.peers.clone()).unwrap();
        }
        assert!(store.create(&spec, spec.peers.clone()).is_err());
        store.clear().unwrap();
        assert!(store.list(Some(&spec)).unwrap().is_empty());
        std::fs::write(&store.0, "preserve corrupt state").unwrap();
        assert!(store.create(&spec, spec.peers.clone()).is_err());
        assert_eq!(
            std::fs::read_to_string(&store.0).unwrap(),
            "preserve corrupt state"
        );
    }
    #[test]
    fn invitation_endpoint_never_carries_secrets_or_uses_remote_plaintext() {
        for url in [
            "http://example.com",
            "http://localhost",
            "https://user:secret@example.com",
            "https://example.com/path",
            "https://example.com/?token=secret",
            "https://example.com/#secret",
            "file:///tmp/token",
            "https://host:0",
        ] {
            assert!(endpoint(url).is_err(), "{url}");
        }
        assert_eq!(
            endpoint("https://workspace.example.com").unwrap().as_str(),
            "https://workspace.example.com/api/network/bootstrap"
        );
        assert!(endpoint("http://127.0.0.1:9080").is_ok());
        assert!(endpoint("http://[::1]:9080").is_ok());
    }

    #[test]
    fn recovery_is_private_persistent_revocable_and_does_not_extend_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("networks.json");
        let store = Store::new(&path);
        let mut spec = spec();
        let issued = store.create(&spec, spec.peers.clone()).unwrap();
        let recovery = secrets::token();
        store.redeem(&spec, &issued.token, Some(&recovery)).unwrap();
        let first = store.list(Some(&spec)).unwrap().remove(0);
        assert!(first.redeemed_at.is_some());
        assert_eq!(first.expires_at, issued.expires_at);
        let saved = std::fs::read_to_string(&store.0).unwrap();
        for secret in [&recovery, &issued.token, &spec.token] {
            assert!(!saved.contains(secret));
        }
        assert!(store.redeem(&spec, &issued.token, None).is_err());
        assert!(
            store
                .redeem(&spec, &issued.token, Some(&secrets::token()))
                .is_err()
        );
        assert!(store.redeem(&spec, &issued.token, Some("bad")).is_err());
        let recovered = Store::new(&path)
            .redeem(&spec, &issued.token, Some(&recovery))
            .unwrap()
            .into_spec(None)
            .unwrap();
        assert_eq!(recovered.token, spec.token);
        assert_eq!(recovered.peers, spec.peers);
        let repeated = store.list(Some(&spec)).unwrap().remove(0);
        assert_eq!(first.redeemed_at, repeated.redeemed_at);
        assert_eq!(first.expires_at, repeated.expires_at);
        spec.token = secrets::token();
        assert!(store.redeem(&spec, &issued.token, Some(&recovery)).is_err());
        spec = self::spec();
        store.revoke(&issued.id).unwrap();
        assert!(store.redeem(&spec, &issued.token, Some(&recovery)).is_err());
        let expired = store.create(&spec, spec.peers.clone()).unwrap();
        store
            .redeem(&spec, &expired.token, Some(&recovery))
            .unwrap();
        store
            .change(|saved| {
                let entry = saved.entries.get_mut(&expired.id).unwrap();
                entry.expires_at = now();
                entry.redemption.as_mut().unwrap().redeemed_at = now().saturating_sub(1);
                Ok(())
            })
            .unwrap();
        assert!(
            store
                .redeem(&spec, &expired.token, Some(&recovery))
                .is_err()
        );
        assert!(store.list(Some(&spec)).unwrap().is_empty());
    }

    #[test]
    fn simultaneous_claims_bind_exactly_one_recovery_secret() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("networks.json");
        let spec = spec();
        let issued = Store::new(&path).create(&spec, spec.peers.clone()).unwrap();
        let barrier = std::sync::Barrier::new(2);
        let winners = std::thread::scope(|scope| {
            let claims = (0..2)
                .map(|_| {
                    let (path, spec, token, barrier) = (&path, &spec, &issued.token, &barrier);
                    scope.spawn(move || {
                        let recovery = secrets::token();
                        barrier.wait();
                        Store::new(path)
                            .redeem(spec, token, Some(&recovery))
                            .is_ok()
                    })
                })
                .collect::<Vec<_>>();
            claims
                .into_iter()
                .map(|claim| claim.join().unwrap())
                .filter(|won| *won)
                .count()
        });
        assert_eq!(winners, 1);
    }

    #[test]
    fn client_reuses_its_private_recovery_after_restart_and_preserves_corrupt_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("networks.json");
        let url = endpoint("https://workspace.example.com").unwrap();
        let token = format!("cbi_{}", secrets::token());
        let first = Enrollment::prepare(&path, &url, &token).unwrap();
        let private = path.with_file_name("network-enrollment.json");
        assert!(!std::fs::read_to_string(&private).unwrap().contains(&token));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&private).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(
            Enrollment::prepare(&path, &url, &token).unwrap().recovery,
            first.recovery
        );
        let other_url = endpoint("https://other.example.com").unwrap();
        let other = Enrollment::prepare(&path, &other_url, &token).unwrap();
        assert_ne!(other.recovery, first.recovery);
        let changed =
            Enrollment::prepare(&path, &other_url, &format!("cbi_{}", secrets::token())).unwrap();
        assert_ne!(changed.recovery, other.recovery);
        std::fs::write(&private, "preserve corrupt enrollment").unwrap();
        assert!(Enrollment::prepare(&path, &url, &token).is_err());
        assert_eq!(
            std::fs::read_to_string(&private).unwrap(),
            "preserve corrupt enrollment"
        );
        clear_enrollment(&path);
        assert!(!private.exists());
    }
}
