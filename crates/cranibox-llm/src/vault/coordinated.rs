//! Upstream decorator: only the live lease holder can rotate a shared OAuth token.
use super::{Vault, lease::RefreshCoordinator, sync};
use cranibox_contracts::{
    llm::*,
    net::PeerTransport,
    vault::{AccountStore, RefreshRequest, StoreError},
};
use std::sync::Arc;
pub struct Coordinated {
    pub inner: Arc<dyn Upstream>,
    pub vault: Arc<Vault>,
    pub coordinator: Arc<RefreshCoordinator>,
    pub transport: Arc<dyn PeerTransport>,
}
impl Coordinated {
    pub async fn refresh_record(
        &self,
        request: RefreshRequest,
    ) -> Result<cranibox_contracts::vault::RefreshResponse, StoreError> {
        let inner = self.inner.clone();
        let node = self.vault.node().to_owned();
        let vault = self.vault.clone();
        let account_id = request.account_id.clone();
        let result = self
            .coordinator
            .refresh(request, move |account, secret| async move {
                let preset = crate::catalog::presets()
                    .into_iter()
                    .find(|p| p.id == account.provider);
                let cx = UpstreamCx {
                    account,
                    secret: secret.clone(),
                    preset,
                    node,
                };
                match inner.refresh(&cx, &secret).await {
                    Ok(refreshed) => Ok(refreshed.secret),
                    Err(UpstreamError::Auth) => {
                        let mut account = cx.account;
                        account.state = AccountState::NeedsSignIn;
                        vault.put(account, None)?;
                        Err(StoreError::Other("needs_sign_in".into()))
                    }
                    Err(_) => Err(StoreError::Other("credential refresh unavailable".into())),
                }
            })
            .await;
        match result {
            Err(StoreError::Other(ref code)) if code == "needs_sign_in" => {
                Ok(cranibox_contracts::vault::RefreshResponse {
                    record: cranibox_contracts::vault::SignedRecord::Account(
                        self.vault.account(&account_id)?,
                    ),
                })
            }
            other => other,
        }
    }
}
#[async_trait::async_trait]
impl Upstream for Coordinated {
    fn provider(&self) -> &'static str {
        self.inner.provider()
    }
    fn wires(&self) -> &[Wire] {
        self.inner.wires()
    }
    async fn send(
        &self,
        cx: &UpstreamCx,
        req: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        self.inner.send(cx, req).await
    }
    async fn models(&self, cx: &UpstreamCx) -> Result<Vec<ModelEntry>, UpstreamError> {
        self.inner.models(cx).await
    }
    async fn refresh(
        &self,
        cx: &UpstreamCx,
        secret: &SecretBox,
    ) -> Result<Refreshed, UpstreamError> {
        if cx.account.kind != AccountKind::OAuth
            || !cx.account.sync
            || (!cx.account.all_nodes && cx.account.enabled_on.len() < 2)
        {
            return self.inner.refresh(cx, secret).await;
        }
        let account = self
            .vault
            .account(&cx.account.id)
            .map_err(|_| UpstreamError::Auth)?;
        let request = RefreshRequest {
            account_id: account.id,
            generation: account.generation,
        };
        let holder = self
            .vault
            .refresh_holder(&request.account_id, crate::auth::now())
            .map_err(|_| UpstreamError::Network("home_offline".into()))?;
        let result = if holder == self.vault.node() {
            self.refresh_record(request).await
        } else {
            sync::request_refresh(&self.vault, self.transport.as_ref(), request).await
        };
        result.map_err(|_| UpstreamError::Network("home_offline".into()))?;
        if self
            .vault
            .get(&cx.account.id)
            .is_some_and(|a| a.state == AccountState::NeedsSignIn)
        {
            return Err(UpstreamError::Auth);
        }
        let secret = self
            .vault
            .get_secret(&cx.account.id)
            .map_err(|_| UpstreamError::Auth)?;
        let expires_at = crate::upstream::apikey::Credential::decode(&secret)?.expires_at;
        Ok(Refreshed { secret, expires_at })
    }
}
