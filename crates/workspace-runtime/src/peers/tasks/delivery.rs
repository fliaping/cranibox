use super::*;
pub(super) fn check_pending_answer(pending: Option<&Answer>, receipt: &Receipt) -> Result<()> {
    if let Some(pending) = pending
        && let Some(saved) = receipt
            .questions
            .iter()
            .find(|question| question.id == pending.question)
            .and_then(|question| question.answer.as_ref())
    {
        ensure!(
            saved == &pending.text,
            "target confirmed a different task answer"
        );
    }
    Ok(())
}
pub(super) fn advance(previous: &Receipt, receipt: &Receipt) -> Result<()> {
    ensure!(
        receipt.dispatch == previous.dispatch && receipt.state != State::Queued,
        "unexpected task receipt"
    );
    ensure!(
        receipt.revision >= previous.revision,
        "task receipt revision regressed"
    );
    if receipt.revision == previous.revision {
        ensure!(
            receipt == previous,
            "task receipt changed without a revision"
        );
        return Ok(());
    }
    ensure!(
        !previous.state.terminal(),
        "a final task receipt cannot change"
    );
    if let Some(session) = previous.session {
        ensure!(
            receipt.session == Some(session),
            "task worker session changed"
        );
    }
    ensure!(
        receipt.questions.len() >= previous.questions.len(),
        "task questions disappeared"
    );
    for (before, after) in previous.questions.iter().zip(&receipt.questions) {
        ensure!(
            before.id == after.id
                && before.text == after.text
                && before
                    .answer
                    .as_ref()
                    .is_none_or(|answer| after.answer.as_ref() == Some(answer)),
            "task question or answer changed"
        );
    }
    ensure!(
        match previous.state {
            State::Queued | State::Available => true,
            State::Running => receipt.state != State::Available,
            State::CancelRequested => matches!(
                receipt.state,
                State::CancelRequested | State::Succeeded | State::Failed | State::Cancelled
            ),
            _ => false,
        },
        "task state regressed"
    );
    Ok(())
}
impl Peers {
    /// Repeated offers are idempotent and also return current execution facts.
    pub async fn deliver_tasks(&self) -> Result<()> {
        for _ in 0..4 {
            let peers = self.clone();
            let pending = tokio::task::spawn_blocking(move || {
                let _lock = config::lock_credentials(&peers.path)?;
                let configuration = Config::read(&peers.path)?;
                let store = Store::new(&peers.path,configuration.node_id.clone());
                if !store.path.try_exists()? { return Ok::<_,anyhow::Error>(None); }
                store.update(|ledger| {
                    dependencies::release_ready(ledger, &configuration);
                    let item = ledger.outgoing.values_mut().filter(|item| dependencies::ready(item) && !item.archived && !item.blocked && item.next_attempt <= now() && item.history.last().is_some_and(|receipt| !receipt.state.terminal())).min_by_key(|item|item.next_attempt);
                    let Some(item) = item else { return Ok(None); };
                    if super::super::messages::binding(&configuration,&item.task.recipient).ok().as_deref() != Some(&item.binding) {
                        item.blocked = true; item.error = Some("Connection or credentials changed. Delivery is blocked; the task may still be running on the original target.".into()); return Ok(None);
                    }
                    let route = configuration.routes[&item.task.recipient].clone();
                    item.next_attempt = now()+30; item.attempts = item.attempts.saturating_add(1);
                    Ok(Some((store.clone(),item.clone(),route)))
                })
            }).await??;
            let Some((store, item, route)) = pending else {
                break;
            };
            let dispatch = item.history.last().unwrap().dispatch;
            let offer = Offer {
                task: item.task.clone(),
                dispatch,
                cancel: item.cancel,
                answer: item.pending_answer.clone(),
            };
            let result = async {
                let value = route
                    .api(&item.task.recipient)?
                    .post(
                        &format!("/api/v1/nodes/{}/tasks", item.task.recipient),
                        serde_json::to_value(offer)?,
                    )
                    .await?;
                let receipt: Receipt = serde_json::from_value(value)?;
                receipt.validate(&item.task)?;
                advance(item.history.last().unwrap(), &receipt)?;
                Ok::<_, anyhow::Error>(receipt)
            }
            .await;
            let peers = self.clone();
            tokio::task::spawn_blocking(move || {
                let _lock = config::lock_credentials(&peers.path)?;
                let bound = Config::read(&peers.path).ok().and_then(|configuration| super::super::messages::binding(&configuration,&item.task.recipient).ok()).as_deref() == Some(&item.binding);
                store.update(|ledger| {
                    let Some(current) = ledger.outgoing.get_mut(&item.task.id).filter(|current|current.binding == item.binding && current.history.last().is_some_and(|receipt|receipt.dispatch == dispatch)) else { return Ok(()); };
                    if current.history.last().unwrap().state.terminal() { return Ok(()); }
                    if !bound { current.blocked = true; current.error = Some("Connection or credentials changed. The old attempt needs review.".into()); return Ok(()); }
                    let result = result.and_then(|receipt| {
                        if receipt.revision >= current.history.last().unwrap().revision {
                            advance(current.history.last().unwrap(), &receipt)?;
                            check_pending_answer(current.pending_answer.as_ref(), &receipt)?;
                        }
                        Ok(receipt)
                    });
                    match result {
                        Ok(receipt) => {
                            let previous = current.history.last_mut().unwrap();
                            // Another delivery process may have persisted a newer receipt first.
                            if receipt.revision >= previous.revision { *previous = receipt; }
                            if current.pending_answer.as_ref().is_some_and(|answer| {
                                let receipt = current.history.last().unwrap();
                                receipt.state != State::Running || receipt.questions.iter().any(|question| question.id == answer.question && question.answer.as_ref() == Some(&answer.text))
                            }) { current.pending_answer = None; }
                            current.synced_at = Some(now()); current.error = None; current.blocked = false;
                            current.next_attempt = if current.cancel != item.cancel || current.pending_answer != item.pending_answer {0} else {now()+3};
                        }
                        Err(error) => {
                            let unavailable = crate::client::unavailable(&error) || route.local_forward_lock.as_ref().is_some_and(|lease| super::super::local_forward::ensure_running(lease).is_err());
                            current.blocked = !unavailable;
                            current.error = Some(if unavailable {"Device unavailable. The last known state is retained; the same attempt will be synchronized again."}else{"The target rejected the task or returned an incompatible receipt. Check access and node versions, then synchronize the same attempt."}.into());
                            current.next_attempt = now()+5*(1u64 << current.attempts.min(6));
                        }
                    }
                    Ok(())
                })
            }).await??;
        }
        Ok(())
    }
}
