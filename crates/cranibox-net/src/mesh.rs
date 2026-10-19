//! Server side of ALPN `cranibox/peer-http/1`.
//!
//! Each QUIC bidirectional stream carries one HTTP/1.1 connection (Upgrade allowed,
//! for terminal / desktop / browser WebSockets). The peer is the TLS-authenticated
//! EndpointId; only allowlisted (paired) ids get past the handshake.
use crate::{allow::Allowlist, path};
use anyhow::Result;
use cranibox_contracts::net::{Authenticated, TransportKind};
use iroh::{EndpointId, endpoint::Connection};
use std::{net::SocketAddr, sync::Arc};
use tokio_util::sync::CancellationToken;

/// Maps an authenticated EndpointId to the caller's NodeId (pinned signing key).
pub type Resolve = Arc<dyn Fn(&EndpointId) -> Option<String> + Send + Sync>;

/// Where accepted peer-http streams go.
#[derive(Clone)]
pub enum MeshTarget {
    /// Serve the Phase 0 `mesh_router()` in process. Every request gets the C2
    /// [`Authenticated`] extension (`transport: Iroh`, `e2e: true`) and a
    /// [`PeerPath`] extension; requests from ids without a NodeId are refused.
    Router {
        router: axum::Router,
        resolve: Resolve,
    },
    /// Splice the stream to an existing local listener (the node's
    /// `NetworkEndpoint`) that already serves the mesh routes, including the
    /// Noise `/e2e/*` endpoints. Used until `mesh_router()` is exported.
    Tcp(SocketAddr),
}

/// Path of the QUIC connection a request arrived on.
#[derive(Clone, Copy, Debug)]
pub struct PeerPath(pub cranibox_contracts::net::PathInfo);

/// Serve one accepted peer-http connection until it closes.
pub async fn serve_connection(
    conn: Connection,
    allow: &Allowlist,
    target: MeshTarget,
    stop: CancellationToken,
) -> Result<()> {
    let peer = conn.remote_id();
    if !allow.contains(&peer) {
        conn.close(crate::close::NOT_ALLOWED.into(), b"not paired");
        anyhow::bail!(
            "refused peer-http from unpaired EndpointId {}",
            peer.fmt_short()
        );
    }
    let node = match &target {
        MeshTarget::Router { resolve, .. } => match resolve(&peer) {
            Some(node) => Some(node),
            None => {
                conn.close(crate::close::NOT_ALLOWED.into(), b"unknown key");
                anyhow::bail!("EndpointId {} matches no pinned key", peer.fmt_short());
            }
        },
        MeshTarget::Tcp(_) => None,
    };
    let mut streams = tokio::task::JoinSet::new();
    loop {
        let accepted = tokio::select! {
            _ = stop.cancelled() => break,
            accepted = conn.accept_bi() => accepted,
        };
        let Ok((send, recv)) = accepted else { break };
        // Re-check per stream: an unpaired peer loses access without reconnecting.
        if !allow.contains(&peer) {
            conn.close(crate::close::NOT_ALLOWED.into(), b"unpaired");
            break;
        }
        let target = target.clone();
        let node = node.clone();
        let info = path::info(&conn);
        streams.spawn(async move {
            let io = tokio::io::join(recv, send);
            let result = match target {
                MeshTarget::Tcp(address) => splice(io, address).await,
                MeshTarget::Router { router, .. } => {
                    let authenticated = Authenticated {
                        node_id: node.unwrap_or_default(),
                        endpoint_id: Some(*peer.as_bytes()),
                        transport: TransportKind::Iroh,
                        e2e: true,
                    };
                    serve_http(io, router, authenticated, PeerPath(info)).await
                }
            };
            if let Err(error) = result {
                tracing::debug!("peer-http stream ended: {error:#}");
            }
        });
        while streams.try_join_next().is_some() {}
    }
    streams.abort_all();
    Ok(())
}

async fn splice<S>(mut io: S, address: SocketAddr) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut tcp = tokio::net::TcpStream::connect(address).await?;
    tcp.set_nodelay(true)?;
    tokio::io::copy_bidirectional(&mut io, &mut tcp).await?;
    Ok(())
}

async fn serve_http<S>(
    io: S,
    router: axum::Router,
    authenticated: Authenticated,
    path: PeerPath,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    use tower_service::Service;
    let service =
        hyper::service::service_fn(move |mut request: http::Request<hyper::body::Incoming>| {
            // Never trust a client-supplied identity header; identity is the extension.
            request.extensions_mut().insert(authenticated.clone());
            request.extensions_mut().insert(path);
            let mut router = router.clone();
            async move { router.call(request.map(axum::body::Body::new)).await }
        });
    hyper::server::conn::http1::Builder::new()
        .serve_connection(hyper_util::rt::TokioIo::new(io), service)
        .with_upgrades()
        .await?;
    Ok(())
}
