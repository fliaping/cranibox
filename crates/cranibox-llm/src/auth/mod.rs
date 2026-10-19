//! Subscription sign-in and local refresh. WS3.
use crate::upstream::apikey::{self, Credential};
use cranibox_contracts::llm::*;
use serde_json::Value;
pub mod signin;
pub use signin::{OAuth, OAuthConfig};

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn require_loopback(raw: &str) -> Result<(), String> {
    let url = crate::catalog::validate_url(raw)?;
    if !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")) {
        return Err("test endpoint must be loopback".into());
    }
    Ok(())
}
pub fn tokens(value: &Value, mut old: Credential) -> Result<Credential, UpstreamError> {
    let token = value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or(UpstreamError::Auth)?;
    old.access_token = token.into();
    if let Some(refresh) = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        old.refresh_token = Some(refresh.into());
    }
    old.expires_at = Some(
        now().saturating_add(
            value
                .get("expires_in")
                .and_then(Value::as_u64)
                .unwrap_or(3600),
        ),
    );
    if let Some(token) = value.get("id_token").and_then(Value::as_str) {
        old.account_id = jwt_account_id(token).or(old.account_id);
    }
    Ok(old)
}
/// JWT claims are metadata only; they never authenticate a user locally.
fn jwt_account_id(token: &str) -> Option<String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(token.split('.').nth(1)?)
        .ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value
        .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
}
pub async fn refresh(
    secret: &SecretBox,
    url: &str,
    client_id: &str,
    client_secret: Option<&str>,
) -> Result<Refreshed, UpstreamError> {
    let old = Credential::decode(secret)?;
    let refresh = old.refresh_token.as_deref().ok_or(UpstreamError::Auth)?;
    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh),
        ("client_id", client_id),
    ];
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret));
    }
    let mut request = apikey::client().post(url).form(&form);
    if client_id == crate::upstream::kimi::CLIENT_ID
        && let Some(device) = old.device_id.as_deref()
    {
        request = crate::upstream::kimi::msh(request, device);
    }
    let response = request
        .send()
        .await
        .map_err(|_| UpstreamError::Network("token endpoint connection failed".into()))?;
    let (status, value) = apikey::json_response(response, 1024 * 1024).await?;
    if status >= 300 || value.get("error").is_some() {
        return Err(apikey::status_error(status, None, Some(&value)));
    }
    let new = tokens(&value, old)?;
    Ok(Refreshed {
        expires_at: new.expires_at,
        secret: new.secret(),
    })
}
/// Explicit only: callers must obtain acknowledgement before importing a rotating CLI login.
pub fn import_codex(
    store: &dyn cranibox_contracts::vault::AccountStore,
    node: &str,
    path: &std::path::Path,
    acknowledge_cli_may_be_signed_out: bool,
) -> Result<AccountSummary, MgmtError> {
    if !acknowledge_cli_may_be_signed_out {
        return Err(MgmtError::Invalid(
            "导入可能使原 Codex CLI 登录失效，请先确认；建议重新登录".into(),
        ));
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(1_048_577).read_to_end(&mut bytes))
        .map_err(|_| MgmtError::Invalid("无法读取 Codex 登录文件".into()))?;
    if bytes.len() > 1_048_576 {
        return Err(MgmtError::Invalid("Codex 登录文件超过大小限制".into()));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| MgmtError::Invalid("Codex 登录文件格式无效".into()))?;
    let tokens = &value["tokens"];
    let access = tokens["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| MgmtError::Invalid("该文件没有订阅 access token".into()))?;
    let credential = Credential {
        access_token: access.into(),
        refresh_token: tokens["refresh_token"].as_str().map(str::to_owned),
        account_id: tokens["account_id"].as_str().map(str::to_owned),
        expires_at: Some(0),
        ..Default::default()
    };
    let summary = signin::summary("codex", node, Some("导入的 Codex 登录".into()));
    store
        .put(summary.clone(), Some(credential.secret()))
        .map_err(|_| MgmtError::Other("vault write failed".into()))?;
    Ok(summary)
}
