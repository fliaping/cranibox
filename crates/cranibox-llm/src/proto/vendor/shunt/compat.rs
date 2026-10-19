//! Cranibox-owned compatibility types, avoiding dependencies on shunt routing/auth.
use axum::{http::StatusCode, response::Response};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponsesFlavor {
    Openai,
    Chatgpt,
    Xai,
    Grok,
}
pub enum AdapterKind {
    Responses,
}
pub struct Route {
    pub provider: String,
    pub adapter: AdapterKind,
    pub model: String,
    pub upstream_model: String,
    pub effort: Option<String>,
    pub service_tier: Option<String>,
}
#[derive(Debug)]
pub enum AdapterFailure {
    UpstreamStatus(StatusCode),
}
#[derive(Debug)]
pub struct AdapterError {
    pub message: String,
    pub response: Box<Response>,
    pub failure: Option<AdapterFailure>,
}
pub fn truncate_for_error(_: &str) -> &'static str {
    "[redacted tool id]"
}
