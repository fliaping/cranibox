//! Contract snapshot test: every fixture must round-trip through its Rust type
//! without losing or adding fields. Changing a contract type without updating
//! its fixture (and the WS7 frontend mock that serves it) fails CI.
use cranibox_contracts::{
    agents::{AgentAdapter, AgentId, Applied, DEFAULT_AGENT, LocalEndpoint},
    app::*,
    config::IrohConfig,
    error::{ErrorCode, UnavailableBody},
    llm::{mgmt::*, *},
    mock::{MemoryAccountStore, MockAppApi, MockLlmMgmt, MockRoster, fixtures},
    net::{self, PairTicket},
    roster::{Roster, RosterView},
    vault::*,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

fn round_trip<T: Serialize + DeserializeOwned>(name: &str, json: &str) {
    let original: Value = serde_json::from_str(json).unwrap_or_else(|e| panic!("{name}: {e}"));
    let typed: T = serde_json::from_value(original.clone())
        .unwrap_or_else(|e| panic!("{name} does not match its contract type: {e}"));
    let again = serde_json::to_value(&typed).unwrap();
    assert_eq!(again, original, "{name} does not round-trip");
}

#[test]
fn every_fixture_round_trips() {
    let mut checked = vec![];
    macro_rules! check {
        ($($ty:ty => $const:ident),* $(,)?) => {$(
            round_trip::<$ty>(stringify!($const), fixtures::$const);
            checked.push(fixtures::$const);
        )*};
    }
    check! {
        cranibox_contracts::access::AccessView => APP_ACCESS,
        cranibox_contracts::access::AuditView => ACCESS_AUDIT,
        Roster => ROSTER,
        PairTicket => PAIR_TICKET,
        IrohConfig => IROH_CONFIG,
        LlmHealth => LLM_HEALTH,
        Status => LLM_STATUS,
        Vec<AccountSummary> => LLM_ACCOUNTS,
        Vec<ModelEntry> => LLM_MODELS,
        Vec<SignInStart> => LLM_SIGNIN_START,
        Vec<NewAccount> => LLM_NEW_ACCOUNT,
        Vec<MgmtEvent> => LLM_EVENTS,
        Applied => AGENTS_APPLIED,
        StatusPill => APP_STATUS,
        Vec<Device> => APP_DEVICES,
        Vec<ApprovalCard> => APP_APPROVALS,
        Vec<AgentInfo> => APP_AGENTS,
        PairInvite => APP_PAIR_INVITE,
        PairAccepted => APP_PAIR_ACCEPTED,
        Vec<AppEvent> => APP_EVENTS,
        Capabilities => APP_CAPABILITIES,
        InstallPlan => APP_INSTALL_PLAN,
        DeviceGrant => APP_DEVICE_GRANT,
        serde_json::Value => APP_NETWORK_PATHS,
        PullReq => VAULT_PULL_REQ,
        PullResp => VAULT_PULL_RESP,
        RefreshRequest => VAULT_REFRESH_REQUEST,
        Vec<UnavailableBody> => ERROR_UNAVAILABLE,
    }
    // Every fixture shipped to the frontend is covered by a type above.
    assert_eq!(checked.len(), fixtures::ALL.len());
    let on_disk = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures"))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "json")
        })
        .count();
    assert_eq!(
        on_disk,
        fixtures::ALL.len(),
        "register new fixtures in mock::fixtures"
    );
}

#[test]
fn presets_toml_schema_parses() {
    let file: PresetFile = toml::from_str(fixtures::PRESETS_TOML).unwrap();
    assert_eq!(file.preset[0].id, "zhipu-coding");
    assert_eq!(file.preset[0].default_wire, Wire::Messages);
    assert!(file.preset[1].responses.is_none() && file.preset[1].models.is_empty());
}

#[test]
fn error_fixture_covers_every_code() {
    let bodies: Vec<UnavailableBody> = serde_json::from_str(fixtures::ERROR_UNAVAILABLE).unwrap();
    let codes: Vec<ErrorCode> = bodies.iter().map(|b| b.error.code).collect();
    assert_eq!(codes, ErrorCode::ALL);
    for code in ErrorCode::ALL {
        assert_eq!(serde_json::to_value(code).unwrap(), code.as_str());
        let body = UnavailableBody::new(code, "m", None);
        assert_eq!(
            serde_json::to_value(&body).unwrap()["error"]["type"],
            "cranibox_unavailable"
        );
    }
}

#[test]
fn constants_are_locked() {
    assert_eq!(net::ALPN_PEER_HTTP, b"cranibox/peer-http/1");
    assert_eq!(LLM_ADDR, "127.0.0.1:3424");
    assert_eq!(LLM_MGMT_SOCKET, "cranibox-llm.sock");
    assert_eq!(
        cranibox_contracts::roster::ROSTER_SIG_DOMAIN,
        b"cranibox-roster-v1\0"
    );
    assert_eq!(VAULT_SIG_DOMAIN, b"cranibox-vault-v1\0");
    assert_eq!(
        net::routes::llm("node-x", net::routes::LLM_CHAT),
        "/api/v1/mesh/nodes/node-x/llm/v1/chat/completions"
    );
    assert_eq!(
        net::routes::vault("node-x", net::routes::VAULT_REFRESH_REQUEST),
        "/api/v1/mesh/nodes/node-x/vault/refresh-request"
    );
    assert_eq!(DEFAULT_AGENT, AgentId::DeepseekHarness);
    let order: Vec<_> = AgentId::ALL.iter().map(|a| a.as_str()).collect();
    assert_eq!(
        order,
        [
            "deepseek-harness",
            "codex",
            "claude-code",
            "pi",
            "hermes",
            "openclaw"
        ]
    );
    for id in AgentId::ALL {
        assert_eq!(serde_json::to_value(id).unwrap(), id.as_str());
        assert_eq!(AgentId::parse(id.as_str()), Some(id));
    }
    let ep = LocalEndpoint::for_agent(AgentId::Codex);
    assert_eq!(
        (ep.base.as_str(), ep.key.as_str()),
        ("http://127.0.0.1:3424", "cbx-codex")
    );
    // Object safety of the shared traits.
    type Objects<'a> = (
        &'a dyn AgentAdapter,
        &'a dyn Upstream,
        &'a dyn SignIn,
        &'a dyn LlmMgmt,
        &'a dyn AppApi,
        &'a dyn AccountStore,
        &'a dyn RosterView,
        &'a dyn net::PeerTransport,
    );
    let _: Option<Objects<'_>> = None;
}

#[test]
fn roster_signing_message_excludes_signature() {
    let mut roster: Roster = serde_json::from_str(fixtures::ROSTER).unwrap();
    let signed = roster.signing_message();
    assert!(signed.starts_with(b"cranibox-roster-v1\0"));
    roster.sig = "00".into();
    assert_eq!(roster.signing_message(), signed);
    roster.epoch += 1;
    assert_ne!(roster.signing_message(), signed);
}

#[test]
fn iroh_config_defaults() {
    let config: IrohConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(config, IrohConfig::default());
    assert!(config.enabled && config.n0_fallback && config.relays.is_empty());
    assert!(serde_json::from_str::<IrohConfig>(r#"{"unknown":1}"#).is_err());
}

#[test]
fn secret_box_is_redacted() {
    let secret = SecretBox::new(b"sk-test".to_vec());
    assert_eq!(format!("{secret:?}"), "SecretBox(<7 bytes>)");
}

#[tokio::test]
async fn mocks_serve_fixtures() {
    let roster = MockRoster::fixture();
    let hub = roster.hub().unwrap();
    assert!(roster.is_owner_node(&hub.node_id));
    assert!(!roster.is_owner_node("node-3d4e5f60718293a4b5c6d7e8f90a1b2c"));
    assert!(roster.claude_node().unwrap().is_claude_node);
    let epochs = roster.subscribe();
    roster.set(None);
    assert!(epochs.has_changed().unwrap());

    let store = MemoryAccountStore::new();
    let accounts: Vec<AccountSummary> = serde_json::from_str(fixtures::LLM_ACCOUNTS).unwrap();
    store
        .put(accounts[0].clone(), Some(SecretBox::new("fake")))
        .unwrap();
    store.put(accounts[0].clone(), None).unwrap();
    assert_eq!(store.get_secret(&accounts[0].id).unwrap().expose(), b"fake");
    assert_eq!(store.list().len(), 1);
    store.remove(&accounts[0].id).unwrap();
    assert_eq!(store.get_secret(&accounts[0].id), Err(StoreError::NotFound));

    let mgmt = MockLlmMgmt::fixture("node-1f2e3d4c5b6a79880716253443526170");
    let added = mgmt
        .add_account(NewAccount::ApiKey {
            preset: "deepseek".into(),
            key: "sk-fake".into(),
            label: None,
            region: None,
        })
        .await
        .unwrap();
    let enabled = mgmt
        .enable(
            &added.id,
            Enable {
                node: None,
                all_nodes: Some(true),
                on: true,
            },
        )
        .await
        .unwrap();
    assert!(enabled.all_nodes);
    assert!(!mgmt.models().await.unwrap().is_empty());

    let app = MockAppApi::fixture();
    let pending = app.status().await.unwrap().approvals_pending;
    app.decide(
        "apr-1",
        ApprovalDecision {
            decision: ApprovalAction::Deny,
        },
    )
    .await
    .unwrap();
    assert_eq!(app.status().await.unwrap().approvals_pending, pending - 1);
    assert!(
        app.decide(
            "apr-2",
            ApprovalDecision {
                decision: ApprovalAction::AllowAlways
            }
        )
        .await
        .is_err()
    );
    let agent = app
        .update_agent(
            AgentId::Pi,
            AgentUpdate {
                managed: true,
                model: Some("deepseek/deepseek-chat".into()),
            },
        )
        .await
        .unwrap();
    assert!(agent.managed);
}

#[test]
fn mock_capabilities_and_grants_follow_the_contract() {
    let app = MockAppApi::fixture();
    let ids: Vec<_> = app.capabilities().items.iter().map(|i| i.id).collect();
    assert_eq!(
        ids,
        [
            CapabilityId::Terminal,
            CapabilityId::Desktop,
            CapabilityId::Computer,
            CapabilityId::Browser
        ]
    );
    let plan: InstallPlan = serde_json::from_value(
        app.capability_action(
            CapabilityId::Computer,
            CapabilityAction::InstallPlan,
            Value::Null,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        app.capability_action(
            CapabilityId::Computer,
            CapabilityAction::Install,
            serde_json::json!({"plan_id":"other"})
        )
        .is_err()
    );
    let item: CapabilityItem = serde_json::from_value(
        app.capability_action(
            CapabilityId::Computer,
            CapabilityAction::Install,
            serde_json::json!({"plan_id":plan.plan_id}),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(item.provider.installed && item.actions.contains(&CapabilityAction::Start));
    let browser: CapabilityItem = serde_json::from_value(
        app.capability_action(CapabilityId::Browser, CapabilityAction::Start, Value::Null)
            .unwrap(),
    )
    .unwrap();
    assert!(browser.sharing.enabled && browser.sharing.address.is_some());
    // An action that does not apply is refused.
    assert!(
        app.capability_action(CapabilityId::Terminal, CapabilityAction::Start, Value::Null)
            .is_err()
    );

    let node = "node-0a1b2c3d4e5f60718293a4b5c6d7e8f9";
    let mut grant = app.device_grant(node).unwrap();
    grant.permissions.computer = !grant.permissions.computer;
    assert_eq!(app.set_device_grant(node, grant).unwrap(), grant);
    assert_eq!(app.device_grant(node).unwrap(), grant);
    assert!(app.set_device_grant(node, DeviceGrant::default()).is_err());
    assert!(app.device_grant("node-unknown").is_err());
}
