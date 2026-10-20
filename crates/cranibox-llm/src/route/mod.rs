//! Request routing.
pub mod health;
pub mod local;
pub mod remote;

use cranibox_contracts::{error::ErrorCode, llm::*};
/// WS6 supplies roster/lease policy and authenticated mesh forwarding. No TCP fallback.
pub enum RouteDecision {
    Local,
    Remote { node: String },
    Unavailable { code: ErrorCode },
}
#[async_trait::async_trait]
pub trait Router: Send + Sync {
    async fn decide(&self, provider: &str, request: &CanonicalRequest) -> RouteDecision;
    async fn forward(
        &self,
        node: &str,
        provider: &str,
        request: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError>;
    /// Forward the count operation to the peer count endpoint, never to generation.
    async fn forward_count_tokens(
        &self,
        _node: &str,
        _provider: &str,
        _request: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        Err(UpstreamError::Unsupported)
    }
}
