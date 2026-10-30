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

/// The recovery key's signature on an owner certificate.
pub fn verify_cert(cert: &OwnerCert) -> Result<()> {
    let key = VerifyingKey::from_bytes(&unhex::<32>(&cert.recovery_pub)?)?;
    key.verify_strict(
        &cert.signing_message(),
        &Signature::from_bytes(&unhex::<64>(&cert.sig)?),
    )
    .context("owner certificate signature is invalid")?;
    unhex::<32>(&cert.owner_pub)?;
    Ok(())
}

/// Whether `cert` belongs to the pinned identity `pinned`: the same recovery key, or a
/// newer certificate the pinned key handed over to a new one. Returns the new key for a
/// handover.
pub fn identity_of(cert: &OwnerCert, pinned: &str, serial: u64) -> Result<Option<String>> {
    if cert.recovery_pub == pinned {
        return Ok(None);
    }
    ensure!(
        cert.previous_pub.as_deref() == Some(pinned),
        "owner certificate is for another identity"
    );
    ensure!(cert.serial > serial, "stale owner certificate");
    let key = VerifyingKey::from_bytes(&unhex::<32>(pinned)?)?;
    key.verify_strict(
        &cert.handover_message(),
        &Signature::from_bytes(&unhex::<64>(
            cert.previous_sig.as_deref().unwrap_or_default(),
        )?),
    )
    .context("recovery handover signature is invalid")?;
    Ok(Some(cert.recovery_pub.clone()))
}
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
    /// The pinned owner key; replaced by [`RosterStore::adopt`] or a certified rotation.
    owner: RwLock<String>,
    /// The pinned recovery key (the identity), once the owner created a recovery code.
    identity: RwLock<Option<String>>,
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
        let identity = fs::read_to_string(path.with_extension("identity"))
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        Ok(Self {
            path,
            identity: RwLock::new(identity),
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
    /// Whether `roster` may replace the current one: signed by the pinned owner, or by a
    /// new owner carrying a newer certificate from the pinned recovery key. Returns
    /// `true` for such an owner rotation.
    pub fn check(&self, roster: &Roster) -> Result<bool> {
        let owner = self.owner();
        let identity = self.identity();
        let serial = self
            .current()
            .and_then(|r| r.owner_cert)
            .map_or(0, |c| c.serial);
        if roster.owner_pub == owner {
            verify(roster, &owner)?;
            if let (Some(identity), Some(cert)) = (&identity, &roster.owner_cert) {
                ensure!(
                    cert.owner_pub == owner,
                    "owner certificate is for another identity"
                );
                identity_of(cert, identity, serial)?;
                verify_cert(cert)?;
            }
            return Ok(false);
        }
        let identity = identity.context("roster owner changed")?;
        let cert = roster
            .owner_cert
            .as_ref()
            .context("roster owner changed without a certificate")?;
        ensure!(
            cert.owner_pub == roster.owner_pub,
            "owner certificate is for another identity"
        );
        ensure!(cert.serial > serial, "stale owner certificate");
        identity_of(cert, &identity, serial)?;
        verify_cert(cert)?;
        verify(roster, &roster.owner_pub)?;
        Ok(true)
    }
    /// The pinned recovery key (hex), if this identity has a recovery code.
    pub fn identity(&self) -> Option<String> {
        self.identity.read().ok()?.clone()
    }
    /// Pin the recovery key named by a certificate the current owner holds.
    pub fn pin_identity(&self, recovery_pub: &str) -> Result<()> {
        unhex::<32>(recovery_pub)?;
        if let Some(pinned) = self.identity() {
            ensure!(pinned == recovery_pub, "a different recovery key is pinned");
            return Ok(());
        }
        crate::config::write_private(
            &self.path.with_extension("identity"),
            recovery_pub.as_bytes(),
            true,
        )?;
        *self
            .identity
            .write()
            .map_err(|_| anyhow::anyhow!("roster lock poisoned"))? = Some(recovery_pub.to_owned());
        Ok(())
    }
    pub fn install(&self, roster: Roster) -> Result<Vec<String>> {
        let rotation = self.check(&roster)?;
        // A verified handover to a new recovery key (`check` accepted it).
        let handover = match (self.identity(), &roster.owner_cert) {
            (Some(pinned), Some(cert)) if cert.recovery_pub != pinned => {
                Some(cert.recovery_pub.clone())
            }
            _ => None,
        };
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
        // The trusted owner vouches for its recovery key the first time it shows a
        // certificate (`check` already verified it); later rotations need that key.
        if !rotation
            && self.identity().is_none()
            && let Some(cert) = &roster.owner_cert
        {
            crate::config::write_private(
                &self.path.with_extension("identity"),
                cert.recovery_pub.as_bytes(),
                true,
            )?;
            *self
                .identity
                .write()
                .map_err(|_| anyhow::anyhow!("roster lock poisoned"))? =
                Some(cert.recovery_pub.clone());
        }
        if let Some(identity) = handover {
            crate::config::write_private(
                &self.path.with_extension("identity"),
                identity.as_bytes(),
                true,
            )?;
            *self
                .identity
                .write()
                .map_err(|_| anyhow::anyhow!("roster lock poisoned"))? = Some(identity);
        }
        if rotation {
            crate::config::write_private(
                &self.path.with_extension("owner"),
                roster.owner_pub.as_bytes(),
                true,
            )?;
            *self
                .owner
                .write()
                .map_err(|_| anyhow::anyhow!("roster lock poisoned"))? = roster.owner_pub.clone();
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
        let identity = match &roster.owner_cert {
            Some(cert) => {
                ensure!(
                    cert.owner_pub == owner,
                    "owner certificate is for another key"
                );
                verify_cert(cert)?;
                crate::config::write_private(
                    &self.path.with_extension("identity"),
                    cert.recovery_pub.as_bytes(),
                    true,
                )?;
                Some(cert.recovery_pub.clone())
            }
            None => {
                let _ = fs::remove_file(self.path.with_extension("identity"));
                None
            }
        };
        *self
            .identity
            .write()
            .map_err(|_| anyhow::anyhow!("roster lock poisoned"))? = identity;
        let epoch = roster.epoch;
        *pinned = owner.to_owned();
        *guard = Some(roster);
        self.changed.send_replace(epoch);
        Ok(())
    }
    pub fn issue(&self, key: &OwnerKey, members: Vec<RosterMember>) -> Result<Vec<String>> {
        let roster = self.prepare(key, members)?;
        self.install(roster)
    }
    /// Sign the next roster without installing it; [`Self::install`] commits it later.
    pub fn prepare(&self, key: &OwnerKey, members: Vec<RosterMember>) -> Result<Roster> {
        let cert = self.current().and_then(|r| r.owner_cert);
        self.signed(key, members, cert, Default::default())
    }
    /// Sign a roster with `key`: the current owner, or a new owner certified by `cert`
    /// (a rotation, with the new key sealed to members in `owner_keys`).
    pub fn issue_with(
        &self,
        key: &OwnerKey,
        members: Vec<RosterMember>,
        cert: Option<OwnerCert>,
        owner_keys: std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<Vec<String>> {
        let roster = self.signed(key, members, cert, owner_keys)?;
        self.install(roster)
    }
    fn signed(
        &self,
        key: &OwnerKey,
        members: Vec<RosterMember>,
        cert: Option<OwnerCert>,
        owner_keys: std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<Roster> {
        ensure!(
            key.public() == self.owner()
                || cert.as_ref().is_some_and(|c| c.owner_pub == key.public()),
            "wrong owner key"
        );
        let epoch = self
            .current()
            .map_or(0, |r| r.epoch)
            .checked_add(1)
            .context("roster epoch exhausted")?;
        let mut roster = Roster {
            version: ROSTER_VERSION,
            owner_pub: key.public(),
            epoch,
            members,
            owner_cert: cert,
            owner_keys,
            sig: String::new(),
        };
        roster.sig = hex(&key.0.sign(&roster.signing_message()).to_bytes());
        Ok(roster)
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

#[cfg(test)]
mod rotation_tests {
    use super::*;
    use ed25519_dalek::Signer;

    fn member(n: u8) -> RosterMember {
        let signing = SigningKey::from_bytes(&[n; 32]);
        RosterMember {
            node_id: format!("node-{:032x}", n),
            signing_key: hex(signing.verifying_key().as_bytes()),
            x25519_pub: hex(&[n.wrapping_add(100); 32]),
            role: Role::Node,
            priority: 128,
            is_claude_node: false,
            name: format!("Device {n}"),
            added_at: 1,
            identity: None,
        }
    }
    fn cert(recovery: &SigningKey, owner: &OwnerKey, serial: u64) -> OwnerCert {
        let mut cert = OwnerCert {
            previous_pub: None,
            previous_sig: None,
            recovery_pub: hex(recovery.verifying_key().as_bytes()),
            owner_pub: owner.public(),
            serial,
            recovery_salt: None,
            sig: String::new(),
        };
        cert.sig = hex(&recovery.sign(&cert.signing_message()).to_bytes());
        cert
    }

    #[test]
    fn only_the_recovery_key_can_replace_the_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roster.json");
        let old = OwnerKey::load_or_create(&dir.path().join("old.key")).unwrap();
        RosterStore::pin(&path, &old.public()).unwrap();
        let store = RosterStore::load(path.clone()).unwrap();
        store
            .issue(&old, vec![member(1), member(2), member(3)])
            .unwrap();
        let new = OwnerKey::load_or_create(&dir.path().join("new.key")).unwrap();
        // Without a pinned recovery key the owner can never change.
        let rogue = RosterStore::load(path.clone()).unwrap();
        assert!(
            rogue
                .issue_with(&new, vec![member(1)], None, Default::default())
                .is_err()
        );

        let recovery = SigningKey::from_bytes(&[42u8; 32]);
        store
            .pin_identity(&hex(recovery.verifying_key().as_bytes()))
            .unwrap();
        store
            .issue_with(
                &old,
                vec![member(1), member(2), member(3)],
                Some(cert(&recovery, &old, 1)),
                Default::default(),
            )
            .unwrap();
        // A certificate from another recovery key is refused.
        let thief = SigningKey::from_bytes(&[7u8; 32]);
        assert!(
            store
                .issue_with(
                    &new,
                    vec![member(1)],
                    Some(cert(&thief, &new, 2)),
                    Default::default()
                )
                .is_err()
        );
        // The real recovery key rotates the owner and drops a lost device.
        let removed = store
            .issue_with(
                &new,
                vec![member(1), member(2)],
                Some(cert(&recovery, &new, 2)),
                Default::default(),
            )
            .unwrap();
        assert_eq!(removed, vec![member(3).node_id]);
        assert_eq!(store.owner(), new.public());
        assert_eq!(
            fs::read_to_string(path.with_extension("owner")).unwrap(),
            new.public()
        );
        // The old owner key (kept by the lost device) can no longer issue rosters...
        assert!(
            store
                .issue(&old, vec![member(1), member(2), member(3)])
                .is_err()
        );
        // ...nor can an old certificate be replayed.
        let mut replay = store.current().unwrap();
        replay.owner_pub = old.public();
        replay.owner_cert = Some(cert(&recovery, &old, 1));
        replay.epoch += 1;
        replay.sig = hex(&old.0.sign(&replay.signing_message()).to_bytes());
        assert!(store.check(&replay).is_err(), "stale certificate");
        // A restart keeps the pins.
        let reopened = RosterStore::load(path).unwrap();
        assert_eq!(reopened.owner(), new.public());
        assert!(reopened.identity().is_some());
    }

    #[test]
    fn the_recovery_key_hands_the_identity_to_a_new_one_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roster.json");
        let owner = OwnerKey::load_or_create(&dir.path().join("owner.key")).unwrap();
        RosterStore::pin(&path, &owner.public()).unwrap();
        let store = RosterStore::load(path.clone()).unwrap();
        let old = SigningKey::from_bytes(&[41; 32]);
        let new = SigningKey::from_bytes(&[42; 32]);
        let other = SigningKey::from_bytes(&[43; 32]);
        store
            .issue_with(
                &owner,
                vec![member(1)],
                Some(cert(&old, &owner, 1)),
                Default::default(),
            )
            .unwrap();
        assert_eq!(store.identity(), Some(hex(old.verifying_key().as_bytes())));
        let handover = |by: &SigningKey, to: &SigningKey, serial: u64| {
            let mut c = cert(to, &owner, serial);
            c.previous_pub = Some(hex(old.verifying_key().as_bytes()));
            c.previous_sig = Some(hex(&by.sign(&c.handover_message()).to_bytes()));
            c.sig = String::new();
            c.sig = hex(&to.sign(&c.signing_message()).to_bytes());
            c
        };
        // Not signed by the pinned key, or not newer: refused.
        assert!(
            store
                .issue_with(
                    &owner,
                    vec![member(1)],
                    Some(handover(&other, &new, 2)),
                    Default::default()
                )
                .is_err()
        );
        assert!(
            store
                .issue_with(
                    &owner,
                    vec![member(1)],
                    Some(handover(&old, &new, 1)),
                    Default::default()
                )
                .is_err()
        );
        let mut bare = cert(&new, &owner, 2);
        bare.previous_pub = None;
        assert!(
            store
                .issue_with(&owner, vec![member(1)], Some(bare), Default::default())
                .is_err()
        );
        store
            .issue_with(
                &owner,
                vec![member(1)],
                Some(handover(&old, &new, 2)),
                Default::default(),
            )
            .unwrap();
        assert_eq!(store.identity(), Some(hex(new.verifying_key().as_bytes())));
        // The re-pin is on disk, and the old key can no longer hand anything over.
        let reopened = RosterStore::load(path).unwrap();
        assert_eq!(
            reopened.identity(),
            Some(hex(new.verifying_key().as_bytes()))
        );
        assert!(
            reopened
                .issue_with(
                    &owner,
                    vec![member(1)],
                    Some(handover(&old, &other, 3)),
                    Default::default()
                )
                .is_err()
        );
    }
}
