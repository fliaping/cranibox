#[test]
fn skeleton_compiles_with_empty_registry() {
    assert_eq!(cranibox_llm::LLM_ADDR, "127.0.0.1:3424");
    assert!(cranibox_llm::upstream::registry().is_empty());
}
