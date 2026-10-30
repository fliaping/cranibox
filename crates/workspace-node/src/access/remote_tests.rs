use super::super::tests::{NODE, fixture};
use super::*;
use agent_workspace_runtime::peers::roster::OwnerKey;
use cranibox_contracts::roster::{OwnerCert, Role, RosterMember};
use ed25519_dalek::{Signer, SigningKey};

const APPROVER: &str = "node-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const STRANGER: &str = "node-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn member(node: &str, n: u8) -> RosterMember {
    RosterMember {
        node_id: node.into(),
        signing_key: hex(SigningKey::from_bytes(&[n; 32]).verifying_key().as_bytes()),
        x25519_pub: hex(&[n + 100; 32]),
        role: Role::Node,
        priority: n,
        is_claude_node: false,
        name: String::new(),
        added_at: 0,
        identity: None,
    }
}
/// This device, the approving device and the requester share one identity whose
/// recovery key comes from a generated recovery code.
fn owner_fixture() -> (tempfile::TempDir, Store, String) {
    let (root, store) = fixture();
    let mut seed = [0u8; 32];
    rand::rng().fill_bytes(&mut seed);
    let code = crate::models::recovery_code(&seed);
    let recovery = crate::models::recovery_signing_key(&code, None).unwrap();
    let models = root.path().join("models");
    std::fs::create_dir_all(&models).unwrap();
    let path = models.join("roster.json");
    let owner = OwnerKey::load_or_create(&models.join("owner.key")).unwrap();
    RosterStore::pin(&path, &owner.public()).unwrap();
    let roster = RosterStore::load(path).unwrap();
    let mut cert = OwnerCert {
        recovery_pub: hex(recovery.verifying_key().as_bytes()),
        owner_pub: owner.public(),
        serial: 1,
        recovery_salt: None,
        previous_pub: None,
        previous_sig: None,
        sig: String::new(),
    };
    cert.sig = hex(&recovery.sign(&cert.signing_message()).to_bytes());
    let own = Config::read(&store.peers).unwrap().node_id;
    roster
        .issue_with(
            &owner,
            vec![member(&own, 1), member(APPROVER, 2), member(NODE, 3)],
            Some(cert),
            Default::default(),
        )
        .unwrap();
    store
        .change(
            NODE,
            [
                (Capability::Terminals, Mode::Ask),
                (Capability::Exec, Mode::Ask),
                (Capability::Update, Mode::Ask),
            ]
            .into(),
        )
        .unwrap();
    (root, store, code)
}
fn waiting(store: &Store, capability: Capability, at: u64) -> RemotePending {
    waiting_from(store, NODE, capability, at)
}
/// A request from `from`; each device has at most one waiting request.
fn waiting_from(store: &Store, from: &str, capability: Capability, at: u64) -> RemotePending {
    let result = store.authorize_at(from, capability, &format!("key-{capability}"), at);
    assert!(
        matches!(result, Err(Denied::ApprovalRequired { .. })),
        "{result:?}"
    );
    let list = store.remote_pending_at(APPROVER, at).unwrap().requests;
    list.into_iter()
        .find(|p| p.request.capability == capability && p.request.from == from)
        .expect("a waiting request")
}
fn signed(
    store: &Store,
    key: &SigningKey,
    pending: &RemotePending,
    scope: DurationScope,
    at: u64,
) -> RemoteApproval {
    let target = Config::read(&store.peers).unwrap().node_id;
    sign(
        key,
        APPROVER,
        &target,
        pending,
        ApprovalDecision::Approve,
        Some(scope),
        "recovery_code",
        at,
    )
    .unwrap()
}
fn key(code: &str) -> SigningKey {
    crate::models::recovery_signing_key(code, None).unwrap()
}

#[test]
fn the_recovery_code_approves_once_and_both_sides_audit_it() {
    let (_root, store, code) = owner_fixture();
    let at = now();
    let pending = waiting(&store, Capability::Terminals, at);
    assert!(!pending.once_only);
    assert_eq!(pending.op_hash, op_hash(&pending.request));
    let assertion = signed(&store, &key(&code), &pending, DurationScope::Hour1, at);
    let decided = store.decide_remote_at(APPROVER, &assertion, at).unwrap();
    assert_eq!(decided.state, RequestState::Approved);
    assert!(
        store
            .authorize_at(NODE, Capability::Terminals, "other-key", at + 1)
            .is_ok()
    );
    // The same assertion again: refused, whatever else changed.
    let replay = store.decide_remote_at(APPROVER, &assertion, at + 2);
    assert_eq!(replay.unwrap_err().code(), "replayed");
    let log = store.audit(50, None, None, None).unwrap();
    let event = log
        .entries
        .iter()
        .find(|e| e.event == EventKind::Approved)
        .expect("approval logged");
    let remote = event.remote.as_ref().expect("remote record");
    assert_eq!(remote.device, APPROVER);
    assert_eq!(remote.auth, "recovery_code");
    assert_eq!(remote.assertion, assertion.id);
    assert!(event.result.starts_with("remote"));
    assert!(
        log.entries
            .iter()
            .any(|e| e.event == EventKind::Refused && e.result == "remote:replayed")
    );
    // The approving device keeps its own record.
    store
        .record_remote(
            "node-target",
            Some(&decided),
            "approved:1h",
            "recovery_code",
            Some(&assertion.id),
        )
        .unwrap();
    let mine = store.audit(1, None, None, None).unwrap().entries.remove(0);
    assert_eq!(mine.event, EventKind::RemoteDecision);
    assert_eq!(mine.remote.unwrap().assertion, assertion.id);
    // Nothing in the log or the policy holds the code.
    let disk = std::fs::read_to_string(store.peers.with_file_name("access-audit.jsonl")).unwrap()
        + &std::fs::read_to_string(store.path()).unwrap();
    assert!(!disk.contains(&code));
}

#[test]
fn wrong_codes_lock_remote_approval_on_the_target() {
    let (_root, store, code) = owner_fixture();
    let at = now();
    let pending = waiting(&store, Capability::Terminals, at);
    let wrong = SigningKey::from_bytes(&[9; 32]);
    for left in (1..ATTEMPTS).rev() {
        let pending = store
            .remote_pending_at(APPROVER, at)
            .unwrap()
            .requests
            .remove(0);
        let refusal = store
            .decide_remote_at(
                APPROVER,
                &signed(&store, &wrong, &pending, DurationScope::Session, at),
                at,
            )
            .unwrap_err();
        assert!(
            matches!(refusal, Refusal::WrongCode { left: l } if l == left),
            "{refusal:?}"
        );
    }
    let refusal = store
        .decide_remote_at(
            APPROVER,
            &signed(&store, &wrong, &pending, DurationScope::Session, at),
            at,
        )
        .unwrap_err();
    let Refusal::Locked { until } = refusal else {
        panic!("{refusal:?}")
    };
    assert_eq!(until, at + 300);
    // Locked: even the right code is refused, and the request stays waiting.
    let right = signed(
        &store,
        &key(&code),
        &pending,
        DurationScope::Session,
        at + 1,
    );
    assert_eq!(
        store
            .decide_remote_at(APPROVER, &right, at + 1)
            .unwrap_err()
            .code(),
        "locked"
    );
    assert_eq!(store.remote_locked(APPROVER).is_some(), until > now());
    // Every wrong code was logged.
    let log = store.audit(50, None, None, None).unwrap();
    assert_eq!(
        log.entries
            .iter()
            .filter(|e| e.result == "remote:wrong_code")
            .count(),
        ATTEMPTS as usize - 1
    );
    // After the lock, the right code works on a new request and clears the count.
    let later = until + 1;
    let pending = waiting(&store, Capability::Terminals, later);
    let right = signed(&store, &key(&code), &pending, DurationScope::Session, later);
    store.decide_remote_at(APPROVER, &right, later).unwrap();
    let again = waiting(&store, Capability::Exec, later);
    let refusal = store
        .decide_remote_at(
            APPROVER,
            &signed(&store, &wrong, &again, DurationScope::Session, later),
            later,
        )
        .unwrap_err();
    assert!(matches!(refusal, Refusal::WrongCode { left } if left == ATTEMPTS - 1));
}

#[test]
fn a_tampered_substituted_or_expired_assertion_is_refused() {
    let (_root, store, code) = owner_fixture();
    let at = now();
    let recovery = key(&code);
    let first = waiting(&store, Capability::Terminals, at);
    store
        .change(STRANGER, [(Capability::Terminals, Mode::Ask)].into())
        .unwrap();
    let second = waiting_from(&store, STRANGER, Capability::Terminals, at);
    // Changing any signed field breaks the signature.
    let mut tampered = signed(&store, &recovery, &first, DurationScope::Session, at);
    tampered.scope = Some(DurationScope::Days7);
    assert_eq!(
        store
            .decide_remote_at(APPROVER, &tampered, at)
            .unwrap_err()
            .code(),
        "wrong_code"
    );
    // A signed decision for one request does not fit another.
    let mut moved = signed(&store, &recovery, &first, DurationScope::Session, at);
    moved.request = second.request.id.clone();
    moved.challenge = second.challenge.clone();
    assert_eq!(
        store
            .decide_remote_at(APPROVER, &moved, at)
            .unwrap_err()
            .code(),
        "wrong_code"
    );
    // Signed over different operation details than the target holds.
    let mut changed = first.clone();
    changed.request.session_key = "something-else".into();
    let mismatch = signed(&store, &recovery, &changed, DurationScope::Session, at);
    assert_eq!(
        store
            .decide_remote_at(APPROVER, &mismatch, at)
            .unwrap_err()
            .code(),
        "operation_mismatch"
    );
    // Sent by a device other than the one it names.
    let pending = store
        .remote_pending_at(APPROVER, at)
        .unwrap()
        .requests
        .remove(0);
    let other = signed(&store, &recovery, &pending, DurationScope::Session, at);
    assert_eq!(
        store.decide_remote_at(NODE, &other, at).unwrap_err().code(),
        "wrong_approver"
    );
    // Too late.
    let pending = store
        .remote_pending_at(APPROVER, at)
        .unwrap()
        .requests
        .remove(0);
    let late = signed(&store, &recovery, &pending, DurationScope::Session, at);
    assert_eq!(
        store
            .decide_remote_at(APPROVER, &late, at + 100)
            .unwrap_err()
            .code(),
        "expired"
    );
    // For another target.
    let pending = store
        .remote_pending_at(APPROVER, at)
        .unwrap()
        .requests
        .remove(0);
    let mut elsewhere = sign(
        &recovery,
        APPROVER,
        STRANGER,
        &pending,
        ApprovalDecision::Approve,
        Some(DurationScope::Session),
        "recovery_code",
        at,
    )
    .unwrap();
    assert_eq!(
        store
            .decide_remote_at(APPROVER, &elsewhere, at)
            .unwrap_err()
            .code(),
        "wrong_target"
    );
    elsewhere.auth = "pin".into();
    assert_eq!(
        store
            .decide_remote_at(APPROVER, &elsewhere, at)
            .unwrap_err()
            .code(),
        "wrong_target"
    );
    // Both requests still wait.
    assert_eq!(
        store
            .remote_pending_at(APPROVER, at)
            .unwrap()
            .requests
            .len(),
        2
    );
}

#[test]
fn only_owner_devices_see_and_decide_and_risky_ones_only_once() {
    let (_root, store, code) = owner_fixture();
    let at = now();
    assert!(matches!(
        store.remote_pending_at(STRANGER, at),
        Err(Refusal::NotOwner)
    ));
    let own = Config::read(&store.peers).unwrap().node_id;
    assert!(matches!(
        store.remote_pending_at(&own, at),
        Err(Refusal::NotOwner)
    ));
    let pending = waiting(&store, Capability::Update, at);
    assert!(pending.once_only);
    let mut stranger = signed(&store, &key(&code), &pending, DurationScope::Session, at);
    stranger.approver = STRANGER.into();
    assert!(matches!(
        store.decide_remote_at(STRANGER, &stranger, at),
        Err(Refusal::NotOwner)
    ));
    let hour = signed(&store, &key(&code), &pending, DurationScope::Hour1, at);
    assert_eq!(
        store
            .decide_remote_at(APPROVER, &hour, at)
            .unwrap_err()
            .code(),
        "once_only"
    );
    let pending = store
        .remote_pending_at(APPROVER, at)
        .unwrap()
        .requests
        .remove(0);
    let once = signed(&store, &key(&code), &pending, DurationScope::Session, at);
    assert_eq!(
        store.decide_remote_at(APPROVER, &once, at).unwrap().state,
        RequestState::Approved
    );
    // Turned off on the target: nothing is shown and nothing is accepted.
    store.set_remote_enabled(false).unwrap();
    assert!(matches!(
        store.remote_pending_at(APPROVER, at),
        Err(Refusal::Disabled)
    ));
    store.set_remote_enabled(true).unwrap();
    assert!(store.remote_pending_at(APPROVER, at).is_ok());
}

#[test]
fn without_a_recovery_code_nothing_can_be_approved_remotely() {
    let (_root, store) = fixture();
    let at = now();
    assert!(store.recovery_verifier().is_none());
    assert!(matches!(
        store.remote_pending_at(APPROVER, at),
        Err(Refusal::NotOwner)
    ));
}

#[test]
fn a_local_decision_needs_no_code() {
    let (_root, store, _code) = owner_fixture();
    let at = now();
    let pending = waiting(&store, Capability::Update, at);
    store
        .decide_at(
            &pending.request.id,
            DecideRequest {
                decision: ApprovalDecision::Approve,
                scope: Some(DurationScope::Session),
            },
            at,
        )
        .unwrap();
}
