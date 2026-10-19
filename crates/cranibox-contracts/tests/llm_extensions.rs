//! Existing implementations keep compiling and never generate to count tokens.
use cranibox_contracts::{llm::*, mock::MockLlmMgmt};

struct Legacy;
#[async_trait::async_trait]
impl Upstream for Legacy {
    fn provider(&self) -> &'static str {
        "legacy"
    }
    fn wires(&self) -> &[Wire] {
        &[Wire::Messages]
    }
    async fn send(
        &self,
        _: &UpstreamCx,
        _: CanonicalRequest,
    ) -> Result<UpstreamResponse, UpstreamError> {
        panic!("count_tokens must not call send")
    }
    async fn models(&self, _: &UpstreamCx) -> Result<Vec<ModelEntry>, UpstreamError> {
        Ok(vec![])
    }
}
#[async_trait::async_trait]
impl SignIn for Legacy {
    async fn start(&self, _: &str, _: SignInOpts) -> Result<SignInStart, MgmtError> {
        unreachable!()
    }
    async fn complete(&self, _: &str, _: SignInInput) -> Result<AccountSummary, MgmtError> {
        unreachable!()
    }
}

#[tokio::test]
async fn legacy_count_tokens_is_explicitly_unsupported() {
    let cx = UpstreamCx {
        account: AccountSummary {
            id: "fake".into(),
            provider: "legacy".into(),
            plan: None,
            label: "fake".into(),
            kind: AccountKind::ApiKey,
            home_node: "local".into(),
            sync: false,
            enabled_on: vec!["local".into()],
            all_nodes: false,
            state: AccountState::Ready,
        },
        secret: SecretBox::new(vec![]),
        preset: None,
        node: "local".into(),
    };
    let request = CanonicalRequest {
        wire: Wire::Messages,
        body: serde_json::json!({}),
        stream: false,
        session_id: None,
        client_agent: None,
        model: "test".into(),
    };
    assert!(matches!(
        Legacy.count_tokens(&cx, request).await,
        Err(UpstreamError::Unsupported)
    ));
}

#[tokio::test]
async fn legacy_cancellation_is_explicitly_unsupported() {
    use cranibox_contracts::llm::mgmt::LlmMgmt;
    assert!(matches!(
        SignIn::cancel(&Legacy, "flow").await,
        Err(MgmtError::Upstream(UpstreamError::Unsupported))
    ));
    assert!(matches!(
        MockLlmMgmt::fixture("local").signin_cancel("flow").await,
        Err(MgmtError::Upstream(UpstreamError::Unsupported))
    ));
    assert_eq!(mgmt::SIGNIN_CANCEL, "/signin/{flow}/cancel");
}
