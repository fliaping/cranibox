//! Dependency decisions belong to the sender. Receivers still explicitly claim each task.
use super::*;
use std::collections::BTreeSet;

const LOCAL_CANCEL: &str = "Cancelled locally before delegation; this attempt was never sent.";

pub(super) fn canonical(mut ids: Vec<Uuid>) -> Result<Vec<Uuid>> {
    ensure!(ids.len() <= 8, "choose up to eight prerequisite tasks");
    for id in &ids {
        identifier(*id)?;
    }
    ids.sort();
    ensure!(
        ids.windows(2).all(|pair| pair[0] != pair[1]),
        "duplicate prerequisite task"
    );
    Ok(ids)
}

pub(super) fn ready(item: &Outgoing) -> bool {
    item.dependencies.is_empty() || item.released
}

fn succeeded(ledger: &Ledger, item: &Outgoing) -> bool {
    item.dependencies.iter().all(|id| {
        ledger.outgoing.get(id).is_some_and(|parent| {
            parent
                .history
                .last()
                .is_some_and(|receipt| receipt.state == State::Succeeded)
        })
    })
}

pub(super) fn validate(ledger: &Ledger) -> Result<()> {
    for item in ledger.outgoing.values() {
        ensure!(
            canonical(item.dependencies.clone())? == item.dependencies,
            "prerequisites must be in canonical order"
        );
        ensure!(
            !item.released || !item.dependencies.is_empty(),
            "invalid dependency release"
        );
        for id in &item.dependencies {
            let parent = ledger
                .outgoing
                .get(id)
                .context("prerequisite must be an existing outgoing task")?;
            ensure!(
                *id != item.task.id
                    && item.task.collaboration.is_some()
                    && parent.task.collaboration == item.task.collaboration,
                "prerequisites must be other tasks in the same collaboration"
            );
        }
        if !ready(item) {
            ensure!(
                item.attempts == 0
                    && item.synced_at.is_none()
                    && item.pending_answer.is_none()
                    && item.history.iter().all(|receipt| {
                        receipt.state == State::Queued
                            || (receipt.state == State::Cancelled
                                && receipt.result.as_deref() == Some(LOCAL_CANCEL)
                                && receipt.session.is_none()
                                && receipt.progress.is_none()
                                && receipt.questions.is_empty())
                    }),
                "unreleased task has delegation history"
            );
        }
        ensure!(
            !item.released || succeeded(ledger, item),
            "released task lacks successful prerequisites"
        );
    }
    fn visit(
        id: Uuid,
        ledger: &Ledger,
        visiting: &mut BTreeSet<Uuid>,
        visited: &mut BTreeSet<Uuid>,
    ) -> Result<()> {
        if visited.contains(&id) {
            return Ok(());
        }
        ensure!(visiting.insert(id), "cyclic task prerequisites");
        for parent in &ledger.outgoing[&id].dependencies {
            visit(*parent, ledger, visiting, visited)?;
        }
        visiting.remove(&id);
        visited.insert(id);
        Ok(())
    }
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for id in ledger.outgoing.keys() {
        visit(*id, ledger, &mut visiting, &mut visited)?;
    }
    Ok(())
}

/// Persist this decision before selecting an offer, so uncertain sends can never be cancelled locally.
pub(super) fn release_ready(ledger: &mut Ledger, configuration: &Config) {
    let release = ledger
        .outgoing
        .iter()
        .filter_map(|(id, item)| {
            let scope = item.task.collaboration.as_ref()?;
            (!ready(item)
                && !item.cancel
                && !item.archived
                && !item.blocked
                && item.task.expires_at > now()
                && item
                    .history
                    .last()
                    .is_some_and(|receipt| !receipt.state.terminal())
                && super::super::messages::binding(configuration, &item.task.recipient)
                    .ok()
                    .as_deref()
                    == Some(&item.binding)
                && collaborations::scope(ledger, scope.id, &item.task.recipient, &item.binding)
                    .is_ok()
                && succeeded(ledger, item))
            .then_some(*id)
        })
        .collect::<Vec<_>>();
    for id in release {
        ledger.outgoing.get_mut(&id).unwrap().released = true;
    }
}

/// An unreleased task cannot have reached a receiver. Stopping it needs no remote confirmation.
pub(super) fn settle_cancellations(ledger: &mut Ledger) -> Result<()> {
    for item in ledger
        .outgoing
        .values_mut()
        .filter(|item| item.cancel && !ready(item))
    {
        let receipt = item
            .history
            .last_mut()
            .context("task dispatch unavailable")?;
        if receipt.state == State::Queued {
            receipt.state = State::Cancelled;
            receipt.result = Some(LOCAL_CANCEL.into());
            receipt.change()?;
        }
    }
    Ok(())
}

pub(super) fn view(ledger: &Ledger, item: &Outgoing) -> Value {
    let prerequisites = item.dependencies.iter().map(|id| {
        let parent = &ledger.outgoing[id];
        json!({"id":id,"title":parent.task.title,"node":parent.task.recipient,"current":parent.history.last()})
    }).collect::<Vec<_>>();
    let state = if item.dependencies.is_empty() {
        None
    } else if item.released {
        Some("released")
    } else if item
        .history
        .last()
        .is_some_and(|receipt| receipt.state == State::Cancelled)
    {
        Some("cancelled_before_dispatch")
    } else {
        Some("waiting")
    };
    json!({"prerequisites":prerequisites,"state":state})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peers::config::Route;

    fn setup() -> (tempfile::TempDir, Store, Config, Uuid, Uuid) {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::new();
        config.node_id = "coordinator".into();
        let token_file = root.path().join("target.token");
        config::write_private(&token_file, config::token().as_bytes(), false).unwrap();
        config.routes.insert(
            "target".into(),
            Route {
                url: "http://127.0.0.1:19001".into(),
                token_file,
                allow_insecure: false,
                local_forward_lock: None,
                alternatives: vec![],
                target_identity: None,
                config_path: None,
            },
        );
        let store = Store::new(&root.path().join("peers.json"), config.node_id.clone());
        let run = Uuid::new_v4();
        store
            .apply(
                &config,
                Command::Collaboration {
                    command: collaborations::Command::Create {
                        id: run,
                        name: "Build and review".into(),
                        goal: "Review after a successful build".into(),
                        members: vec!["target".into()],
                        project_id: None,
                    },
                },
            )
            .unwrap();
        let parent = Uuid::new_v4();
        store
            .apply(&config, create(parent, Some(run), vec![]))
            .unwrap();
        (root, store, config, run, parent)
    }
    fn create(id: Uuid, run_id: Option<Uuid>, depends_on: Vec<Uuid>) -> Command {
        Command::Create {
            id,
            dispatch: Uuid::from_u128(55),
            node: "target".into(),
            title: "Review".into(),
            description: "Report evidence".into(),
            target_session: None,
            run_id,
            depends_on,
        }
    }
    fn finish(store: &Store, id: Uuid, state: State) {
        store
            .update(|ledger| {
                let receipt = ledger
                    .outgoing
                    .get_mut(&id)
                    .unwrap()
                    .history
                    .last_mut()
                    .unwrap();
                receipt.state = state;
                receipt.session = Some(Uuid::from_u128(99));
                receipt.result = Some("Explicit worker report".into());
                receipt.change()?;
                Ok(())
            })
            .unwrap();
    }
    #[test]
    fn prerequisites_require_existing_distinct_tasks_in_the_same_run() {
        let (_root, store, config, run, parent) = setup();
        let id = Uuid::new_v4();
        let saved = fs::read(&store.path).unwrap();
        for command in [
            create(id, None, vec![parent]),
            create(id, Some(run), vec![id]),
            create(id, Some(run), vec![Uuid::new_v4()]),
            create(id, Some(run), vec![parent, parent]),
            create(id, Some(run), vec![Uuid::nil()]),
            create(id, Some(run), vec![parent; 9]),
        ] {
            assert!(store.apply(&config, command).is_err());
            assert_eq!(fs::read(&store.path).unwrap(), saved);
        }
        store
            .apply(&config, create(id, Some(run), vec![parent]))
            .unwrap();
        assert!(store.apply(&config, create(id, Some(run), vec![])).is_err());
    }
    #[test]
    fn only_a_successful_receipt_releases_a_dependency_and_restart_retains_the_decision() {
        let (_root, store, config, run, parent) = setup();
        let id = Uuid::new_v4();
        store
            .apply(&config, create(id, Some(run), vec![parent]))
            .unwrap();
        store
            .update(|ledger| {
                release_ready(ledger, &config);
                Ok(())
            })
            .unwrap();
        assert!(!ready(&store.read().unwrap().outgoing[&id]));
        finish(&store, parent, State::Failed);
        store
            .update(|ledger| {
                release_ready(ledger, &config);
                Ok(())
            })
            .unwrap();
        assert!(!ready(&store.read().unwrap().outgoing[&id]));
        store
            .apply(
                &config,
                Command::Retry {
                    id: parent,
                    dispatch: Uuid::from_u128(55),
                    new_dispatch: Uuid::new_v4(),
                },
            )
            .unwrap();
        finish(&store, parent, State::Succeeded);
        store
            .update(|ledger| {
                release_ready(ledger, &config);
                Ok(())
            })
            .unwrap();
        let restored = Store::new(
            &store.path.with_file_name("peers.json"),
            store.owner.clone(),
        );
        assert!(restored.read().unwrap().outgoing[&id].released);
        assert_eq!(restored.read().unwrap().outgoing[&id].attempts, 0);
    }
    #[test]
    fn pending_cancellation_is_local_even_after_credential_rotation_and_can_be_retried() {
        let (_root, store, mut config, run, parent) = setup();
        let id = Uuid::new_v4();
        store
            .apply(&config, create(id, Some(run), vec![parent]))
            .unwrap();
        let route = config.routes.remove("target").unwrap();
        let cancel = Command::Cancel {
            id,
            dispatch: Uuid::from_u128(55),
        };
        store.apply(&config, cancel.clone()).unwrap();
        store.apply(&config, cancel).unwrap();
        let ledger = store.read().unwrap();
        assert_eq!(
            ledger.outgoing[&id].history.last().unwrap().state,
            State::Cancelled
        );
        assert_eq!(ledger.outgoing[&id].attempts, 0);
        config.routes.insert("target".into(), route);
        store
            .apply(
                &config,
                Command::Retry {
                    id,
                    dispatch: Uuid::from_u128(55),
                    new_dispatch: Uuid::new_v4(),
                },
            )
            .unwrap();
        assert!(!ready(&store.read().unwrap().outgoing[&id]));
        assert_eq!(store.read().unwrap().outgoing[&id].history.len(), 2);
    }
    #[test]
    fn stopping_a_run_cancels_waiting_work_and_never_releases_it_later() {
        let (_root, store, config, run, parent) = setup();
        let id = Uuid::new_v4();
        store
            .apply(&config, create(id, Some(run), vec![parent]))
            .unwrap();
        store
            .apply(
                &config,
                Command::Collaboration {
                    command: collaborations::Command::Stop {
                        id: run,
                        revision: 1,
                    },
                },
            )
            .unwrap();
        finish(&store, parent, State::Succeeded);
        store
            .update(|ledger| {
                release_ready(ledger, &config);
                Ok(())
            })
            .unwrap();
        let ledger = store.read().unwrap();
        assert!(!ledger.outgoing[&id].released);
        assert_eq!(
            ledger.outgoing[&id].history.last().unwrap().state,
            State::Cancelled
        );
        store
            .apply(
                &config,
                Command::Collaboration {
                    command: collaborations::Command::Close {
                        id: run,
                        revision: 2,
                        result: "Reviewed all results".into(),
                    },
                },
            )
            .unwrap();
    }
    #[test]
    fn released_work_uses_remote_cancellation_even_before_the_first_response() {
        let (_root, store, config, run, parent) = setup();
        let id = Uuid::new_v4();
        store
            .apply(&config, create(id, Some(run), vec![parent]))
            .unwrap();
        finish(&store, parent, State::Succeeded);
        store
            .update(|ledger| {
                release_ready(ledger, &config);
                Ok(())
            })
            .unwrap();
        store
            .apply(
                &config,
                Command::Cancel {
                    id,
                    dispatch: Uuid::from_u128(55),
                },
            )
            .unwrap();
        let ledger = store.read().unwrap();
        assert!(ledger.outgoing[&id].cancel && ledger.outgoing[&id].released);
        assert_eq!(
            ledger.outgoing[&id].history.last().unwrap().state,
            State::Queued
        );
    }
    #[test]
    fn corrupt_dependency_cycles_and_forged_release_are_preserved_and_rejected() {
        let (_root, store, config, run, parent) = setup();
        let id = Uuid::new_v4();
        store
            .apply(&config, create(id, Some(run), vec![parent]))
            .unwrap();
        let saved = fs::read(&store.path).unwrap();
        assert!(
            store
                .update(|ledger| {
                    ledger.outgoing.get_mut(&parent).unwrap().dependencies = vec![id];
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(fs::read(&store.path).unwrap(), saved);
        let mut ledger: Ledger = serde_json::from_slice(&saved).unwrap();
        ledger.outgoing.get_mut(&id).unwrap().released = true;
        let corrupt = serde_json::to_vec(&ledger).unwrap();
        fs::write(&store.path, &corrupt).unwrap();
        assert!(store.read().is_err());
        assert_eq!(fs::read(&store.path).unwrap(), corrupt);
    }
}
