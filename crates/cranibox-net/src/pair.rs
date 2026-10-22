//! Link / QR pairing over ALPN [`crate::ALPN_PAIR`].
//!
//! 1. The inviter creates a one-time [`PairTicket`] (15 min, at most 16 open).
//! 2. The joiner dials the inviter's EndpointId from the ticket (TLS pins it) and
//!    proves the ticket secret; both sides exchange their public identities. The
//!    joiner's identity must carry the EndpointId it authenticated with.
//! 3. Both sides show the existing SAS pairing code and send their local decision.
//!    Only two matching decisions allow credentials to be exchanged and committed
//!    (grant + route, see [`PairHost::apply`]); rejection or timeout changes nothing.
use crate::{IrohNode, addr, relay::Relay, ticket};
use anyhow::{Context, Result, bail, ensure};
use cranibox_contracts::net::{
    IrohAddr, PAIR_TICKET_TTL_SECS, PAIR_TICKET_VERSION, PairPurpose, PairTicket,
};
use iroh::{EndpointId, endpoint::Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{oneshot, watch};

pub const MAX_OPEN_TICKETS: usize = 16;
const MAX_FRAME: u32 = 32 * 1024;
const DECISION_TIMEOUT: Duration = Duration::from_secs(PAIR_TICKET_TTL_SECS);
const STEP_TIMEOUT: Duration = Duration::from_secs(30);

/// Which side of a pairing this node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Inviter,
    Joiner,
}

/// A confirmed pairing to persist.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub side: Side,
    pub purpose: PairPurpose,
    /// The peer's public identity (`PublicIdentity` JSON), already checked by [`PairHost::verify`].
    pub peer_identity: Value,
    pub peer_node_id: String,
    pub peer_addr: IrohAddr,
    /// Bearer secret of the grant this node issues to the peer, if any.
    pub issued: Option<String>,
    /// Bearer secret the peer issued to this node, if any (store as the route token).
    pub received: Option<String>,
    /// Joiner of an owner-device pairing: the inviter's identity to join (roster and
    /// sealed owner key), see [`PairHost::bundle`].
    pub identity: Option<Value>,
}

/// Feature bit: the device exchanges owner identities in owner-device pairings.
pub const FEATURE_IDENTITY: u8 = 1;

/// Node-side hooks: identities, the SAS code and persisting the result.
#[async_trait::async_trait]
pub trait PairHost: Send + Sync + 'static {
    /// This node's public identity (`PublicIdentity` JSON).
    fn identity(&self) -> Value;
    /// Verify the peer identity's self-signature, that its signing key is
    /// `endpoint`, and return `(peer NodeId, SAS pairing code)`.
    fn verify(&self, peer: &Value, endpoint: &EndpointId) -> Result<(String, String)>;
    /// Persist a confirmed pairing. Must be atomic: on error nothing changed.
    async fn apply(&self, outcome: Outcome) -> Result<()>;
    /// Joiner: this device's roster member, sent so the inviter can add it.
    async fn member(&self) -> Option<Value> {
        None
    }
    /// Inviter: add `member` (the joiner, whose identity is `peer`) to this device's
    /// identity and return what it needs to join: the new roster and the owner key
    /// sealed to the member's key.
    async fn bundle(&self, _member: &Value, _peer: &Value) -> Option<Value> {
        None
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum Frame {
    Hello {
        v: u8,
        #[serde(with = "hex16")]
        secret: [u8; 16],
        identity: Value,
        addr: IrohAddr,
        #[serde(default)]
        features: u8,
    },
    Welcome {
        identity: Value,
        purpose: PairPurpose,
        addr: IrohAddr,
        #[serde(default)]
        features: u8,
    },
    /// Joiner → inviter after both committed: the joiner's roster member (or null).
    Member {
        member: Value,
    },
    /// Inviter → joiner: the identity to join (or null).
    Identity {
        bundle: Option<Value>,
    },
    Reject {
        reason: String,
    },
    Decision {
        matches: bool,
    },
    Commit {
        credential: Option<String>,
    },
}
mod hex16 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8; 16], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.iter().map(|b| format!("{b:02x}")).collect::<String>())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 16], D::Error> {
        let text = String::deserialize(d)?;
        let mut out = [0u8; 16];
        if text.len() != 32 {
            return Err(serde::de::Error::custom("secret length"));
        }
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)
                .map_err(serde::de::Error::custom)?;
        }
        Ok(out)
    }
}

async fn write_frame<W: tokio::io::AsyncWrite + Unpin>(w: &mut W, frame: &Frame) -> Result<()> {
    let bytes = serde_json::to_vec(frame)?;
    w.write_u32(bytes.len() as u32).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}
async fn read_frame<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> Result<Frame> {
    let len = r
        .read_u32()
        .await
        .context("pairing peer closed the connection")?;
    ensure!(len <= MAX_FRAME, "pairing frame too large");
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    serde_json::from_slice(&buf).context("invalid pairing frame")
}

struct Invite {
    secret: [u8; 16],
    expires_at: u64,
    purpose: PairPurpose,
}

/// State of a pairing flow, as shown to the user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowState {
    /// Compare the code on both devices, then confirm or reject.
    Comparing,
    Paired,
    Rejected,
    Failed(String),
}

#[derive(Clone, Debug, Serialize)]
pub struct FlowView {
    pub flow_id: String,
    pub side: Side,
    pub purpose: PairPurpose,
    pub peer_node_id: String,
    pub sas: String,
    pub state: FlowState,
}

struct Flow {
    view: FlowView,
    decide: Option<oneshot::Sender<bool>>,
    done: watch::Receiver<FlowState>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Pairing service of one node.
#[derive(Clone)]
pub struct Pairing {
    node: IrohNode,
    host: Arc<dyn PairHost>,
    node_id: String,
    relay: Option<Relay>,
    hub: Option<IrohAddr>,
    invites: Arc<Mutex<Vec<Invite>>>,
    flows: Arc<Mutex<HashMap<String, Flow>>>,
}

/// Result of [`Pairing::invite`].
#[derive(Clone, Debug)]
pub struct Invitation {
    pub ticket: PairTicket,
    pub link: String,
    pub web_link: Option<String>,
    pub qr_svg: String,
}

impl Pairing {
    /// `relay`: the embedded relay of this node (Hub), so ticket holders may use it
    /// before they are paired. `hub`: the Hub address to put into tickets.
    pub fn new(
        node: IrohNode,
        node_id: String,
        host: Arc<dyn PairHost>,
        relay: Option<Relay>,
        hub: Option<IrohAddr>,
    ) -> Self {
        Self {
            node,
            host,
            node_id,
            relay,
            hub,
            invites: Arc::default(),
            flows: Arc::default(),
        }
    }

    /// Create a one-time invitation (15 minutes, at most 16 open at once).
    pub fn invite(&self, purpose: PairPurpose, public_url: Option<&str>) -> Result<Invitation> {
        let mut secret = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::rng(), &mut secret);
        let expires_at = now() + PAIR_TICKET_TTL_SECS;
        {
            let mut invites = self.invites.lock().unwrap();
            invites.retain(|i| i.expires_at > now());
            ensure!(
                invites.len() < MAX_OPEN_TICKETS,
                "Too many open invitations. Wait for one to expire or be used."
            );
            invites.push(Invite {
                secret,
                expires_at,
                purpose,
            });
        }
        if let Some(relay) = &self.relay {
            relay.allow_pairing(&secret, Duration::from_secs(PAIR_TICKET_TTL_SECS));
        }
        let ticket = PairTicket {
            v: PAIR_TICKET_VERSION,
            inviter: self.node.addr(),
            inviter_node_id: self.node_id.clone(),
            secret,
            expires_at,
            purpose,
            hub: self.hub.clone(),
        };
        let link = ticket::app_link(&ticket)?;
        Ok(Invitation {
            web_link: public_url
                .map(|url| ticket::web_link(url, &ticket))
                .transpose()?,
            qr_svg: ticket::qr_svg(&link)?,
            link,
            ticket,
        })
    }

    pub fn open_invitations(&self) -> usize {
        let mut invites = self.invites.lock().unwrap();
        invites.retain(|i| i.expires_at > now());
        invites.len()
    }

    /// Redeem a ticket: constant-time, single use, not expired.
    fn redeem(&self, secret: &[u8; 16]) -> Option<PairPurpose> {
        let mut invites = self.invites.lock().unwrap();
        invites.retain(|i| i.expires_at > now());
        let index = invites
            .iter()
            .position(|i| bool::from(subtle::ConstantTimeEq::ct_eq(&i.secret[..], &secret[..])))?;
        Some(invites.remove(index).purpose)
    }

    pub fn flows(&self) -> Vec<FlowView> {
        self.flows
            .lock()
            .unwrap()
            .values()
            .map(|f| {
                let mut view = f.view.clone();
                view.state = f.done.borrow().clone();
                view
            })
            .collect()
    }
    pub fn flow(&self, id: &str) -> Option<FlowView> {
        self.flows().into_iter().find(|f| f.flow_id == id)
    }

    /// Confirm (`matches = true`) or reject the code of a flow on this device.
    /// Waits until the pairing is committed on both sides (or failed).
    pub async fn confirm(&self, flow: &str, matches: bool) -> Result<FlowState> {
        let mut done = {
            let mut flows = self.flows.lock().unwrap();
            let entry = flows
                .get_mut(flow)
                .context("Unknown or finished pairing.")?;
            if let Some(decide) = entry.decide.take() {
                let _ = decide.send(matches);
            }
            entry.done.clone()
        };
        let state = tokio::time::timeout(STEP_TIMEOUT * 2, async {
            loop {
                let state = done.borrow().clone();
                if state != FlowState::Comparing {
                    return state;
                }
                if done.changed().await.is_err() {
                    return FlowState::Failed("pairing ended".into());
                }
            }
        })
        .await
        .unwrap_or_else(|_| FlowState::Failed("the other device did not answer".into()));
        Ok(state)
    }

    fn register(&self, view: FlowView) -> (oneshot::Receiver<bool>, watch::Sender<FlowState>) {
        let (decide, decision) = oneshot::channel();
        let (state, done) = watch::channel(FlowState::Comparing);
        let mut flows = self.flows.lock().unwrap();
        // Keep finished flows briefly for status queries; cap the table.
        if flows.len() >= 64 {
            flows.retain(|_, f| *f.done.borrow() == FlowState::Comparing);
        }
        flows.insert(
            view.flow_id.clone(),
            Flow {
                view,
                decide: Some(decide),
                done,
            },
        );
        (decision, state)
    }

    /// Joiner: open the link, prove the ticket and get the SAS code.
    pub async fn accept(&self, link: &str) -> Result<FlowView> {
        let ticket = ticket::parse_link(link)?;
        ensure!(
            ticket.expires_at > now(),
            "This pairing link has expired. Ask for a new one."
        );
        ensure!(
            ticket.inviter_node_id != self.node_id,
            "This link was created on this device."
        );
        let target = addr::endpoint_addr(&ticket.inviter)?;
        if let Some(relay) = target.relay_urls().next().cloned() {
            // A Hub relay admits ticket holders by a token derived from the secret.
            let token = crate::relay::pairing_token(&ticket.secret);
            let existing = self.node.endpoint().insert_relay(
                relay.clone(),
                Arc::new(iroh::RelayConfig::new(relay, None).with_auth_token(token)),
            );
            existing.await;
        }
        let conn = tokio::time::timeout(STEP_TIMEOUT, self.node.connect(&target, crate::ALPN_PAIR))
            .await
            .context("The other device did not answer. Make sure it is online.")??;
        let (mut send, mut recv) = conn.open_bi().await?;
        write_frame(
            &mut send,
            &Frame::Hello {
                v: PAIR_TICKET_VERSION,
                secret: ticket.secret,
                identity: self.host.identity(),
                addr: self.node.addr(),
                features: FEATURE_IDENTITY,
            },
        )
        .await?;
        let (identity, purpose, peer_addr, features) =
            match tokio::time::timeout(STEP_TIMEOUT, read_frame(&mut recv)).await?? {
                Frame::Welcome {
                    identity,
                    purpose,
                    addr,
                    features,
                } => (identity, purpose, addr, features),
                Frame::Reject { reason } => bail!("{reason}"),
                _ => bail!("unexpected pairing answer"),
            };
        let (peer_node_id, sas) = self.host.verify(&identity, &conn.remote_id())?;
        ensure!(
            peer_node_id == ticket.inviter_node_id && purpose == ticket.purpose,
            "The device that answered does not match the link."
        );
        let mut peer_addr = peer_addr;
        peer_addr.endpoint_id = addr::hex_id(&conn.remote_id());
        if peer_addr.relay_url.is_none() {
            peer_addr.relay_url = ticket.inviter.relay_url.clone();
        }
        let view = FlowView {
            flow_id: uuid(),
            side: Side::Joiner,
            purpose,
            peer_node_id: peer_node_id.clone(),
            sas,
            state: FlowState::Comparing,
        };
        let (decision, state) = self.register(view.clone());
        let pending = Pending {
            host: self.host.clone(),
            side: Side::Joiner,
            purpose,
            identity,
            peer_node_id,
            peer_addr,
            peer_features: features,
            conn,
        };
        tokio::spawn(pending.run(send, recv, decision, state));
        Ok(view)
    }

    /// Inviter: serve one incoming pairing connection.
    pub async fn handle(&self, conn: Connection) -> Result<()> {
        let (mut send, mut recv) = tokio::time::timeout(STEP_TIMEOUT, conn.accept_bi()).await??;
        let Frame::Hello {
            v,
            secret,
            identity,
            addr: peer_addr,
            features,
        } = tokio::time::timeout(STEP_TIMEOUT, read_frame(&mut recv)).await??
        else {
            bail!("unexpected pairing frame");
        };
        let reject = |reason: &str| Frame::Reject {
            reason: reason.into(),
        };
        if v != PAIR_TICKET_VERSION {
            write_frame(&mut send, &reject("Update Cranibox on both devices.")).await?;
            bail!("pairing version {v}");
        }
        let Some(purpose) = self.redeem(&secret) else {
            write_frame(
                &mut send,
                &reject("This pairing link was already used or has expired. Ask for a new one."),
            )
            .await?;
            let _ = send.finish();
            tokio::time::sleep(Duration::from_millis(200)).await;
            bail!("invalid, reused or expired pairing secret");
        };
        let (peer_node_id, sas) = match self.host.verify(&identity, &conn.remote_id()) {
            Ok(ok) => ok,
            Err(error) => {
                write_frame(&mut send, &reject("The device identity is invalid.")).await?;
                return Err(error);
            }
        };
        write_frame(
            &mut send,
            &Frame::Welcome {
                identity: self.host.identity(),
                purpose,
                addr: self.node.addr(),
                features: FEATURE_IDENTITY,
            },
        )
        .await?;
        let mut peer_addr = peer_addr;
        peer_addr.endpoint_id = addr::hex_id(&conn.remote_id());
        let view = FlowView {
            flow_id: uuid(),
            side: Side::Inviter,
            purpose,
            peer_node_id: peer_node_id.clone(),
            sas,
            state: FlowState::Comparing,
        };
        let (decision, state) = self.register(view);
        Pending {
            host: self.host.clone(),
            side: Side::Inviter,
            purpose,
            identity,
            peer_node_id,
            peer_addr,
            peer_features: features,
            conn,
        }
        .run(send, recv, decision, state)
        .await;
        Ok(())
    }
}

fn uuid() -> String {
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rng(), &mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn credential() -> String {
    let mut bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rng(), &mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

struct Pending {
    host: Arc<dyn PairHost>,
    side: Side,
    purpose: PairPurpose,
    identity: Value,
    peer_node_id: String,
    peer_addr: IrohAddr,
    peer_features: u8,
    conn: Connection,
}
impl Pending {
    async fn run(
        self,
        mut send: iroh::endpoint::SendStream,
        mut recv: iroh::endpoint::RecvStream,
        decision: oneshot::Receiver<bool>,
        state: watch::Sender<FlowState>,
    ) {
        let result = self.exchange(&mut send, &mut recv, decision).await;
        let final_state = match result {
            Ok(true) => FlowState::Paired,
            Ok(false) => FlowState::Rejected,
            Err(error) => FlowState::Failed(format!("{error:#}")),
        };
        let _ = send.finish();
        state.send_replace(final_state);
        tokio::time::sleep(Duration::from_millis(300)).await;
        self.conn.close(crate::close::DONE.into(), b"pairing done");
    }

    async fn exchange(
        &self,
        send: &mut iroh::endpoint::SendStream,
        recv: &mut iroh::endpoint::RecvStream,
        decision: oneshot::Receiver<bool>,
    ) -> Result<bool> {
        // Phase 1: require each device's local consent before sending credentials.
        // Keep both futures alive so a partially read frame is never discarded.
        let matches = tokio::time::timeout(DECISION_TIMEOUT, async {
            let local = async {
                let matches = decision.await.unwrap_or(false);
                write_frame(send, &Frame::Decision { matches }).await?;
                anyhow::Ok(matches)
            };
            let remote = async {
                match read_frame(recv).await? {
                    Frame::Decision { matches } => Ok(matches),
                    _ => bail!("unexpected pairing frame"),
                }
            };
            tokio::pin!(local, remote);
            tokio::select! {
                result = &mut local => {
                    if result? { remote.await } else { Ok(false) }
                }
                result = &mut remote => {
                    if result? { local.await } else { Ok(false) }
                }
            }
        })
        .await
        .context("The pairing code was not confirmed on both devices in time.")??;
        if !matches {
            return Ok(false);
        }
        // Phase 2: exchange credentials, then persist atomically.
        let issues = self.side == Side::Inviter || self.purpose == PairPurpose::OwnerDevice;
        let issued = issues.then(credential);
        write_frame(
            send,
            &Frame::Commit {
                credential: issued.clone(),
            },
        )
        .await?;
        let received = tokio::time::timeout(STEP_TIMEOUT, async {
            loop {
                match read_frame(recv).await? {
                    // Both devices may have confirmed at the same time.
                    Frame::Decision { matches: true } => continue,
                    Frame::Decision { matches: false } => {
                        bail!("the other device rejected the code")
                    }
                    Frame::Commit { credential } => return anyhow::Ok(credential),
                    _ => bail!("unexpected pairing frame"),
                }
            }
        })
        .await
        .context("the other device did not finish pairing")??;
        // Phase 3 (owner devices, both sides new enough): the joiner joins the inviter's
        // identity. Failing here never undoes the pairing itself.
        let mut joined = None;
        if self.purpose == PairPurpose::OwnerDevice && self.peer_features & FEATURE_IDENTITY != 0 {
            let identity = tokio::time::timeout(STEP_TIMEOUT, async {
                match self.side {
                    Side::Joiner => {
                        let member = self.host.member().await.unwrap_or(Value::Null);
                        write_frame(send, &Frame::Member { member }).await?;
                        match read_frame(recv).await? {
                            Frame::Identity { bundle } => anyhow::Ok(bundle),
                            _ => bail!("unexpected pairing frame"),
                        }
                    }
                    Side::Inviter => {
                        let member = match read_frame(recv).await? {
                            Frame::Member { member } => member,
                            _ => bail!("unexpected pairing frame"),
                        };
                        let bundle = if member.is_null() {
                            None
                        } else {
                            self.host.bundle(&member, &self.identity).await
                        };
                        write_frame(send, &Frame::Identity { bundle }).await?;
                        Ok(None)
                    }
                }
            })
            .await;
            joined = match identity {
                Ok(Ok(bundle)) => bundle,
                Ok(Err(error)) => {
                    eprintln!("pairing identity exchange: {error:#}");
                    None
                }
                Err(_) => None,
            };
        }
        let expects = self.side == Side::Joiner || self.purpose == PairPurpose::OwnerDevice;
        ensure!(
            received.is_some() == expects
                && received
                    .as_ref()
                    .is_none_or(|c| c.len() == 64 && c.bytes().all(|b| b.is_ascii_hexdigit())),
            "invalid pairing credential"
        );
        self.host
            .apply(Outcome {
                side: self.side,
                purpose: self.purpose,
                peer_identity: self.identity.clone(),
                peer_node_id: self.peer_node_id.clone(),
                peer_addr: self.peer_addr.clone(),
                issued,
                received,
                identity: joined,
            })
            .await?;
        Ok(true)
    }
}
