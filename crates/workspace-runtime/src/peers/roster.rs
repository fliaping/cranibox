//! Owner-signed device roster. The owner fingerprint is pinned separately from the roster.
mod transfer;
use super::identity::{hex, unhex};
use anyhow::{Context, Result, ensure};
use cranibox_contracts::roster::*;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use fs2::FileExt;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::RwLock,
};
use tokio::sync::watch;
pub use transfer::SealedOwnerKey;

pub fn verify(roster: &Roster, owner: &str) -> Result<()> {
    ensure!(
        roster.version == ROSTER_VERSION && roster.epoch > 0,
        "unsupported roster"
    );
    ensure!(roster.owner_pub == owner, "roster owner changed");
    let key = VerifyingKey::from_bytes(&unhex::<32>(owner)?)?;
    key.verify_strict(
        &roster.signing_message(),
        &Signature::from_bytes(&unhex::<64>(&roster.sig)?),
    )?;
    let mut nodes = BTreeSet::new();
    let mut signing = BTreeSet::new();
    let mut encryption = BTreeSet::new();
    ensure!(
        !roster.members.is_empty() && roster.members.len() <= 256,
        "invalid roster size"
    );
    for member in &roster.members {
        ensure!(
            crate::protocol::valid_node_id(&member.node_id),
            "invalid member ID"
        );
        VerifyingKey::from_bytes(&unhex::<32>(&member.signing_key)?)?;
        ensure!(
            unhex::<32>(&member.x25519_pub)? != [0; 32],
            "invalid encryption key"
        );
        ensure!(
            nodes.insert(&member.node_id)
                && signing.insert(&member.signing_key)
                && encryption.insert(&member.x25519_pub),
            "duplicate roster member or key"
        );
    }
    ensure!(
        roster
            .members
            .iter()
            .filter(|m| m.role == Role::Hub)
            .count()
            <= 1,
        "only one Hub is allowed"
    );
    ensure!(
        roster.members.iter().filter(|m| m.is_claude_node).count() <= 1,
        "only one Claude node is allowed"
    );
    Ok(())
}
pub struct RosterStore {
    path: PathBuf,
    /// The pinned identity; replaced only by [`RosterStore::adopt`].
    owner: RwLock<String>,
    current: RwLock<Option<Roster>>,
    changed: watch::Sender<u64>,
}
impl RosterStore {
    /// The fingerprint must come from local bootstrap or a SAS-verified pairing, never from a received roster.
    pub fn open(path: PathBuf, owner: String) -> Result<Self> {
        let current = match fs::read(&path) {
            Ok(bytes) => {
                let r: Roster = serde_json::from_slice(&bytes)?;
                verify(&r, &owner)?;
                Some(r)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let epoch = current.as_ref().map_or(0, |r| r.epoch);
        Ok(Self {
            path,
            owner: RwLock::new(owner),
            current: RwLock::new(current),
            changed: watch::channel(epoch).0,
        })
    }
    pub fn load(path: PathBuf) -> Result<Self> {
        let owner = fs::read_to_string(path.with_extension("owner"))?;
        Self::open(path, owner.trim().to_owned())
    }
    /// Pin only after the pairing coordinator has verified both displayed SAS values.
    pub fn pin(path: &Path, owner: &str) -> Result<()> {
        unhex::<32>(owner)?;
        let pin = path.with_extension("owner");
        if pin.exists() {
            ensure!(
                fs::read_to_string(&pin)?.trim() == owner,
                "owner already pinned"
            );
            return Ok(());
        }
        crate::config::write_private(&pin, owner.as_bytes(), false)
    }
    pub fn install(&self, roster: Roster) -> Result<Vec<String>> {
        verify(&roster, &self.owner())?;
        let mut guard = self
            .current
            .write()
            .map_err(|_| anyhow::anyhow!("roster lock poisoned"))?;
        fs::create_dir_all(
            self.path
                .parent()
                .context("roster needs parent directory")?,
        )?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.path.with_extension("lock"))?;
        lock.lock_exclusive()?;
        let disk = match fs::read(&self.path) {
            Ok(b) => Some(serde_json::from_slice::<Roster>(&b)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        if let Some(old) = disk.as_ref() {
            verify(old, &self.owner())?;
            ensure!(roster.epoch > old.epoch, "stale roster epoch");
        }
        let removed = disk
            .as_ref()
            .map(|old| {
                old.members
                    .iter()
                    .filter(|m| roster.member(&m.node_id).is_none())
                    .map(|m| m.node_id.clone())
                    .collect()
            })
            .unwrap_or_default();
        let temp = self
            .path
            .with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        crate::config::write_private(&temp, &serde_json::to_vec_pretty(&roster)?, false)?;
        fs::rename(&temp, &self.path)?;
        #[cfg(unix)]
        fs::File::open(self.path.parent().unwrap())?.sync_all()?;
        let epoch = roster.epoch;
        *guard = Some(roster);
        self.changed.send_replace(epoch);
        // Caller must rotate all account DEKs before announcing the new roster.
        Ok(removed)
    }
    /// The pinned owner public key (hex).
    pub fn owner(&self) -> String {
        self.owner.read().map(|o| o.clone()).unwrap_or_default()
    }
    /// Join another identity: replace the pinned owner and the roster in one step. Only
    /// for a roster received over a SAS-verified pairing that names this device.
    pub fn adopt(&self, owner: &str, roster: Roster) -> Result<()> {
        unhex::<32>(owner)?;
        verify(&roster, owner)?;
        let mut guard = self
            .current
            .write()
            .map_err(|_| anyhow::anyhow!("roster lock poisoned"))?;
        let mut pinned = self
            .owner
            .write()
            .map_err(|_| anyhow::anyhow!("roster lock poisoned"))?;
        fs::create_dir_all(
            self.path
                .parent()
                .context("roster needs parent directory")?,
        )?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.path.with_extension("lock"))?;
        lock.lock_exclusive()?;
        crate::config::write_private(&self.path, &serde_json::to_vec_pretty(&roster)?, true)?;
        crate::config::write_private(&self.path.with_extension("owner"), owner.as_bytes(), true)?;
        let epoch = roster.epoch;
        *pinned = owner.to_owned();
        *guard = Some(roster);
        self.changed.send_replace(epoch);
        Ok(())
    }
    pub fn issue(&self, key: &OwnerKey, members: Vec<RosterMember>) -> Result<Vec<String>> {
        ensure!(key.public() == self.owner(), "wrong owner key");
        let epoch = self
            .current()
            .map_or(0, |r| r.epoch)
            .checked_add(1)
            .context("roster epoch exhausted")?;
        let mut roster = Roster {
            version: ROSTER_VERSION,
            owner_pub: self.owner(),
            epoch,
            members,
            sig: String::new(),
        };
        roster.sig = hex(&key.0.sign(&roster.signing_message()).to_bytes());
        self.install(roster)
    }
}
impl RosterView for RosterStore {
    fn current(&self) -> Option<Roster> {
        self.current.read().ok()?.clone()
    }
    fn is_owner_node(&self, node: &str) -> bool {
        self.current().is_some_and(|r| r.member(node).is_some())
    }
    fn hub(&self) -> Option<RosterMember> {
        self.current()?
            .members
            .into_iter()
            .find(|m| m.role == Role::Hub)
    }
    fn claude_node(&self) -> Option<RosterMember> {
        self.current()?
            .members
            .into_iter()
            .find(|m| m.is_claude_node)
    }
    fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }
}
/// Private owner signing seed, stored apart from the public roster. Never included in Debug/JSON.
pub struct OwnerKey(SigningKey);
impl OwnerKey {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                use rand::Rng;
                let seed: [u8; 32] = rand::rng().random();
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                crate::config::write_private(path, &seed, false)?;
                seed.to_vec()
            }
            Err(e) => return Err(e.into()),
        };
        Ok(Self(SigningKey::from_bytes(
            &bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid owner seed"))?,
        )))
    }
    pub fn public(&self) -> String {
        hex(self.0.verifying_key().as_bytes())
    }
    /// For HPKE distribution by the authenticated pairing coordinator only.
    pub fn with_seed<T>(&self, use_seed: impl FnOnce(&[u8; 32]) -> T) -> T {
        use_seed(self.0.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn member() -> RosterMember {
        RosterMember {
            node_id: "device-a".into(),
            signing_key: hex(SigningKey::from_bytes(&[2; 32]).verifying_key().as_bytes()),
            x25519_pub: hex(&[3; 32]),
            role: Role::Hub,
            priority: 0,
            is_claude_node: false,
            name: "My computer".into(),
            added_at: 0,
            identity: None,
        }
    }
    #[test]
    fn signed_roster_persists_and_rejects_replay_or_tampering() {
        let root = tempfile::tempdir().unwrap();
        let key = OwnerKey::load_or_create(&root.path().join("owner.key")).unwrap();
        let path = root.path().join("roster.json");
        RosterStore::pin(&path, &key.public()).unwrap();
        let store = RosterStore::load(path.clone()).unwrap();
        store.issue(&key, vec![member()]).unwrap();
        assert!(store.is_owner_node("device-a"));
        assert!(!store.is_owner_node("guest"));
        let mut roster = store.current().unwrap();
        assert!(store.install(roster.clone()).is_err());
        roster.epoch += 1;
        assert!(store.install(roster).is_err());
        assert_eq!(RosterStore::load(path).unwrap().current().unwrap().epoch, 1);
    }
}
