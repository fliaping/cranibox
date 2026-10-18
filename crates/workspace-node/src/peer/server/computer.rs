//! One short, revocable input lease, sharing the viewer's controller slot.
use super::*;
use agent_workspace_runtime::peers::computer::{self, OwnerCommand, Provider, Request};
use tokio::sync::OwnedSemaphorePermit;
use uuid::Uuid;

pub(super) struct Lease {
    id: Uuid,
    caller: Caller,
    provider: Provider,
    epoch: u64,
    expires: Instant,
    connection: CancellationToken,
    _permit: OwnedSemaphorePermit,
}
impl Lease {
    fn valid(&self, app: &App, configuration: &Config) -> bool {
        !app.stop.is_cancelled()
            && !self.connection.is_cancelled()
            && self.expires > Instant::now()
            && !configuration.computer_paused
            && configuration.node_id == app.node
            && configuration.computer_epoch == self.epoch
            && configuration.computer.as_ref() == Some(&self.provider)
            && configuration
                .grants
                .get(&self.caller.node)
                .is_some_and(|g| {
                    g.permissions.computer && config::matches(&g.token, &self.caller.grant.token)
                })
    }
    fn owned(&self, caller: &Caller, id: Uuid) -> bool {
        self.id == id
            && self.caller.node == caller.node
            && config::matches(&self.caller.grant.token, &caller.grant.token)
    }
}
pub(super) async fn prune(app: &App, configuration: Option<&Config>) {
    let mut lease = app.computer.lock().await;
    if lease
        .as_ref()
        .is_some_and(|l| configuration.is_none_or(|c| !l.valid(app, c)))
    {
        *lease = None;
    }
}
fn access(app: &App, caller: &Caller) -> Result<(Config, Provider), Failure> {
    let configuration = Config::read(&app.path).map_err(|_| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "peer configuration unavailable",
        )
    })?;
    if app.stop.is_cancelled()
        || app.connections.is_cancelled()
        || configuration.node_id != app.node
    {
        return Err(failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "node listener stopped",
        ));
    }
    if !configuration
        .grants
        .get(&caller.node)
        .is_some_and(|g| g.permissions.computer && config::matches(&g.token, &caller.grant.token))
    {
        return Err(failure(
            StatusCode::FORBIDDEN,
            "Computer Use capability not granted by target node",
        ));
    }
    let provider = configuration.computer.clone().ok_or_else(|| {
        failure(
            StatusCode::CONFLICT,
            "Computer Use sharing is disabled on target",
        )
    })?;
    Ok((configuration, provider))
}
async fn call(provider: &Provider, payload: Value, session: &str) -> Result<Value, Failure> {
    computer::bridge(provider, payload, session)
        .await
        .map_err(|e| failure(StatusCode::BAD_GATEWAY, &e.to_string()))
}
pub(super) async fn owner(app: &App, command: OwnerCommand) -> Result<Value> {
    if matches!(command, OwnerCommand::EmergencyStop {}) {
        *app.computer.lock().await = None;
    }
    computer::owner(&app.path, command).await
}
pub(super) async fn manage(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    Json(request): Json<Request>,
) -> Result<Json<Value>, Failure> {
    check_target(&app, &node)?;
    request
        .validate()
        .map_err(|e| failure(StatusCode::BAD_REQUEST, &e.to_string()))?;
    let (configuration, _) = access(&app, &caller)?;
    prune(&app, Some(&configuration)).await;
    let session = format!("peer-{}", caller.node);
    // Emergency stop bypasses the action gate, so a slow screenshot cannot delay it.
    if matches!(request, Request::EmergencyStop {}) {
        *app.computer.lock().await = None;
        let result = computer::owner(&app.path, OwnerCommand::EmergencyStop {})
            .await
            .map_err(|e| failure(StatusCode::SERVICE_UNAVAILABLE, &e.to_string()))?;
        return Ok(Json(json!({"node":node,"result":result})));
    }
    // Never queue input behind another provider call. Each action gets a fresh authorization check.
    let _gate = app.computer_actions.try_lock().map_err(|_| {
        failure(
            StatusCode::CONFLICT,
            "Computer Use action in progress; inspect status before submitting another action",
        )
    })?;
    let (configuration, provider) = access(&app, &caller)?;
    prune(&app, Some(&configuration)).await;
    let result = match request {
        Request::Status {} => {
            let mut status = call(&provider, json!({"action":"status"}), &session).await?;
            if configuration.computer_paused {
                status["paused"] = json!(true);
                status["state"] = json!("paused");
            }
            let lease = app.computer.lock().await;
            status["control_busy"] = json!(app.desktops.available_permits() == 0);
            status["lease_expires_in_secs"] = json!(lease.as_ref().map(|l| {
                l.expires
                    .saturating_duration_since(Instant::now())
                    .as_secs()
            }));
            status
        }
        Request::Acquire { ttl_secs } => {
            if configuration.computer_paused {
                return Err(failure(
                    StatusCode::CONFLICT,
                    "Computer Use is paused; resume on the target device",
                ));
            }
            let status = call(&provider, json!({"action":"status"}), &session).await?;
            if status["paused"] != false || status["backend_available"] != true {
                return Err(failure(
                    StatusCode::CONFLICT,
                    "Computer Use provider is paused or unavailable",
                ));
            }
            // A grant or provider may change while status was being probed.
            let (fresh, current) = access(&app, &caller)?;
            if fresh.computer_paused
                || fresh.computer_epoch != configuration.computer_epoch
                || current != provider
            {
                return Err(failure(
                    StatusCode::CONFLICT,
                    "Computer Use provider changed; inspect status",
                ));
            }
            let permit =
                app.desktops.clone().try_acquire_owned().map_err(|_| {
                    failure(StatusCode::CONFLICT, "desktop already has a controller")
                })?;
            let id = Uuid::new_v4();
            *app.computer.lock().await = Some(Lease {
                id,
                caller,
                provider,
                epoch: configuration.computer_epoch,
                expires: Instant::now() + Duration::from_secs(ttl_secs),
                connection: app.connections.clone(),
                _permit: permit,
            });
            json!({"lease":id,"expires_in_secs":ttl_secs})
        }
        Request::Renew {
            lease: id,
            ttl_secs,
        } => {
            let mut lease = app.computer.lock().await;
            let lease = lease
                .as_mut()
                .filter(|l| l.valid(&app, &configuration) && l.owned(&caller, id))
                .ok_or_else(|| {
                    failure(
                        StatusCode::CONFLICT,
                        "input lease expired, revoked or belongs to another caller",
                    )
                })?;
            lease.expires = Instant::now() + Duration::from_secs(ttl_secs);
            json!({"lease":id,"expires_in_secs":ttl_secs})
        }
        Request::Release { lease: id } => {
            let mut lease = app.computer.lock().await;
            if !lease.as_ref().is_some_and(|l| l.owned(&caller, id)) {
                return Err(failure(
                    StatusCode::CONFLICT,
                    "input lease is not owned by caller",
                ));
            }
            *lease = None;
            json!({"released":true})
        }
        Request::Action { lease: id, action } => {
            if action.mutates() {
                let lease = app.computer.lock().await;
                if !lease.as_ref().is_some_and(|l| {
                    l.valid(&app, &configuration) && id.is_some_and(|id| l.owned(&caller, id))
                }) {
                    return Err(failure(
                        StatusCode::CONFLICT,
                        "a current caller-owned input lease is required",
                    ));
                }
            }
            call(
                &provider,
                action
                    .payload()
                    .map_err(|e| failure(StatusCode::BAD_REQUEST, &e.to_string()))?,
                &session,
            )
            .await?
        }
        Request::EmergencyStop {} => unreachable!(),
    };
    Ok(Json(json!({"node":node,"result":result})))
}
