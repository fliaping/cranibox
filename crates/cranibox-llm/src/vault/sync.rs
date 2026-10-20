//! Transport-neutral anti-entropy. The transport must refuse unencrypted owner routes.
use super::*;
use cranibox_contracts::net::{BoxBody, PeerTransport, routes};
use futures_util::StreamExt;
async fn call<T: serde::de::DeserializeOwned>(
    transport: &dyn PeerTransport,
    node: &str,
    path: &str,
    body: impl Serialize,
) -> Result<T, StoreError> {
    let request = http::Request::post(routes::vault(node, path))
        .header("content-type", "application/json")
        .body(BoxBody::full(serde_json::to_vec(&body).map_err(other)?))
        .map_err(other)?;
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        transport.request(&node.to_owned(), request),
    )
    .await
    .map_err(other)?
    .map_err(other)?;
    if !response.status().is_success() {
        return Err(other("sync rejected"));
    }
    let mut body = response.into_body().into_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = tokio::time::timeout(std::time::Duration::from_secs(10), body.next())
        .await
        .map_err(other)?
    {
        let chunk = chunk.map_err(other)?;
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            return Err(other("sync too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(other)
}
pub async fn exchange(
    vault: &Vault,
    transport: &dyn PeerTransport,
    node: &str,
) -> Result<(), StoreError> {
    if node == vault.node() || !vault.roster.is_owner_node(node) {
        return Err(other("not an owner peer"));
    }
    // Equal-version records are included to reconcile concurrent writers deterministically.
    let local = vault.pull(PullReq::default())?;
    let incoming: PullResp = call(transport, node, routes::VAULT_PULL, PullReq::default()).await?;
    vault.push(Push {
        records: incoming.records,
    })?;
    let _: serde_json::Value = call(
        transport,
        node,
        routes::VAULT_PUSH,
        Push {
            records: local.records,
        },
    )
    .await?;
    Ok(())
}
pub async fn request_refresh(
    vault: &Vault,
    transport: &dyn PeerTransport,
    request: RefreshRequest,
) -> Result<RefreshResponse, StoreError> {
    let holder = vault.refresh_holder(&request.account_id, now())?;
    if holder == vault.node() {
        return Err(other("use local refresh coordinator"));
    }
    let response: RefreshResponse =
        call(transport, &holder, routes::VAULT_REFRESH_REQUEST, request).await?;
    vault.push(Push {
        records: vec![response.record.clone()],
    })?;
    Ok(response)
}
