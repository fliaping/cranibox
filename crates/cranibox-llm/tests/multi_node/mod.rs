use cranibox_contracts::{llm::*, mock::MockRoster, roster::*, vault::*};
use cranibox_llm::vault::{
    Vault, VaultKeys,
    lease::{RefreshCoordinator, TTL},
};
use std::{collections::BTreeSet, sync::Arc};
fn keys(n: u8) -> VaultKeys {
    VaultKeys::from_seed(format!("node-{n}"), [n; 32], [n + 10; 32])
}
fn roster() -> Arc<MockRoster> {
    Arc::new(MockRoster::new(Some(Roster {
        version: 1,
        owner_pub: "test-owner".into(),
        epoch: 1,
        owner_cert: None,
        owner_keys: Default::default(),
        sig: "fixture".into(),
        members: (1..=3)
            .map(|n| {
                let k = keys(n);
                RosterMember {
                    node_id: k.node.clone(),
                    signing_key: k.signing_public.clone(),
                    x25519_pub: k.encryption_public(),
                    role: if n == 1 { Role::Hub } else { Role::Node },
                    priority: n,
                    is_claude_node: n == 3,
                    name: format!("Device {n}"),
                    added_at: 0,
                    identity: None,
                }
            })
            .collect(),
    })))
}
fn summary(kind: AccountKind) -> AccountSummary {
    AccountSummary {
        id: "account".into(),
        provider: "test".into(),
        plan: Some("plan".into()),
        label: "Test account".into(),
        kind,
        home_node: "node-1".into(),
        sync: kind != AccountKind::ClaudeOAuth,
        enabled_on: vec!["node-1".into()],
        all_nodes: false,
        state: AccountState::Ready,
    }
}
fn sync(a: &Vault, b: &Vault) {
    b.push(Push {
        records: a.pull(PullReq::default()).unwrap().records,
    })
    .unwrap();
}
#[test]
fn encrypted_sync_requires_local_enablement_and_persists() {
    let root = tempfile::tempdir().unwrap();
    let r = roster();
    let a = Vault::open(&root.path().join("a/vault.db"), keys(1), r.clone()).unwrap();
    let path = root.path().join("b/vault.db");
    let b = Vault::open(&path, keys(2), r.clone()).unwrap();
    a.put(
        summary(AccountKind::ApiKey),
        Some(SecretBox::new(b"fake-secret-never-plaintext".to_vec())),
    )
    .unwrap();
    sync(&a, &b);
    assert_eq!(b.list().len(), 1);
    assert!(matches!(b.get_secret("account"), Err(StoreError::NoSecret)));
    let mut s = b.get("account").unwrap();
    s.enabled_on.push("node-2".into());
    b.put(s, None).unwrap();
    assert_eq!(
        b.get_secret("account").unwrap().expose(),
        b"fake-secret-never-plaintext"
    );
    let dump = serde_json::to_string(&a.pull(PullReq::default()).unwrap()).unwrap();
    assert!(!dump.contains("fake-secret"));
    drop(b);
    let b = Vault::open(&path, keys(2), r).unwrap();
    assert_eq!(
        b.get_secret("account").unwrap().expose(),
        b"fake-secret-never-plaintext"
    );
    for entry in std::fs::read_dir(path.parent().unwrap()).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        assert!(!bytes.windows(11).any(|w| w == b"fake-secret"));
    }
}
#[test]
fn rejects_tampering_guest_signatures_and_never_syncs_claude() {
    let root = tempfile::tempdir().unwrap();
    let r = roster();
    let a = Vault::open(&root.path().join("a/vault.db"), keys(1), r.clone()).unwrap();
    let b = Vault::open(&root.path().join("b/vault.db"), keys(2), r).unwrap();
    a.put(
        summary(AccountKind::ClaudeOAuth),
        Some(SecretBox::new(b"fake-claude".to_vec())),
    )
    .unwrap();
    assert!(a.pull(PullReq::default()).unwrap().records.is_empty());
    let mut s = summary(AccountKind::ApiKey);
    s.id = "api".into();
    a.put(s, Some(SecretBox::new(b"fake-key".to_vec())))
        .unwrap();
    let mut p = a.pull(PullReq::default()).unwrap();
    if let SignedRecord::Account(a) = &mut p.records[0] {
        a.label = "tampered".into();
    }
    assert!(b.push(Push { records: p.records }).is_err());
    assert!(b.list().is_empty());
    let mut p = a.pull(PullReq::default()).unwrap();
    if let SignedRecord::Account(a) = &mut p.records[0] {
        a.writer = "guest".into();
    }
    assert!(b.push(Push { records: p.records }).is_err());
}
#[test]
fn expired_holder_takeover_and_delete_tombstone() {
    let root = tempfile::tempdir().unwrap();
    let r = roster();
    let a = Vault::open(&root.path().join("a/vault.db"), keys(1), r.clone()).unwrap();
    let b = Vault::open(&root.path().join("b/vault.db"), keys(2), r).unwrap();
    let mut s = summary(AccountKind::OAuth);
    s.enabled_on.push("node-2".into());
    a.put(s, Some(SecretBox::new(b"fake-token".to_vec())))
        .unwrap();
    sync(&a, &b);
    let at = cranibox_llm::auth::now();
    let first = a
        .claim_lease("account", at, &BTreeSet::from(["node-2".into()]))
        .unwrap()
        .unwrap();
    sync(&a, &b);
    assert_eq!(
        b.claim_lease("account", at + TTL - 1, &BTreeSet::new())
            .unwrap()
            .unwrap()
            .holder,
        "node-1"
    );
    let takeover = b
        .claim_lease("account", at + TTL + 1, &BTreeSet::new())
        .unwrap()
        .unwrap();
    assert_eq!(takeover.holder, "node-2");
    assert!(takeover.epoch > first.epoch);
    let old = a.pull(PullReq::default()).unwrap();
    a.remove("account").unwrap();
    sync(&a, &b);
    assert!(b.list().is_empty());
    b.push(Push {
        records: old.records,
    })
    .unwrap();
    assert!(b.list().is_empty());
}
#[tokio::test]
async fn concurrent_refresh_runs_vendor_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let root = tempfile::tempdir().unwrap();
    let a = Arc::new(Vault::open(&root.path().join("vault.db"), keys(1), roster()).unwrap());
    a.put(
        summary(AccountKind::OAuth),
        Some(SecretBox::new(b"old-fake-token".to_vec())),
    )
    .unwrap();
    let coordinator = RefreshCoordinator::new(a.clone());
    let calls = AtomicUsize::new(0);
    let run = || {
        coordinator.refresh(
            RefreshRequest {
                account_id: "account".into(),
                generation: 0,
            },
            |_, _| async {
                calls.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
                Ok(SecretBox::new(b"new-fake-token".to_vec()))
            },
        )
    };
    let (x, y) = tokio::join!(run(), run());
    assert!(x.is_ok());
    assert!(y.is_ok());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(a.account("account").unwrap().generation, 1);
}

#[test]
fn handback_and_epoch_survive_disable_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let r = roster();
    let a = Vault::open(&root.path().join("a/vault.db"), keys(1), r.clone()).unwrap();
    let path = root.path().join("b/vault.db");
    let b = Vault::open(&path, keys(2), r.clone()).unwrap();
    let mut summary = summary(AccountKind::OAuth);
    summary.enabled_on.push("node-2".into());
    a.put(summary, Some(SecretBox::new(b"fake-token".to_vec())))
        .unwrap();
    sync(&a, &b);
    let at = cranibox_llm::auth::now();
    let takeover = b
        .claim_lease("account", at, &BTreeSet::new())
        .unwrap()
        .unwrap();
    let yielded = b
        .claim_lease("account", at + 1, &BTreeSet::from(["node-1".into()]))
        .unwrap()
        .unwrap();
    assert_eq!(yielded.expires_at, at + 1);
    assert!(yielded.epoch > takeover.epoch);
    sync(&b, &a);
    let returned = a
        .claim_lease("account", at + 2, &BTreeSet::from(["node-2".into()]))
        .unwrap()
        .unwrap();
    assert_eq!(returned.holder, "node-1");
    assert!(returned.epoch > yielded.epoch);
    // Disable only after the holder yields. The persistent floor prevents epoch reuse.
    a.yield_lease("account", at).unwrap();
    let mut s = a.get("account").unwrap();
    s.enabled_on = vec!["node-2".into()];
    a.put(s, None).unwrap();
    sync(&a, &b);
    assert!(b.lease("account").unwrap().is_none());
    drop(b);
    let b = Vault::open(&path, keys(2), r).unwrap();
    let mut s = b.get("account").unwrap();
    s.enabled_on.push("node-1".into());
    b.put(s, None).unwrap();
    let next = b
        .claim_lease("account", at, &BTreeSet::new())
        .unwrap()
        .unwrap();
    assert!(next.epoch > returned.epoch);
}

#[test]
fn revocation_rotates_wraps_and_removed_home_migrates() {
    use cranibox_llm::vault::crypto;
    let root = tempfile::tempdir().unwrap();
    let r = roster();
    let a = Vault::open(&root.path().join("a/vault.db"), keys(1), r.clone()).unwrap();
    let b = Vault::open(&root.path().join("b/vault.db"), keys(2), r.clone()).unwrap();
    a.put(
        summary(AccountKind::ApiKey),
        Some(SecretBox::new(b"fake-key".to_vec())),
    )
    .unwrap();
    sync(&a, &b);
    let old = b.account("account").unwrap();
    assert!(crypto::open(&old, &keys(1)).is_ok());
    let mut changed = r.current().unwrap();
    changed.epoch += 1;
    changed.members.retain(|m| m.node_id != "node-1");
    r.set(Some(changed));
    b.rotate_for_roster().unwrap();
    let rotated = b.account("account").unwrap();
    assert_eq!(rotated.home_node, "node-2");
    assert!(!rotated.wraps.contains_key("node-1"));
    assert_ne!(rotated.sealed, old.sealed);
    assert!(crypto::open(&rotated, &keys(1)).is_err());
    assert!(a.get_secret("account").is_err());
}
