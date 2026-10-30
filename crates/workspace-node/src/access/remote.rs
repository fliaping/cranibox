//! Deciding, on one of the owner's devices, a request that waits on another of them.
//!
//! The target stays the only enforcer. It shows its waiting requests only to devices in
//! the owner-signed roster over the verified end-to-end channel, with a single-use
//! challenge per request and approver. To decide one, the person types the identity's
//! recovery code (or passphrase) on the approving device. That device derives the
//! recovery key in memory, signs a short-lived assertion naming the target, the request,
//! the hash of exactly what was shown, the approving device, the challenge and a fresh id,
//! and drops the key. No device keeps the code; every device holds the recovery public
//! key, pinned with the identity, as the verifier. The target checks the signature
//! against it, counts wrong ones and locks remote approval after too many, accepts each
//! assertion and challenge once, and records who approved, how and the assertion id.
//! Approving a request on the device it waits on needs no extra step: the person is
//! already in front of it.
use super::*;
use agent_workspace_runtime::peers::{
    identity::{hex, unhex},
    roster::RosterStore,
};
use cranibox_contracts::access::{
    REMOTE_APPROVAL_TTL, RemoteApproval, RemotePending, RemotePendingList, RemoteRecord,
};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// How the approving person proved it is the owner: the identity's recovery code or
/// recovery passphrase, both turned into the same recovery key.
pub const AUTH_METHODS: [&str; 2] = ["recovery_code", "recovery_passphrase"];
/// Wrong signatures one approving device may send before remote approval locks for it.
pub const ATTEMPTS: u32 = 5;
/// Wrong signatures from all devices together before remote approval locks for all.
pub const GLOBAL_ATTEMPTS: u32 = 10;
/// First lock, doubling with each further wrong signature, at most [`LOCK_MAX`].
const LOCK_SECS: u64 = 300;
const LOCK_MAX: u64 = 3600;
/// Key of the counter across all approving devices.
const ALL: &str = "*";
/// Clock difference tolerated between the owner's devices, in seconds.
const SKEW: u64 = 30;
/// Waiting challenges kept at most, across all approvers.
const CHALLENGES: usize = 256;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Challenge {
    request: String,
    approver: String,
    pub(super) expires_at: u64,
}

/// Wrong signatures counted on the target, and the lock they caused.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Lockout {
    pub(super) failures: u32,
    pub(super) locked_until: u64,
    /// When the last wrong signature arrived; an hour of quiet forgets the count.
    pub(super) last: u64,
}
impl Lockout {
    pub(super) fn stale(&self, at: u64) -> bool {
        self.locked_until <= at && self.last + LOCK_MAX <= at
    }
    /// One more wrong signature; `limit` failures lock, longer with each further one.
    fn fail(&mut self, limit: u32, at: u64) {
        self.failures += 1;
        self.last = at;
        if self.failures >= limit {
            let lock = (LOCK_SECS << (self.failures - limit).min(8)).min(LOCK_MAX);
            self.locked_until = at + lock;
        }
    }
}

/// Why a remote decision was refused. The message is safe to show and to log.
#[derive(Debug)]
pub enum Refusal {
    /// The caller, or this device, is not one of the owner's devices.
    NotOwner,
    /// The owner turned remote approval off on this device.
    Disabled,
    /// This identity has no recovery code yet, so nothing can verify an approval.
    NoRecovery,
    /// The signature does not match the recovery key: wrong code or passphrase.
    WrongCode {
        left: u32,
    },
    /// Too many wrong signatures; remote approval is locked until then.
    Locked {
        until: u64,
    },
    Invalid(&'static str),
    Unavailable(anyhow::Error),
}
impl Refusal {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotOwner => "not_owner",
            Self::Disabled => "disabled",
            Self::NoRecovery => "no_recovery",
            Self::WrongCode { .. } => "wrong_code",
            Self::Locked { .. } => "locked",
            Self::Invalid(reason) => reason,
            Self::Unavailable(_) => "unavailable",
        }
    }
}

/// Hex SHA-256 over what a request asks for: who, which capability, which operation with
/// which parameters, and when it was made. An approval for one request does not fit any
/// other.
pub fn op_hash(request: &AccessRequest) -> String {
    let value = serde_json::json!({
        "id": request.id,
        "from": request.from,
        "capability": request.capability,
        "session_key": request.session_key,
        "created_at": request.created_at,
        "path": request.path,
        "operation": request.operation,
        "detail": request.detail,
    });
    hex(&Sha256::digest(
        cranibox_contracts::canonical::to_vec(&value).unwrap_or_default(),
    ))
}
/// Requests another device may only approve for this one call: always-confirm and
/// high-risk operations, ones the review flagged, and owner-only capabilities.
pub fn once_only(request: &AccessRequest) -> bool {
    request.capability.requires_approval()
        || request
            .operation
            .as_ref()
            .is_some_and(|o| o.confirm || o.risk == Risk::Danger)
        || decision::escalates(request.review.as_ref())
}
pub fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buffer);
    hex(&buffer)
}

impl Store {
    pub(crate) fn roster(&self) -> Option<RosterStore> {
        let path = self.peers.with_file_name("models").join("roster.json");
        path.exists()
            .then(|| RosterStore::load(path).ok())
            .flatten()
    }
    /// The pinned recovery public key: the verifier of remote approvals.
    pub fn recovery_verifier(&self) -> Option<String> {
        let roster = self.roster()?;
        let pinned = roster.identity()?;
        let cert = roster.current()?.owner_cert?;
        (cert.recovery_pub == pinned).then_some(pinned)
    }
    /// The lock on remote approval from `approver` (or from all devices), if any.
    pub fn remote_locked(&self, approver: &str) -> Option<u64> {
        let at = now();
        self.transaction(at, |state| {
            Ok([approver, ALL]
                .iter()
                .filter_map(|k| state.lockouts.get(*k))
                .map(|l| l.locked_until)
                .filter(|until| *until > at)
                .max())
        })
        .ok()
        .flatten()
    }
    fn own_node(&self) -> Result<String> {
        Ok(Config::read(&self.peers)?.node_id)
    }
    /// Both this device and `other` are the owner's devices, and they are not the same.
    pub fn owner_peer(&self, other: &str) -> bool {
        let Ok(own) = self.own_node() else {
            return false;
        };
        own != other
            && self
                .roster()
                .is_some_and(|r| r.is_owner_node(&own) && r.is_owner_node(other))
    }
    /// The owner's other devices: id and name.
    pub fn owner_devices(&self) -> Vec<(String, String)> {
        let Ok(own) = self.own_node() else {
            return vec![];
        };
        let Some(roster) = self.roster().and_then(|r| r.current()) else {
            return vec![];
        };
        if roster.member(&own).is_none() {
            return vec![];
        }
        roster
            .members
            .into_iter()
            .filter(|m| m.node_id != own)
            .map(|m| {
                let name = if m.name.is_empty() {
                    self.name(&m.node_id)
                } else {
                    m.name.clone()
                };
                (m.node_id, name)
            })
            .collect()
    }
    /// Whether the owner's other devices may decide requests waiting here (default on).
    pub fn remote_enabled(&self) -> bool {
        self.transaction(now(), |state| Ok(!state.remote_off))
            .unwrap_or(false)
    }
    pub fn set_remote_enabled(&self, enabled: bool) -> Result<()> {
        self.transaction(now(), |state| {
            if state.remote_off != enabled {
                return Ok(());
            }
            state.remote_off = !enabled;
            if !enabled {
                state.challenges.clear();
            }
            self.event(
                EventKind::PolicyChanged,
                None,
                None,
                None,
                Some("remote_approval"),
                if enabled { "on" } else { "off" },
                now(),
            )
        })
    }
    /// Requests waiting here, for the owner device `approver`, each with a challenge
    /// that device must sign to decide it.
    pub fn remote_pending(
        &self,
        approver: &str,
    ) -> std::result::Result<RemotePendingList, Refusal> {
        self.remote_pending_at(approver, now())
    }
    pub(crate) fn remote_pending_at(
        &self,
        approver: &str,
        at: u64,
    ) -> std::result::Result<RemotePendingList, Refusal> {
        if !self.owner_peer(approver) {
            return Err(Refusal::NotOwner);
        }
        let node = self.own_node().map_err(Refusal::Unavailable)?;
        self.transaction(at, |state| {
            if state.remote_off {
                return Ok(Err(Refusal::Disabled));
            }
            let mut pending: Vec<_> = state
                .requests
                .values()
                .filter(|r| r.state == RequestState::Pending && r.expires_at > at)
                .cloned()
                .collect();
            pending.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
            let mut requests = vec![];
            for request in pending {
                // Reuse a challenge with enough time left, so polling does not pile them up.
                let current = state
                    .challenges
                    .iter()
                    .find(|(_, c)| {
                        c.request == request.id
                            && c.approver == approver
                            && c.expires_at > at + SKEW
                    })
                    .map(|(id, c)| (id.clone(), c.expires_at));
                let (challenge, expires_at) = match current {
                    Some(current) => current,
                    None => {
                        if state.challenges.len() >= CHALLENGES {
                            continue;
                        }
                        let id = random_hex(32);
                        let expires_at = request.expires_at.min(at + REMOTE_APPROVAL_TTL);
                        state.challenges.insert(
                            id.clone(),
                            Challenge {
                                request: request.id.clone(),
                                approver: approver.into(),
                                expires_at,
                            },
                        );
                        (id, expires_at)
                    }
                };
                requests.push(RemotePending {
                    op_hash: op_hash(&request),
                    once_only: once_only(&request),
                    challenge,
                    challenge_expires_at: expires_at,
                    request,
                });
            }
            Ok(Ok(RemotePendingList { node, requests }))
        })
        .map_err(Refusal::Unavailable)?
    }
    /// Verifies a signed decision from the owner device `caller` and applies it.
    pub fn decide_remote(
        &self,
        caller: &str,
        assertion: &RemoteApproval,
    ) -> std::result::Result<AccessRequest, Refusal> {
        self.decide_remote_at(caller, assertion, now())
    }
    pub(crate) fn decide_remote_at(
        &self,
        caller: &str,
        assertion: &RemoteApproval,
        at: u64,
    ) -> std::result::Result<AccessRequest, Refusal> {
        let result = self.verify_remote(caller, assertion, at);
        if let Err(refusal) = &result
            && !matches!(refusal, Refusal::Unavailable(_))
        {
            let _ = self.remote_refused(caller, assertion, refusal.code(), at);
        }
        result
    }
    fn verify_remote(
        &self,
        caller: &str,
        a: &RemoteApproval,
        at: u64,
    ) -> std::result::Result<AccessRequest, Refusal> {
        use Refusal::Invalid;
        if !self.owner_peer(caller) {
            return Err(Refusal::NotOwner);
        }
        let own = self.own_node().map_err(Refusal::Unavailable)?;
        if a.v != 1 || a.approver != caller {
            return Err(Invalid("wrong_approver"));
        }
        if a.target != own {
            return Err(Invalid("wrong_target"));
        }
        if !AUTH_METHODS.contains(&a.auth.as_str()) {
            return Err(Invalid("unknown_auth"));
        }
        let verifier = self.recovery_verifier().ok_or(Refusal::NoRecovery)?;
        // Locked: refuse before looking at the signature, so a lock also stops a right guess.
        if let Some(until) = self
            .transaction(at, |state| {
                Ok([caller, ALL]
                    .iter()
                    .filter_map(|k| state.lockouts.get(*k))
                    .map(|l| l.locked_until)
                    .filter(|until| *until > at)
                    .max())
            })
            .map_err(Refusal::Unavailable)?
        {
            return Err(Refusal::Locked { until });
        }
        let signed = (|| -> Result<()> {
            let key = ed25519_dalek::VerifyingKey::from_bytes(&unhex::<32>(&verifier)?)?;
            let signature = ed25519_dalek::Signature::from_bytes(&unhex::<64>(&a.sig)?);
            key.verify_strict(&a.signing_message()?, &signature)?;
            Ok(())
        })();
        if signed.is_err() {
            return Err(self
                .transaction(at, |state| {
                    let own = state.lockouts.entry(caller.into()).or_default();
                    own.fail(ATTEMPTS, at);
                    let (failures, mut until) = (own.failures, own.locked_until);
                    let all = state.lockouts.entry(ALL.into()).or_default();
                    all.fail(GLOBAL_ATTEMPTS, at);
                    until = until.max(all.locked_until);
                    let left = ATTEMPTS
                        .saturating_sub(failures)
                        .min(GLOBAL_ATTEMPTS.saturating_sub(all.failures));
                    Ok(if until > at {
                        Refusal::Locked { until }
                    } else {
                        Refusal::WrongCode { left }
                    })
                })
                .map_err(Refusal::Unavailable)?);
        }
        if a.issued_at > at + SKEW
            || a.expires_at <= a.issued_at
            || a.expires_at - a.issued_at > REMOTE_APPROVAL_TTL
            || a.expires_at + SKEW <= at
        {
            return Err(Invalid("expired"));
        }
        let decision = match a.decision.as_str() {
            "approve" => ApprovalDecision::Approve,
            "deny" => ApprovalDecision::Deny,
            _ => return Err(Invalid("bad_decision")),
        };
        if a.id.len() > 64 || a.nonce.len() < 16 || a.nonce.len() > 128 {
            return Err(Invalid("bad_assertion"));
        }
        let record = RemoteRecord {
            device: caller.into(),
            device_name: Some(self.name(caller)).filter(|n| n != caller),
            auth: a.auth.clone(),
            assertion: a.id.clone(),
        };
        self.transaction(at, |state| {
            if state.remote_off {
                return Ok(Err(Refusal::Disabled));
            }
            // The right recovery key: forget earlier wrong guesses.
            state.lockouts.remove(caller);
            state.lockouts.remove(ALL);
            if state.assertions.contains_key(&a.id) {
                return Ok(Err(Invalid("replayed")));
            }
            let Some(challenge) = state.challenges.get(&a.challenge).cloned() else {
                return Ok(Err(Invalid("unknown_challenge")));
            };
            if challenge.approver != caller || challenge.request != a.request {
                return Ok(Err(Invalid("unknown_challenge")));
            }
            // Signed and fresh: from here the challenge and the assertion are spent,
            // whatever happens next.
            state.challenges.remove(&a.challenge);
            state.assertions.insert(a.id.clone(), a.expires_at + SKEW);
            if challenge.expires_at + SKEW <= at {
                return Ok(Err(Invalid("expired")));
            }
            let Some(request) = state
                .requests
                .get(&a.request)
                .filter(|r| r.state == RequestState::Pending && r.expires_at > at)
                .cloned()
            else {
                return Ok(Err(Invalid("request_unavailable")));
            };
            if op_hash(&request) != a.op_hash {
                return Ok(Err(Invalid("operation_mismatch")));
            }
            if decision == ApprovalDecision::Approve {
                match a.scope {
                    None => return Ok(Err(Invalid("bad_decision"))),
                    Some(scope) if scope != DurationScope::Session && once_only(&request) => {
                        return Ok(Err(Invalid("once_only")));
                    }
                    _ => {}
                }
            }
            let input = DecideRequest {
                decision,
                scope: a.scope.filter(|_| decision == ApprovalDecision::Approve),
            };
            self.decide_in(state, &a.request, input, at, Some(&record))
                .map(Ok)
        })
        .map_err(Refusal::Unavailable)?
    }
    fn remote_refused(
        &self,
        caller: &str,
        a: &RemoteApproval,
        reason: &str,
        at: u64,
    ) -> Result<()> {
        self.transaction(at, |state| {
            // Every wrong code is recorded; other repeats once a minute.
            if reason != "wrong_code"
                && state
                    .refusals
                    .insert(format!("remote:{caller}:{reason}"), at / 60)
                    == Some(at / 60)
            {
                return Ok(());
            }
            self.append(AuditEvent {
                index: 0,
                at,
                event: EventKind::Refused,
                from: Some(caller.into()),
                name: Some(self.name(caller)),
                capability: None,
                session_key: None,
                basis: Some("remote_approval".into()),
                result: format!("remote:{reason}"),
                prev: String::new(),
                operation: None,
                status: None,
                transport: None,
                detail: None,
                review: None,
                remote: Some(RemoteRecord {
                    device: caller.into(),
                    device_name: None,
                    auth: a.auth.chars().take(32).collect(),
                    assertion: a.id.chars().take(64).collect(),
                }),
            })
        })
    }
    /// Records, on the approving device, what the person decided for `target`.
    pub fn record_remote(
        &self,
        target: &str,
        request: Option<&AccessRequest>,
        result: &str,
        auth: &str,
        assertion: Option<&str>,
    ) -> Result<()> {
        let _lock = config::lock_credentials(&self.path()).context("lock access policy")?;
        self.append(AuditEvent {
            index: 0,
            at: now(),
            event: EventKind::RemoteDecision,
            from: request.map(|r| r.from.clone()),
            name: request.map(|r| r.name.clone()),
            capability: request.map(|r| r.capability),
            session_key: None,
            basis: request.map(|r| r.id.clone()),
            result: result.into(),
            prev: String::new(),
            operation: request.and_then(|r| r.operation.clone()),
            status: None,
            transport: None,
            detail: request.and_then(|r| r.detail.clone()),
            review: None,
            remote: Some(RemoteRecord {
                device: target.into(),
                device_name: Some(self.name(target)).filter(|n| n != target),
                auth: auth.into(),
                assertion: assertion.unwrap_or_default().into(),
            }),
        })
    }
}

/// Builds the assertion for one shown request and signs it with the recovery key the
/// person just unlocked on `approver`. The caller drops the key right after.
#[allow(clippy::too_many_arguments)]
pub fn sign(
    recovery: &ed25519_dalek::SigningKey,
    approver: &str,
    target: &str,
    pending: &RemotePending,
    decision: ApprovalDecision,
    scope: Option<DurationScope>,
    auth: &str,
    at: u64,
) -> Result<RemoteApproval> {
    use ed25519_dalek::Signer;
    let mut assertion = RemoteApproval {
        v: 1,
        id: uuid::Uuid::new_v4().to_string(),
        target: target.into(),
        request: pending.request.id.clone(),
        op_hash: op_hash(&pending.request),
        decision: if decision == ApprovalDecision::Approve {
            "approve"
        } else {
            "deny"
        }
        .into(),
        scope: scope.filter(|_| decision == ApprovalDecision::Approve),
        approver: approver.into(),
        auth: auth.into(),
        issued_at: at,
        expires_at: at + 60,
        nonce: random_hex(16),
        challenge: pending.challenge.clone(),
        sig: String::new(),
    };
    assertion.sig = hex(&recovery.sign(&assertion.signing_message()?).to_bytes());
    Ok(assertion)
}

#[cfg(test)]
#[path = "remote_tests.rs"]
mod tests;
