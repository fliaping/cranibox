use super::*;

impl Peers {
    /// Background retries retain the original MessageID and payload.
    pub async fn deliver_messages(&self) -> Result<()> {
        for _ in 0..4 {
            let peers = self.clone();
            let pending = tokio::task::spawn_blocking(move || {
                let _lock = config::lock_credentials(&peers.path)?;
                let configuration = Config::read(&peers.path)?;
                let store = Store::new(&peers.path, configuration.node_id.clone());
                if !store.path.try_exists()? { return Ok::<_, anyhow::Error>(None); }
                store.update(|mailbox| {
                    let item=mailbox.outgoing.values_mut().filter(|item| matches!(item.status.as_str(), "queued" | "received") && item.next_attempt <= now()).min_by_key(|item| item.next_attempt);
                    let Some(item)=item else { return Ok(None); };
                    if item.message.expires_at <= now() { if item.status == "queued" { item.status="expired".into(); } item.next_attempt=u64::MAX; return Ok(None); }
                    if binding(&configuration, &item.message.recipient).ok().as_deref() != Some(&item.binding) { item.status="blocked".into(); item.error=Some("Connection or credentials changed. Check the target before sending a new message.".into()); return Ok(None); }
                    let route=configuration.routes[&item.message.recipient].clone();
                    item.next_attempt=now()+30; item.attempts=item.attempts.saturating_add(1);
                    Ok(Some((store.clone(), item.clone(), route)))
                })
            }).await??;
            let Some((store, item, route)) = pending else {
                break;
            };
            let path = format!("/api/v1/nodes/{}/messages", item.message.recipient);
            let result = async {
                let api = route.api(&item.message.recipient)?;
                let receipt = if item.status == "received" {
                    api.get(&format!("{path}/{}", item.message.id)).await?
                } else {
                    api.post(&path, serde_json::to_value(&item.message)?)
                        .await?
                };
                ensure!(
                    receipt["id"] == item.message.id.to_string()
                        && receipt["sender"] == item.message.sender
                        && receipt["recipient"] == item.message.recipient
                        && receipt["sender_session"] == json!(item.message.sender_session)
                        && receipt["target_session"] == json!(item.message.target_session)
                        && matches!(
                            receipt["status"].as_str(),
                            Some("received" | "acknowledged")
                        ),
                    "invalid message receipt"
                );
                Ok::<_, anyhow::Error>(receipt["status"].as_str().unwrap().to_owned())
            }
            .await;
            let peers = self.clone();
            tokio::task::spawn_blocking(move || {
                let _lock=config::lock_credentials(&peers.path)?;
                let same=Config::read(&peers.path).ok().and_then(|configuration| binding(&configuration, &item.message.recipient).ok()).as_deref() == Some(&item.binding);
                store.update(|mailbox| {
                    let Some(current)=mailbox.outgoing.get_mut(&item.message.id).filter(|current| current.binding == item.binding && current.message == item.message) else { return Ok(()); };
                    if current.status == "acknowledged" { return Ok(()); }
                    if !same { current.status="blocked".into(); current.error=Some("Connection or credentials changed. Delivery result needs review.".into()); return Ok(()); }
                    match result {
                        Ok(status) => { current.status=status; current.error=None; current.next_attempt=now()+15; }
                        Err(error) => {
                            let limited=crate::client::rate_limited(&error);
                            let transient=limited || crate::client::unavailable(&error) || route.local_forward_lock.as_ref().is_some_and(|lease| super::super::local_forward::ensure_running(lease).is_err());
                            if !transient { current.status="blocked".into(); }
                            current.error=Some(if limited {"Target rate limit reached. The node will retry the same message later."} else if transient {"Device unavailable. The node will retry the same message."} else {"Target rejected this request or returned an incompatible receipt. Check access and node versions, then retry."}.into());
                            current.next_attempt=now()+5*(1u64 << current.attempts.min(6));
                        }
                    }
                    Ok(())
                })
            }).await??;
        }
        Ok(())
    }
}
