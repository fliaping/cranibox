//! Codex subscription adapter. WS3.
// Wire constants adapted from pleaseai/shunt src/auth/codex and
// src/adapters/responses@9d42ba251dd1f0651e9d150767e3ff98a1ae042b (MIT OR Apache-2.0).
use super::apikey::{self, Credential};
use cranibox_contracts::llm::*;
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub struct Codex {
    base: String,
    token_url: String,
}
impl Default for Codex {
    fn default() -> Self {
        Self {
            base: "https://chatgpt.com/backend-api/codex".into(),
            token_url: "https://auth.openai.com/oauth/token".into(),
        }
    }
}
impl Codex {
    pub fn for_loopback(base: String, token_url: String) -> Result<Self, String> {
        crate::auth::require_loopback(&base)?;
        crate::auth::require_loopback(&token_url)?;
        Ok(Self { base, token_url })
    }
}
#[async_trait::async_trait]
impl Upstream for Codex {
    fn provider(&self) -> &'static str {
        "codex"
    }
    fn wires(&self) -> &[Wire] {
        &[Wire::Responses]
    }
    async fn send(
        &self,
        cx: &UpstreamCx,
        req: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        if req.wire != Wire::Responses {
            return Err(UpstreamError::Unsupported);
        }
        let credential = Credential::decode(&cx.secret)?;
        let mut body = req.body;
        body["store"] = false.into();
        body["stream"] = true.into();
        if body.get("instructions").is_none() {
            body["instructions"] = "".into();
        }
        if let Some(map) = body.as_object_mut() {
            for key in ["max_output_tokens", "temperature", "top_p"] {
                map.remove(key);
            }
        }
        let mut request = apikey::client()
            .post(apikey::endpoint(&self.base, Wire::Responses, false)?)
            .bearer_auth(&credential.access_token)
            .header("accept", "text/event-stream")
            .header("openai-beta", "responses=experimental")
            .header("originator", "codex_cli_rs");
        if let Some(account) = credential.account_id {
            request = request.header("chatgpt-account-id", account);
        }
        if let Some(session) = req.session_id {
            body["prompt_cache_key"] = session.clone().into();
            request = request
                .header("session-id", &session)
                .header("thread-id", &session);
        }
        let response = request
            .json(&body)
            .send()
            .await
            .map_err(|_| UpstreamError::Network("Codex connection failed".into()))?;
        apikey::response(response, Wire::Responses).await
    }
    async fn refresh(
        &self,
        _cx: &UpstreamCx,
        secret: &SecretBox,
    ) -> Result<Refreshed, UpstreamError> {
        crate::auth::refresh(secret, &self.token_url, CLIENT_ID, None).await
    }
    async fn models(&self, _cx: &UpstreamCx) -> Result<Vec<ModelEntry>, UpstreamError> {
        Ok(["gpt-5.4", "gpt-5.4-mini", "gpt-5.3-codex"]
            .iter()
            .map(|m| crate::catalog::model("codex", m, Wire::Responses))
            .collect())
    }
}
