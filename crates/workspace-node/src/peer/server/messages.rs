use super::*;
use agent_workspace_runtime::peers::messages::{Envelope, Store};
use uuid::Uuid;

async fn access(
    app: App,
    caller: Caller,
    message: Option<Envelope>,
    id: Option<Uuid>,
) -> Result<Json<Value>, Failure> {
    tokio::task::spawn_blocking(move || {
        let _lock = config::lock_credentials(&app.path).map_err(|_|failure(StatusCode::SERVICE_UNAVAILABLE, "message configuration unavailable"))?;
        let current = Config::read(&app.path).map_err(|_|failure(StatusCode::SERVICE_UNAVAILABLE, "message configuration unavailable"))?;
        if current.node_id != app.node || !current.grants.get(&caller.node).is_some_and(|grant| grant.permissions.messages && config::matches(&grant.token, &caller.grant.token)) {
            return Err(failure(StatusCode::FORBIDDEN, "device messages capability not granted"));
        }
        let store = Store::new(&app.path, app.node);
        let result = match message {
            Some(message) => store.receive(&caller.node, message),
            None => store.received(&caller.node, id.expect("receipt route supplies ID")),
        };
        result.map(Json).map_err(|_| failure(StatusCode::BAD_REQUEST, "Message or receipt unavailable. Check its identity, content, lifetime and mailbox capacity."))
    }).await.map_err(|_|failure(StatusCode::SERVICE_UNAVAILABLE, "device mailbox unavailable"))?
}
pub(super) async fn receive(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    Json(message): Json<Envelope>,
) -> Result<Json<Value>, Failure> {
    check_target(&app, &node)?;
    access(app, caller, Some(message), None).await
}
pub(super) async fn receipt(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path((node, id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, Failure> {
    check_target(&app, &node)?;
    access(app, caller, None, Some(id)).await
}
