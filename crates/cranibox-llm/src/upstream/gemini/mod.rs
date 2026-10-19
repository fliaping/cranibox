//! Gemini Code Assist: Google OAuth with operator-provided OAuth client, no CLI credential import.
// Adapted from pleaseai/shunt src/auth/google and adapters/gemini
// @9d42ba251dd1f0651e9d150767e3ff98a1ae042b (MIT OR Apache-2.0).
use super::apikey::{self, Credential};
use crate::proto::{
    SseDecoder,
    vendor::shunt::model::{gemini::GeminiSseMachine, gemini_request},
};
use bytes::Bytes;
use cranibox_contracts::{
    llm::*,
    net::{BodyError, BoxBody},
};
use futures_util::StreamExt;
use serde_json::Value;
pub struct Gemini {
    base: String,
    token_url: String,
    client_id: Option<String>,
    client_secret: Option<String>,
}
impl Default for Gemini {
    fn default() -> Self {
        Self {
            base: "https://cloudcode-pa.googleapis.com".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            client_id: std::env::var("CRANIBOX_GOOGLE_CLIENT_ID").ok(),
            client_secret: std::env::var("CRANIBOX_GOOGLE_CLIENT_SECRET").ok(),
        }
    }
}
impl Gemini {
    pub fn for_loopback(base: String, token_url: String) -> Result<Self, String> {
        crate::auth::require_loopback(&base)?;
        crate::auth::require_loopback(&token_url)?;
        Ok(Self {
            base,
            token_url,
            client_id: Some("test-client".into()),
            client_secret: Some("test-secret".into()),
        })
    }
    pub async fn discover(base: &str, token: &str) -> Result<String, UpstreamError> {
        let response = apikey::client()
            .post(format!(
                "{}/v1internal:loadCodeAssist",
                base.trim_end_matches('/')
            ))
            .bearer_auth(token)
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|_| UpstreamError::Network("Code Assist discovery failed".into()))?;
        let (status, value) = apikey::json_response(response, 1024 * 1024).await?;
        if status >= 300 {
            return Err(apikey::status_error(status, None, Some(&value)));
        }
        value
            .get("cloudaicompanionProject")
            .or_else(|| value.get("projectId"))
            .and_then(|p| p.as_str().or_else(|| p.get("id").and_then(Value::as_str)))
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                UpstreamError::Status(
                    409,
                    "请先在 Gemini Code Assist 完成账号开通；未获得 project id".into(),
                )
            })
    }
}
#[async_trait::async_trait]
impl Upstream for Gemini {
    fn provider(&self) -> &'static str {
        "gemini"
    }
    fn wires(&self) -> &[Wire] {
        &[Wire::Messages]
    }
    async fn send(
        &self,
        cx: &UpstreamCx,
        req: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        let credential = Credential::decode(&cx.secret)?;
        let project = credential.project.as_deref().ok_or(UpstreamError::Auth)?;
        let payload = gemini_request::translate_request_for_model(&req.body, &req.model)
            .map_err(|_| crate::proto::invalid("request cannot be converted to Gemini"))?;
        let payload = gemini_request::wrap_code_assist_envelope(&req.model, project, payload);
        let method = if req.stream {
            "streamGenerateContent?alt=sse"
        } else {
            "generateContent"
        };
        let response = apikey::client()
            .post(format!(
                "{}/v1internal:{method}",
                self.base.trim_end_matches('/')
            ))
            .bearer_auth(&credential.access_token)
            .json(&payload)
            .send()
            .await
            .map_err(|_| UpstreamError::Network("Gemini connection failed".into()))?;
        if !response.status().is_success() {
            return apikey::response(response, Wire::Messages).await;
        }
        let body = if req.stream {
            BoxBody::new(async_stream::try_stream! {
                let mut stream=response.bytes_stream();let mut decoder=SseDecoder::default();let mut machine=GeminiSseMachine::new_streaming(&req.model);
                while let Some(chunk)=stream.next().await{
                    let chunk=chunk.map_err(|_|Box::new(std::io::Error::other("Gemini stream interrupted")) as BodyError)?;
                    for event in decoder.push(&chunk)?{
                        let events=machine.process_chunk_checked(&event.data).map_err(|_|Box::new(std::io::Error::other("invalid Gemini stream")) as BodyError)?;
                        for event in events{yield Bytes::from(format!("event: {}\ndata: {}\n\n",event.event,event.data));}
                    }
                }
                decoder.finish()?;
                for event in machine.transport_close_checked().map_err(|_|Box::new(std::io::Error::other("incomplete Gemini stream")) as BodyError)?{yield Bytes::from(format!("event: {}\ndata: {}\n\n",event.event,event.data));}
            })
        } else {
            let (_, value) = apikey::json_response(response, 32 * 1024 * 1024).await?;
            let mut machine = GeminiSseMachine::new(&req.model);
            machine
                .process_chunk_checked(&value)
                .map_err(|_| UpstreamError::Status(502, "invalid Gemini response".into()))?;
            machine
                .transport_close_checked()
                .map_err(|_| UpstreamError::Status(502, "incomplete Gemini response".into()))?;
            let value = machine
                .final_json_checked()
                .map_err(|_| UpstreamError::Status(502, "invalid Gemini response".into()))?;
            BoxBody::full(serde_json::to_vec(&value).expect("JSON"))
        };
        Ok(UpstreamResponse {
            status: 200,
            headers: vec![(
                "content-type".into(),
                if req.stream {
                    "text/event-stream"
                } else {
                    "application/json"
                }
                .into(),
            )],
            wire: Wire::Messages,
            body,
        })
    }
    async fn refresh(
        &self,
        _cx: &UpstreamCx,
        secret: &SecretBox,
    ) -> Result<Refreshed, UpstreamError> {
        crate::auth::refresh(
            secret,
            &self.token_url,
            self.client_id
                .as_deref()
                .ok_or(UpstreamError::Unsupported)?,
            self.client_secret.as_deref(),
        )
        .await
    }
    async fn models(&self, _cx: &UpstreamCx) -> Result<Vec<ModelEntry>, UpstreamError> {
        Ok(["gemini-2.5-pro", "gemini-2.5-flash"]
            .iter()
            .map(|m| crate::catalog::model("gemini", m, Wire::Messages))
            .collect())
    }
}
