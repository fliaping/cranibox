//! Stable loopback routes survive DHCP changes. Only live discovery supplies destinations.
use agent_workspace_runtime::peers::local_forward::Lease;
use anyhow::{Result, ensure};
use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Semaphore, watch},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Destination {
    pub socks: SocketAddr,
    pub target: SocketAddrV4,
}
pub(super) struct Forward {
    pub port: u16,
    target: watch::Sender<Option<Destination>>,
    stop: CancellationToken,
    task: Option<tokio::task::JoinHandle<()>>,
    lease: Option<Lease>,
}
impl Forward {
    pub async fn bind(port: u16, slots: Arc<Semaphore>, lease_path: &Path) -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
        let lease = Lease::acquire(lease_path)?;
        let port = listener.local_addr()?.port();
        let (target, destination) = watch::channel(None);
        let stop = CancellationToken::new();
        let task = tokio::spawn(serve(listener, destination, slots, stop.clone()));
        Ok(Self {
            port,
            target,
            stop,
            task: Some(task),
            lease: Some(lease),
        })
    }
    pub fn update(&self, target: Option<Destination>) {
        self.target.send_if_modified(|value| {
            if *value == target {
                false
            } else {
                *value = target;
                true
            }
        });
    }
    pub async fn shutdown(mut self) {
        self.lease = None;
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for Forward {
    fn drop(&mut self) {
        self.lease = None;
        self.stop.cancel();
    }
}
async fn serve(
    listener: TcpListener,
    destination: watch::Receiver<Option<Destination>>,
    slots: Arc<Semaphore>,
    stop: CancellationToken,
) {
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => break,
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
            incoming = listener.accept() => {
                let Ok((mut local, _)) = incoming else { break; };
                let Ok(permit) = slots.clone().try_acquire_owned() else { continue; };
                let mut changes = destination.clone();
                let Some(target) = *changes.borrow_and_update() else { continue; };
                tasks.spawn(async move {
                    let _permit = permit;
                    tokio::select! {
                        // An address change or stopped provider closes old transports. Shells stay target-owned.
                        _ = changes.changed() => {},
                        _ = async {
                            let mut remote = tokio::time::timeout(Duration::from_secs(5), socks(target)).await??;
                            tokio::io::copy_bidirectional(&mut local, &mut remote).await?;
                            Ok::<_, anyhow::Error>(())
                        } => {},
                    }
                });
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}
pub(super) async fn socks(destination: Destination) -> Result<TcpStream> {
    ensure!(
        destination.socks.ip().is_loopback(),
        "SOCKS endpoint must be local."
    );
    let mut stream = TcpStream::connect(destination.socks).await?;
    stream.write_all(&[5, 1, 0]).await?;
    let mut greeting = [0; 2];
    stream.read_exact(&mut greeting).await?;
    ensure!(greeting == [5, 0], "EasyTier SOCKS negotiation failed.");
    let mut request = vec![5, 1, 0, 1];
    request.extend_from_slice(&destination.target.ip().octets());
    request.extend_from_slice(&destination.target.port().to_be_bytes());
    stream.write_all(&request).await?;
    let mut reply = [0; 4];
    stream.read_exact(&mut reply).await?;
    ensure!(
        reply[0] == 5 && reply[1] == 0 && reply[2] == 0,
        "EasyTier target is unavailable."
    );
    let length = match reply[3] {
        1 => 4,
        4 => 16,
        3 => stream.read_u8().await? as usize,
        _ => anyhow::bail!("Invalid SOCKS reply."),
    };
    let mut address = vec![0; length + 2];
    stream.read_exact(&mut address).await?;
    Ok(stream)
}
