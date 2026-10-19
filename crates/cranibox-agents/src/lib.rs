//! Agent adapters (C4). **Owned by WS5.**
//!
//! Phase 0 skeleton: the registry signature and order are fixed here; WS5
//! adds the six adapters (see `docs/design/tasks/05-agent-adapters.md`).
pub use cranibox_contracts::agents::*;

/// All adapters, in [`AgentId::ALL`] order: DSH, Codex, Claude Code, Pi,
/// Hermes, OpenClaw. Empty until WS5 lands the adapters.
pub fn registry() -> &'static [&'static dyn AgentAdapter] {
    &[]
}

/// Adapter for `id`, if registered.
pub fn adapter(id: AgentId) -> Option<&'static dyn AgentAdapter> {
    registry().iter().copied().find(|a| a.id() == id)
}
