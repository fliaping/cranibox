use cranibox_agents::{AgentId, DEFAULT_AGENT, registry};

#[test]
fn registry_order_matches_contract() {
    let ids: Vec<AgentId> = registry().iter().map(|a| a.id()).collect();
    // Registered adapters must appear in contract order (prefix of AgentId::ALL order).
    let expected: Vec<AgentId> = AgentId::ALL
        .into_iter()
        .filter(|id| ids.contains(id))
        .collect();
    assert_eq!(ids, expected);
    assert_eq!(DEFAULT_AGENT, AgentId::DeepseekHarness);
}
