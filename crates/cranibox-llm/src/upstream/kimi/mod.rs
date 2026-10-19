//! Kimi Code subscription adapter. WS3.
// Adapted from pleaseai/shunt src/auth/kimi and adapters/anthropic@9d42ba251dd1f0651e9d150767e3ff98a1ae042b (MIT OR Apache-2.0).
use super::apikey::{self, Credential};
use cranibox_contracts::llm::*;
pub const CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
pub fn msh(request: reqwest::RequestBuilder, device: &str) -> reqwest::RequestBuilder {
    request
        .header("x-msh-platform", "cranibox")
        .header("x-msh-version", env!("CARGO_PKG_VERSION"))
        .header("x-msh-device-name", "Cranibox")
        .header(
            "x-msh-device-model",
            format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        )
        .header("x-msh-device-id", device)
}
pub struct Kimi {
    base: Option<String>,
    token_url: String,
}
impl Default for Kimi {
    fn default() -> Self {
        Self {
            base: None,
            token_url: "https://auth.kimi.com/api/oauth/token".into(),
        }
    }
}
impl Kimi {
    pub fn for_loopback(base: String, token_url: String) -> Result<Self, String> {
        crate::auth::require_loopback(&base)?;
        crate::auth::require_loopback(&token_url)?;
        Ok(Self {
            base: Some(base),
            token_url,
        })
    }
}
#[async_trait::async_trait]
impl Upstream for Kimi {
    fn provider(&self) -> &'static str {
        "kimi"
    }
    fn wires(&self) -> &[Wire] {
        &[Wire::Messages, Wire::Chat]
    }
    async fn send(
        &self,
        cx: &UpstreamCx,
        req: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        let credential = Credential::decode(&cx.secret)?;
        let mut body = req.body;
        let preset = crate::catalog::preset("kimi-code");
        crate::upstream::cn::normalize(preset.as_ref(), &mut body)?;
        let base = match &self.base {
            Some(v) => v.clone(),
            None => format!(
                "https://api.kimi.{}/coding{}",
                if credential.region.as_deref() == Some("cn") {
                    "com"
                } else {
                    "ai"
                },
                if req.wire == Wire::Chat { "/v1" } else { "" }
            ),
        };
        let request = msh(
            apikey::client()
                .post(apikey::endpoint(&base, req.wire, false)?)
                .bearer_auth(&credential.access_token)
                .header("anthropic-version", "2023-06-01"),
            credential.device_id.as_deref().unwrap_or(&cx.account.id),
        );
        apikey::response(
            request
                .json(&body)
                .send()
                .await
                .map_err(|_| UpstreamError::Network("Kimi connection failed".into()))?,
            req.wire,
        )
        .await
    }
    async fn refresh(
        &self,
        _cx: &UpstreamCx,
        secret: &SecretBox,
    ) -> Result<Refreshed, UpstreamError> {
        crate::auth::refresh(secret, &self.token_url, CLIENT_ID, None).await
    }
    async fn models(&self, _cx: &UpstreamCx) -> Result<Vec<ModelEntry>, UpstreamError> {
        Ok(vec![crate::catalog::model(
            "kimi",
            "kimi-for-coding",
            Wire::Messages,
        )])
    }
}
