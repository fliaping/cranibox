//! GitHub device login -> short-lived Copilot credential. WS3.
// Ported from yetone/magpie internal/provider/copilot_{signin,accounts,entitlement}.go
// @865b61808690bc9fd716257949414ec97618a5a5 (MIT).
use super::apikey::{self, Credential};
use cranibox_contracts::llm::*;
use serde_json::Value;
pub const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
pub struct Copilot {
    token_url: String,
    base: Option<String>,
}
impl Default for Copilot {
    fn default() -> Self {
        Self {
            token_url: "https://api.github.com/copilot_internal/v2/token".into(),
            base: None,
        }
    }
}
impl Copilot {
    fn validate_base(&self, base: &str) -> Result<(), UpstreamError> {
        if let Some(expected) = &self.base {
            if base == expected {
                return Ok(());
            }
            return Err(UpstreamError::Auth);
        }
        let url = crate::catalog::validate_url(base).map_err(|_| UpstreamError::Auth)?;
        if url.scheme() != "https"
            || url.port_or_known_default() != Some(443)
            || !url
                .host_str()
                .is_some_and(|h| h == "api.githubcopilot.com" || h.ends_with(".githubcopilot.com"))
        {
            return Err(UpstreamError::Auth);
        }
        Ok(())
    }
    pub fn for_loopback(base: String, token_url: String) -> Result<Self, String> {
        crate::auth::require_loopback(&base)?;
        crate::auth::require_loopback(&token_url)?;
        Ok(Self {
            base: Some(base),
            token_url,
        })
    }
    pub async fn exchange(&self, github: &str) -> Result<Credential, UpstreamError> {
        let response = apikey::client()
            .get(&self.token_url)
            .bearer_auth(github)
            .header("accept", "application/json")
            .header("editor-version", "vscode/1.104.0")
            .header("editor-plugin-version", "copilot-chat/0.30.0")
            .send()
            .await
            .map_err(|_| UpstreamError::Network("Copilot token exchange failed".into()))?;
        let (status, value) = apikey::json_response(response, 1024 * 1024).await?;
        if status >= 300 {
            return Err(apikey::status_error(status, None, Some(&value)));
        }
        let token = value["token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(UpstreamError::Auth)?;
        let base = match &self.base {
            Some(base) => base.clone(),
            None => {
                let base = value
                    .pointer("/endpoints/api")
                    .and_then(Value::as_str)
                    .unwrap_or("https://api.githubcopilot.com");
                let url = crate::catalog::validate_url(base).map_err(|_| UpstreamError::Auth)?;
                let host = url.host_str().unwrap_or("");
                if !(host == "api.githubcopilot.com" || host.ends_with(".githubcopilot.com"))
                    || url.port_or_known_default() != Some(443)
                {
                    return Err(UpstreamError::Auth);
                }
                base.into()
            }
        };
        Ok(Credential {
            access_token: token.into(),
            github_token: Some(github.into()),
            expires_at: Some(
                value["expires_at"]
                    .as_u64()
                    .unwrap_or(crate::auth::now() + 1200),
            ),
            endpoints: [(Wire::Chat, base.clone()), (Wire::Responses, base)].into(),
            ..Default::default()
        })
    }
}
#[async_trait::async_trait]
impl Upstream for Copilot {
    fn provider(&self) -> &'static str {
        "copilot"
    }
    fn wires(&self) -> &[Wire] {
        &[Wire::Chat, Wire::Responses]
    }
    async fn send(
        &self,
        cx: &UpstreamCx,
        req: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        let credential = Credential::decode(&cx.secret)?;
        let base = credential
            .endpoints
            .get(&req.wire)
            .ok_or(UpstreamError::Unsupported)?;
        // Credentials cannot override the broker's destination allowlist.
        self.validate_base(base)?;
        let request = apikey::client()
            .post(apikey::endpoint(base, req.wire, false)?)
            .bearer_auth(&credential.access_token)
            .header("editor-version", "vscode/1.104.0")
            .header("editor-plugin-version", "copilot-chat/0.30.0")
            .header("copilot-integration-id", "vscode-chat")
            .header("x-request-id", uuid::Uuid::new_v4().to_string())
            .json(&req.body);
        apikey::response(
            request
                .send()
                .await
                .map_err(|_| UpstreamError::Network("Copilot connection failed".into()))?,
            req.wire,
        )
        .await
    }
    async fn refresh(
        &self,
        _cx: &UpstreamCx,
        secret: &SecretBox,
    ) -> Result<Refreshed, UpstreamError> {
        let old = Credential::decode(secret)?;
        let new = self
            .exchange(old.github_token.as_deref().ok_or(UpstreamError::Auth)?)
            .await?;
        Ok(Refreshed {
            expires_at: new.expires_at,
            secret: new.secret(),
        })
    }
    async fn models(&self, cx: &UpstreamCx) -> Result<Vec<ModelEntry>, UpstreamError> {
        let credential = Credential::decode(&cx.secret)?;
        let base = credential
            .endpoints
            .get(&Wire::Chat)
            .ok_or(UpstreamError::Unsupported)?;
        self.validate_base(base)?;
        let response = apikey::client()
            .get(format!("{}/models", base.trim_end_matches('/')))
            .bearer_auth(&credential.access_token)
            .header("copilot-integration-id", "vscode-chat")
            .send()
            .await
            .map_err(|_| UpstreamError::Network("model discovery failed".into()))?;
        let (status, value) = apikey::json_response(response, 4 * 1024 * 1024).await?;
        if status >= 300 {
            return Err(apikey::status_error(status, None, Some(&value)));
        }
        Ok(value["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v["id"].as_str())
            .map(|id| crate::catalog::model("copilot", id, Wire::Chat))
            .collect())
    }
}
