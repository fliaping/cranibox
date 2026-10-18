//! Owner-scoped collaboration runs. Membership never grants remote capabilities.
use super::*;
use std::collections::BTreeSet;

pub(super) const CAPACITY: usize = 32;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub id: Uuid,
    pub owner: String,
    pub name: String,
    pub goal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Uuid>,
}
impl Scope {
    pub(super) fn validate(&self, owner: &str) -> Result<()> {
        identifier(self.id)?;
        ensure!(
            self.owner == owner && valid_node_id(owner),
            "invalid collaboration owner"
        );
        text(&self.name, 240)?;
        text(&self.goal, 4096)?;
        if let Some(project) = self.project_id {
            identifier(project)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum State {
    Open,
    Stopping,
    Closed,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    scope: Scope,
    members: Vec<String>,
    bindings: BTreeMap<String, String>,
    state: State,
    revision: u64,
    created_at: u64,
    result: Option<String>,
    pub(super) archived: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Create {
        id: Uuid,
        name: String,
        goal: String,
        members: Vec<String>,
        project_id: Option<Uuid>,
    },
    Stop {
        id: Uuid,
        revision: u64,
    },
    Close {
        id: Uuid,
        revision: u64,
        result: String,
    },
    Archive {
        id: Uuid,
        revision: u64,
    },
}

fn members(value: &[String], owner: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= 16,
        "choose 1 to 16 collaboration devices"
    );
    let mut seen = BTreeSet::new();
    ensure!(
        value
            .iter()
            .all(|node| valid_node_id(node) && node != owner && seen.insert(node)),
        "invalid or duplicate collaboration device"
    );
    Ok(())
}
pub(super) fn validate(ledger: &Ledger) -> Result<()> {
    ensure!(
        ledger.collaborations.len() <= CAPACITY,
        "collaboration capacity exceeded"
    );
    for (id, run) in &ledger.collaborations {
        run.scope.validate(&ledger.owner)?;
        members(&run.members, &ledger.owner)?;
        ensure!(
            *id == run.scope.id && run.revision > 0 && run.created_at > 0,
            "invalid collaboration record"
        );
        ensure!(
            run.bindings.len() == run.members.len()
                && run.members.iter().all(|node| run
                    .bindings
                    .get(node)
                    .is_some_and(|binding| binding.len() == 64
                        && binding.bytes().all(|b| b.is_ascii_hexdigit()))),
            "invalid collaboration binding"
        );
        ensure!(
            (run.state == State::Closed) == run.result.is_some()
                && (!run.archived || run.state == State::Closed),
            "invalid collaboration final state"
        );
        if let Some(result) = &run.result {
            text(result, 8192)?;
        }
    }
    for item in ledger.outgoing.values() {
        if let Some(scope) = &item.task.collaboration {
            let run = ledger
                .collaborations
                .get(&scope.id)
                .context("outgoing collaboration unavailable")?;
            ensure!(
                *scope == run.scope
                    && run.bindings.get(&item.task.recipient) == Some(&item.binding),
                "task collaboration binding mismatch"
            );
            ensure!(
                run.state != State::Closed
                    || item
                        .history
                        .last()
                        .is_some_and(|receipt| receipt.state.terminal()),
                "closed collaboration contains unfinished tasks"
            );
            ensure!(
                run.state != State::Stopping
                    || item
                        .history
                        .last()
                        .is_some_and(|receipt| receipt.state.terminal())
                    || item.cancel,
                "stopping collaboration contains an uncancelled task"
            );
        }
    }
    Ok(())
}

/// The run and its task tombstones are reclaimed together, preserving aggregate results.
pub(super) fn prune(ledger: &mut Ledger) {
    let expired = ledger
        .collaborations
        .iter()
        .filter(|(id, run)| {
            run.archived
                && run.created_at.saturating_add(TTL) <= now()
                && ledger
                    .outgoing
                    .values()
                    .filter(|item| {
                        item.task
                            .collaboration
                            .as_ref()
                            .is_some_and(|scope| scope.id == **id)
                    })
                    .all(|item| item.archived && item.task.expires_at <= now())
        })
        .map(|(id, _)| *id)
        .collect::<BTreeSet<_>>();
    ledger.outgoing.retain(|_, item| {
        item.task
            .collaboration
            .as_ref()
            .is_none_or(|scope| !expired.contains(&scope.id))
    });
    ledger.collaborations.retain(|id, _| !expired.contains(id));
}

pub(super) fn scope(ledger: &Ledger, id: Uuid, node: &str, binding: &str) -> Result<Scope> {
    let run = ledger
        .collaborations
        .get(&id)
        .context("collaboration unavailable")?;
    ensure!(
        run.state == State::Open && !run.archived,
        "collaboration no longer accepts new tasks or retries"
    );
    ensure!(
        run.bindings.get(node).map(String::as_str) == Some(binding),
        "device is not a member or its connection/credentials changed; create a new collaboration"
    );
    Ok(run.scope.clone())
}

fn fence(run: &Record, revision: u64) -> Result<()> {
    ensure!(
        run.revision == revision,
        "collaboration changed; read its current revision before acting"
    );
    Ok(())
}
fn repeated(run: &Record, revision: u64) -> bool {
    run.revision == revision || revision.checked_add(1) == Some(run.revision)
}
pub(super) fn apply(ledger: &mut Ledger, configuration: &Config, command: Command) -> Result<()> {
    match command {
        Command::Create {
            id,
            name,
            goal,
            mut members,
            project_id,
        } => {
            let scope = Scope {
                id,
                owner: ledger.owner.clone(),
                name,
                goal,
                project_id,
            };
            scope.validate(&ledger.owner)?;
            self::members(&members, &ledger.owner)?;
            members.sort();
            if let Some(run) = ledger.collaborations.get(&id) {
                ensure!(
                    run.scope == scope && run.members == members,
                    "collaboration ID already used with different content"
                );
                return Ok(());
            }
            prune(ledger);
            ensure!(
                ledger.collaborations.len() < CAPACITY,
                "collaboration history is full; archive finished collaborations and wait for retention to expire"
            );
            let bindings = members
                .iter()
                .map(|node| {
                    Ok((
                        node.clone(),
                        super::super::messages::binding(configuration, node)?,
                    ))
                })
                .collect::<Result<_>>()?;
            ledger.collaborations.insert(
                id,
                Record {
                    scope,
                    members,
                    bindings,
                    state: State::Open,
                    revision: 1,
                    created_at: now(),
                    result: None,
                    archived: false,
                },
            );
        }
        Command::Stop { id, revision } => {
            let run = ledger
                .collaborations
                .get_mut(&id)
                .context("collaboration unavailable")?;
            if run.state == State::Stopping && repeated(run, revision) {
                return Ok(());
            }
            fence(run, revision)?;
            ensure!(
                run.state == State::Open && !run.archived,
                "collaboration is no longer open"
            );
            run.state = State::Stopping;
            run.revision = run
                .revision
                .checked_add(1)
                .context("collaboration revision exhausted")?;
            for item in ledger.outgoing.values_mut().filter(|item| {
                item.task
                    .collaboration
                    .as_ref()
                    .is_some_and(|scope| scope.id == id)
            }) {
                if item
                    .history
                    .last()
                    .is_some_and(|receipt| !receipt.state.terminal())
                {
                    item.cancel = true;
                    item.pending_answer = None;
                    item.next_attempt = 0;
                    // Delivery rechecks the original binding before sending cancellation.
                    item.blocked = false;
                    item.error = None;
                }
            }
        }
        Command::Close {
            id,
            revision,
            result,
        } => {
            text(&result, 8192)?;
            let run = ledger
                .collaborations
                .get_mut(&id)
                .context("collaboration unavailable")?;
            if run.state == State::Closed
                && run.result.as_ref() == Some(&result)
                && repeated(run, revision)
            {
                return Ok(());
            }
            fence(run, revision)?;
            ensure!(
                run.state != State::Closed && !run.archived,
                "collaboration is already closed"
            );
            ensure!(
                ledger
                    .outgoing
                    .values()
                    .filter(|item| item
                        .task
                        .collaboration
                        .as_ref()
                        .is_some_and(|scope| scope.id == id))
                    .all(|item| item
                        .history
                        .last()
                        .is_some_and(|receipt| receipt.state.terminal())),
                "wait for every task's confirmed final result before closing the collaboration"
            );
            run.state = State::Closed;
            run.result = Some(result);
            run.revision = run
                .revision
                .checked_add(1)
                .context("collaboration revision exhausted")?;
        }
        Command::Archive { id, revision } => {
            let run = ledger
                .collaborations
                .get_mut(&id)
                .context("collaboration unavailable")?;
            if run.archived && repeated(run, revision) {
                return Ok(());
            }
            fence(run, revision)?;
            ensure!(
                run.state == State::Closed,
                "close the collaboration before archiving"
            );
            run.archived = true;
            run.revision = run
                .revision
                .checked_add(1)
                .context("collaboration revision exhausted")?;
            for item in ledger.outgoing.values_mut().filter(|item| {
                item.task
                    .collaboration
                    .as_ref()
                    .is_some_and(|scope| scope.id == id)
            }) {
                item.archived = true;
            }
        }
    }
    Ok(())
}

pub(super) fn view(ledger: &Ledger, configuration: &Config) -> Vec<Value> {
    let mut runs = ledger.collaborations.values().filter(|run| !run.archived).map(|run| {
        let tasks = ledger.outgoing.values().filter(|item| item.task.collaboration.as_ref().is_some_and(|scope| scope.id == run.scope.id)).collect::<Vec<_>>();
        let receipts = tasks.iter().filter_map(|item| item.history.last()).collect::<Vec<_>>();
        let count = |state| receipts.iter().filter(|receipt| receipt.state == state).count();
        let waiting = tasks.iter().filter(|item| !dependencies::ready(item) && item.history.last().is_some_and(|receipt| !receipt.state.terminal())).count();
        let blocked = tasks.iter().filter(|item| item.blocked || super::super::messages::binding(configuration, &item.task.recipient).ok().as_deref() != Some(&item.binding)).count();
        json!({"scope":run.scope,"members":run.members,"state":run.state,"revision":run.revision,"created_at":run.created_at,"result":run.result,"stats":{"total":tasks.len(),"active":receipts.iter().filter(|receipt| !receipt.state.terminal()).count(),"succeeded":count(super::State::Succeeded),"failed":count(super::State::Failed),"cancelled":count(super::State::Cancelled),"questions":receipts.iter().filter(|receipt| receipt.state == super::State::Running && receipt.questions.iter().any(|question| question.answer.is_none())).count(),"blocked":blocked,"waiting":waiting}})
    }).collect::<Vec<_>>();
    runs.sort_by_key(|run| std::cmp::Reverse(run["created_at"].as_u64()));
    runs
}

pub(super) fn schema() -> Value {
    let id = json!({"type":"string","format":"uuid"});
    let revision = json!({"type":"integer","minimum":1});
    json!({"type":"object","oneOf":[
        {"type":"object","properties":{"operation":{"const":"create"},"id":id,"name":{"type":"string","maxLength":240},"goal":{"type":"string","maxLength":4096},"members":{"type":"array","minItems":1,"maxItems":16,"uniqueItems":true,"items":{"type":"string"}},"project_id":id},"required":["operation","id","name","goal","members"],"additionalProperties":false},
        {"type":"object","properties":{"operation":{"const":"stop"},"id":id,"revision":revision},"required":["operation","id","revision"],"additionalProperties":false},
        {"type":"object","properties":{"operation":{"const":"close"},"id":id,"revision":revision,"result":{"type":"string","maxLength":8192}},"required":["operation","id","revision","result"],"additionalProperties":false},
        {"type":"object","properties":{"operation":{"const":"archive"},"id":id,"revision":revision},"required":["operation","id","revision"],"additionalProperties":false}
    ]})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peers::config::Route;

    fn setup() -> (tempfile::TempDir, Store, Config, Uuid) {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::new();
        config.node_id = "coordinator".into();
        for node in ["laptop", "desktop", "outsider"] {
            let token_file = root.path().join(node);
            config::write_private(&token_file, config::token().as_bytes(), false).unwrap();
            config.routes.insert(
                node.into(),
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
        }
        let store = Store::new(&root.path().join("peers.json"), config.node_id.clone());
        let id = Uuid::new_v4();
        store
            .apply(
                &config,
                super::super::Command::Collaboration {
                    command: create(id),
                },
            )
            .unwrap();
        (root, store, config, id)
    }
    fn create(id: Uuid) -> Command {
        Command::Create {
            id,
            name: "Release review".into(),
            goal: "Check the same project on two devices".into(),
            members: vec!["laptop".into(), "desktop".into()],
            project_id: Some(Uuid::from_u128(7)),
        }
    }
    fn task(id: Uuid, node: &str, run: Uuid) -> super::super::Command {
        super::super::Command::Create {
            id,
            dispatch: Uuid::from_u128(55),
            node: node.into(),
            title: "Review".into(),
            description: "Report findings".into(),
            target_session: None,
            run_id: Some(run),
            depends_on: Vec::new(),
        }
    }
    fn apply(store: &Store, config: &Config, command: Command) -> Result<()> {
        store.apply(config, super::super::Command::Collaboration { command })
    }
    fn final_receipts(store: &Store) {
        store
            .update(|ledger| {
                for item in ledger.outgoing.values_mut() {
                    let receipt = item.history.last_mut().unwrap();
                    receipt.state = super::super::State::Failed;
                    receipt.result = Some("Explicit final report".into());
                    receipt.change()?;
                }
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn run_membership_context_and_credentials_are_bound() {
        let (_root, store, mut config, run) = setup();
        let id = Uuid::new_v4();
        let command = task(id, "laptop", run);
        store.apply(&config, command.clone()).unwrap();
        let saved = fs::read(&store.path).unwrap();
        store.apply(&config, command).unwrap();
        assert_eq!(fs::read(&store.path).unwrap(), saved);
        assert!(
            store
                .apply(&config, task(Uuid::new_v4(), "outsider", run))
                .is_err()
        );
        let ledger = store.read().unwrap();
        let scope = ledger.outgoing[&id].task.collaboration.as_ref().unwrap();
        assert_eq!(scope.owner, "coordinator");
        assert_eq!(scope.project_id, Some(Uuid::from_u128(7)));
        let mut invalid = ledger.outgoing[&id].task.clone();
        invalid.sender = "another".into();
        assert!(invalid.validate().is_err());
        config.routes.get_mut("laptop").unwrap().url = "http://127.0.0.1:19002".into();
        assert!(
            store
                .apply(&config, task(Uuid::new_v4(), "laptop", run))
                .is_err()
        );
        assert_eq!(
            store.view(&config).unwrap()["collaborations"][0]["stats"]["blocked"],
            1
        );
    }

    #[test]
    fn stopping_is_atomic_fenced_idempotent_and_never_implies_stopped() {
        let (_root, store, config, run) = setup();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let command = task(first, "laptop", run);
        store.apply(&config, command.clone()).unwrap();
        store.apply(&config, task(second, "desktop", run)).unwrap();
        assert!(
            apply(
                &store,
                &config,
                Command::Stop {
                    id: run,
                    revision: 7
                }
            )
            .is_err()
        );
        apply(
            &store,
            &config,
            Command::Stop {
                id: run,
                revision: 1,
            },
        )
        .unwrap();
        let saved = fs::read(&store.path).unwrap();
        apply(
            &store,
            &config,
            Command::Stop {
                id: run,
                revision: 1,
            },
        )
        .unwrap();
        assert_eq!(fs::read(&store.path).unwrap(), saved);
        store.apply(&config, command).unwrap();
        assert!(
            store
                .apply(&config, task(Uuid::new_v4(), "laptop", run))
                .is_err()
        );
        let ledger = store.read().unwrap();
        assert!(
            ledger.outgoing.values().all(|item| item.cancel
                && item.history.last().unwrap().state == super::super::State::Queued)
        );
        assert!(
            apply(
                &store,
                &config,
                Command::Close {
                    id: run,
                    revision: 2,
                    result: "Not actually finished".into()
                }
            )
            .is_err()
        );
        final_receipts(&store);
        assert!(
            store
                .apply(
                    &config,
                    super::super::Command::Retry {
                        id: first,
                        dispatch: Uuid::from_u128(55),
                        new_dispatch: Uuid::new_v4()
                    }
                )
                .is_err()
        );
        apply(
            &store,
            &config,
            Command::Close {
                id: run,
                revision: 2,
                result: "Reviewed both reports".into(),
            },
        )
        .unwrap();
        apply(
            &store,
            &config,
            Command::Close {
                id: run,
                revision: 2,
                result: "Reviewed both reports".into(),
            },
        )
        .unwrap();
        assert!(
            apply(
                &store,
                &config,
                Command::Close {
                    id: run,
                    revision: 3,
                    result: "Different final summary".into()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn archived_tasks_remain_in_run_and_retention_reclaims_them_together() {
        let (_root, store, config, run) = setup();
        let id = Uuid::new_v4();
        store.apply(&config, task(id, "laptop", run)).unwrap();
        final_receipts(&store);
        store
            .apply(
                &config,
                super::super::Command::Archive {
                    id,
                    dispatch: Uuid::from_u128(55),
                },
            )
            .unwrap();
        assert_eq!(
            store.view(&config).unwrap()["outgoing"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        apply(
            &store,
            &config,
            Command::Close {
                id: run,
                revision: 1,
                result: "Reviewed".into(),
            },
        )
        .unwrap();
        apply(
            &store,
            &config,
            Command::Archive {
                id: run,
                revision: 2,
            },
        )
        .unwrap();
        assert!(
            store.view(&config).unwrap()["outgoing"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        store
            .update(|ledger| {
                ledger.collaborations.get_mut(&run).unwrap().created_at = 1;
                let task = &mut ledger.outgoing.get_mut(&id).unwrap().task;
                task.created_at = 1;
                task.expires_at = 1 + TTL;
                prune(ledger);
                Ok(())
            })
            .unwrap();
        let ledger = store.read().unwrap();
        assert!(ledger.collaborations.is_empty());
        assert!(ledger.outgoing.is_empty());
    }

    #[test]
    fn content_conflicts_invalid_members_and_corrupt_scope_preserve_records() {
        let (_root, store, config, run) = setup();
        let saved = fs::read(&store.path).unwrap();
        apply(&store, &config, create(run)).unwrap();
        assert_eq!(fs::read(&store.path).unwrap(), saved);
        if let Command::Create { mut members, .. } = create(run) {
            members.push("desktop".into());
            assert!(
                apply(
                    &store,
                    &config,
                    Command::Create {
                        id: Uuid::new_v4(),
                        name: "Bad".into(),
                        goal: "Bad".into(),
                        members,
                        project_id: None
                    }
                )
                .is_err()
            );
        }
        assert!(
            apply(
                &store,
                &config,
                Command::Create {
                    id: run,
                    name: "Changed".into(),
                    goal: "Other".into(),
                    members: vec!["desktop".into()],
                    project_id: None
                }
            )
            .is_err()
        );
        assert_eq!(fs::read(&store.path).unwrap(), saved);
        let id = Uuid::new_v4();
        store.apply(&config, task(id, "laptop", run)).unwrap();
        let saved = fs::read(&store.path).unwrap();
        assert!(
            store
                .update(|ledger| {
                    ledger
                        .outgoing
                        .get_mut(&id)
                        .unwrap()
                        .task
                        .collaboration
                        .as_mut()
                        .unwrap()
                        .goal = "Tampered".into();
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(fs::read(&store.path).unwrap(), saved);
    }
}
