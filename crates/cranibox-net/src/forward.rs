//! Loopback forwards for out-of-process clients (CLI, MCP, the gateway).
//!
//! Each paired peer gets a stable `127.0.0.1:<port>` listener. Every accepted TCP
//! connection is spliced onto a new QUIC stream of the pooled iroh connection
//! (ALPN [`crate::ALPN_PEER_FWD`]) to that peer, so the existing reqwest / WebSocket client code works unchanged.
//! The forward is a dumb pipe: callers still authenticate inside it with the
//! route's credential (bearer or Noise session), so other local users cannot
//! borrow this node's identity by connecting to the port.
use crate::node::IrohNode;
use anyhow::Result;
use iroh::EndpointAddr;
use std::net::{Ipv4Addr, SocketAddr};
use tokio::{net::TcpListener, sync::watch};
use tokio_util::sync::CancellationToken;

pub struct Forward {
    pub address: SocketAddr,
    target: watch::Sender<EndpointAddr>,
    stop: CancellationToken,
}
impl Forward {
    /// Bind `127.0.0.1:port` (0 = ephemeral) and forward to `target`.
    pub async fn bind(node: IrohNode, port: u16, target: EndpointAddr) -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
        let address = listener.local_addr()?;
        let (tx, rx) = watch::channel(target);
        let stop = CancellationToken::new();
        tokio::spawn(serve(node, listener, rx, stop.clone()));
        Ok(Self {
            address,
            target: tx,
            stop,
        })
    }
    /// New relay URL or direct addresses for the same peer.
    pub fn update(&self, target: EndpointAddr) {
        self.target.send_if_modified(|current| {
            let changed = *current != target;
            *current = target;
            changed
        });
    }
    pub fn target(&self) -> EndpointAddr {
        self.target.borrow().clone()
    }
}
impl Drop for Forward {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

async fn serve(
    node: IrohNode,
    listener: TcpListener,
    target: watch::Receiver<EndpointAddr>,
    stop: CancellationToken,
) {
    let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(64));
    loop {
        let accepted = tokio::select! {
            _ = stop.cancelled() => break,
            accepted = listener.accept() => accepted,
        };
        let Ok((mut tcp, peer)) = accepted else {
            continue;
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            continue;
        };
        let node = node.clone();
        let addr = target.borrow().clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let _ = tcp.set_nodelay(true);
            let result = async {
                let conn = node.lane_connection(&addr, crate::Lane::Forward).await?;
                let (send, recv) = conn.open_bi().await?;
                let mut quic = tokio::io::join(recv, send);
                tokio::select! {
                    _ = stop.cancelled() => {}
                    copied = tokio::io::copy_bidirectional(&mut tcp, &mut quic) => { copied?; }
                }
                anyhow::Ok(())
            }
            .await;
            if let Err(error) = result {
                tracing::debug!("iroh forward to {} ended: {error:#}", addr.id.fmt_short());
            }
        });
    }
}
