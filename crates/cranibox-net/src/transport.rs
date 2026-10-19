//! Client side of ALPN `cranibox/peer-http/1`: the C2 [`PeerTransport`] over iroh.
use crate::{Lane, node::IrohNode, path};
use cranibox_contracts::{
    NodeId,
    net::{BoxBody, PathInfo, PeerTransport, TransportError, TransportKind},
};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use iroh::EndpointAddr;
use std::{sync::Arc, time::Duration};

/// Resolves a NodeId to the peer's iroh address (from routes, the roster or a ticket).
pub type Lookup = Arc<dyn Fn(&str) -> Option<EndpointAddr> + Send + Sync>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone)]
pub struct IrohTransport {
    node: IrohNode,
    lookup: Lookup,
}
impl IrohTransport {
    pub fn new(node: IrohNode, lookup: Lookup) -> Self {
        Self { node, lookup }
    }
    pub fn node(&self) -> &IrohNode {
        &self.node
    }
}

/// hyper body adapter for the contract's streaming [`BoxBody`].
struct StreamBody(
    std::pin::Pin<
        Box<
            dyn futures_util::Stream<
                    Item = Result<bytes::Bytes, cranibox_contracts::net::BodyError>,
                > + Send,
        >,
    >,
);
impl hyper::body::Body for StreamBody {
    type Data = bytes::Bytes;
    type Error = cranibox_contracts::net::BodyError;
    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        self.0
            .poll_next_unpin(cx)
            .map(|chunk| chunk.map(|r| r.map(hyper::body::Frame::data)))
    }
}

/// Send one HTTP/1.1 request on a fresh bidirectional stream of `conn`.
pub async fn send(
    conn: &iroh::endpoint::Connection,
    request: http::Request<BoxBody>,
) -> Result<http::Response<BoxBody>, TransportError> {
    let (send, recv) = conn
        .open_bi()
        .await
        .map_err(|e| TransportError::Other(format!("open stream: {e}")))?;
    let io = hyper_util::rt::TokioIo::new(tokio::io::join(recv, send));
    let (mut sender, connection) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|_| TransportError::Handshake)?;
    tokio::spawn(async move {
        let _ = connection.with_upgrades().await;
    });
    let (mut parts, body) = request.into_parts();
    if !parts.headers.contains_key(http::header::HOST) {
        parts
            .headers
            .insert(http::header::HOST, http::HeaderValue::from_static("peer"));
    }
    let request = http::Request::from_parts(parts, StreamBody(body.into_stream()));
    let response = sender
        .send_request(request)
        .await
        .map_err(|e| TransportError::Other(format!("peer request: {e}")))?;
    let (parts, body) = response.into_parts();
    let stream = body
        .into_data_stream()
        .map(|chunk| chunk.map_err(|e| Box::new(e) as cranibox_contracts::net::BodyError));
    Ok(http::Response::from_parts(parts, BoxBody::new(stream)))
}

#[async_trait::async_trait]
impl PeerTransport for IrohTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Iroh
    }
    async fn request(
        &self,
        peer: &NodeId,
        req: http::Request<BoxBody>,
    ) -> Result<http::Response<BoxBody>, TransportError> {
        let addr = (self.lookup)(peer).ok_or(TransportError::Unreachable)?;
        let conn = tokio::time::timeout(CONNECT_TIMEOUT, self.node.peer_connection(&addr))
            .await
            .map_err(|_| TransportError::Timeout)?
            .map_err(|e| {
                if e.to_string().contains("does not match") {
                    TransportError::Handshake
                } else {
                    TransportError::Unreachable
                }
            })?;
        let response = send(&conn, req).await;
        if response.is_err() && conn.close_reason().is_some() {
            // Never resend: the request may have reached the peer. Only forget the dead
            // connection so the next request dials again.
            self.node.pool.lock().await.remove(&(addr.id, Lane::Peer));
        }
        // HTTP statuses (including 401/403) pass through with their error bodies.
        response
    }
    async fn path(&self, peer: &NodeId) -> Option<PathInfo> {
        let addr = (self.lookup)(peer)?;
        let pool = self.node.pool.lock().await;
        pool.get(&(addr.id, Lane::Peer))
            .filter(|p| p.conn.close_reason().is_none())
            .map(|p| path::info(&p.conn))
    }
}
