//! Upstream registry. **Registry owned by WS2.** WS3 and WS4 each add one
//! line in their marked block; nothing else in this file changes.
use cranibox_contracts::llm::Upstream;
use std::sync::Arc;

pub mod apikey;
pub mod claude;
pub mod cn;
pub mod codex;
pub mod copilot;
pub mod gemini;
pub mod kimi;

/// Every compiled-in upstream, in lookup order.
pub fn registry() -> Vec<Arc<dyn Upstream>> {
    #[allow(unused_mut)]
    let mut upstreams: Vec<Arc<dyn Upstream>> = Vec::new();
    // --- WS2: API key and custom providers ---

    // --- WS3: codex, kimi, gemini, copilot, cn ---

    // --- WS4: claude ---

    upstreams
}
