use super::*;
use agent_workspace_runtime::peers::browser::{self, Provider, Request};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

fn access(app: &App, caller: &Caller) -> Result<Config, Failure> {
    let c = Config::read(&app.path).map_err(|_| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "peer configuration unavailable",
        )
    })?;
    if app.stop.is_cancelled() || app.connections.is_cancelled() || c.node_id != app.node {
        return Err(failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "node listener stopped",
        ));
    }
    if !c
        .grants
        .get(&caller.node)
        .is_some_and(|g| g.permissions.browser && g.same_credential(&caller.grant))
    {
        return Err(failure(
            StatusCode::FORBIDDEN,
            "Browser Use capability not granted by target node",
        ));
    }
    Ok(c)
}
fn valid(app: &App, caller: &Caller, provider: &Provider, epoch: u64) -> bool {
    access(app, caller).is_ok_and(|c| {
        !c.browser_paused && c.browser_epoch == epoch && c.browser.as_ref() == Some(provider)
    })
}
pub(super) async fn manage(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    Json(command): Json<Request>,
) -> Result<Json<Value>, Failure> {
    check_target(&app, &node)?;
    let c = access(&app, &caller)?;
    let provider = c.browser.clone().ok_or_else(|| {
        failure(
            StatusCode::CONFLICT,
            "Browser sharing is disabled on target",
        )
    })?;
    let mut result = match command {
        Request::EmergencyStop {} => {
            browser::owner(&app.path, browser::OwnerCommand::EmergencyStop {}).await
        }
        Request::Status {} => browser::owner(&app.path, browser::OwnerCommand::Status {}).await,
    }
    .map_err(|_| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "Browser status unavailable",
        )
    })?;
    // Do not return provider metadata after authorization changes during a slow probe.
    let fresh = access(&app, &caller)?;
    if fresh.browser != Some(provider) {
        return Err(failure(
            StatusCode::CONFLICT,
            "Browser sharing changed; inspect status",
        ));
    }
    result["paused"] = json!(fresh.browser_paused);
    result["control_busy"] = json!(app.browsers.available_permits() == 0);
    Ok(Json(json!({"node":node,"result":result})))
}
pub(super) async fn upgrade(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    secret: Option<Extension<crate::peer::secure_ws::StreamSecret>>,
    ws: WebSocketUpgrade,
) -> Result<Response, Failure> {
    check_target(&app, &node)?;
    let c = access(&app, &caller)?;
    let provider = c.browser.ok_or_else(|| {
        failure(
            StatusCode::CONFLICT,
            "Browser sharing is disabled on target",
        )
    })?;
    if c.browser_paused {
        return Err(failure(
            StatusCode::CONFLICT,
            "Browser sharing is paused; resume on target device",
        ));
    }
    let permit = app.browsers.clone().try_acquire_owned().map_err(|_| {
        failure(
            StatusCode::CONFLICT,
            "Browser already has a remote controller",
        )
    })?;
    let (_, endpoint) = provider.discover().await.map_err(|_| {
        failure(
            StatusCode::BAD_GATEWAY,
            "Configured browser CDP is unavailable",
        )
    })?;
    let upstream = browser::connect(
        endpoint
            .into_client_request()
            .map_err(|_| failure(StatusCode::BAD_GATEWAY, "Invalid CDP endpoint"))?,
        None,
    )
    .await
    .map_err(|_| {
        failure(
            StatusCode::BAD_GATEWAY,
            "Browser CDP connection unavailable",
        )
    })?;
    let check = move || valid(&app, &caller, &provider, c.browser_epoch);
    if !check() {
        return Err(failure(
            StatusCode::CONFLICT,
            "Browser access changed during connection",
        ));
    }
    Ok(ws
        .max_message_size(browser::MAX_MESSAGE + crate::peer::secure_ws::MARGIN)
        .max_frame_size(browser::MAX_MESSAGE + crate::peer::secure_ws::MARGIN)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            let socket = crate::peer::secure_ws::SecureSocket::new(
                socket,
                secret.as_ref().map(|Extension(s)| s),
            );
            let _ = crate::peer::browser_proxy::relay(socket, upstream, check).await;
        }))
}
