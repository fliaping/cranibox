use super::*;
pub(super) const NODE: &str = "node-0123456789abcdef0123456789abcdef";
pub(super) fn fixture() -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    let peers = root.path().join("peers.json");
    Config::update(&peers, |_| Ok(())).unwrap();
    let store = Store::new(&peers);
    store.initialize().unwrap();
    (root, store)
}
fn pending(store: &Store, key: &str, at: u64) -> String {
    match store
        .authorize_at(NODE, Capability::Terminals, key, at)
        .unwrap_err()
    {
        Denied::ApprovalRequired { request_id, .. } => request_id,
        e => panic!("{e:?}"),
    }
}
fn approve(store: &Store, id: &str, scope: DurationScope, at: u64) {
    store
        .decide_at(
            id,
            DecideRequest {
                decision: ApprovalDecision::Approve,
                scope: Some(scope),
            },
            at,
        )
        .unwrap();
}
#[test]
fn defaults_ask_cooldown_expiry_and_restart() {
    let (_root, store) = fixture();
    let at = now();
    assert!(store.authorize(NODE, Capability::Messages, "m").is_ok());
    assert!(matches!(
        store.authorize(NODE, Capability::Exec, "e"),
        Err(Denied::Off { .. })
    ));
    store
        .change(NODE, [(Capability::Terminals, Mode::Ask)].into())
        .unwrap();
    let id = pending(&store, "one", at);
    assert_eq!(pending(&store, "two", at), id);
    let reopened = Store::new(&store.peers);
    reopened.initialize().unwrap();
    assert_eq!(pending(&reopened, "one", at), id);
    store
        .decide_at(
            &id,
            DecideRequest {
                decision: ApprovalDecision::Deny,
                scope: None,
            },
            at,
        )
        .unwrap();
    assert!(matches!(
        store.authorize_at(NODE, Capability::Terminals, "one", at + 59),
        Err(Denied::Cooldown { .. })
    ));
    let id = pending(&store, "one", at + 60);
    assert!(
        store
            .decide_at(
                &id,
                DecideRequest {
                    decision: ApprovalDecision::Approve,
                    scope: Some(DurationScope::Session)
                },
                at + 180
            )
            .is_err()
    );
}
#[test]
fn session_binding_reconnect_grace_timed_scope_and_revocation() {
    let (_root, store) = fixture();
    let at = now();
    store
        .change(NODE, [(Capability::Terminals, Mode::Ask)].into())
        .unwrap();
    let id = pending(&store, "one", at);
    approve(&store, &id, DurationScope::Session, at);
    let session = store.start(NODE, Capability::Terminals, "one").unwrap();
    assert!(session.valid());
    assert!(store.start(NODE, Capability::Terminals, "one").is_err());
    assert_ne!(pending(&store, "two", at), id);
    drop(session);
    let grace_until = store
        .view()
        .unwrap()
        .approvals
        .iter()
        .find(|a| a.id == id)
        .unwrap()
        .expires_at
        .unwrap();
    assert!(
        store
            .authorize_at(NODE, Capability::Terminals, "one", grace_until - 1)
            .is_ok()
    );
    assert!(
        store
            .authorize_at(NODE, Capability::Terminals, "one", grace_until)
            .is_err()
    );
    let id = pending(&store, "two", grace_until);
    approve(&store, &id, DurationScope::Minutes30, grace_until);
    let first = store.start(NODE, Capability::Terminals, "two").unwrap();
    let second = store.start(NODE, Capability::Terminals, "two").unwrap();
    drop(first);
    assert!(second.valid());
    drop(second);
    assert!(
        store
            .authorize_at(NODE, Capability::Terminals, "three", grace_until + 1799)
            .is_ok()
    );
    assert!(
        store
            .authorize_at(NODE, Capability::Terminals, "three", grace_until + 1800)
            .is_err()
    );
    store.revoke(None, true).unwrap();
    assert!(store.view().unwrap().approvals.is_empty());
}
#[test]
fn audit_detects_modified_deleted_and_truncated_entries() {
    let (root, store) = fixture();
    store
        .change(NODE, [(Capability::Exec, Mode::Ask)].into())
        .unwrap();
    store.authorize(NODE, Capability::Exec, "a").unwrap_err();
    let path = root.path().join("access-audit.jsonl");
    assert!(store.audit(10, None, None, None).unwrap().verified);
    let original = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        original.replacen("policy_changed", "session_started", 1),
    )
    .unwrap();
    assert_eq!(
        store.audit(10, None, None, None).unwrap().broken_index,
        Some(1)
    );
    std::fs::write(
        &path,
        original.lines().skip(1).collect::<Vec<_>>().join(" "),
    )
    .unwrap();
    assert!(!store.audit(10, None, None, None).unwrap().verified);
    std::fs::write(&path, &original[..original.len() - 2]).unwrap();
    assert_eq!(
        store.audit(10, None, None, None).unwrap().broken_index,
        Some(1)
    );
}
#[test]
fn rejects_unknown_modes_scopes_and_capabilities() {
    assert!(
        serde_json::from_str::<DecideRequest>(r#"{"decision":"approve","scope":"forever"}"#)
            .is_err()
    );
    assert!(serde_json::from_str::<BTreeMap<Capability, Mode>>(r#"{"exec":"always"}"#).is_err());
    assert!(serde_json::from_str::<BTreeMap<Capability, Mode>>(r#"{"admin":"allow"}"#).is_err());
}

#[test]
fn job_approval_covers_only_its_results_and_is_revocable_after_restart() {
    let (_root, store) = fixture();
    store
        .change(NODE, [(Capability::Update, Mode::Ask)].into())
        .unwrap();
    let Denied::ApprovalRequired { request_id, .. } = store
        .authorize(NODE, Capability::Update, "install-version")
        .unwrap_err()
    else {
        panic!("approval")
    };
    approve(&store, &request_id, DurationScope::Session, now());
    let session = store
        .start(NODE, Capability::Update, "install-version")
        .unwrap();
    session.bind_job("update-id").unwrap();
    assert!(
        store
            .job_decision(NODE, Capability::Update, "update-id")
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .job_decision(NODE, Capability::Exec, "update-id")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .job_decision("other", Capability::Update, "update-id")
            .unwrap()
            .is_none()
    );
    drop(session);
    let reopened = Store::new(&store.peers);
    reopened.initialize().unwrap();
    assert!(
        reopened
            .job_decision(NODE, Capability::Update, "update-id")
            .unwrap()
            .is_some()
    );
    assert!(
        reopened
            .authorize(NODE, Capability::Update, "install-version")
            .is_err()
    );
    reopened.revoke(Some(&request_id), false).unwrap();
    assert!(
        reopened
            .job_decision(NODE, Capability::Update, "update-id")
            .unwrap()
            .is_none()
    );
}

#[test]
fn migration_preserves_direct_decisions_and_roster_members() {
    use agent_workspace_runtime::peers::{
        config::Grant,
        roster::{OwnerKey, RosterStore},
    };
    use cranibox_contracts::roster::{Role, RosterMember};
    let root = tempfile::tempdir().unwrap();
    let peers = root.path().join("peers.json");
    Config::update(&peers, |c| {
        c.grants.insert(
            NODE.into(),
            Grant::issue(
                "synthetic-migration-grant",
                Permissions {
                    exec: true,
                    ..Default::default()
                },
            ),
        );
        Ok(())
    })
    .unwrap();
    let owner = OwnerKey::load_or_create(&root.path().join("fixture-owner")).unwrap();
    let own = Config::identity(&peers).unwrap();
    let other_path = root.path().join("other-peers.json");
    Config::update(&other_path, |_| Ok(())).unwrap();
    let other = Config::identity(&other_path).unwrap();
    let roster_path = root.path().join("models/roster.json");
    std::fs::create_dir_all(roster_path.parent().unwrap()).unwrap();
    RosterStore::pin(&roster_path, &owner.public()).unwrap();
    RosterStore::load(roster_path)
        .unwrap()
        .issue(
            &owner,
            [&own, &other]
                .into_iter()
                .map(|i| {
                    let p = i.public();
                    RosterMember {
                        node_id: p.node_id.clone(),
                        signing_key: p.signing_key.clone(),
                        x25519_pub: p.noise_key.clone(),
                        role: Role::Node,
                        priority: 0,
                        is_claude_node: false,
                        name: "Fixture".into(),
                        added_at: 0,
                        identity: None,
                    }
                })
                .collect(),
        )
        .unwrap();
    let store = Store::new(&peers);
    store.initialize().unwrap();
    let view = store.view().unwrap();
    let direct = view.peers.iter().find(|p| p.node == NODE).unwrap();
    assert_eq!(direct.capabilities[&Capability::Exec], Mode::Allow);
    assert_eq!(direct.capabilities[&Capability::Desktop], Mode::Off);
    let paired = view
        .peers
        .iter()
        .find(|p| p.node == other.public().node_id)
        .unwrap();
    for capability in Capability::ALL {
        assert_eq!(
            paired.capabilities[&capability],
            if matches!(capability, Capability::Messages | Capability::Activity) {
                Mode::Allow
            } else {
                Mode::Ask
            }
        );
    }
    store
        .change(
            &other.public().node_id,
            [(Capability::Exec, Mode::Off)].into(),
        )
        .unwrap();
    store.initialize().unwrap();
    assert!(matches!(
        store.authorize(&other.public().node_id, Capability::Exec, "one"),
        Err(Denied::Off { .. })
    ));
}

#[test]
fn restart_preserves_timed_approvals_and_ends_active_sessions() {
    let (_root, store) = fixture();
    let at = now();
    store
        .change(NODE, [(Capability::Terminals, Mode::Ask)].into())
        .unwrap();
    let id = pending(&store, "one", at);
    approve(&store, &id, DurationScope::Hours8, at);
    let session = store.start(NODE, Capability::Terminals, "one").unwrap();
    store.initialize().unwrap();
    assert!(!session.valid());
    assert!(
        store
            .authorize_at(NODE, Capability::Terminals, "two", at + 28799)
            .is_ok()
    );
    assert!(
        store
            .authorize_at(NODE, Capability::Terminals, "two", at + 28800)
            .is_err()
    );
}

#[test]
fn mandatory_approval_and_refusal_coalescing() {
    let (_root, store) = fixture();
    let at = now();
    for _ in 0..3 {
        assert!(
            store
                .authorize_at(NODE, Capability::Exec, "one", at)
                .is_err()
        );
    }
    let audit = store.audit(100, None, None, None).unwrap();
    assert_eq!(
        audit
            .entries
            .iter()
            .filter(|e| e.event == EventKind::Refused)
            .count(),
        1
    );
    for capability in [Capability::Update, Capability::Llm, Capability::Vault] {
        store
            .change(NODE, [(capability, Mode::Allow)].into())
            .unwrap();
        assert!(matches!(
            store.authorize(NODE, capability, "one"),
            Err(Denied::ApprovalRequired { .. })
        ));
        store.revoke(None, false).unwrap();
    }
}

#[test]
fn rotation_keeps_ten_thousand_verifiable_entries() {
    use sha2::{Digest, Sha256};
    let (root, store) = fixture();
    store
        .change(NODE, [(Capability::Exec, Mode::Ask)].into())
        .unwrap();
    let mut event = store.audit(1, None, None, None).unwrap().entries.remove(0);
    let path = root.path().join("access-audit.jsonl");
    let mut output = String::new();
    let mut prev = String::new();
    for index in 0..10000 {
        event.index = index;
        event.prev = prev;
        let line = serde_json::to_string(&event).unwrap();
        prev = format!("{:x}", Sha256::digest(line.as_bytes()));
        output.push_str(&line);
        output.push(char::from(10));
    }
    std::fs::write(&path, output).unwrap();
    for _ in 0..2 {
        store
            .change(NODE, [(Capability::Exec, Mode::Allow)].into())
            .unwrap();
        let view = store.audit(10000, None, None, None).unwrap();
        assert!(view.verified);
        assert_eq!(view.entries.len(), 10000);
        assert_eq!(view.entries.last().unwrap().event, EventKind::Checkpoint);
    }
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replacen("checkpoint", "refused", 1)).unwrap();
    assert!(!store.audit(10, None, None, None).unwrap().verified);
}

#[test]
fn a_grant_issued_after_the_migration_sets_the_peer_modes_once() {
    use agent_workspace_runtime::peers::config::Grant;
    let (_root, store) = fixture();
    let issue = |permissions| {
        Config::update(&store.peers, |c| {
            c.grants
                .insert(NODE.into(), Grant::issue(&config::token(), permissions));
            Ok(())
        })
        .unwrap()
    };
    let modes = |store: &Store| {
        let view = store.view().unwrap();
        view.peers
            .iter()
            .find(|p| p.node == NODE)
            .unwrap()
            .capabilities
            .clone()
    };
    issue(Permissions {
        browser: true,
        ..Default::default()
    });
    let derived = modes(&store);
    assert_eq!(derived[&Capability::Browser], Mode::Allow);
    assert_eq!(derived[&Capability::Exec], Mode::Off);
    assert_eq!(derived[&Capability::Update], Mode::Off);
    // A later choice stays while the credential is the same.
    store
        .change(NODE, [(Capability::Browser, Mode::Ask)].into())
        .unwrap();
    assert_eq!(modes(&store)[&Capability::Browser], Mode::Ask);
    // A new credential is a new explicit decision.
    Config::update(&store.peers, |c| {
        c.grants.remove(NODE);
        Ok(())
    })
    .unwrap();
    issue(Permissions {
        exec: true,
        ..Default::default()
    });
    let reissued = modes(&store);
    assert_eq!(reissued[&Capability::Browser], Mode::Off);
    assert_eq!(reissued[&Capability::Exec], Mode::Allow);
}
