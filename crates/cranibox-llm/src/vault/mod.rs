//! Durable encrypted C7 account store. SQLite contains signed ciphertext, never credentials.
pub mod coordinated;
pub mod crypto;
pub mod keys;
pub mod lease;
pub mod sync;
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use cranibox_contracts::{
    canonical,
    llm::{AccountKind, AccountState, AccountSummary, SecretBox},
    roster::{Roster, RosterView},
    vault::*,
};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;
/// Keep the cause: database, signature and roster errors name no secret, and a
/// bare "failed" hides which of them a rare failure was.
fn other(cause: impl std::fmt::Display) -> StoreError {
    StoreError::Other(format!("vault operation failed: {cause}"))
}
pub(crate) fn now() -> u64 {
    crate::auth::now()
}
/// The runtime can supply its existing identity without exposing the signing seed.
type RecordSigner = Arc<dyn Fn(&[u8]) -> String + Send + Sync>;
pub struct VaultKeys {
    pub node: String,
    pub signing_public: String,
    encryption: SecretBox,
    signer: RecordSigner,
}
impl VaultKeys {
    pub fn new(
        node: String,
        signing_public: String,
        encryption: [u8; 32],
        signer: RecordSigner,
    ) -> Self {
        Self {
            node,
            signing_public,
            encryption: SecretBox::new(encryption.to_vec()),
            signer,
        }
    }
    pub fn from_seed(node: String, signing: [u8; 32], encryption: [u8; 32]) -> Self {
        let key = SigningKey::from_bytes(&signing);
        let public = hex::encode(key.verifying_key().as_bytes());
        Self::new(
            node,
            public,
            encryption,
            Arc::new(move |m| hex::encode(key.sign(m).to_bytes())),
        )
    }
    pub fn encryption_public(&self) -> String {
        use hpke::{Deserializable, Kem, Serializable};
        type K = hpke::kem::X25519HkdfSha256;
        let sk = <K as Kem>::PrivateKey::from_bytes(self.encryption.expose()).expect("32 byte key");
        hex::encode(K::sk_to_pk(&sk).to_bytes())
    }
}
// Metadata is inside the authenticated envelope so account states survive sync without extending C7.
#[derive(Serialize, Deserialize)]
struct Envelope {
    secret: String,
    plan: Option<String>,
    state: AccountState,
    lease_epoch: u64,
    #[serde(default)]
    deleted: bool,
}
impl Drop for Envelope {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.secret.zeroize();
    }
}
pub struct Vault {
    db: Mutex<Connection>,
    keys: VaultKeys,
    roster: Arc<dyn RosterView>,
}
impl Vault {
    pub fn open(
        path: &Path,
        keys: VaultKeys,
        roster: Arc<dyn RosterView>,
    ) -> Result<Self, StoreError> {
        let parent = path.parent().ok_or_else(|| other("parent"))?;
        std::fs::create_dir_all(parent).map_err(other)?;
        if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(other("symlink"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                .map_err(other)?;
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(path)
                .map_err(other)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(other)?;
        }
        let db = Connection::open(path).map_err(other)?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(other)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA secure_delete=ON; CREATE TABLE IF NOT EXISTS records(key TEXT PRIMARY KEY, value TEXT NOT NULL, lease_epoch INTEGER NOT NULL DEFAULT 0); CREATE TABLE IF NOT EXISTS epochs(key TEXT PRIMARY KEY, value INTEGER NOT NULL);").map_err(other)?;
        let vault = Self {
            db: Mutex::new(db),
            keys,
            roster,
        };
        vault.current_roster()?;
        Ok(vault)
    }
    pub fn node(&self) -> &str {
        &self.keys.node
    }
    fn current_roster(&self) -> Result<Roster, StoreError> {
        let r = self.roster.current().ok_or_else(|| other("no roster"))?;
        let member = r.member(&self.keys.node).ok_or(StoreError::NoSecret)?;
        if member.signing_key != self.keys.signing_public
            || member.x25519_pub != self.keys.encryption_public()
        {
            return Err(other("identity mismatch"));
        }
        Ok(r)
    }
    fn read(db: &Connection, key: &str) -> Result<Option<SignedRecord>, StoreError> {
        let text: Option<String> = db
            .query_row("SELECT value FROM records WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()
            .map_err(other)?;
        text.map(|s| serde_json::from_str(&s).map_err(other))
            .transpose()
    }
    fn save(db: &Connection, record: &SignedRecord, epoch: u64) -> Result<(), StoreError> {
        db.execute("INSERT INTO records(key,value,lease_epoch) VALUES(?1,?2,?3) ON CONFLICT(key) DO UPDATE SET value=excluded.value,lease_epoch=excluded.lease_epoch",params![record.key(),serde_json::to_string(record).map_err(other)?,epoch]).map_err(other)?;
        Ok(())
    }
    fn epoch_floor(db: &Connection, id: &str) -> Result<u64, StoreError> {
        db.query_row("SELECT value FROM epochs WHERE key=?1", [id], |r| r.get(0))
            .optional()
            .map(|v| v.unwrap_or(0))
            .map_err(other)
    }
    fn remember_epoch(db: &Connection, id: &str, epoch: u64) -> Result<(), StoreError> {
        db.execute("INSERT INTO epochs(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=MAX(value,excluded.value)",params![id,epoch]).map_err(other)?;
        Ok(())
    }
    fn invalidate_lease(db: &Connection, id: &str) -> Result<(), StoreError> {
        if let Some(SignedRecord::Lease(l)) = Self::read(db, &format!("lease:{id}"))? {
            Self::remember_epoch(db, id, l.epoch)?;
        }
        let epoch: Option<u64> = db
            .query_row(
                "SELECT lease_epoch FROM records WHERE key=?1",
                [format!("account:{id}")],
                |r| r.get(0),
            )
            .optional()
            .map_err(other)?;
        Self::remember_epoch(db, id, epoch.unwrap_or(0))?;
        db.execute("DELETE FROM records WHERE key=?1", [format!("lease:{id}")])
            .map_err(other)?;
        Ok(())
    }
    fn envelope(&self, a: &Account) -> Result<Envelope, StoreError> {
        let plain = crypto::open(a, &self.keys)?;
        serde_json::from_slice(&plain).map_err(other)
    }
    pub fn account(&self, id: &str) -> Result<Account, StoreError> {
        let db = self.db.lock().map_err(other)?;
        match Self::read(&db, &format!("account:{id}"))? {
            Some(SignedRecord::Account(a)) => Ok(a),
            _ => Err(StoreError::NotFound),
        }
    }
    pub fn enablement(&self, id: &str) -> Result<Enablement, StoreError> {
        let db = self.db.lock().map_err(other)?;
        Self::enabled_record(&db, id)
    }
    fn enabled_record(db: &Connection, id: &str) -> Result<Enablement, StoreError> {
        match Self::read(db, &format!("enablement:{id}"))? {
            Some(SignedRecord::Enablement(e)) => Ok(e),
            _ => Err(StoreError::NotFound),
        }
    }
    pub fn lease(&self, id: &str) -> Result<Option<Lease>, StoreError> {
        let db = self.db.lock().map_err(other)?;
        match Self::read(&db, &format!("lease:{id}"))? {
            Some(SignedRecord::Lease(l)) => Ok(Some(l)),
            _ => Ok(None),
        }
    }
    fn summary(&self, a: &Account, e: &Enablement) -> Result<AccountSummary, StoreError> {
        let envelope = self.envelope(a)?;
        if envelope.deleted {
            return Err(StoreError::NotFound);
        }
        let r = self.current_roster()?;
        Ok(AccountSummary {
            id: a.id.clone(),
            provider: a.provider.clone(),
            kind: a.kind,
            plan: envelope.plan.clone(),
            label: a.label.clone(),
            home_node: a.home_node.clone(),
            sync: a.sync == SyncPolicy::Synced,
            enabled_on: if e.all_nodes {
                r.members.iter().map(|m| m.node_id.clone()).collect()
            } else {
                e.enabled_on
                    .iter()
                    .filter(|n| r.member(n).is_some())
                    .cloned()
                    .collect()
            },
            all_nodes: e.all_nodes,
            state: envelope.state.clone(),
        })
    }
    fn signed(&self, mut record: SignedRecord) -> Result<SignedRecord, StoreError> {
        let message = signing_message(&record)?;
        let sig = (self.keys.signer)(&message);
        match &mut record {
            SignedRecord::Account(a) => a.sig = sig,
            SignedRecord::Enablement(e) => e.sig = sig,
            SignedRecord::Lease(l) => l.sig = sig,
        };
        Ok(record)
    }
    /// New roster members receive new wraps; removed devices cannot decrypt future records.
    /// A fresh DEK and nonce are used even if the credential itself has not changed.
    pub fn rotate_for_roster(&self) -> Result<(), StoreError> {
        let roster = self.current_roster()?;
        let mut db = self.db.lock().map_err(other)?;
        let tx = db.transaction().map_err(other)?;
        let accounts = all_records(&tx)?
            .into_iter()
            .filter_map(|r| {
                if let SignedRecord::Account(a) = r {
                    Some(a)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for mut a in accounts {
            if a.sync == SyncPolicy::NeverSync && a.home_node != self.node() {
                continue;
            }
            let plain = crypto::open(&a, &self.keys)?;
            if roster.member(&a.home_node).is_none() {
                a.home_node = self.node().into();
            }
            a.version = a.version.checked_add(1).ok_or_else(|| other("version"))?;
            a.writer = self.node().into();
            a.updated_at = now();
            crypto::seal(&mut a, &plain, &roster)?;
            let epoch = self.envelope(&a)?.lease_epoch;
            let id = a.id.clone();
            Self::save(&tx, &self.signed(SignedRecord::Account(a))?, epoch)?;
            let mut e = Self::enabled_record(&tx, &id)?;
            if e.enabled_on.iter().any(|n| roster.member(n).is_none()) {
                e.enabled_on.retain(|n| roster.member(n).is_some());
                e.version = e.version.checked_add(1).ok_or_else(|| other("version"))?;
                e.writer = self.node().into();
                Self::save(&tx, &self.signed(SignedRecord::Enablement(e.clone()))?, 0)?;
            }
            if lease::enabled(&e, &roster).len() < 2 {
                Self::invalidate_lease(&tx, &id)?;
            }
        }
        tx.commit().map_err(other)?;
        Ok(())
    }
    pub fn pull(&self, request: PullReq) -> Result<PullResp, StoreError> {
        self.current_roster()?;
        let db = self.db.lock().map_err(other)?;
        let records = all_records(&db)?;
        let synced: BTreeSet<_> = records
            .iter()
            .filter_map(|r| match r {
                SignedRecord::Account(a) if a.sync == SyncPolicy::Synced => Some(a.id.clone()),
                _ => None,
            })
            .collect();
        Ok(PullResp {
            records: records
                .into_iter()
                .filter(|r| {
                    let (id, version) = match r {
                        SignedRecord::Account(a) => (&a.id, a.version),
                        SignedRecord::Enablement(e) => (&e.account_id, e.version),
                        SignedRecord::Lease(l) => (&l.account_id, l.epoch),
                    };
                    synced.contains(id) && request.since.get(&r.key()).is_none_or(|v| *v <= version)
                })
                .collect(),
        })
    }
    /// Verify the complete bundle before beginning an atomic SQLite transaction.
    pub fn push(&self, push: Push) -> Result<(), StoreError> {
        if push.records.len() > 4096 {
            return Err(other("too many records"));
        }
        let roster = self.current_roster()?;
        for record in &push.records {
            verify_record(record, &roster)?;
        }
        let mut db = self.db.lock().map_err(other)?;
        let tx = db.transaction().map_err(other)?;
        let mut records = push.records;
        records.sort_by_key(|r| match r {
            SignedRecord::Account(_) => 0,
            SignedRecord::Enablement(_) => 1,
            SignedRecord::Lease(_) => 2,
        });
        for record in records {
            let old = Self::read(&tx, &record.key())?;
            let mut epoch = 0;
            let newer = match (&record, &old) {
                (SignedRecord::Account(a), old) => {
                    if a.sync != SyncPolicy::Synced || a.kind == AccountKind::ClaudeOAuth {
                        return Err(other("never-sync account"));
                    }
                    if a.wraps.keys().any(|n| roster.member(n).is_none())
                        || roster
                            .members
                            .iter()
                            .any(|m| !a.wraps.contains_key(&m.node_id))
                    {
                        return Err(other("incorrect recipients"));
                    }
                    let envelope = self.envelope(a)?;
                    epoch = envelope.lease_epoch;
                    if let Some(SignedRecord::Account(previous)) = old {
                        if a.kind != previous.kind
                            || a.provider != previous.provider
                            || (a.home_node != previous.home_node
                                && roster.member(&previous.home_node).is_some())
                        {
                            return Err(other("immutable account metadata changed"));
                        }
                        if a.kind == AccountKind::OAuth {
                            (a.generation, epoch, a.version, a.updated_at, &a.writer)
                                > (
                                    previous.generation,
                                    self.envelope(previous)?.lease_epoch,
                                    previous.version,
                                    previous.updated_at,
                                    &previous.writer,
                                )
                        } else {
                            (a.version, a.updated_at, &a.writer)
                                > (previous.version, previous.updated_at, &previous.writer)
                        }
                    } else {
                        true
                    }
                }
                (SignedRecord::Enablement(e), old) => {
                    let Some(SignedRecord::Account(a)) =
                        Self::read(&tx, &format!("account:{}", e.account_id))?
                    else {
                        return Err(StoreError::NotFound);
                    };
                    if a.sync != SyncPolicy::Synced
                        || e.enabled_on.iter().any(|n| roster.member(n).is_none())
                    {
                        return Err(other("invalid enablement"));
                    }
                    !matches!(old,Some(SignedRecord::Enablement(p)) if (e.version,&e.writer)<=(p.version,&p.writer))
                }
                (SignedRecord::Lease(l), old) => {
                    if old.is_none() && l.epoch <= Self::epoch_floor(&tx, &l.account_id)? {
                        continue;
                    }
                    lease::validate(self, &tx, l, &roster, now())?;
                    // Higher epochs reconcile a partition even if the old holder renewed locally.
                    !matches!(old,Some(SignedRecord::Lease(p)) if (l.epoch,l.expires_at,&l.holder)<=(p.epoch,p.expires_at,&p.holder))
                }
            };
            if newer {
                Self::save(&tx, &record, epoch)?;
                match &record {
                    SignedRecord::Lease(l) => Self::remember_epoch(&tx, &l.account_id, l.epoch)?,
                    SignedRecord::Enablement(e) if lease::enabled(e, &roster).len() <= 1 => {
                        Self::invalidate_lease(&tx, &e.account_id)?
                    }
                    _ => {}
                }
            }
        }
        tx.commit().map_err(other)?;
        Ok(())
    }
}
fn all_records(db: &Connection) -> Result<Vec<SignedRecord>, StoreError> {
    let mut stmt = db
        .prepare("SELECT value FROM records ORDER BY key")
        .map_err(other)?;
    stmt.query_map([], |r| r.get::<_, String>(0))
        .map_err(other)?
        .map(|r| serde_json::from_str(&r.map_err(other)?).map_err(other))
        .collect()
}
fn signing_message(record: &SignedRecord) -> Result<Vec<u8>, StoreError> {
    let mut unsigned = record.clone();
    match &mut unsigned {
        SignedRecord::Account(a) => a.sig.clear(),
        SignedRecord::Enablement(e) => e.sig.clear(),
        SignedRecord::Lease(l) => l.sig.clear(),
    };
    canonical::signing_message(VAULT_SIG_DOMAIN, &unsigned).map_err(other)
}
fn verify_record(record: &SignedRecord, roster: &Roster) -> Result<(), StoreError> {
    let (writer, sig) = match record {
        SignedRecord::Account(a) => (&a.writer, &a.sig),
        SignedRecord::Enablement(e) => (&e.writer, &e.sig),
        SignedRecord::Lease(l) => (&l.holder, &l.sig),
    };
    let member = roster
        .member(writer)
        .ok_or_else(|| other("non-member signature"))?;
    let public: [u8; 32] = hex::decode(&member.signing_key)
        .map_err(other)?
        .try_into()
        .map_err(|_| other("key"))?;
    let signature: [u8; 64] = hex::decode(sig)
        .map_err(other)?
        .try_into()
        .map_err(|_| other("signature"))?;
    VerifyingKey::from_bytes(&public)
        .map_err(other)?
        .verify_strict(
            &signing_message(record)?,
            &Signature::from_bytes(&signature),
        )
        .map_err(other)
}
impl AccountStore for Vault {
    fn list(&self) -> Vec<AccountSummary> {
        let Ok(db) = self.db.lock() else {
            return vec![];
        };
        let Ok(records) = all_records(&db) else {
            return vec![];
        };
        records
            .iter()
            .filter_map(|r| {
                if let SignedRecord::Account(a) = r {
                    let e = Self::enabled_record(&db, &a.id).ok()?;
                    self.summary(a, &e).ok()
                } else {
                    None
                }
            })
            .collect()
    }
    fn get(&self, id: &str) -> Option<AccountSummary> {
        let a = self.account(id).ok()?;
        let e = self.enablement(id).ok()?;
        self.summary(&a, &e).ok()
    }
    fn get_secret(&self, id: &str) -> Result<SecretBox, StoreError> {
        let a = self.account(id)?;
        let e = self.enablement(id)?;
        self.current_roster()?;
        if !e.all_nodes && !e.enabled_on.contains(self.node()) {
            return Err(StoreError::NoSecret);
        }
        let envelope = self.envelope(&a)?;
        if envelope.deleted {
            return Err(StoreError::NotFound);
        }
        Ok(SecretBox::new(B64.decode(&envelope.secret).map_err(other)?))
    }
    fn put(&self, summary: AccountSummary, secret: Option<SecretBox>) -> Result<(), StoreError> {
        let roster = self.current_roster()?;
        if summary.id.is_empty()
            || summary.id.len() > 256
            || summary
                .enabled_on
                .iter()
                .any(|n| roster.member(n).is_none())
            || roster.member(&summary.home_node).is_none()
        {
            return Err(other("invalid account"));
        }
        if summary.kind == AccountKind::ClaudeOAuth
            && (summary.sync
                || summary.all_nodes
                || summary.home_node != self.node()
                || summary.enabled_on.iter().any(|n| n != self.node()))
        {
            return Err(other("Claude must remain on its home node"));
        }
        let mut db = self.db.lock().map_err(other)?;
        let tx = db.transaction().map_err(other)?;
        let previous = match Self::read(&tx, &format!("account:{}", summary.id))? {
            Some(SignedRecord::Account(a)) => Some(a),
            _ => None,
        };
        if previous.as_ref().is_some_and(|a| {
            a.kind != summary.kind
                || a.home_node != summary.home_node
                || a.provider != summary.provider
        }) {
            return Err(other("immutable metadata"));
        }
        let old_envelope = previous.as_ref().map(|a| self.envelope(a)).transpose()?;
        if old_envelope.as_ref().is_some_and(|e| e.deleted) {
            return Err(StoreError::NotFound);
        }
        let current_lease = match Self::read(&tx, &format!("lease:{}", summary.id))? {
            Some(SignedRecord::Lease(l)) => Some(l),
            _ => None,
        };
        let credential_changed = secret.as_ref().is_some_and(|s| {
            old_envelope
                .as_ref()
                .is_some_and(|e| B64.encode(s.expose()) != e.secret)
        });
        if summary.kind == AccountKind::OAuth
            && credential_changed
            && (summary.all_nodes && roster.members.len() > 1 || summary.enabled_on.len() > 1)
        {
            let l = current_lease
                .as_ref()
                .ok_or_else(|| other("refresh lease required"))?;
            if l.holder != self.node() || l.expires_at <= now() {
                return Err(other("refresh lease not held"));
            }
        }
        let version = previous
            .as_ref()
            .map_or(0, |a| a.version)
            .checked_add(1)
            .ok_or_else(|| other("version"))?;
        let generation = previous
            .as_ref()
            .map_or(0, |a| a.generation)
            .checked_add(u64::from(credential_changed))
            .ok_or_else(|| other("generation"))?;
        let mut a = Account {
            id: summary.id.clone(),
            provider: summary.provider,
            kind: summary.kind,
            label: summary.label,
            home_node: summary.home_node,
            sync: if summary.sync {
                SyncPolicy::Synced
            } else {
                SyncPolicy::NeverSync
            },
            version,
            writer: self.node().into(),
            updated_at: now(),
            sealed: String::new(),
            wraps: BTreeMap::new(),
            generation,
            sig: String::new(),
        };
        let envelope = Envelope {
            secret: match secret {
                Some(s) => B64.encode(s.expose()),
                None => old_envelope
                    .as_ref()
                    .map(|e| e.secret.clone())
                    .ok_or(StoreError::NoSecret)?,
            },
            plan: summary.plan,
            state: summary.state,
            lease_epoch: old_envelope
                .as_ref()
                .map_or(0, |e| e.lease_epoch)
                .max(current_lease.as_ref().map_or(0, |l| l.epoch))
                .max(Self::epoch_floor(&tx, &summary.id)?),
            deleted: false,
        };
        let plain = Zeroizing::new(serde_json::to_vec(&envelope).map_err(other)?);
        crypto::seal(&mut a, &plain, &roster)?;
        let old_e = Self::enabled_record(&tx, &a.id).ok();
        let enabled_on: BTreeSet<_> = summary.enabled_on.into_iter().collect();
        if let Some(l) = current_lease.as_ref()
            && !summary.all_nodes
            && !enabled_on.contains(&l.holder)
            && l.expires_at > now()
            && enabled_on.len() > 1
        {
            return Err(other("handoff lease before disabling holder"));
        }
        let e = Enablement {
            account_id: a.id.clone(),
            enabled_on,
            all_nodes: summary.all_nodes,
            version: old_e
                .as_ref()
                .map_or(0, |e| e.version)
                .checked_add(1)
                .ok_or_else(|| other("version"))?,
            writer: self.node().into(),
            sig: String::new(),
        };
        Self::save(
            &tx,
            &self.signed(SignedRecord::Account(a))?,
            envelope.lease_epoch,
        )?;
        Self::save(&tx, &self.signed(SignedRecord::Enablement(e))?, 0)?;
        if !summary.all_nodes
            && summary.kind == AccountKind::OAuth
            && Self::enabled_record(&tx, &summary.id)?.enabled_on.len() <= 1
        {
            Self::invalidate_lease(&tx, &summary.id)?;
        }
        tx.commit().map_err(other)?;
        Ok(())
    }
    fn remove(&self, id: &str) -> Result<(), StoreError> {
        let roster = self.current_roster()?;
        let mut a = self.account(id)?;
        let mut envelope = self.envelope(&a)?;
        envelope.deleted = true;
        use zeroize::Zeroize;
        envelope.secret.zeroize();
        envelope.secret.clear();
        a.version = a.version.checked_add(1).ok_or_else(|| other("version"))?;
        a.generation = a
            .generation
            .checked_add(1)
            .ok_or_else(|| other("generation"))?;
        a.writer = self.node().into();
        a.updated_at = now();
        let plain = Zeroizing::new(serde_json::to_vec(&envelope).map_err(other)?);
        crypto::seal(&mut a, &plain, &roster)?;
        let db = self.db.lock().map_err(other)?;
        Self::save(
            &db,
            &self.signed(SignedRecord::Account(a))?,
            envelope.lease_epoch,
        )
    }
}
