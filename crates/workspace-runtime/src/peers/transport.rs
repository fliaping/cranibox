//! C2 `PeerTransport` over the existing direct routes (WS0, thin wrapper).
//!
//! One wrapper covers plain HTTP and Noise: [`Route::api`](super::config::Route::api)
//! already seals every call with Noise IK end-to-end frames once the target key
//! is pinned, and refuses plaintext for pinned targets. Behaviour is unchanged;
//! this only adapts the JSON request/response API to the transport contract.
//!
//! Limits (by design for Phase 0): JSON bodies up to 1 MiB, `GET` and `POST`
//! only, buffered responses. Streaming (SSE) over non-iroh transports is the
//! secure_ws adaptation owned by WS1 (ARCHITECTURE §4.3).
use super::Store;
use cranibox_contracts::{
    NodeId,
    async_trait::async_trait,
    http,
    net::{BoxBody, PathInfo, PathKind, PeerTransport, TransportError, TransportKind, routes},
};
use futures_util::StreamExt;

const MAX_BODY: usize = 1024 * 1024;

/// Existing HTTP/Noise routes from `peers.json` as a [`PeerTransport`].
#[derive(Clone)]
pub struct DirectTransport {
    store: Store,
}
impl DirectTransport {
    pub fn new(store: Store) -> Self {
        Self { store }
    }
}

async fn collect(body: BoxBody) -> Result<Vec<u8>, TransportError> {
    let mut stream = body.into_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| TransportError::Other(e.to_string()))?;
        if bytes.len() + chunk.len() > MAX_BODY {
            return Err(TransportError::Other("request body too large".into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn json_response(status: u16, value: &serde_json::Value) -> http::Response<BoxBody> {
    http::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(BoxBody::full(serde_json::to_vec(value).unwrap_or_default()))
        .expect("valid response parts")
}

#[async_trait]
impl PeerTransport for DirectTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Direct
    }
    async fn request(
        &self,
        peer: &NodeId,
        req: http::Request<BoxBody>,
    ) -> Result<http::Response<BoxBody>, TransportError> {
        let route = self
            .store
            .route(peer)
            .map_err(|_| TransportError::Unreachable)?;
        let api = route
            .api(peer)
            .map_err(|e| TransportError::Other(e.to_string()))?;
        let (parts, body) = req.into_parts();
        let path = parts
            .uri
            .path_and_query()
            .map_or_else(|| parts.uri.path(), |p| p.as_str());
        let rest = path
            .strip_prefix(routes::MESH_PREFIX)
            .ok_or_else(|| TransportError::Other("not a mesh path".into()))?;
        // The JSON client maps /api/v1 back onto /api/v1/mesh for direct targets.
        let api_path = format!("/api/v1{rest}");
        let bytes = collect(body).await?;
        let result = match parts.method {
            http::Method::GET => api.get(&api_path).await,
            http::Method::POST => {
                let body = if bytes.is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_slice(&bytes)
                        .map_err(|_| TransportError::Other("JSON bodies only".into()))?
                };
                api.post(&api_path, body).await
            }
            _ => return Err(TransportError::Other("method not supported".into())),
        };
        match result {
            Ok(value) => Ok(json_response(200, &value)),
            Err(error) => match crate::client::http_status(&error) {
                Some((401, _)) => Err(TransportError::Unauthorized),
                Some(_) if crate::client::unavailable(&error) => Err(TransportError::Unreachable),
                Some((status, message)) => Ok(json_response(
                    status,
                    &serde_json::json!({"error": message}),
                )),
                None if crate::client::timed_out(&error) => Err(TransportError::Timeout),
                None if crate::client::unavailable(&error) => Err(TransportError::Unreachable),
                None => Err(TransportError::Other(format!("{error:#}"))),
            },
        }
    }
    async fn path(&self, peer: &NodeId) -> Option<PathInfo> {
        self.store.route(peer).ok().map(|_| PathInfo {
            kind: PathKind::Direct,
            rtt_ms: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peers::config::{Config, Route};

    fn request(path: &str) -> http::Request<BoxBody> {
        http::Request::get(path).body(BoxBody::empty()).unwrap()
    }

    #[tokio::test]
    async fn unknown_peers_and_foreign_paths_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("peers.json");
        let token = root.path().join("route.token");
        std::fs::write(&token, "secret").unwrap();
        let peer = "node-0a1b2c3d4e5f60718293a4b5c6d7e8f9".to_string();
        // Port 9 (discard) on loopback is closed in test environments.
        Config::update(&path, |c| {
            c.routes.insert(
                peer.clone(),
                Route {
                    url: "http://127.0.0.1:9/".into(),
                    token_file: token.clone(),
                    allow_insecure: false,
                    local_forward_lock: None,
                    alternatives: vec![],
                    target_identity: None,
                    config_path: None,
                },
            );
            Ok(())
        })
        .unwrap();
        let transport = DirectTransport::new(Store::new(path));
        assert_eq!(transport.kind(), TransportKind::Direct);
        let missing = "node-3d4e5f60718293a4b5c6d7e8f90a1b2c".to_string();
        assert_eq!(
            transport
                .request(&missing, request("/api/v1/mesh/nodes"))
                .await
                .unwrap_err(),
            TransportError::Unreachable
        );
        assert!(transport.path(&missing).await.is_none());
        assert_eq!(transport.path(&peer).await.unwrap().kind, PathKind::Direct);
        assert!(matches!(
            transport.request(&peer, request("/api/v1/nodes")).await,
            Err(TransportError::Other(_))
        ));
        assert_eq!(
            transport
                .request(&peer, request("/api/v1/mesh/nodes"))
                .await
                .unwrap_err(),
            TransportError::Unreachable
        );
    }
}
