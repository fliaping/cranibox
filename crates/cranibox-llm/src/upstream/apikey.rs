//! API-key and custom-provider upstream driven by presets. WS2.
use cranibox_contracts::{llm::*, net::BoxBody};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;
use std::{collections::BTreeMap, sync::OnceLock, time::Duration};

/// Serialized only inside C7 SecretBox, never in an account summary.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Credential {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub endpoints: BTreeMap<Wire, String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub github_token: Option<String>,
}
impl Credential {
    pub fn decode(secret: &SecretBox) -> Result<Self, UpstreamError> {
        serde_json::from_slice(secret.expose()).map_err(|_| UpstreamError::Auth)
    }
    pub fn secret(&self) -> SecretBox {
        SecretBox::new(serde_json::to_vec(self).expect("credential serialization"))
    }
}
pub fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(90))
            .user_agent(concat!("cranibox/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("HTTP client")
    })
}
pub fn endpoint(base: &str, wire: Wire, count: bool) -> Result<String, UpstreamError> {
    crate::catalog::validate_url(base)
        .map_err(|_| UpstreamError::Status(400, "invalid upstream endpoint".into()))?;
    let path = match (wire, count) {
        (Wire::Chat, _) => "chat/completions",
        (Wire::Responses, _) => "responses",
        (Wire::Messages, true) => "v1/messages/count_tokens",
        (Wire::Messages, false) => "v1/messages",
    };
    let base = base.trim_end_matches('/');
    let path = if wire == Wire::Messages && base.ends_with("/v1") {
        path.trim_start_matches("v1/")
    } else {
        path
    };
    Ok(format!("{base}/{path}"))
}
pub async fn json_response(
    response: reqwest::Response,
    limit: usize,
) -> Result<(u16, Value), UpstreamError> {
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|_| UpstreamError::Network("upstream body interrupted".into()))?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(UpstreamError::Status(
                502,
                "upstream JSON exceeds limit".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let value = serde_json::from_slice(&bytes)
        .map_err(|_| UpstreamError::Status(502, "invalid upstream JSON".into()))?;
    Ok((status, value))
}
pub fn status_error(status: u16, retry_after: Option<u64>, value: Option<&Value>) -> UpstreamError {
    let code = value
        .and_then(|v| v.pointer("/error/code").or_else(|| v.get("error")))
        .and_then(Value::as_str)
        .unwrap_or("");
    if matches!(
        code,
        "token_revoked" | "invalid_grant" | "refresh_token_reused"
    ) || status == 401
    {
        return UpstreamError::Auth;
    }
    if matches!(code, "insufficient_quota" | "quota_exhausted") || status == 402 {
        return UpstreamError::QuotaExhausted { reset_at: None };
    }
    if status == 429 {
        return UpstreamError::RateLimited { retry_after };
    }
    UpstreamError::Status(
        if (400..600).contains(&status) {
            status
        } else {
            502
        },
        "upstream rejected request".into(),
    )
}
pub async fn response(
    response: reqwest::Response,
    wire: Wire,
) -> Result<UpstreamResponse, UpstreamError> {
    let status = response.status().as_u16();
    if !response.status().is_success() {
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|h| h.to_str().ok())
            .and_then(|v| v.parse().ok());
        let value = json_response(response, 65536).await.ok().map(|(_, v)| v);
        return Err(status_error(status, retry, value.as_ref()));
    }
    let headers = response
        .headers()
        .iter()
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "content-type" | "retry-after" | "x-request-id" | "request-id"
            )
        })
        .filter_map(|(n, v)| v.to_str().ok().map(|v| (n.to_string(), v.to_string())))
        .collect();
    let body = BoxBody::new(response.bytes_stream().map(|r| {
        r.map_err(|_| {
            Box::new(std::io::Error::other("upstream stream interrupted"))
                as cranibox_contracts::net::BodyError
        })
    }));
    Ok(UpstreamResponse {
        status,
        headers,
        wire,
        body,
    })
}
pub fn endpoints(cx: &UpstreamCx, credential: &Credential) -> BTreeMap<Wire, String> {
    if !credential.endpoints.is_empty() {
        return credential.endpoints.clone();
    }
    cx.preset
        .as_ref()
        .map(|p| {
            [
                (Wire::Chat, &p.chat),
                (Wire::Responses, &p.responses),
                (Wire::Messages, &p.anthropic),
            ]
            .into_iter()
            .filter_map(|(w, v)| v.clone().map(|v| (w, v)))
            .collect()
        })
        .unwrap_or_default()
}
pub struct ApiKey;
impl ApiKey {
    pub async fn send_count(
        &self,
        cx: &UpstreamCx,
        req: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        self.send_inner(cx, req, true).await
    }
    async fn send_inner(
        &self,
        cx: &UpstreamCx,
        req: CanonicalRequest,
        count: bool,
    ) -> Result<UpstreamResponse, UpstreamError> {
        let credential = Credential::decode(&cx.secret)?;
        let endpoints = endpoints(cx, &credential);
        let base = endpoints.get(&req.wire).ok_or(UpstreamError::Unsupported)?;
        if count && req.wire != Wire::Messages {
            return Err(UpstreamError::Unsupported);
        }
        let mut body = req.body;
        if count {
            body.as_object_mut()
                .ok_or(UpstreamError::Unsupported)?
                .remove("stream");
        }
        crate::upstream::cn::normalize(cx.preset.as_ref(), &mut body)?;
        let zhipu = cx
            .preset
            .as_ref()
            .is_some_and(|p| p.quirks.iter().any(|q| q == "zhipu_envelope"));
        let cache_key = format!(
            "{}:{:x}",
            cx.account.id,
            sha2::Sha256::digest([credential.access_token.as_bytes(), base.as_bytes()].concat())
        );
        let mut selected = if zhipu && req.wire == Wire::Chat {
            learned()
                .lock()
                .expect("endpoint cache")
                .get(&cache_key)
                .cloned()
                .unwrap_or_else(|| base.clone())
        } else {
            base.clone()
        };
        for attempt in 0..2 {
            let mut request = client()
                .post(endpoint(&selected, req.wire, count)?)
                .json(&body);
            request = if req.wire == Wire::Messages {
                request
                    .header("x-api-key", &credential.access_token)
                    .header("anthropic-version", "2023-06-01")
            } else {
                request.bearer_auth(&credential.access_token)
            };
            if cx
                .preset
                .as_ref()
                .is_some_and(|p| p.quirks.iter().any(|q| q == "kimi_msh"))
            {
                request = crate::upstream::kimi::msh(
                    request,
                    credential.device_id.as_deref().unwrap_or(&cx.account.id),
                );
            }
            let result = request
                .send()
                .await
                .map_err(|_| UpstreamError::Network("upstream connection failed".into()))?;
            let is_json = result
                .headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok())
                .is_some_and(|v| v.contains("json"));
            if zhipu && (!req.stream || is_json || !result.status().is_success()) {
                let retry = result
                    .headers()
                    .get("retry-after")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse().ok());
                let (status, value) = json_response(result, 32 * 1024 * 1024).await?;
                let code = value.get("code").or_else(|| value.pointer("/error/code"));
                let is_1113 = code.is_some_and(|v| v == 1113 || v == "1113");
                if attempt == 0
                    && req.wire == Wire::Chat
                    && is_1113
                    && let Some(coding) = coding_endpoint(&selected)
                {
                    selected = coding;
                    continue;
                }
                if status >= 300
                    || value.get("error").is_some()
                    || value.get("success") == Some(&Value::Bool(false))
                {
                    return Err(crate::upstream::cn::envelope_error(status, retry, &value));
                }
                if selected != *base {
                    remember(&cache_key, &selected);
                }
                return Ok(UpstreamResponse {
                    status,
                    headers: vec![("content-type".into(), "application/json".into())],
                    wire: req.wire,
                    body: BoxBody::full(serde_json::to_vec(&value).expect("JSON")),
                });
            }
            if result.status().is_success() && selected != *base {
                remember(&cache_key, &selected);
            }
            return response(result, req.wire).await;
        }
        Err(UpstreamError::RateLimited { retry_after: None })
    }
}
#[async_trait::async_trait]
impl Upstream for ApiKey {
    fn provider(&self) -> &'static str {
        "apikey"
    }
    fn wires(&self) -> &[Wire] {
        &[Wire::Chat, Wire::Responses, Wire::Messages]
    }
    async fn send(
        &self,
        cx: &UpstreamCx,
        req: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        self.send_inner(cx, req, false).await
    }
    async fn models(&self, cx: &UpstreamCx) -> Result<Vec<ModelEntry>, UpstreamError> {
        let credential = Credential::decode(&cx.secret)?;
        let preset = cx.preset.as_ref();
        let wire = preset.map_or_else(
            || {
                credential
                    .endpoints
                    .keys()
                    .next()
                    .copied()
                    .unwrap_or(Wire::Chat)
            },
            |p| p.default_wire,
        );
        let fallback = if credential.models.is_empty() {
            preset.map(|p| p.models.clone()).unwrap_or_default()
        } else {
            credential.models.clone()
        };
        if preset.is_some_and(|p| p.no_list) || !credential.models.is_empty() {
            return Ok(fallback
                .iter()
                .map(|m| crate::catalog::model(&cx.account.provider, m, wire))
                .collect());
        }
        let endpoints = endpoints(cx, &credential);
        let base = endpoints.get(&wire).ok_or(UpstreamError::Unsupported)?;
        let url = if wire == Wire::Messages && !base.trim_end_matches('/').ends_with("/v1") {
            format!("{}/v1/models", base.trim_end_matches('/'))
        } else {
            format!("{}/models", base.trim_end_matches('/'))
        };
        crate::catalog::validate_url(&url).map_err(|_| UpstreamError::Unsupported)?;
        let request = client().get(url);
        let request = if wire == Wire::Messages {
            request
                .header("x-api-key", &credential.access_token)
                .header("anthropic-version", "2023-06-01")
        } else {
            request.bearer_auth(&credential.access_token)
        };
        let discovered = async {
            let response = request
                .send()
                .await
                .map_err(|_| UpstreamError::Network("model discovery connection failed".into()))?;
            let (status, value) = json_response(response, 4 * 1024 * 1024).await?;
            if status >= 300 {
                return Err(status_error(status, None, Some(&value)));
            }
            let data = value["data"].as_array().ok_or(UpstreamError::Unsupported)?;
            Ok::<_, UpstreamError>(
                data.iter()
                    .take(10000)
                    .filter_map(|m| m["id"].as_str().filter(|s| !s.is_empty()))
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
            )
        }
        .await;
        let models = match discovered {
            Ok(models) => models,
            Err(_) if !fallback.is_empty() => fallback,
            Err(e) => return Err(e),
        };
        Ok(models
            .iter()
            .map(|m| crate::catalog::model(&cx.account.provider, m, wire))
            .collect())
    }
}

// Learned endpoints are scoped to an account + key hash. Never follow an upstream redirect.
fn learned() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>> {
    static LEARNED: OnceLock<std::sync::Mutex<std::collections::HashMap<String, String>>> =
        OnceLock::new();
    LEARNED.get_or_init(Default::default)
}
fn remember(key: &str, endpoint: &str) {
    let mut cache = learned().lock().expect("endpoint cache");
    if cache.len() >= 4096 {
        cache.clear();
    }
    cache.insert(key.into(), endpoint.into());
}
fn coding_endpoint(base: &str) -> Option<String> {
    let mut url = crate::catalog::validate_url(base).ok()?;
    if !matches!(
        url.host_str(),
        Some("open.bigmodel.cn" | "api.z.ai" | "localhost" | "127.0.0.1" | "[::1]")
    ) {
        return None;
    }
    if url.path().trim_end_matches('/') != "/api/paas/v4" {
        return None;
    }
    url.set_path("/api/coding/paas/v4");
    Some(url.into())
}
