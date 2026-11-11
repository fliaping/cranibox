//! One-hop forwarding through another device (ALPN [`crate::ALPN_PEER_HOP`]).
//!
//! When device A reaches B only through a slow relay but both have a fast direct path to
//! C, A's loopback forward can go A → C → B. Each stream starts with a header naming the
//! final target; C asks its policy whether A may be forwarded there, dials B on the
//! forward lane and splices the two streams. C never reads the traffic: callers already
//! authenticate and encrypt end to end inside forwards (Noise session with B), exactly
//! as through a relay. One hop only: C reaches B directly, never through another hop.
use crate::{allow::Allowlist, node::IrohNode};
use anyhow::{Context, Result, bail, ensure};
use iroh::{EndpointAddr, EndpointId, endpoint::Connection};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

const MAGIC: &[u8; 4] = b"HOP1";
const OK: u8 = 0;
const REFUSED: u8 = 1;
const UNREACHABLE: u8 = 2;
/// Concurrent streams one requester may forward through this device.
const PER_CONNECTION: usize = 64;

/// Whether `requester` may be forwarded to `target`, and where `target` is.
pub type Policy = Arc<dyn Fn(&EndpointId, &EndpointId) -> Option<EndpointAddr> + Send + Sync>;

/// Open a stream to `target` through `via`. Returns once `via` has reached `target`.
pub async fn open(
    node: &IrohNode,
    via: &EndpointAddr,
    target: EndpointId,
) -> Result<(iroh::endpoint::SendStream, iroh::endpoint::RecvStream)> {
    let conn = node.lane_connection(via, crate::Lane::Hop).await?;
    let (mut send, mut recv) = conn.open_bi().await?;
    let mut header = MAGIC.to_vec();
    header.extend_from_slice(target.as_bytes());
    send.write_all(&header).await?;
    // The forwarding device may need a full dial (up to 15 s) to reach the target.
    let status = tokio::time::timeout(Duration::from_secs(20), recv.read_u8())
        .await
        .context("forwarding device did not answer")??;
    match status {
        OK => Ok((send, recv)),
        REFUSED => bail!("{} refused to forward", via.id.fmt_short()),
        _ => bail!("{} cannot reach {}", via.id.fmt_short(), target.fmt_short()),
    }
}

/// Serve one accepted hop connection until it closes. `slots` bounds the streams this
/// device forwards for everyone together.
pub async fn serve_connection(
    conn: Connection,
    allow: &Allowlist,
    node: IrohNode,
    policy: Policy,
    slots: Arc<tokio::sync::Semaphore>,
    stop: CancellationToken,
) -> Result<()> {
    let requester = conn.remote_id();
    if !allow.contains(&requester) {
        conn.close(crate::close::NOT_ALLOWED.into(), b"not paired");
        bail!(
            "refused hop from unpaired EndpointId {}",
            requester.fmt_short()
        );
    }
    let own = Arc::new(tokio::sync::Semaphore::new(PER_CONNECTION));
    let mut streams = tokio::task::JoinSet::new();
    loop {
        let accepted = tokio::select! {
            _ = stop.cancelled() => break,
            accepted = conn.accept_bi() => accepted,
        };
        let Ok((send, recv)) = accepted else { break };
        if !allow.contains(&requester) {
            conn.close(crate::close::NOT_ALLOWED.into(), b"unpaired");
            break;
        }
        let (Ok(mine), Ok(shared)) = (
            own.clone().try_acquire_owned(),
            slots.clone().try_acquire_owned(),
        ) else {
            continue; // Dropping the streams resets them; the requester falls back.
        };
        let (node, policy, stop) = (node.clone(), policy.clone(), stop.clone());
        streams.spawn(async move {
            let _permits = (mine, shared);
            if let Err(error) = forward(node, policy, requester, send, recv, stop).await {
                tracing::debug!("hop for {} ended: {error:#}", requester.fmt_short());
            }
        });
        while streams.try_join_next().is_some() {}
    }
    streams.abort_all();
    Ok(())
}

async fn forward(
    node: IrohNode,
    policy: Policy,
    requester: EndpointId,
    mut send: iroh::endpoint::SendStream,
    mut recv: iroh::endpoint::RecvStream,
    stop: CancellationToken,
) -> Result<()> {
    let mut header = [0u8; 36];
    tokio::time::timeout(Duration::from_secs(10), recv.read_exact(&mut header))
        .await
        .context("hop header timed out")??;
    ensure!(&header[..4] == MAGIC, "not a hop stream");
    let target = EndpointId::from_bytes(header[4..].try_into().expect("32 bytes"))
        .context("invalid hop target")?;
    let Some(addr) = (target != requester)
        .then(|| policy(&requester, &target))
        .flatten()
    else {
        send.write_u8(REFUSED).await?;
        send.finish()?;
        bail!("not allowed to forward to {}", target.fmt_short());
    };
    let onward = async {
        let conn = node.lane_connection(&addr, crate::Lane::Forward).await?;
        anyhow::Ok(conn.open_bi().await?)
    };
    let (onward_send, onward_recv) = match onward.await {
        Ok(streams) => streams,
        Err(error) => {
            send.write_u8(UNREACHABLE).await?;
            send.finish()?;
            return Err(error);
        }
    };
    send.write_u8(OK).await?;
    let mut back = tokio::io::join(recv, send);
    let mut onward = tokio::io::join(onward_recv, onward_send);
    tokio::select! {
        _ = stop.cancelled() => {}
        copied = tokio::io::copy_bidirectional(&mut back, &mut onward) => { copied?; }
    }
    Ok(())
}
