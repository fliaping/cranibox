//! Receiver-owned, opt-in task workers. Configuration never travels with a task.
use super::*;
use crate::peers::executions::{self, AgentLaunch, Requester};
use std::path::Path;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Report {
    HumanReview,
    AgentProposal,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub enabled: bool,
    pub senders: Vec<String>,
    pub agent: runs::Options,
    pub parallel: u8,
    pub report: Report,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Get {},
    Prepare {
        revision: u64,
        options: Options,
    },
    Save {
        revision: u64,
        options: Options,
        digest: String,
    },
    Pause {
        revision: u64,
    },
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    generation: Uuid,
    options: Options,
    credentials: BTreeMap<String, String>,
    program: String,
    review_digest: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    owner: String,
    revision: u64,
    policy: Option<Policy>,
}
fn read(path: &Path, owner: &str) -> Result<Settings> {
    let file = match fs::File::open(path.with_file_name("task-worker.json")) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings {
                owner: owner.into(),
                revision: 0,
                policy: None,
            });
        }
        Err(error) => return Err(error.into()),
    };
    ensure!(
        file.metadata()?.len() <= 64 * 1024,
        "worker configuration is too large"
    );
    let settings: Settings =
        serde_json::from_reader(file).context("read worker policy; existing data was preserved")?;
    ensure!(
        settings.owner == owner,
        "worker policy belongs to another device"
    );
    if let Some(policy) = &settings.policy {
        identifier(policy.generation)?;
        shape(&policy.options)?;
        ensure!(
            policy.options.senders.len() == policy.credentials.len()
                && policy.options.senders.iter().all(|node| policy
                    .credentials
                    .get(node)
                    .is_some_and(|hash| hash.len() == 64
                        && hash.bytes().all(|byte| byte.is_ascii_hexdigit())))
                && Path::new(&policy.program).is_absolute(),
            "invalid worker policy binding"
        );
    }
    Ok(settings)
}
fn shape(options: &Options) -> Result<()> {
    ensure!(
        (1..=4).contains(&options.parallel),
        "automatic workers allow one to four parallel processes"
    );
    ensure!(
        !options.senders.is_empty()
            && options.senders.len() <= 32
            && options.senders.iter().all(|node| valid_node_id(node))
            && options
                .senders
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == options.senders.len(),
        "select one to 32 distinct sender devices"
    );
    ensure!(
        options.agent.communication && options.agent.cwd.is_absolute(),
        "automatic workers require task communication and an absolute working directory"
    );
    Ok(())
}
fn policy_active(policy: &Policy, configuration: &Config, sender: &str) -> bool {
    configuration.grants.get(sender).is_some_and(|grant| {
        grant.permissions.messages
            && policy.credentials.get(sender) == Some(&credential(&grant.token))
    })
}
fn prepare(
    path: &Path,
    configuration: &Config,
    revision: u64,
    mut options: Options,
) -> Result<Value> {
    let saved = read(path, &configuration.node_id)?;
    ensure!(
        saved.revision == revision,
        "worker policy changed; reload and review it again"
    );
    shape(&options)?;
    options.senders.sort();
    options.agent.cwd =
        fs::canonicalize(&options.agent.cwd).context("worker directory must exist")?;
    let mut credentials = BTreeMap::new();
    for sender in &options.senders {
        ensure!(
            sender != &configuration.node_id,
            "choose another device as task sender"
        );
        let grant = configuration
            .grants
            .get(sender)
            .filter(|grant| grant.permissions.messages)
            .context("each sender requires a current messages grant on this device")?;
        credentials.insert(sender.clone(), credential(&grant.token));
    }
    let review = executions::Store::new(path, configuration.node_id.clone()).apply(
        executions::Command::PrepareAgent {
            agent: AgentLaunch {
                name: "Automatic collaboration worker".into(),
                adapter: options.agent.adapter,
                access: options.agent.access,
                cwd: options.agent.cwd.clone(),
                prompt: "Task instructions are supplied only after a matching task arrives.".into(),
            },
        },
    )?;
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(
            &json!({"owner":configuration.node_id,"revision":revision,"options":options,"credentials":credentials,"launch":review["launch"]})
        )?)
    );
    Ok(
        json!({"owner":configuration.node_id,"revision":revision,"options":options,"launch":review["launch"],"available":review["available"],"digest":digest}),
    )
}
pub(super) fn apply(path: &Path, configuration: &Config, command: Command) -> Result<Value> {
    // The caller holds the peers credential lock, including the review/save boundary.
    let mut settings = read(path, &configuration.node_id)?;
    match command {
        Command::Get {} => return view(path, configuration, &settings),
        Command::Prepare { revision, options } => {
            return prepare(path, configuration, revision, options);
        }
        Command::Save {
            revision,
            options,
            digest,
        } => {
            // A lost save response may be retried without starting a new generation.
            if settings.revision == revision.saturating_add(1)
                && settings.policy.as_ref().is_some_and(|policy| {
                    policy.review_digest == digest && policy.options == options
                })
            {
                return view(path, configuration, &settings);
            }
            let review = prepare(path, configuration, revision, options)?;
            ensure!(
                review["digest"] == digest && review["available"] == true,
                "Agent or grants changed, or CLI is unavailable; review the worker policy again"
            );
            let options: Options = serde_json::from_value(review["options"].clone())?;
            let credentials = options
                .senders
                .iter()
                .map(|node| (node.clone(), credential(&configuration.grants[node].token)))
                .collect();
            settings.policy = Some(Policy {
                generation: Uuid::new_v4(),
                options,
                credentials,
                program: review["launch"]["program"]
                    .as_str()
                    .context("Agent executable unavailable")?
                    .into(),
                review_digest: digest,
            });
        }
        Command::Pause { revision } => {
            if settings.revision == revision.saturating_add(1)
                && settings
                    .policy
                    .as_ref()
                    .is_some_and(|policy| !policy.options.enabled)
            {
                return view(path, configuration, &settings);
            }
            ensure!(
                settings.revision == revision,
                "worker policy changed; reload before pausing"
            );
            settings
                .policy
                .as_mut()
                .context("no automatic worker policy configured")?
                .options
                .enabled = false;
        }
    }
    settings.revision = settings
        .revision
        .checked_add(1)
        .context("worker revision overflow")?;
    config::write_private(
        &path.with_file_name("task-worker.json"),
        &serde_json::to_vec(&settings)?,
        true,
    )?;
    view(path, configuration, &settings)
}
fn view(path: &Path, configuration: &Config, settings: &Settings) -> Result<Value> {
    let store = Store::new(path, configuration.node_id.clone());
    let ledger = store.read()?;
    let records = executions::Store::new(path, configuration.node_id.clone()).records()?;
    let mut runs = Vec::new();
    for item in ledger.incoming.values().filter(|item| !item.archived) {
        let receipt = item.history.last().context("task receipt unavailable")?;
        if let Some(binding) = item
            .runs
            .iter()
            .find(|binding| binding.dispatch == receipt.dispatch && binding.automatic.is_some())
        {
            let record = records.iter().find(|record| record.id == binding.id);
            let same_policy = settings.policy.as_ref().is_some_and(|policy| {
                binding
                    .automatic
                    .as_ref()
                    .is_some_and(|automatic| automatic.generation == policy.generation)
            });
            let phase = if receipt.state.terminal() {
                "reported"
            } else if !authorized(configuration, item) {
                "grant_revoked"
            } else if record.is_some_and(|record| record.state.active()) {
                "active"
            } else if record.is_none() && !same_policy {
                "review_required"
            } else if record.is_none()
                && settings
                    .policy
                    .as_ref()
                    .is_some_and(|policy| !policy.options.enabled)
            {
                "paused"
            } else if record.is_none() {
                "reserved"
            } else {
                "review_required"
            };
            let error = record.and_then(|record| record.error.as_deref()).or_else(|| {
                if record.is_none() && !same_policy && !receipt.state.terminal() {
                    Some("Worker policy changed before enqueue. Review this task and report its unstarted attempt explicitly.")
                } else if record.is_none() && records.len() >= executions::CAPACITY {
                    Some("Execution history is full. Archive old runs; deduplication records remain for seven days.")
                } else { None }
            });
            runs.push(json!({"task":item.task.id,"title":item.task.title,"execution":binding.id,"phase":phase,"state":record.map(|record| record.state),"report":binding.automatic.as_ref().map(|automatic| automatic.report),"error":error}));
        }
    }
    let senders = configuration.grants.iter().filter(|(_, grant)| grant.permissions.messages).map(|(node, _)| json!({"node":node,"selected":settings.policy.as_ref().is_some_and(|policy| policy.credentials.contains_key(node)),"binding_active":settings.policy.as_ref().is_some_and(|policy| policy_active(policy, configuration, node))})).collect::<Vec<_>>();
    let ready = settings.policy.as_ref().map(|policy| {
        let directory = fs::canonicalize(&policy.options.agent.cwd)
            .is_ok_and(|cwd| cwd == policy.options.agent.cwd);
        let agent = executions::Store::new(path, configuration.node_id.clone())
            .apply(executions::Command::PrepareAgent {
                agent: AgentLaunch {
                    name: "Worker availability".into(),
                    adapter: policy.options.agent.adapter,
                    access: policy.options.agent.access,
                    cwd: policy.options.agent.cwd.clone(),
                    prompt: "Check availability only.".into(),
                },
            })
            .is_ok_and(|review| {
                review["available"] == true && review["launch"]["program"] == policy.program
            });
        json!({"directory":directory,"agent":agent})
    });
    Ok(
        json!({"owner":configuration.node_id,"revision":settings.revision,"policy":settings.policy.as_ref().map(|policy| &policy.options),"generation":settings.policy.as_ref().map(|policy| policy.generation),"senders":senders,"runs":runs,"ready":ready}),
    )
}
fn launch(
    path: &Path,
    configuration: &Config,
    store: &Store,
    policy: &Policy,
    id: Uuid,
    dispatch: Uuid,
) -> Result<()> {
    ensure!(
        fs::canonicalize(&policy.options.agent.cwd)? == policy.options.agent.cwd,
        "worker directory changed; review policy again"
    );
    let binding = store.update(|ledger| {
        let item = ledger.incoming.get_mut(&id).context("incoming task unavailable")?;
        runs::eligible(item, configuration, dispatch)?;
        ensure!(policy_active(policy, configuration, &item.task.sender) && policy.credentials.get(&item.task.sender) == Some(&item.credential), "sender is no longer authorized by this worker policy");
        if let Some(run) = item.runs.iter().find(|run| run.dispatch == dispatch) {
            ensure!(run.automatic.as_ref().is_some_and(|automatic| automatic.generation == policy.generation), "dispatch already belongs to another worker");
            return Ok(run.clone());
        }
        let review = runs::prepare(path, configuration, item, dispatch, &policy.options.agent)?;
        ensure!(review["available"] == true && review["launch"]["program"] == policy.program, "Agent executable changed or unavailable; review worker policy again");
        let mut launch: executions::Launch = serde_json::from_value(review["launch"].clone())?;
        if policy.options.report == Report::AgentProposal {
            if let Some(input) = &mut launch.stdin {
                *input = input.replace("the local operator will review and submit the task result", "the receiver applies its explicitly configured reporting policy").replace("the operator must review and report it", "the receiver applies its explicitly configured reporting policy");
            }
            launch.stdin.as_mut().context("worker instructions unavailable")?.push_str("\nThe receiver explicitly enabled automatic reporting for this worker. Save a concrete outcome and evidence with cranibox_task/propose, then exit successfully. The node reports this proposal only after your process stops; an exit code alone never completes the task.\n");
        }
        let binding = runs::Binding { dispatch, id: Uuid::new_v4(), session: Uuid::new_v4(), options: policy.options.agent.clone(), launch, digest: review["digest"].as_str().context("launch digest unavailable")?.into(), proposal: None, automatic: Some(runs::Automatic { generation: policy.generation, report: policy.options.report }) };
        let receipt = current(&mut item.history, dispatch)?;
        receipt.session = Some(binding.session);
        receipt.state = State::Running;
        receipt.progress = Some("Accepted under this device's explicit automatic worker policy. Inspect the bound execution for output and reporting status.".into());
        receipt.change()?;
        item.runs.push(binding.clone());
        Ok(binding)
    })?;
    let ledger = store.read()?;
    let item = ledger
        .incoming
        .get(&id)
        .context("incoming task unavailable")?;
    let grant = configuration
        .grants
        .get(&item.task.sender)
        .context("sender grant unavailable")?;
    executions::Store::new(path, configuration.node_id.clone())
        .for_automatic_sender(Requester::new(item.task.sender.clone(), &grant.token))
        .start_with_session(binding.id, Some(binding.session), binding.launch)
}
fn settle(
    path: &Path,
    configuration: &Config,
    store: &Store,
    item: &Incoming,
    binding: &runs::Binding,
    record: &executions::Record,
) -> Result<()> {
    ensure!(
        record.session_id == binding.session
            && record.launch == binding.launch
            && record.requester.is_none()
            && record.automatic_sender.is_some(),
        "automatic task execution identity changed"
    );
    let receipt = item.history.last().context("task receipt unavailable")?;
    if !authorized(configuration, item) || receipt.state.terminal() {
        return Ok(());
    }
    if receipt.state == State::CancelRequested {
        if record.state.active() {
            executions::Store::new(path, configuration.node_id.clone())
                .apply(executions::Command::Stop { id: binding.id })?;
        } else if matches!(
            record.state,
            executions::State::Succeeded | executions::State::Failed | executions::State::Cancelled
        ) {
            store.apply(configuration, super::Command::Finish { id: item.task.id, dispatch: binding.dispatch, session: binding.session, outcome: Finish::Cancelled, result: "Automatic worker stopped after cancellation; this confirms process termination, not rollback of earlier changes.".into() })?;
        }
        return Ok(());
    }
    let Some(automatic) = &binding.automatic else {
        return Ok(());
    };
    let Some(proposal) = &binding.proposal else {
        return Ok(());
    };
    let exited = record.state == executions::State::Succeeded && record.exit_code == Some(0);
    let failed = record.state == executions::State::Failed
        && record.exit_code.is_some()
        && proposal.outcome == Finish::Failed;
    if automatic.report == Report::AgentProposal
        && (exited || failed)
        && receipt.state == State::Running
        && receipt
            .questions
            .iter()
            .all(|question| question.answer.is_some())
    {
        store.apply(
            configuration,
            super::Command::Finish {
                id: item.task.id,
                dispatch: binding.dispatch,
                session: binding.session,
                outcome: proposal.outcome,
                result: proposal.result.clone(),
            },
        )?;
    }
    Ok(())
}
/// One bounded service tick. UI closure has no effect; interrupted executions are never replayed.
pub fn tick(path: &Path) -> Result<()> {
    let _lock = config::lock_credentials(path)?;
    let configuration = Config::read(path)?;
    let settings = read(path, &configuration.node_id)?;
    let store = Store::new(path, configuration.node_id.clone());
    let ledger = store.read()?;
    let records = executions::Store::new(path, configuration.node_id.clone()).records()?;
    let mut active = 0;
    let mut total_active = records
        .iter()
        .filter(|record| record.state.active())
        .count();
    for item in ledger.incoming.values() {
        let receipt = item.history.last().context("task receipt unavailable")?;
        if let Some(binding) = item
            .runs
            .iter()
            .find(|binding| binding.dispatch == receipt.dispatch && binding.automatic.is_some())
        {
            if let Some(record) = records.iter().find(|record| record.id == binding.id) {
                active += usize::from(record.state.active());
                settle(path, &configuration, &store, item, binding, record)?;
            } else if receipt.state == State::CancelRequested && authorized(&configuration, item) {
                store.apply(&configuration, super::Command::Finish { id: item.task.id, dispatch: binding.dispatch, session: binding.session, outcome: Finish::Cancelled, result: "Automatic task cancelled before its reserved process was enqueued; no process was launched for this attempt.".into() })?;
            }
        }
    }
    let Some(policy) = settings.policy.filter(|policy| policy.options.enabled) else {
        return Ok(());
    };
    // Oldest first with a stable tie-breaker, rather than UUID insertion order.
    let mut pending = ledger
        .incoming
        .values()
        .filter(|item| {
            !item.archived
                && item.task.target_session.is_none()
                && item.task.expires_at > now()
                && authorized(&configuration, item)
                && policy_active(&policy, &configuration, &item.task.sender)
        })
        .collect::<Vec<_>>();
    pending.sort_by_key(|item| (item.task.created_at, item.task.id));
    for item in pending {
        if active >= usize::from(policy.options.parallel) || total_active >= executions::PARALLEL {
            break;
        }
        let receipt = item.history.last().context("task receipt unavailable")?;
        let reserved = item
            .runs
            .iter()
            .find(|binding| binding.dispatch == receipt.dispatch);
        let resume = reserved.is_some_and(|binding| {
            binding
                .automatic
                .as_ref()
                .is_some_and(|automatic| automatic.generation == policy.generation)
                && !records.iter().any(|record| record.id == binding.id)
        });
        if (receipt.state == State::Available && reserved.is_none())
            || (receipt.state == State::Running && resume)
        {
            // A failed enqueue leaves one durable reservation; it never creates a second run.
            if launch(
                path,
                &configuration,
                &store,
                &policy,
                item.task.id,
                receipt.dispatch,
            )
            .is_ok()
            {
                active += 1;
                total_active += 1;
            }
        }
    }
    Ok(())
}
