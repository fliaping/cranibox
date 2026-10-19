use super::*;
impl Store {
    pub(in crate::peers) fn activity(
        &self,
    ) -> Result<(
        Vec<super::super::activity::Task>,
        Vec<super::super::activity::Run>,
    )> {
        use super::super::activity::{Direction, Run, Task};
        let ledger = self.read()?;
        let incoming = ledger
            .incoming
            .values()
            .filter(|item| !item.archived)
            .map(|item| Task {
                direction: Direction::Incoming,
                task: item.task.clone(),
                current: item.history.last().expect("validated history").clone(),
                cancel_requested: item
                    .history
                    .last()
                    .is_some_and(|receipt| receipt.state == State::CancelRequested),
            });
        let outgoing = ledger
            .outgoing
            .values()
            .filter(|item| {
                !item.archived
                    || item.task.collaboration.as_ref().is_some_and(|scope| {
                        ledger
                            .collaborations
                            .get(&scope.id)
                            .is_some_and(|run| !run.archived)
                    })
            })
            .map(|item| Task {
                direction: Direction::Outgoing,
                task: item.task.clone(),
                current: item.history.last().expect("validated history").clone(),
                cancel_requested: item.cancel,
            });
        let runs = ledger
            .collaborations
            .values()
            .filter(|run| !run.archived)
            .map(|run| Run {
                scope: run.scope.clone(),
                members: run.members.clone(),
                state: match run.state {
                    collaborations::State::Open => "open",
                    collaborations::State::Stopping => "stopping",
                    collaborations::State::Closed => "closed",
                }
                .into(),
                revision: run.revision,
                created_at: run.created_at,
                result: run.result.clone(),
            })
            .collect();
        Ok((incoming.chain(outgoing).collect(), runs))
    }
    pub fn new(peers: &std::path::Path, owner: String) -> Self {
        Self {
            path: peers.with_file_name("device-tasks.json"),
            owner,
        }
    }
    fn history(task: &Task, history: &[Receipt], incoming: bool) -> Result<()> {
        ensure!(
            !history.is_empty() && history.len() <= MAX_ATTEMPTS,
            "invalid task history"
        );
        let mut identifiers = std::collections::BTreeSet::new();
        for (index, receipt) in history.iter().enumerate() {
            receipt.validate(task)?;
            ensure!(
                identifiers.insert(receipt.dispatch)
                    && (!incoming || receipt.state != State::Queued),
                "invalid task dispatch"
            );
            ensure!(
                index + 1 == history.len() || receipt.state.retryable(),
                "retry requires a confirmed failure or cancellation"
            );
        }
        Ok(())
    }
    pub(super) fn read(&self) -> Result<Ledger> {
        let file = match fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Ledger {
                    owner: self.owner.clone(),
                    ..Ledger::default()
                });
            }
            Err(error) => return Err(error.into()),
        };
        ensure!(
            file.metadata()?.len() <= MAX_STORE,
            "task ledger is too large"
        );
        let ledger: Ledger = serde_json::from_reader(file)
            .context("read task ledger; existing data was preserved")?;
        ensure!(
            valid_node_id(&self.owner) && ledger.owner == self.owner,
            "task ledger belongs to another node"
        );
        ensure!(
            ledger.incoming.len() + ledger.outgoing.len() <= MAX_TASKS,
            "task capacity exceeded"
        );
        for (id, item) in &ledger.incoming {
            item.task.validate()?;
            ensure!(
                *id == item.task.id
                    && item.task.recipient == self.owner
                    && item.credential.len() == 64
                    && item.credential.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid incoming task binding"
            );
            Self::history(&item.task, &item.history, true)?;
            runs::validate(item)?;
        }
        for (id, item) in &ledger.outgoing {
            item.task.validate()?;
            ensure!(
                *id == item.task.id
                    && item.task.sender == self.owner
                    && !ledger.incoming.contains_key(id)
                    && item.binding.len() == 64
                    && item.binding.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid outgoing task binding"
            );
            Self::history(&item.task, &item.history, false)?;
            if let Some(answer) = &item.pending_answer {
                answer.validate()?;
                let receipt = item.history.last().unwrap();
                ensure!(
                    !item.cancel
                        && receipt.state == State::Running
                        && receipt
                            .questions
                            .iter()
                            .any(|question| question.id == answer.question
                                && question.answer.is_none()),
                    "invalid pending task answer"
                );
            }
        }
        collaborations::validate(&ledger)?;
        dependencies::validate(&ledger)?;
        Ok(ledger)
    }
    pub(super) fn update<T>(&self, change: impl FnOnce(&mut Ledger) -> Result<T>) -> Result<T> {
        let _lock = config::lock_credentials(&self.path)?;
        let mut ledger = self.read()?;
        let before = serde_json::to_vec(&ledger)?;
        let result = change(&mut ledger)?;
        dependencies::settle_cancellations(&mut ledger)?;
        collaborations::validate(&ledger)?;
        dependencies::validate(&ledger)?;
        let bytes = serde_json::to_vec(&ledger)?;
        ensure!(bytes.len() as u64 <= MAX_STORE, "task ledger is too large");
        if bytes != before {
            config::write_private(&self.path, &bytes, true)?;
        }
        Ok(result)
    }
    fn room(ledger: &mut Ledger) -> Result<()> {
        collaborations::prune(ledger);
        ledger.incoming.retain(|_, item| {
            !(item.archived
                && item.task.expires_at <= now()
                && item
                    .history
                    .last()
                    .is_some_and(|receipt| receipt.state.terminal()))
        });
        ledger.outgoing.retain(|_, item| {
            !(item.archived
                && item
                    .task
                    .collaboration
                    .as_ref()
                    .is_none_or(|scope| !ledger.collaborations.contains_key(&scope.id))
                && item.task.expires_at <= now()
                && item
                    .history
                    .last()
                    .is_some_and(|receipt| receipt.state.terminal()))
        });
        ensure!(
            ledger.incoming.len() + ledger.outgoing.len() < MAX_TASKS,
            "task ledger is full; archive finished expired tasks"
        );
        Ok(())
    }
    /// Authentication and the messages grant are checked under the configuration lock by the caller.
    pub fn receive(&self, caller: &str, token: &str, offer: Offer) -> Result<Receipt> {
        offer.task.validate()?;
        identifier(offer.dispatch)?;
        if let Some(answer) = &offer.answer {
            answer.validate()?;
        }
        ensure!(
            offer.task.sender == caller && offer.task.recipient == self.owner,
            "task identity mismatch"
        );
        ensure!(
            offer.task.created_at <= now().saturating_add(300),
            "sender clock is ahead"
        );
        self.update(|ledger| {
            if !ledger.incoming.contains_key(&offer.task.id) {
                ensure!((offer.task.expires_at > now() || offer.cancel) && !ledger.outgoing.contains_key(&offer.task.id), "task expired or ID already used");
                Self::room(ledger)?;
                let mut receipt = Receipt::queued(&offer.task, offer.dispatch);
                receipt.state = State::Available; receipt.change()?;
                ledger.incoming.insert(offer.task.id, Incoming { task: offer.task.clone(), history: vec![receipt], credential: credential(token), archived: false, runs: Vec::new() });
            }
            let item = ledger.incoming.get_mut(&offer.task.id).unwrap();
            ensure!(item.task == offer.task && item.credential == credential(token), "task ID already used with different content or grant");
            // A duplicate from an old attempt returns its own immutable receipt, never updates the new attempt.
            if let Some(index) = item.history.iter().position(|receipt| receipt.dispatch == offer.dispatch) {
                if offer.cancel && index + 1 == item.history.len() { item.history[index].cancel()?; }
                if !offer.cancel && index + 1 == item.history.len() && let Some(answer) = &offer.answer { item.history[index].reply(answer)?; }
                return Ok(item.history[index].clone());
            }
            ensure!((item.task.expires_at > now() || offer.cancel) && item.history.len() < MAX_ATTEMPTS && item.history.last().is_some_and(|receipt| receipt.state.retryable()), "previous attempt is not confirmed failed or cancelled, task expired, or attempt limit reached");
            let mut receipt = Receipt::queued(&item.task, offer.dispatch);
            receipt.state = State::Available; receipt.change()?;
            if offer.cancel { receipt.cancel()?; }
            item.history.push(receipt.clone()); item.archived = false;
            Ok(receipt)
        })
    }
    pub fn receipt(&self, caller: &str, token: &str, id: Uuid, dispatch: Uuid) -> Result<Receipt> {
        let ledger = self.read()?;
        let item = ledger
            .incoming
            .get(&id)
            .filter(|item| item.task.sender == caller && item.credential == credential(token))
            .context("task receipt unavailable")?;
        item.history
            .iter()
            .find(|receipt| receipt.dispatch == dispatch)
            .cloned()
            .context("task dispatch unavailable")
    }
    pub(super) fn view(&self, configuration: &Config) -> Result<Value> {
        let ledger = self.read()?;
        let incoming = ledger.incoming.values().filter(|item| !item.archived).map(|item| json!({"task":item.task,"history":item.history,"current":item.history.last(),"grant_active":authorized(configuration,item),"claim_expired":item.task.expires_at <= now(),"run":item.runs.iter().find(|run| item.history.last().is_some_and(|receipt| receipt.dispatch == run.dispatch)).map(|run| json!({"id":run.id,"session":run.session}))})).collect::<Vec<_>>();
        let outgoing = ledger.outgoing.values().filter(|item| !item.archived || item.task.collaboration.as_ref().is_some_and(|scope| ledger.collaborations.get(&scope.id).is_some_and(|run| !run.archived))).map(|item| {
            let bound = super::super::messages::binding(configuration, &item.task.recipient).ok().as_deref() == Some(&item.binding);
            json!({"task":item.task,"history":item.history,"current":item.history.last(),"pending_answer":item.pending_answer,"cancel_requested":item.cancel,"blocked":item.blocked || !bound,"error":if bound {item.error.clone()}else{Some("Connection or credentials changed. Check the original target before continuing.".into())},"synced_at":item.synced_at,"attempts":item.attempts,"dependencies":dependencies::view(&ledger,item)})
        }).collect::<Vec<_>>();
        Ok(
            json!({"owner":self.owner,"incoming":incoming,"outgoing":outgoing,"capacity":MAX_TASKS,"used":ledger.incoming.len()+ledger.outgoing.len(),"max_attempts":MAX_ATTEMPTS,"collaborations":collaborations::view(&ledger, configuration),"collaboration_capacity":collaborations::CAPACITY,"collaboration_used":ledger.collaborations.len()}),
        )
    }
    pub(super) fn apply(&self, configuration: &Config, command: Command) -> Result<()> {
        if matches!(command, Command::List {}) {
            return Ok(());
        }
        self.update(|ledger| {
            match command {
                Command::List {} => unreachable!(),
                Command::Collaboration { command } => collaborations::apply(ledger, configuration, command)?,
                Command::Run { .. } | Command::Automatic { .. } => anyhow::bail!("use the local task run entry"),
                Command::Create { id, dispatch, node, title, description, target_session, run_id, depends_on } => {
                    identifier(dispatch)?;
                    let dependencies = dependencies::canonical(depends_on)?;
                    let binding = super::super::messages::binding(configuration, &node)?;
                    let created_at = now();
                    // An uncertain retry may read back an existing task even after its run stopped.
                    let collaboration = if let Some(run_id) = run_id {
                        if let Some(existing) = ledger.outgoing.get(&id) {
                            ensure!(existing.task.collaboration.as_ref().map(|scope| scope.id) == Some(run_id), "task belongs to a different collaboration");
                            existing.task.collaboration.clone()
                        }
                        else { Some(collaborations::scope(ledger, run_id, &node, &binding)?) }
                    } else { None };
                    let task = Task { id, sender: self.owner.clone(), recipient: node, target_session, collaboration, title, description, created_at, expires_at: created_at + TTL };
                    task.validate()?;
                    if let Some(item) = ledger.outgoing.get(&id) {
                        ensure!(item.binding == binding && item.task.recipient == task.recipient && item.task.target_session == task.target_session && item.task.collaboration == task.collaboration && item.task.title == task.title && item.task.description == task.description && item.dependencies == dependencies && item.history.first().is_some_and(|receipt| receipt.dispatch == dispatch), "task ID already used with different content, prerequisites, dispatch or connection");
                        return Ok(());
                    }
                    ensure!(!ledger.incoming.contains_key(&id), "task ID already used");
                    Self::room(ledger)?;
                    let receipt = Receipt::queued(&task, dispatch);
                    ledger.outgoing.insert(id, Outgoing { task, history: vec![receipt], binding, cancel: false, archived: false, attempts: 0, next_attempt: 0, synced_at: None, blocked: false, error: None, pending_answer: None, dependencies, released: false });
                }
                Command::Claim { id, dispatch, session } => {
                    identifier(session)?;
                    let item = ledger.incoming.get_mut(&id).context("incoming task unavailable")?;
                    ensure!(authorized(configuration,item) && item.task.expires_at > now(), "task grant revoked/rotated or claim deadline expired");
                    let receipt = current(&mut item.history, dispatch)?;
                    if receipt.state == State::Running && receipt.session == Some(session) { return Ok(()); }
                    ensure!(receipt.state == State::Available, "task is not available; another session may own it");
                    if let Some(target) = item.task.target_session {
                        ensure!(session == target, "task is addressed to a different Agent session");
                        super::super::agent_sessions::Registry::new(&self.path,self.owner.clone()).require_active(session)?;
                    }
                    receipt.session = Some(session); receipt.state = State::Running; receipt.change()?;
                }
                Command::Progress { id, dispatch, session, text: progress } => {
                    text(&progress,4096)?;
                    let item = ledger.incoming.get_mut(&id).context("incoming task unavailable")?;
                    let receipt = current(&mut item.history, dispatch)?;
                    ensure!(receipt.session == Some(session) && matches!(receipt.state, State::Running | State::CancelRequested), "task is not running in this session");
                    if receipt.progress.as_ref() != Some(&progress) { receipt.progress = Some(progress); receipt.change()?; }
                }
                Command::Ask { id, dispatch, session, question, text: prompt } => {
                    identifier(question)?; text(&prompt,4096)?;
                    let item = ledger.incoming.get_mut(&id).context("incoming task unavailable")?;
                    ensure!(authorized(configuration,item), "task grant revoked or rotated");
                    let receipt = current(&mut item.history,dispatch)?;
                    ensure!(receipt.session == Some(session), "task belongs to another session");
                    if let Some(saved) = receipt.questions.iter().find(|saved| saved.id == question) {
                        ensure!(saved.text == prompt, "question ID already used with different text");
                        return Ok(());
                    }
                    ensure!(receipt.state == State::Running, "task is not running");
                    ensure!(receipt.questions.len() < MAX_QUESTIONS && receipt.questions.last().is_none_or(|saved| saved.answer.is_some()), "answer the pending question first; up to eight questions per dispatch");
                    receipt.questions.push(Question { id: question, text: prompt, answer: None });
                    receipt.change()?;
                }
                Command::Answer { id, dispatch, question, text: reply } => {
                    let answer = Answer { question, text: reply }; answer.validate()?;
                    let item = ledger.outgoing.get_mut(&id).context("only the task sender can answer")?;
                    ensure!(super::super::messages::binding(configuration,&item.task.recipient)? == item.binding, "connection or credentials changed");
                    let receipt = current(&mut item.history,dispatch)?;
                    let saved = receipt.questions.iter().find(|saved| saved.id == question).context("question unavailable in this dispatch; synchronize first")?;
                    if let Some(reply) = &saved.answer {
                        ensure!(reply == &answer.text, "question already has a different answer");
                        return Ok(());
                    }
                    ensure!(!item.cancel && receipt.state == State::Running, "task is no longer accepting answers");
                    if let Some(pending) = &item.pending_answer {
                        ensure!(pending == &answer, "a different answer is already queued; synchronize before continuing");
                        return Ok(());
                    }
                    item.pending_answer = Some(answer); item.next_attempt = 0; item.blocked = false; item.error = None;
                }
                Command::Finish { id, dispatch, session, outcome, result } => {
                    text(&result,8192)?;
                    let item = ledger.incoming.get_mut(&id).context("incoming task unavailable")?;
                    if let Some(run) = item.runs.iter().find(|run| run.dispatch == dispatch) {
                        let executions = super::super::executions::Store::new(&self.path, self.owner.clone());
                        if executions.records()?.iter().any(|record| record.id == run.id) {
                            ensure!(!executions.record(run.id)?.state.active(), "wait for the Agent process to stop before reporting the reviewed result");
                        }
                    }
                    let receipt = current(&mut item.history, dispatch)?;
                    let state = State::from(outcome);
                    ensure!(receipt.session == Some(session), "task belongs to another session");
                    if receipt.state.terminal() {
                        ensure!(receipt.state == state && receipt.result.as_ref() == Some(&result), "task already has a different final result");
                        return Ok(());
                    }
                    ensure!(matches!(receipt.state, State::Running | State::CancelRequested), "claim the task before reporting a result");
                    receipt.state = state; receipt.result = Some(result); receipt.change()?;
                }
                Command::Decline { id, dispatch, reason } => {
                    text(&reason,8192)?;
                    let item = ledger.incoming.get_mut(&id).context("incoming task unavailable")?;
                    let receipt = current(&mut item.history, dispatch)?;
                    if receipt.state == State::Failed && receipt.session.is_none() && receipt.result.as_ref() == Some(&reason) { return Ok(()); }
                    ensure!(receipt.state == State::Available, "only an unclaimed task can be declined");
                    receipt.state = State::Failed; receipt.result = Some(reason); receipt.change()?;
                }
                Command::Cancel { id, dispatch } => {
                    if let Some(item) = ledger.outgoing.get_mut(&id) {
                        let sent = dependencies::ready(item);
                        let receipt = current(&mut item.history, dispatch)?;
                        if !sent && receipt.state == State::Cancelled && item.cancel { return Ok(()); }
                        ensure!(!receipt.state.terminal(), "task already finished");
                        if sent { ensure!(super::super::messages::binding(configuration, &item.task.recipient)? == item.binding, "connection or credentials changed"); }
                        item.cancel = true; item.pending_answer = None; item.next_attempt = 0; item.blocked = false; item.error = None;
                    } else {
                        let item = ledger.incoming.get_mut(&id).context("task unavailable")?;
                        current(&mut item.history, dispatch)?.cancel()?;
                    }
                }
                Command::Retry { id, dispatch, new_dispatch } => {
                    identifier(new_dispatch)?;
                    if let Some(item) = ledger.outgoing.get(&id) && let Some(scope) = &item.task.collaboration {
                        collaborations::scope(ledger, scope.id, &item.task.recipient, &item.binding)?;
                    }
                    let item = ledger.outgoing.get_mut(&id).context("outgoing task unavailable")?;
                    ensure!(super::super::messages::binding(configuration, &item.task.recipient)? == item.binding, "connection or credentials changed");
                    // Retrying an uncertain retry response uses the same pair of attempt IDs.
                    if item.history.len() >= 2 && item.history.last().unwrap().dispatch == new_dispatch && item.history[item.history.len()-2].dispatch == dispatch { return Ok(()); }
                    ensure!(item.task.expires_at > now() && item.history.len() < MAX_ATTEMPTS && !item.history.iter().any(|receipt| receipt.dispatch == new_dispatch), "task expired, attempt limit reached, or dispatch ID already used");
                    ensure!(current(&mut item.history, dispatch)?.state.retryable(), "retry requires a confirmed failure or cancellation; an outage is not proof execution stopped");
                    item.history.push(Receipt::queued(&item.task,new_dispatch));
                    item.cancel = false; item.pending_answer = None; item.archived = false; item.blocked = false; item.error = None; item.next_attempt = 0; item.synced_at = None;
                }
                Command::Sync { id, dispatch } => {
                    let item = ledger.outgoing.get_mut(&id).context("outgoing task unavailable")?;
                    current(&mut item.history,dispatch)?;
                    ensure!(super::super::messages::binding(configuration, &item.task.recipient)? == item.binding, "connection or credentials changed");
                    item.blocked = false; item.error = None; item.next_attempt = 0;
                }
                Command::Archive { id, dispatch } => {
                    if let Some(item) = ledger.outgoing.get_mut(&id) {
                        ensure!(current(&mut item.history,dispatch)?.state.terminal(), "only finished tasks can be archived"); item.archived = true;
                    } else {
                        let item = ledger.incoming.get_mut(&id).context("task unavailable")?;
                        ensure!(current(&mut item.history,dispatch)?.state.terminal(), "only finished tasks can be archived"); item.archived = true;
                    }
                }
            }
            Ok(())
        })
    }
}
