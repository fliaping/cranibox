#[test]
fn registered_adapters_and_presets_are_valid() {
    assert_eq!(cranibox_llm::LLM_ADDR, "127.0.0.1:3424");
    let registry = cranibox_llm::upstream::registry();
    for name in ["apikey", "codex", "kimi", "gemini", "copilot"] {
        assert!(registry.iter().any(|u| u.provider() == name));
    }
    let presets = cranibox_llm::catalog::presets();
    assert!(presets.len() >= 20);
    let mut ids = std::collections::HashSet::new();
    for preset in presets {
        cranibox_llm::catalog::validate(&preset).unwrap();
        assert!(ids.insert(preset.id));
    }
}
