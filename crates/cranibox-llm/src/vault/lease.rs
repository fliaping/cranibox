//! Fifteen-minute OAuth leases, deterministic takeover and refresh single-flight.
use super::*;
use std::future::Future;
use tokio::sync::Mutex as AsyncMutex;
pub const TTL: u64 = 900;
pub const RENEW_EVERY: u64 = 300;
pub(crate) fn validate(
    _vault: &Vault,
    db: &Connection,
    l: &Lease,
    roster: &Roster,
    at: u64,
) -> Result<(), StoreError> {
    let Some(SignedRecord::Account(a)) = Vault::read(db, &format!("account:{}", l.account_id))?
    else {
        return Err(StoreError::NotFound);
    };
    let e = Vault::enabled_record(db, &l.account_id)?;
    let enabled = enabled(&e, roster);
    if a.kind != AccountKind::OAuth
        || a.sync != SyncPolicy::Synced
        || enabled.len() < 2
        || !enabled.contains(&l.holder)
        || l.epoch == 0
        || l.expires_at > at.saturating_add(TTL + 30)
    {
        return Err(other("invalid lease"));
    }
    Ok(())
}
pub(crate) fn enabled(e: &Enablement, r: &Roster) -> BTreeSet<String> {
    r.members
        .iter()
        .filter(|m| e.all_nodes || e.enabled_on.contains(&m.node_id))
        .map(|m| m.node_id.clone())
        .collect()
}
fn preference(a: &Account, e: &Enablement, r: &Roster) -> Vec<String> {
    let enabled = enabled(e, r);
    let mut nodes: Vec<_> = r
        .members
        .iter()
        .filter(|m| enabled.contains(&m.node_id))
        .collect();
    nodes.sort_by_key(|m| {
        (
            if m.role == cranibox_contracts::roster::Role::Hub {
                0
            } else if m.node_id == a.home_node {
                1
            } else {
                2
            },
            m.priority,
            m.node_id.clone(),
        )
    });
    nodes.into_iter().map(|m| m.node_id.clone()).collect()
}
impl Vault {
    /// Called with live probes of all enabled nodes. Unexpired leases are never stolen.
    pub fn claim_lease(
        &self,
        id: &str,
        at: u64,
        reachable: &BTreeSet<String>,
    ) -> Result<Option<Lease>, StoreError> {
        let roster = self.current_roster()?;
        let a = self.account(id)?;
        let e = self.enablement(id)?;
        if a.kind != AccountKind::OAuth
            || a.sync != SyncPolicy::Synced
            || enabled(&e, &roster).len() < 2
        {
            return Ok(None);
        }
        let previous = self.lease(id)?;
        // Only hand back after the router's recovery hysteresis marks the preferred peer stable.
        if let Some(l) = previous.as_ref()
            && l.holder == self.node()
            && l.expires_at > at
            && preference(&a, &e, &roster)
                .first()
                .is_some_and(|n| n != self.node() && reachable.contains(n))
        {
            return self.yield_lease(id, at).map(Some);
        }
        if let Some(l) = previous.as_ref()
            && l.expires_at > at
        {
            return if l.holder == self.node()
                && l.expires_at.saturating_sub(at) <= TTL - RENEW_EVERY
            {
                self.renew_lease(id, at).map(Some)
            } else {
                Ok(Some(l.clone()))
            };
        }
        let preferred = preference(&a, &e, &roster)
            .into_iter()
            .find(|n| n == self.node() || reachable.contains(n));
        if preferred.as_deref() != Some(self.node()) {
            return Ok(previous);
        }
        let floor = {
            let db = self.db.lock().map_err(other)?;
            Self::epoch_floor(&db, id)?
        };
        let epoch = previous
            .as_ref()
            .map_or(0, |l| l.epoch)
            .max(floor)
            .checked_add(1)
            .ok_or_else(|| other("lease epoch"))?;
        self.publish_lease(Lease {
            account_id: id.into(),
            holder: self.node().into(),
            epoch,
            expires_at: at.saturating_add(TTL),
            sig: String::new(),
        })
        .map(Some)
    }
    fn publish_lease(&self, lease: Lease) -> Result<Lease, StoreError> {
        let SignedRecord::Lease(lease) = self.signed(SignedRecord::Lease(lease))? else {
            unreachable!()
        };
        let roster = self.current_roster()?;
        let db = self.db.lock().map_err(other)?;
        validate(
            self,
            &db,
            &lease,
            &roster,
            now().max(lease.expires_at.saturating_sub(TTL)),
        )?;
        if let Some(SignedRecord::Lease(old)) =
            Self::read(&db, &format!("lease:{}", lease.account_id))?
            && (lease.epoch < old.epoch || (lease.epoch == old.epoch && lease.holder != old.holder))
        {
            return Err(other("stale lease"));
        }
        Self::save(&db, &SignedRecord::Lease(lease.clone()), 0)?;
        Self::remember_epoch(&db, &lease.account_id, lease.epoch)?;
        Ok(lease)
    }
    pub fn renew_lease(&self, id: &str, at: u64) -> Result<Lease, StoreError> {
        let mut l = self.lease(id)?.ok_or(StoreError::NotFound)?;
        if l.holder != self.node() || l.expires_at <= at {
            return Err(other("lease not held"));
        }
        l.expires_at = at.saturating_add(TTL);
        self.publish_lease(l)
    }
    /// Stable preferred nodes claim a new epoch after the old holder yields its lease.
    pub fn yield_lease(&self, id: &str, at: u64) -> Result<Lease, StoreError> {
        let mut l = self.lease(id)?.ok_or(StoreError::NotFound)?;
        if l.holder != self.node() {
            return Err(other("lease not held"));
        }
        l.expires_at = at;
        l.epoch = l.epoch.checked_add(1).ok_or_else(|| other("epoch"))?;
        self.publish_lease(l)
    }
    pub fn refresh_holder(&self, id: &str, at: u64) -> Result<String, StoreError> {
        let a = self.account(id)?;
        let e = self.enablement(id)?;
        let roster = self.current_roster()?;
        let enabled = enabled(&e, &roster);
        if enabled.len() == 1 {
            return enabled.into_iter().next().ok_or(StoreError::NoSecret);
        }
        if a.kind != AccountKind::OAuth {
            return Err(other("not OAuth"));
        }
        self.lease(id)?
            .filter(|l| l.expires_at > at)
            .map(|l| l.holder)
            .ok_or_else(|| other("no active refresh lease"))
    }
}
/// Own one coordinator per vault. The lock covers generation check, vendor refresh and FULL-sync commit.
pub struct RefreshCoordinator {
    vault: Arc<Vault>,
    locks: AsyncMutex<BTreeMap<String, Arc<AsyncMutex<()>>>>,
}
impl RefreshCoordinator {
    pub fn new(vault: Arc<Vault>) -> Self {
        Self {
            vault,
            locks: AsyncMutex::new(BTreeMap::new()),
        }
    }
    pub async fn maintain_lease(
        &self,
        id: &str,
        at: u64,
        reachable: &BTreeSet<String>,
    ) -> Result<Option<Lease>, StoreError> {
        let lock = self
            .locks
            .lock()
            .await
            .entry(id.into())
            .or_default()
            .clone();
        let _guard = lock.lock().await;
        self.vault.claim_lease(id, at, reachable)
    }
    pub async fn refresh<F, Fut>(
        &self,
        request: RefreshRequest,
        refresh: F,
    ) -> Result<RefreshResponse, StoreError>
    where
        F: FnOnce(AccountSummary, SecretBox) -> Fut,
        Fut: Future<Output = Result<SecretBox, StoreError>>,
    {
        let lock = self
            .locks
            .lock()
            .await
            .entry(request.account_id.clone())
            .or_default()
            .clone();
        let _guard = lock.lock().await;
        let current = self.vault.account(&request.account_id)?;
        if current.generation > request.generation {
            return Ok(RefreshResponse {
                record: SignedRecord::Account(current),
            });
        }
        if current.generation != request.generation
            || self.vault.refresh_holder(&request.account_id, now())? != self.vault.node()
        {
            return Err(other("refresh must be requested from lease holder"));
        }
        let summary = self
            .vault
            .get(&request.account_id)
            .ok_or(StoreError::NotFound)?;
        let secret = self.vault.get_secret(&request.account_id)?;
        let rotated = refresh(summary.clone(), secret).await?;
        // Only return/broadcast after the SQLite FULL-sync commit succeeds.
        self.vault.put(summary, Some(rotated))?;
        Ok(RefreshResponse {
            record: SignedRecord::Account(self.vault.account(&request.account_id)?),
        })
    }
}
