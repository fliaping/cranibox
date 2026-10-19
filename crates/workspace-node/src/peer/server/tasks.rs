use super::*;
use agent_workspace_runtime::peers::tasks::{Offer, Store};
use uuid::Uuid;

pub(super) async fn receive(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    Json(offer): Json<Offer>,
) -> Result<Json<Value>, Failure> {
    check_target(&app, &node)?;
    access(app, caller, Some(offer), None).await
}
pub(super) async fn receipt(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path((node, id, dispatch)): Path<(String, Uuid, Uuid)>,
) -> Result<Json<Value>, Failure> {
    check_target(&app, &node)?;
    access(app, caller, None, Some((id, dispatch))).await
}
async fn access(
    app: App,
    caller: Caller,
    offer: Option<Offer>,
    ids: Option<(Uuid, Uuid)>,
) -> Result<Json<Value>, Failure> {
    tokio::task::spawn_blocking(move || {
        let _lock = config::lock_credentials(&app.path).map_err(|_|failure(StatusCode::SERVICE_UNAVAILABLE,"task configuration unavailable"))?;
        let current = Config::read(&app.path).map_err(|_|failure(StatusCode::SERVICE_UNAVAILABLE,"task configuration unavailable"))?;
        if current.node_id != app.node || !current.grants.get(&caller.node).is_some_and(|grant|grant.permissions.messages && grant.same_credential(&caller.grant)) {
            return Err(failure(StatusCode::FORBIDDEN,"device messages and task access not granted"));
        }
        let store = Store::new(&app.path,app.node);
        let result = if let Some(offer) = offer { store.receive(&caller.node,&caller.grant.credential,offer) } else { let (id,dispatch) = ids.expect("receipt route supplies IDs"); store.receipt(&caller.node,&caller.grant.credential,id,dispatch) };
        result.and_then(|receipt| Ok(serde_json::to_value(receipt)?)).map(Json).map_err(|_|failure(StatusCode::BAD_REQUEST,"Task or receipt unavailable. Check its identity, content, grant, lifetime and dispatch state."))
    }).await.map_err(|_|failure(StatusCode::SERVICE_UNAVAILABLE,"task ledger unavailable"))?
}
