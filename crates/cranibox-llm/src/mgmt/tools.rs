//! Optional management helpers. Curl is parsed as data and never executed.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Deserialize, Serialize)]
pub struct ImportCurl {
    pub curl: String,
    pub label: String,
}
#[derive(Deserialize, Serialize, Default)]
pub struct Probe {
    pub model: Option<String>,
    pub wire: Option<Wire>,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct ProbeResult {
    pub ok: bool,
    pub wire: Wire,
    pub latency_ms: u64,
    pub error: Option<String>,
}
pub fn parse_curl(input: &ImportCurl) -> Result<NewAccount, MgmtError> {
    if input.curl.len() > 1_048_576 {
        return Err(MgmtError::Invalid("curl exceeds size limit".into()));
    }
    let words =
        shlex::split(&input.curl).ok_or_else(|| MgmtError::Invalid("curl 引号不完整".into()))?;
    let mut args = words.iter();
    if args.next().is_none_or(|s| s != "curl") {
        return Err(MgmtError::Invalid("请输入 curl 命令".into()));
    }
    let (mut target, mut key, mut data) = (None, None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-H" | "--header" => {
                let header = args
                    .next()
                    .ok_or_else(|| MgmtError::Invalid("curl header missing".into()))?;
                let (name, value) = header
                    .split_once(':')
                    .ok_or_else(|| MgmtError::Invalid("invalid curl header".into()))?;
                let value = value.trim();
                if name.eq_ignore_ascii_case("authorization") {
                    key = value.strip_prefix("Bearer ").map(str::to_owned);
                }
                if name.eq_ignore_ascii_case("x-api-key") {
                    key = Some(value.to_string());
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--json" => {
                let value = args
                    .next()
                    .ok_or_else(|| MgmtError::Invalid("curl body missing".into()))?;
                if value.starts_with('@') {
                    return Err(MgmtError::Invalid(
                        "curl 文件引用不支持，请粘贴 JSON 请求体".into(),
                    ));
                }
                data = Some(value);
            }
            "--url" => {
                target = args.next().cloned();
            }
            "-X" | "--request" => {
                if args.next().is_none_or(|s| s != "POST") {
                    return Err(MgmtError::Invalid("只支持 POST curl".into()));
                }
            }
            "-s" | "-S" | "-sS" | "--silent" | "--show-error" | "-N" | "--no-buffer" => {}
            value if !value.starts_with('-') && target.is_none() => {
                target = Some(value.to_string())
            }
            _ => {
                return Err(MgmtError::Invalid(
                    "curl 含不支持的选项；仅接受 URL、POST、header 和 JSON body".into(),
                ));
            }
        }
    }
    let target = target.ok_or_else(|| MgmtError::Invalid("curl URL missing".into()))?;
    let mut url = catalog::validate_url(&target).map_err(MgmtError::Invalid)?;
    let (wire, suffix) = if url.path().ends_with("/chat/completions") {
        (Wire::Chat, "/chat/completions")
    } else if url.path().ends_with("/responses") {
        (Wire::Responses, "/responses")
    } else if url.path().ends_with("/v1/messages") {
        (Wire::Messages, "/v1/messages")
    } else {
        return Err(MgmtError::Invalid(
            "curl URL 需要指向 Chat、Responses 或 Messages 接口".into(),
        ));
    };
    let path = url.path().trim_end_matches(suffix).to_string();
    url.set_path(&path);
    let key = key.ok_or_else(|| MgmtError::Invalid("curl 中没有 Bearer / x-api-key".into()))?;
    validate_key(&key)?;
    let mut models = Vec::new();
    if let Some(data) = data {
        let value: serde_json::Value = serde_json::from_str(data)
            .map_err(|_| MgmtError::Invalid("curl body must be JSON".into()))?;
        if let Some(model) = value["model"].as_str() {
            models.push(model.to_owned());
        }
    }
    Ok(NewAccount::Custom {
        label: input.label.clone(),
        key,
        endpoints: BTreeMap::from([(wire, url.to_string().trim_end_matches('/').to_string())]),
        models,
    })
}
impl LocalMgmt {
    pub async fn import_curl(&self, request: ImportCurl) -> Result<AccountSummary, MgmtError> {
        self.add_account(parse_curl(&request)?).await
    }
    /// A user-triggered, billable minimal request; never falls back to a different account.
    pub async fn test_account(
        &self,
        id: &str,
        request: Probe,
    ) -> Result<Vec<ProbeResult>, MgmtError> {
        self.engine.probe(id, request).await
    }
}
impl Client {
    pub async fn import_curl(&self, request: ImportCurl) -> Result<AccountSummary, MgmtError> {
        self.call(
            http::Method::POST,
            "/accounts/import-curl",
            Some(json!(request)),
        )
        .await
    }
    pub async fn test_account(
        &self,
        id: &str,
        request: Probe,
    ) -> Result<Vec<ProbeResult>, MgmtError> {
        self.call(
            http::Method::POST,
            &account_path(id, "/test")?,
            Some(json!(request)),
        )
        .await
    }
}
pub(super) async fn import(State(m): State<LocalMgmt>, Json(v): Json<ImportCurl>) -> Response {
    answer(m.import_curl(v).await)
}
pub(super) async fn probe(
    State(m): State<LocalMgmt>,
    Path(id): Path<String>,
    Json(v): Json<Probe>,
) -> Response {
    answer(m.test_account(&id, v).await)
}
