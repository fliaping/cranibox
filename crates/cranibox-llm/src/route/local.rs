//! Local execution: account pool, session affinity, retry before first byte. **WS2.**
use cranibox_contracts::{
    error::ErrorCode,
    llm::{AccountKind, AccountState, AccountSummary, UpstreamError},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct Pool {
    cooldown: HashMap<String, (Instant, u32)>,
}
impl Pool {
    pub fn candidates(
        &self,
        accounts: Vec<AccountSummary>,
        provider: &str,
        node: &str,
        session: &str,
    ) -> Result<Vec<AccountSummary>, ErrorCode> {
        let accounts: Vec<_> = accounts
            .into_iter()
            .filter(|a| a.provider == provider)
            .collect();
        if accounts.is_empty() {
            return Err(if provider == "claude" {
                ErrorCode::ClaudeNodeOffline
            } else {
                ErrorCode::AllAccountsExhausted
            });
        }
        let enabled: Vec<_> = accounts
            .into_iter()
            .filter(|a| {
                (a.all_nodes || a.enabled_on.iter().any(|n| n == node))
                    && (a.kind != AccountKind::ClaudeOAuth
                        || (a.home_node == node && !a.sync && !a.all_nodes))
            })
            .collect();
        if enabled.is_empty() {
            return Err(ErrorCode::AccountDisabledHere);
        }
        let needs_sign_in = enabled
            .iter()
            .all(|a| matches!(a.state, AccountState::NeedsSignIn));
        let mut ready: Vec<_> = enabled.into_iter().filter(|a| {
            let state_ok = matches!(a.state, AccountState::Ready) || matches!(a.state, AccountState::QuotaExhausted { reset_at: Some(at) } if at <= crate::auth::now());
            state_ok && self.cooldown.get(&a.id).is_none_or(|(until,_)| *until <= Instant::now())
        }).collect();
        ready.sort_by_cached_key(|a| {
            std::cmp::Reverse(Sha256::digest(format!("{session}:{}", a.id).as_bytes()).to_vec())
        });
        if ready.is_empty() {
            Err(if needs_sign_in {
                ErrorCode::NeedsSignIn
            } else {
                ErrorCode::AllAccountsExhausted
            })
        } else {
            Ok(ready)
        }
    }
    pub fn failed(&mut self, id: &str, error: &UpstreamError) {
        let failures = self
            .cooldown
            .get(id)
            .map_or(0, |(_, n)| *n)
            .saturating_add(1);
        let seconds = match error {
            UpstreamError::RateLimited {
                retry_after: Some(s),
            } => *s,
            UpstreamError::QuotaExhausted { reset_at: Some(at) } => {
                at.saturating_sub(crate::auth::now())
            }
            UpstreamError::QuotaExhausted { reset_at: None } => 600,
            _ => 60u64
                .saturating_mul(1u64 << failures.min(5).saturating_sub(1))
                .min(600),
        }
        .clamp(1, 31_536_000);
        self.cooldown.insert(
            id.into(),
            (Instant::now() + Duration::from_secs(seconds), failures),
        );
    }
    pub fn success(&mut self, id: &str) {
        self.cooldown.remove(id);
    }
}
pub fn retryable(e: &UpstreamError) -> bool {
    matches!(
        e,
        UpstreamError::Auth
            | UpstreamError::RateLimited { .. }
            | UpstreamError::QuotaExhausted { .. }
            | UpstreamError::Network(_)
    )
}
