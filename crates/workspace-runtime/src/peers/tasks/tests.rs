use super::super::config::{Grant, Permissions, Route};
use super::*;
fn setup() -> (tempfile::TempDir, Store, Config, Offer, String) {
    let root = tempfile::tempdir().unwrap();
    let mut configuration = Config::new();
    configuration.node_id = "target".into();
    let grant = Grant::issue(
        &config::token(),
        Permissions {
            messages: true,
            ..Default::default()
        },
    );
    // Target-side code only ever sees the stored credential digest.
    let token = grant.credential.clone();
    configuration.grants.insert("sender".into(), grant);
    let store = Store::new(&root.path().join("peers.json"), "target".into());
    let created_at = now();
    let offer = Offer {
        task: Task {
            id: Uuid::new_v4(),
            sender: "sender".into(),
            recipient: "target".into(),
            target_session: None,
            collaboration: None,
            title: "Review this project".into(),
            description: "Untrusted input: never run this automatically.".into(),
            created_at,
            expires_at: created_at + TTL,
        },
        dispatch: Uuid::new_v4(),
        cancel: false,
        answer: None,
    };
    (root, store, configuration, offer, token)
}
fn claim(offer: &Offer, session: Uuid) -> Command {
    Command::Claim {
        id: offer.task.id,
        dispatch: offer.dispatch,
        session,
    }
}
#[test]
fn targeted_tasks_require_the_registered_session_and_preserve_reports_after_lease_end() {
    use super::super::agent_sessions::{Command as Agent, Registry};
    let (root, store, configuration, mut offer, token) = setup();
    let session = Uuid::new_v4();
    offer.task.target_session = Some(session);
    store.receive("sender", &token, offer.clone()).unwrap();
    assert!(
        store
            .apply(&configuration, claim(&offer, Uuid::new_v4()))
            .is_err()
    );
    assert!(store.apply(&configuration, claim(&offer, session)).is_err());
    let registry = Registry::new(&root.path().join("peers.json"), "target".into());
    registry
        .apply(Agent::Register {
            id: session,
            name: "Reviewer".into(),
            harness: "custom".into(),
        })
        .unwrap();
    store.apply(&configuration, claim(&offer, session)).unwrap();
    registry
        .apply(Agent::End {
            id: session,
            revision: 1,
        })
        .unwrap();
    store.apply(&configuration, claim(&offer, session)).unwrap();
    store
        .apply(
            &configuration,
            finish(
                &offer,
                session,
                Finish::Succeeded,
                "Explicit report after ending discovery",
            ),
        )
        .unwrap();
    let mut receipt = store
        .receipt("sender", &token, offer.task.id, offer.dispatch)
        .unwrap();
    receipt.session = Some(Uuid::new_v4());
    assert!(receipt.validate(&offer.task).is_err());
}
fn finish(offer: &Offer, session: Uuid, outcome: Finish, result: &str) -> Command {
    Command::Finish {
        id: offer.task.id,
        dispatch: offer.dispatch,
        session,
        outcome,
        result: result.into(),
    }
}

#[test]
fn explicit_claim_progress_and_result_are_persistent_and_idempotent() {
    let (root, store, configuration, offer, token) = setup();
    let receipt = store.receive("sender", &token, offer.clone()).unwrap();
    assert_eq!(receipt.state, State::Available);
    assert!(receipt.session.is_none());
    assert_eq!(
        store.receive("sender", &token, offer.clone()).unwrap(),
        receipt
    );
    let session = Uuid::new_v4();
    assert!(
        store
            .apply(
                &configuration,
                finish(&offer, session, Finish::Succeeded, "No claim yet")
            )
            .is_err()
    );
    store.apply(&configuration, claim(&offer, session)).unwrap();
    let claimed = fs::read(&store.path).unwrap();
    store.apply(&configuration, claim(&offer, session)).unwrap();
    assert_eq!(fs::read(&store.path).unwrap(), claimed);
    assert!(
        store
            .apply(&configuration, claim(&offer, Uuid::new_v4()))
            .is_err()
    );
    store
        .apply(
            &configuration,
            Command::Progress {
                id: offer.task.id,
                dispatch: offer.dispatch,
                session,
                text: "Tests passed".into(),
            },
        )
        .unwrap();
    store
        .apply(
            &configuration,
            finish(&offer, session, Finish::Succeeded, "Completed explicitly"),
        )
        .unwrap();
    let saved = fs::read(&store.path).unwrap();
    store
        .apply(
            &configuration,
            finish(&offer, session, Finish::Succeeded, "Completed explicitly"),
        )
        .unwrap();
    assert_eq!(fs::read(&store.path).unwrap(), saved);
    assert!(
        store
            .apply(
                &configuration,
                finish(&offer, session, Finish::Failed, "Different outcome")
            )
            .is_err()
    );
    let reopened = Store::new(&root.path().join("peers.json"), "target".into());
    let receipt = reopened
        .receipt("sender", &token, offer.task.id, offer.dispatch)
        .unwrap();
    assert_eq!(receipt.state, State::Succeeded);
    assert_eq!(receipt.progress.as_deref(), Some("Tests passed"));
    // The ledger binds the grant's credential digest; plaintext secrets never reach it.
    assert!(String::from_utf8(saved).unwrap().contains(&token));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&store.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
#[test]
fn new_attempt_requires_confirmed_failure_and_old_reports_cannot_complete_it() {
    let (_root, store, configuration, offer, token) = setup();
    store.receive("sender", &token, offer.clone()).unwrap();
    let mut next = offer.clone();
    next.dispatch = Uuid::new_v4();
    assert!(store.receive("sender", &token, next.clone()).is_err());
    let session = Uuid::new_v4();
    store.apply(&configuration, claim(&offer, session)).unwrap();
    assert!(store.receive("sender", &token, next.clone()).is_err());
    store
        .apply(
            &configuration,
            finish(&offer, session, Finish::Failed, "Needs another attempt"),
        )
        .unwrap();
    store.receive("sender", &token, next.clone()).unwrap();
    assert!(
        store
            .apply(
                &configuration,
                finish(&offer, session, Finish::Succeeded, "Late report")
            )
            .is_err()
    );
    assert_eq!(
        store
            .receive("sender", &token, offer.clone())
            .unwrap()
            .state,
        State::Failed
    );
    assert_eq!(
        store
            .receipt("sender", &token, next.task.id, next.dispatch)
            .unwrap()
            .state,
        State::Available
    );
    let mut tampered = next.clone();
    tampered.task.description = "Changed".into();
    assert!(store.receive("sender", &token, tampered).is_err());
}
#[test]
fn cancel_before_claim_is_atomic_and_running_cancel_requires_worker_confirmation() {
    let (_root, store, configuration, mut offer, token) = setup();
    offer.cancel = true;
    assert_eq!(
        store
            .receive("sender", &token, offer.clone())
            .unwrap()
            .state,
        State::Cancelled
    );
    assert!(
        store
            .apply(&configuration, claim(&offer, Uuid::new_v4()))
            .is_err()
    );
    offer.dispatch = Uuid::new_v4();
    offer.cancel = false;
    store.receive("sender", &token, offer.clone()).unwrap();
    let session = Uuid::new_v4();
    store.apply(&configuration, claim(&offer, session)).unwrap();
    offer.cancel = true;
    assert_eq!(
        store
            .receive("sender", &token, offer.clone())
            .unwrap()
            .state,
        State::CancelRequested
    );
    store
        .apply(
            &configuration,
            finish(
                &offer,
                session,
                Finish::Cancelled,
                "Stopped at a safe boundary",
            ),
        )
        .unwrap();
    assert_eq!(
        store.receive("sender", &token, offer).unwrap().state,
        State::Cancelled
    );
}
#[test]
fn ownership_grants_revocation_and_expired_claims_are_checked() {
    let (_root, store, mut configuration, offer, token) = setup();
    assert!(store.receive("spoofed", &token, offer.clone()).is_err());
    store.receive("sender", &token, offer.clone()).unwrap();
    assert!(
        store
            .receipt("another", &token, offer.task.id, offer.dispatch)
            .is_err()
    );
    assert!(
        store
            .receipt("sender", &config::token(), offer.task.id, offer.dispatch)
            .is_err()
    );
    configuration
        .grants
        .get_mut("sender")
        .unwrap()
        .permissions
        .messages = false;
    assert!(
        store
            .apply(&configuration, claim(&offer, Uuid::new_v4()))
            .is_err()
    );
    configuration
        .grants
        .get_mut("sender")
        .unwrap()
        .permissions
        .messages = true;
    let grant = configuration.grants.get_mut("sender").unwrap();
    *grant = Grant::issue(&config::token(), grant.permissions.clone());
    assert!(
        store
            .apply(&configuration, claim(&offer, Uuid::new_v4()))
            .is_err()
    );
    let mut expired = offer.clone();
    expired.task.id = Uuid::new_v4();
    expired.task.created_at = now() - TTL;
    expired.task.expires_at = now();
    assert!(store.receive("sender", &token, expired).is_err());
    let mut large = offer;
    large.task.id = Uuid::new_v4();
    large.task.description = "界".repeat(3000);
    assert!(store.receive("sender", &token, large).is_err());
}
#[test]
fn corrupt_or_foreign_ledgers_are_preserved_and_archive_keeps_deduplication() {
    let (_root, store, configuration, offer, token) = setup();
    fs::write(&store.path, b"broken").unwrap();
    assert!(store.receive("sender", &token, offer.clone()).is_err());
    assert_eq!(fs::read(&store.path).unwrap(), b"broken");
    fs::remove_file(&store.path).unwrap();
    store.receive("sender", &token, offer.clone()).unwrap();
    assert!(
        store
            .apply(
                &configuration,
                Command::Archive {
                    id: offer.task.id,
                    dispatch: offer.dispatch
                }
            )
            .is_err()
    );
    store
        .apply(
            &configuration,
            Command::Decline {
                id: offer.task.id,
                dispatch: offer.dispatch,
                reason: "Not equipped for this task".into(),
            },
        )
        .unwrap();
    store
        .apply(
            &configuration,
            Command::Archive {
                id: offer.task.id,
                dispatch: offer.dispatch,
            },
        )
        .unwrap();
    assert!(
        store.view(&configuration).unwrap()["incoming"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .receive("sender", &token, offer.clone())
            .unwrap()
            .state,
        State::Failed
    );
    assert_eq!(store.read().unwrap().incoming.len(), 1);
    let foreign = Store {
        path: store.path.clone(),
        owner: "different".into(),
    };
    assert!(foreign.read().is_err());
}
#[test]
fn concurrent_local_agents_cannot_both_claim_one_attempt() {
    let (_root, store, configuration, offer, token) = setup();
    store.receive("sender", &token, offer.clone()).unwrap();
    let workers = (0..8)
        .map(|_| {
            let store = store.clone();
            let configuration = configuration.clone();
            let offer = offer.clone();
            std::thread::spawn(move || {
                store
                    .apply(&configuration, claim(&offer, Uuid::new_v4()))
                    .is_ok()
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(|success| *success)
            .count(),
        1
    );
}
#[test]
fn outgoing_retries_are_idempotent_and_do_not_treat_outages_as_failure() {
    let (root, store, mut configuration, offer, _) = setup();
    configuration.node_id = "sender".into();
    let store = Store {
        path: store.path,
        owner: "sender".into(),
    };
    let token_file = root.path().join("route.token");
    config::write_private(&token_file, config::token().as_bytes(), false).unwrap();
    configuration.routes.insert(
        "target".into(),
        Route {
            alternatives: vec![],
            url: "http://127.0.0.1:9000".into(),
            token_file,
            allow_insecure: false,
            local_forward_lock: None,
        },
    );
    let create = Command::Create {
        id: offer.task.id,
        dispatch: offer.dispatch,
        node: "target".into(),
        title: offer.task.title,
        description: offer.task.description,
        target_session: None,
        run_id: None,
        depends_on: Vec::new(),
    };
    store.apply(&configuration, create.clone()).unwrap();
    let saved = fs::read(&store.path).unwrap();
    store.apply(&configuration, create).unwrap();
    assert_eq!(fs::read(&store.path).unwrap(), saved);
    let new_dispatch = Uuid::new_v4();
    let retry = Command::Retry {
        id: offer.task.id,
        dispatch: offer.dispatch,
        new_dispatch,
    };
    assert!(store.apply(&configuration, retry.clone()).is_err());
    store
        .update(|ledger| {
            let receipt = ledger
                .outgoing
                .get_mut(&offer.task.id)
                .unwrap()
                .history
                .last_mut()
                .unwrap();
            receipt.state = State::Failed;
            receipt.result = Some("Explicit refusal".into());
            receipt.change()?;
            Ok(())
        })
        .unwrap();
    store.apply(&configuration, retry.clone()).unwrap();
    store.apply(&configuration, retry).unwrap();
    assert_eq!(
        store.read().unwrap().outgoing[&offer.task.id].history.len(),
        2
    );
    assert!(
        store
            .apply(
                &configuration,
                Command::Cancel {
                    id: offer.task.id,
                    dispatch: offer.dispatch
                }
            )
            .is_err()
    );
    configuration.routes.get_mut("target").unwrap().url = "http://127.0.0.1:9001".into();
    assert!(
        store
            .apply(
                &configuration,
                Command::Sync {
                    id: offer.task.id,
                    dispatch: new_dispatch
                }
            )
            .is_err()
    );
    assert_eq!(
        store.view(&configuration).unwrap()["outgoing"][0]["blocked"],
        true
    );
}
#[test]
fn receipt_validation_rejects_unbound_regressing_or_mutated_final_results() {
    let (_root, _, _, offer, _) = setup();
    let queued = Receipt::queued(&offer.task, offer.dispatch);
    let mut receipt = queued.clone();
    receipt.state = State::Running;
    receipt.session = Some(Uuid::new_v4());
    receipt.change().unwrap();
    receipt.validate(&offer.task).unwrap();
    super::delivery::advance(&queued, &receipt).unwrap();
    let mut regressed = receipt.clone();
    regressed.state = State::Available;
    regressed.session = None;
    regressed.change().unwrap();
    assert!(super::delivery::advance(&receipt, &regressed).is_err());
    let mut final_result = receipt.clone();
    final_result.state = State::Succeeded;
    final_result.result = Some("Done".into());
    final_result.change().unwrap();
    let mut changed = final_result.clone();
    changed.result = Some("Changed".into());
    changed.change().unwrap();
    assert!(super::delivery::advance(&final_result, &changed).is_err());
    changed = receipt;
    changed.recipient = "wrong".into();
    assert!(changed.validate(&offer.task).is_err());
}

#[test]
fn questions_are_bounded_owned_and_idempotent_and_late_answers_do_not_resume_work() {
    let (_root, store, mut configuration, mut offer, token) = setup();
    store.receive("sender", &token, offer.clone()).unwrap();
    let session = Uuid::new_v4();
    let question = Uuid::new_v4();
    let ask = Command::Ask {
        id: offer.task.id,
        dispatch: offer.dispatch,
        session,
        question,
        text: "Which branch should I check?".into(),
    };
    assert!(store.apply(&configuration, ask.clone()).is_err());
    store.apply(&configuration, claim(&offer, session)).unwrap();
    store.apply(&configuration, ask.clone()).unwrap();
    let saved = fs::read(&store.path).unwrap();
    store.apply(&configuration, ask.clone()).unwrap();
    assert_eq!(saved, fs::read(&store.path).unwrap());
    let another = Command::Ask {
        id: offer.task.id,
        dispatch: offer.dispatch,
        session,
        question: Uuid::new_v4(),
        text: "Second question".into(),
    };
    assert!(store.apply(&configuration, another).is_err());
    let wrong_session = Command::Ask {
        id: offer.task.id,
        dispatch: offer.dispatch,
        session: Uuid::new_v4(),
        question,
        text: "Which branch should I check?".into(),
    };
    assert!(store.apply(&configuration, wrong_session).is_err());
    offer.answer = Some(Answer {
        question,
        text: "Use the release branch.".into(),
    });
    let answered = store.receive("sender", &token, offer.clone()).unwrap();
    assert_eq!(
        answered.questions[0].answer.as_deref(),
        Some("Use the release branch.")
    );
    assert_eq!(
        answered,
        store.receive("sender", &token, offer.clone()).unwrap()
    );
    store.apply(&configuration, ask).unwrap();
    offer.answer.as_mut().unwrap().text = "Different answer".into();
    assert!(store.receive("sender", &token, offer.clone()).is_err());
    for _ in 1..MAX_QUESTIONS {
        let question = Uuid::new_v4();
        store
            .apply(
                &configuration,
                Command::Ask {
                    id: offer.task.id,
                    dispatch: offer.dispatch,
                    session,
                    question,
                    text: "Next question".into(),
                },
            )
            .unwrap();
        offer.answer = Some(Answer {
            question,
            text: "Continue checking.".into(),
        });
        store.receive("sender", &token, offer.clone()).unwrap();
    }
    assert!(
        store
            .apply(
                &configuration,
                Command::Ask {
                    id: offer.task.id,
                    dispatch: offer.dispatch,
                    session,
                    question: Uuid::new_v4(),
                    text: "Over capacity".into()
                }
            )
            .is_err()
    );
    configuration.grants.clear();
    assert!(
        store
            .apply(
                &configuration,
                Command::Ask {
                    id: offer.task.id,
                    dispatch: offer.dispatch,
                    session,
                    question: Uuid::new_v4(),
                    text: "No grant".into()
                }
            )
            .is_err()
    );
    store
        .apply(
            &configuration,
            finish(
                &offer,
                session,
                Finish::Succeeded,
                "Completed after replies",
            ),
        )
        .unwrap();
    let final_receipt = store
        .receipt("sender", &token, offer.task.id, offer.dispatch)
        .unwrap();
    offer.answer = Some(Answer {
        question: Uuid::new_v4(),
        text: "Late answer must not change completion.".into(),
    });
    assert_eq!(
        store.receive("sender", &token, offer).unwrap(),
        final_receipt
    );
}

#[test]
fn queued_answers_survive_reload_and_cannot_be_replaced_or_cross_dispatches() {
    let (root, store, mut configuration, offer, _) = setup();
    let token_file = root.path().join("answer.token");
    config::write_private(&token_file, config::token().as_bytes(), false).unwrap();
    configuration.routes.insert(
        "sender".into(),
        Route {
            alternatives: vec![],
            url: "http://127.0.0.1:9000".into(),
            token_file,
            allow_insecure: false,
            local_forward_lock: None,
        },
    );
    store
        .apply(
            &configuration,
            Command::Create {
                id: offer.task.id,
                dispatch: offer.dispatch,
                node: "sender".into(),
                title: "Check".into(),
                description: "Review".into(),
                target_session: None,
                run_id: None,
                depends_on: Vec::new(),
            },
        )
        .unwrap();
    let question = Uuid::new_v4();
    store
        .update(|ledger| {
            let receipt = ledger
                .outgoing
                .get_mut(&offer.task.id)
                .unwrap()
                .history
                .last_mut()
                .unwrap();
            receipt.state = State::Running;
            receipt.session = Some(Uuid::new_v4());
            receipt.questions.push(Question {
                id: question,
                text: "Use which version?".into(),
                answer: None,
            });
            receipt.change()
        })
        .unwrap();
    let reply = Command::Answer {
        id: offer.task.id,
        dispatch: offer.dispatch,
        question,
        text: "Use the checked version.".into(),
    };
    store.apply(&configuration, reply.clone()).unwrap();
    let saved = fs::read(&store.path).unwrap();
    store.apply(&configuration, reply.clone()).unwrap();
    assert_eq!(fs::read(&store.path).unwrap(), saved);
    let reopened = Store::new(&root.path().join("peers.json"), "target".into());
    assert_eq!(
        reopened.view(&configuration).unwrap()["outgoing"][0]["pending_answer"]["question"],
        question.to_string()
    );
    assert!(
        store
            .apply(
                &configuration,
                Command::Answer {
                    id: offer.task.id,
                    dispatch: offer.dispatch,
                    question,
                    text: "Replace queued answer".into()
                }
            )
            .is_err()
    );
    assert!(
        store
            .apply(
                &configuration,
                Command::Answer {
                    id: offer.task.id,
                    dispatch: Uuid::new_v4(),
                    question,
                    text: "Use the checked version.".into()
                }
            )
            .is_err()
    );
    store
        .apply(
            &configuration,
            Command::Cancel {
                id: offer.task.id,
                dispatch: offer.dispatch,
            },
        )
        .unwrap();
    assert!(
        store.read().unwrap().outgoing[&offer.task.id]
            .pending_answer
            .is_none()
    );
    assert!(store.apply(&configuration, reply).is_err());
}

#[test]
fn receipt_updates_preserve_question_and_answer_history() {
    let (_root, store, configuration, offer, token) = setup();
    store.receive("sender", &token, offer.clone()).unwrap();
    let session = Uuid::new_v4();
    store.apply(&configuration, claim(&offer, session)).unwrap();
    store
        .apply(
            &configuration,
            Command::Ask {
                id: offer.task.id,
                dispatch: offer.dispatch,
                session,
                question: Uuid::new_v4(),
                text: "Check this?".into(),
            },
        )
        .unwrap();
    let before = store
        .receipt("sender", &token, offer.task.id, offer.dispatch)
        .unwrap();
    let mut after = before.clone();
    after.change().unwrap();
    after.questions.clear();
    assert!(delivery::advance(&before, &after).is_err());
    after.questions = before.questions.clone();
    after.questions[0].text = "Changed question".into();
    assert!(delivery::advance(&before, &after).is_err());
    after.questions = before.questions.clone();
    after.questions[0].answer = Some("Yes".into());
    delivery::advance(&before, &after).unwrap();
    let queued = Answer {
        question: after.questions[0].id,
        text: "Yes".into(),
    };
    delivery::check_pending_answer(Some(&queued), &before).unwrap();
    delivery::check_pending_answer(Some(&queued), &after).unwrap();
    delivery::check_pending_answer(None, &after).unwrap();
    let mut changed = after.clone();
    changed.change().unwrap();
    changed.questions[0].answer = Some("No".into());
    assert!(delivery::check_pending_answer(Some(&queued), &changed).is_err());
    assert!(delivery::advance(&after, &changed).is_err());
    changed.questions[0].answer = None;
    assert!(delivery::advance(&after, &changed).is_err());
    assert!(serde_json::from_value::<Command>(json!({"operation":"list","unknown":true})).is_err());
}

#[test]
fn cancellation_and_new_attempts_never_accept_a_late_answer() {
    let (_root, store, configuration, mut offer, token) = setup();
    store.receive("sender", &token, offer.clone()).unwrap();
    let session = Uuid::new_v4();
    store.apply(&configuration, claim(&offer, session)).unwrap();
    let question = Uuid::new_v4();
    let ask = Command::Ask {
        id: offer.task.id,
        dispatch: offer.dispatch,
        session,
        question,
        text: "Should I continue?".into(),
    };
    store.apply(&configuration, ask.clone()).unwrap();
    offer.answer = Some(Answer {
        question,
        text: "Continue checking.".into(),
    });
    offer.cancel = true;
    let stopping = store.receive("sender", &token, offer.clone()).unwrap();
    assert_eq!(stopping.state, State::CancelRequested);
    assert!(stopping.questions[0].answer.is_none());
    // An answer already in flight cannot resume a cancelled task.
    offer.cancel = false;
    assert_eq!(
        store.receive("sender", &token, offer.clone()).unwrap(),
        stopping
    );
    store
        .apply(
            &configuration,
            finish(
                &offer,
                session,
                Finish::Cancelled,
                "Stopped at a safe boundary.",
            ),
        )
        .unwrap();
    let cancelled = store
        .receipt("sender", &token, offer.task.id, offer.dispatch)
        .unwrap();
    assert_eq!(
        store.receive("sender", &token, offer.clone()).unwrap(),
        cancelled
    );
    let old = offer.clone();
    offer.dispatch = Uuid::new_v4();
    offer.answer = None;
    store.receive("sender", &token, offer.clone()).unwrap();
    store.apply(&configuration, claim(&offer, session)).unwrap();
    // Even reusing a question UUID in a new attempt cannot import the old reply.
    store
        .apply(
            &configuration,
            Command::Ask {
                id: offer.task.id,
                dispatch: offer.dispatch,
                session,
                question,
                text: "Should I continue?".into(),
            },
        )
        .unwrap();
    assert_eq!(store.receive("sender", &token, old).unwrap(), cancelled);
    let current = store
        .receipt("sender", &token, offer.task.id, offer.dispatch)
        .unwrap();
    assert_eq!(current.state, State::Running);
    assert!(current.questions[0].answer.is_none());
}
