//! End-to-end encrypted node-to-node calls (Noise IK over HTTP).
//!
//! The caller already knows the target's static Noise key (pinned, self-signed by the
//! target identity), so a single round trip establishes a session:
//!
//! 1. `POST …/e2e/handshake` carries Noise IK message 1. Its payload, encrypted to the
//!    target's static key, holds the caller's signed identity and grant secret.
//! 2. The target checks the grant, pins or compares the caller key, and answers with IK
//!    message 2 (ephemeral–ephemeral DH: transport keys have forward secrecy).
//! 3. `POST …/e2e/call` frames carry an inner API request encrypted with explicit,
//!    never-reused nonces; the target keeps a replay window per session.
//!
//! Any relay, gateway or overlay network between the nodes only sees ciphertext, the
//! session ID and the expected target NodeID. Primitives come from `snow`
//! (X25519, ChaCha20-Poly1305, BLAKE2s); nothing here implements cryptography itself.
use super::identity::{Identity, NOISE_PARAMS, PublicIdentity, hex, now};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

const TAG: usize = 16;
/// Plaintext bytes per Noise transport message (Noise caps a message at 65535 bytes).
const CHUNK: usize = 65_000;
/// Upper bound for one encrypted request or response body.
pub const MAX_FRAME: usize = 48 * 1024 * 1024;
const MAX_CHUNKS: usize = MAX_FRAME / CHUNK + 1;
pub const MAX_HANDSHAKE: usize = 4096;
/// Clock skew accepted for handshake timestamps.
const SKEW: u64 = 300;
pub const SESSION_LIFETIME: Duration = Duration::from_secs(3600);
pub const SESSION_IDLE: Duration = Duration::from_secs(600);
const MAX_SESSIONS: usize = 512;
/// Out-of-order tolerance for concurrent requests sharing one session.
const REPLAY_WINDOW: usize = 4096;
pub const SESSION_HEADER: &str = "x-cranibox-session";
pub const STREAM_AUTH_HEADER: &str = "x-cranibox-e2e-auth";

fn prologue(target: &str) -> Vec<u8> {
    let mut value = b"cranibox-e2e-v1\0".to_vec();
    value.extend_from_slice(target.as_bytes());
    value
}
fn params() -> snow::params::NoiseParams {
    NOISE_PARAMS.parse().expect("valid Noise parameters")
}

/// Handshake payload sent by the caller (inside Noise IK message 1).
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub version: u32,
    pub caller: PublicIdentity,
    pub secret: String,
    pub at: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Welcome {
    version: u32,
    session: String,
    expires_in: u64,
}

// ---------------------------------------------------------------- framing
fn seal(
    transport: &snow::StatelessTransportState,
    start: u64,
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let chunks: Vec<&[u8]> = if plaintext.is_empty() {
        vec![&[][..]]
    } else {
        plaintext.chunks(CHUNK).collect()
    };
    ensure!(chunks.len() <= MAX_CHUNKS, "encrypted payload too large");
    let mut out = Vec::with_capacity(plaintext.len() + chunks.len() * (TAG + 5) + 10);
    out.extend_from_slice(&start.to_be_bytes());
    out.extend_from_slice(&(chunks.len() as u32).to_be_bytes());
    let mut buffer = vec![0u8; CHUNK + 1 + TAG];
    for (index, chunk) in chunks.iter().enumerate() {
        let mut message = Vec::with_capacity(chunk.len() + 1);
        message.push(u8::from(index + 1 == chunks.len()));
        message.extend_from_slice(chunk);
        let nonce = start
            .checked_add(index as u64)
            .context("nonce space exhausted")?;
        let len = transport
            .write_message(nonce, &message, &mut buffer)
            .map_err(|_| anyhow::anyhow!("encryption failed"))?;
        out.extend_from_slice(&(len as u32).to_be_bytes());
        out.extend_from_slice(&buffer[..len]);
    }
    Ok(out)
}
/// Frame header: first nonce and number of chunks, without decrypting.
pub fn frame_nonces(frame: &[u8]) -> Result<(u64, u64)> {
    ensure!(frame.len() >= 12, "invalid encrypted frame");
    let start = u64::from_be_bytes(frame[0..8].try_into()?);
    let count = u32::from_be_bytes(frame[8..12].try_into()?) as u64;
    ensure!(
        count >= 1 && count as usize <= MAX_CHUNKS,
        "invalid encrypted frame"
    );
    start
        .checked_add(count)
        .context("invalid encrypted frame")?;
    Ok((start, count))
}
fn open(transport: &snow::StatelessTransportState, frame: &[u8]) -> Result<(u64, Vec<u8>)> {
    let (start, count) = frame_nonces(frame)?;
    let mut offset = 12;
    let mut out = Vec::new();
    let mut buffer = vec![0u8; CHUNK + 1 + TAG];
    for index in 0..count {
        ensure!(frame.len() >= offset + 4, "truncated encrypted frame");
        let len = u32::from_be_bytes(frame[offset..offset + 4].try_into()?) as usize;
        offset += 4;
        ensure!(
            len > TAG && len <= CHUNK + 1 + TAG && frame.len() >= offset + len,
            "truncated encrypted frame"
        );
        let size = transport
            .read_message(start + index, &frame[offset..offset + len], &mut buffer)
            .map_err(|_| anyhow::anyhow!("encrypted frame failed authentication"))?;
        offset += len;
        ensure!(size >= 1, "invalid encrypted chunk");
        let last = buffer[0] == 1;
        ensure!(
            buffer[0] <= 1 && last == (index + 1 == count),
            "encrypted frame was truncated or reordered"
        );
        out.extend_from_slice(&buffer[1..size]);
        ensure!(out.len() <= MAX_FRAME, "encrypted payload too large");
    }
    ensure!(offset == frame.len(), "trailing data after encrypted frame");
    Ok((start, out))
}

/// Inner request: method, path and raw JSON body bytes.
pub struct Inner {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
}
impl Inner {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.body.len() + self.path.len() + 8);
        out.push(self.method.len() as u8);
        out.extend_from_slice(self.method.as_bytes());
        out.extend_from_slice(&(self.path.len() as u16).to_be_bytes());
        out.extend_from_slice(self.path.as_bytes());
        out.extend_from_slice(&self.body);
        out
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        let method_len = *bytes.first().context("invalid inner request")? as usize;
        ensure!(
            (1..=8).contains(&method_len) && bytes.len() >= 3 + method_len,
            "invalid inner request"
        );
        let method = std::str::from_utf8(&bytes[1..1 + method_len])?.to_owned();
        let at = 1 + method_len;
        let path_len = u16::from_be_bytes(bytes[at..at + 2].try_into()?) as usize;
        ensure!(bytes.len() >= at + 2 + path_len, "invalid inner request");
        let path = std::str::from_utf8(&bytes[at + 2..at + 2 + path_len])?.to_owned();
        ensure!(
            matches!(method.as_str(), "GET" | "POST")
                && path.starts_with("/api/v1/mesh/")
                && !path.contains("/e2e/"),
            "unsupported inner request"
        );
        Ok(Self {
            method,
            path,
            body: bytes[at + 2 + path_len..].to_vec(),
        })
    }
}

// ---------------------------------------------------------------- responder
/// A responder-side session. The caller identity is authenticated by Noise (static key)
/// and the grant secret; permissions are re-read from the grant for every call.
pub struct ServerSession {
    pub caller: PublicIdentity,
    pub credential: String,
    transport: snow::StatelessTransportState,
    seen: BTreeSet<u64>,
    floor: Option<u64>,
    send: u64,
    created: Instant,
    used: Instant,
}
impl ServerSession {
    fn fresh(&self) -> bool {
        self.created.elapsed() < SESSION_LIFETIME && self.used.elapsed() < SESSION_IDLE
    }
    /// Reserve `count` receive nonces starting at `start`; fails on any reuse.
    fn admit(&mut self, start: u64, count: u64) -> Result<()> {
        let end = start + count;
        ensure!(
            self.floor.is_none_or(|floor| start > floor)
                && self.seen.range(start..end).next().is_none(),
            "replayed or stale encrypted message"
        );
        self.seen.extend(start..end);
        while self.seen.len() > REPLAY_WINDOW {
            self.floor = self.seen.pop_first();
        }
        Ok(())
    }
    /// Decrypt a call. Nonces are only reserved after authentication succeeds.
    pub fn open_request(&mut self, frame: &[u8]) -> Result<(u64, Inner)> {
        let (start, count) = frame_nonces(frame)?;
        ensure!(
            self.floor.is_none_or(|floor| start > floor)
                && self.seen.range(start..start + count).next().is_none(),
            "replayed or stale encrypted message"
        );
        let (start, plaintext) = open(&self.transport, frame)?;
        self.admit(start, count)?;
        self.used = Instant::now();
        Ok((start, Inner::decode(&plaintext)?))
    }
    pub fn seal_response(&mut self, request: u64, status: u16, body: &[u8]) -> Result<Vec<u8>> {
        let mut plaintext = Vec::with_capacity(body.len() + 10);
        plaintext.extend_from_slice(&request.to_be_bytes());
        plaintext.extend_from_slice(&status.to_be_bytes());
        plaintext.extend_from_slice(body);
        let count = plaintext.len().div_ceil(CHUNK).max(1) as u64;
        let start = self.send;
        self.send = start.checked_add(count).context("nonce space exhausted")?;
        seal(&self.transport, start, &plaintext)
    }
    /// Verify a WebSocket upgrade proof bound to this session and request path.
    pub fn open_stream_proof(&mut self, proof: &str, path: &str) -> Result<()> {
        let frame = decode_hex(proof)?;
        let (start, count) = frame_nonces(&frame)?;
        ensure!(count == 1, "invalid stream proof");
        let (_, plaintext) = open(&self.transport, &frame)?;
        let claim: StreamClaim = serde_json::from_slice(&plaintext)?;
        ensure!(
            claim.path == path && claim.at.abs_diff(now()) <= SKEW,
            "stream proof does not match this request"
        );
        self.admit(start, 1)?;
        self.used = Instant::now();
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StreamClaim {
    path: String,
    at: u64,
}

/// Responder-side session table, bounded and expiring.
#[derive(Default)]
pub struct Sessions {
    items: BTreeMap<String, ServerSession>,
}
impl Sessions {
    pub fn get(&mut self, id: &str) -> Option<&mut ServerSession> {
        self.items.retain(|_, s| s.fresh());
        self.items.get_mut(id)
    }
    pub fn remove(&mut self, id: &str) {
        self.items.remove(id);
    }
    /// Drop sessions whose grant no longer matches (revocation or rotation).
    pub fn retain(&mut self, mut keep: impl FnMut(&ServerSession) -> bool) {
        self.items.retain(|_, s| s.fresh() && keep(s));
    }
    fn insert(&mut self, id: String, session: ServerSession) {
        self.items.retain(|_, s| s.fresh());
        while self.items.len() >= MAX_SESSIONS {
            let oldest = self
                .items
                .iter()
                .min_by_key(|(_, s)| s.used)
                .map(|(id, _)| id.clone())
                .expect("nonempty");
            self.items.remove(&oldest);
        }
        self.items.insert(id, session);
    }
}

/// First half of the responder handshake: decrypt message 1 and authenticate the
/// caller's static key against its signed identity. The grant check happens next.
pub struct Pending {
    state: snow::HandshakeState,
    pub hello: Hello,
}
pub fn respond(identity: &Identity, message: &[u8]) -> Result<Pending> {
    ensure!(message.len() <= MAX_HANDSHAKE, "handshake too large");
    let prologue = prologue(identity.node_id());
    let mut state = snow::Builder::new(params())
        .local_private_key(identity.noise_private())?
        .prologue(&prologue)?
        .build_responder()?;
    let mut payload = vec![0u8; MAX_HANDSHAKE];
    let size = state
        .read_message(message, &mut payload)
        .map_err(|_| anyhow::anyhow!("handshake failed"))?;
    let hello: Hello = serde_json::from_slice(&payload[..size])?;
    ensure!(hello.version == 1, "unsupported handshake");
    hello.caller.verify()?;
    ensure!(
        state.get_remote_static() == Some(&hello.caller.noise_bytes()?[..]),
        "caller key does not match its identity"
    );
    ensure!(
        hello.at.abs_diff(now()) <= SKEW,
        "handshake timestamp outside the accepted clock skew"
    );
    Ok(Pending { state, hello })
}
impl Pending {
    /// Finish after the grant was checked; returns message 2.
    pub fn accept(mut self, sessions: &mut Sessions, credential: String) -> Result<Vec<u8>> {
        let mut id = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::rng(), &mut id);
        let id = hex(&id);
        let welcome = serde_json::to_vec(&Welcome {
            version: 1,
            session: id.clone(),
            expires_in: SESSION_LIFETIME.as_secs(),
        })?;
        let mut message = vec![0u8; MAX_HANDSHAKE];
        let size = self.state.write_message(&welcome, &mut message)?;
        message.truncate(size);
        let transport = self.state.into_stateless_transport_mode()?;
        sessions.insert(
            id,
            ServerSession {
                caller: self.hello.caller,
                credential,
                transport,
                seen: BTreeSet::new(),
                floor: None,
                send: 0,
                created: Instant::now(),
                used: Instant::now(),
            },
        );
        Ok(message)
    }
}

// ---------------------------------------------------------------- initiator
pub struct ClientSession {
    pub id: String,
    transport: snow::StatelessTransportState,
    next: std::sync::atomic::AtomicU64,
    created: Instant,
}
pub struct Initiator {
    state: snow::HandshakeState,
}
impl Initiator {
    pub fn start(
        local: &Identity,
        target: &PublicIdentity,
        secret: &str,
    ) -> Result<(Self, Vec<u8>)> {
        target.verify()?;
        let remote = target.noise_bytes()?;
        let prologue = prologue(&target.node_id);
        let mut state = snow::Builder::new(params())
            .local_private_key(local.noise_private())?
            .remote_public_key(&remote)?
            .prologue(&prologue)?
            .build_initiator()?;
        let hello = serde_json::to_vec(&Hello {
            version: 1,
            caller: local.public().clone(),
            secret: secret.to_owned(),
            at: now(),
        })?;
        let mut message = vec![0u8; MAX_HANDSHAKE];
        let size = state.write_message(&hello, &mut message)?;
        message.truncate(size);
        Ok((Self { state }, message))
    }
    pub fn finish(mut self, message: &[u8]) -> Result<ClientSession> {
        ensure!(message.len() <= MAX_HANDSHAKE, "handshake too large");
        let mut payload = vec![0u8; MAX_HANDSHAKE];
        let size = self
            .state
            .read_message(message, &mut payload)
            .map_err(|_| anyhow::anyhow!("target failed end-to-end authentication"))?;
        let welcome: Welcome = serde_json::from_slice(&payload[..size])?;
        ensure!(
            welcome.version == 1
                && welcome.session.len() == 32
                && welcome.session.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid handshake response"
        );
        Ok(ClientSession {
            id: welcome.session,
            transport: self.state.into_stateless_transport_mode()?,
            next: 0.into(),
            created: Instant::now(),
        })
    }
}
impl ClientSession {
    pub fn fresh(&self) -> bool {
        // Renew before the target's lifetime ends so calls do not race expiry.
        self.created.elapsed() + Duration::from_secs(60) < SESSION_LIFETIME
    }
    fn reserve(&self, count: u64) -> u64 {
        self.next
            .fetch_add(count, std::sync::atomic::Ordering::SeqCst)
    }
    pub fn seal_request(&self, inner: &Inner) -> Result<(u64, Vec<u8>)> {
        let plaintext = inner.encode();
        let count = plaintext.len().div_ceil(CHUNK).max(1) as u64;
        let start = self.reserve(count);
        Ok((start, seal(&self.transport, start, &plaintext)?))
    }
    pub fn open_response(&self, request: u64, frame: &[u8]) -> Result<(u16, Vec<u8>)> {
        let (_, plaintext) = open(&self.transport, frame)?;
        ensure!(plaintext.len() >= 10, "invalid encrypted response");
        // The response names the request nonce: responses cannot be swapped.
        ensure!(
            u64::from_be_bytes(plaintext[0..8].try_into()?) == request,
            "encrypted response does not belong to this request"
        );
        let status = u16::from_be_bytes(plaintext[8..10].try_into()?);
        Ok((status, plaintext[10..].to_vec()))
    }
    /// Proof that a WebSocket upgrade comes from this session's caller.
    pub fn stream_proof(&self, path: &str) -> Result<String> {
        let claim = serde_json::to_vec(&StreamClaim {
            path: path.to_owned(),
            at: now(),
        })?;
        let start = self.reserve(1);
        Ok(hex(&seal(&self.transport, start, &claim)?))
    }
}
fn decode_hex(value: &str) -> Result<Vec<u8>> {
    ensure!(
        value.len().is_multiple_of(2) && value.len() <= 2048,
        "invalid encrypted proof"
    );
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).context("invalid encrypted proof"))
        .collect()
}
/// Client error reporting an unknown or expired responder session (safe to redo once:
/// the target could not decrypt, so it did not act on the request).
pub const SESSION_GONE: u16 = 410;

pub fn unexpected(status: u16) -> Result<()> {
    if status == SESSION_GONE {
        bail!("end-to-end session expired")
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pair() -> (Identity, Identity, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let a = Identity::create_new(&root.path().join("a/peers.json")).unwrap();
        let b = Identity::create_new(&root.path().join("b/peers.json")).unwrap();
        (a, b, root)
    }
    fn session(
        caller: &Identity,
        target: &Identity,
        sessions: &mut Sessions,
    ) -> (ClientSession, String) {
        let (initiator, m1) = Initiator::start(caller, target.public(), &"a".repeat(64)).unwrap();
        let pending = respond(target, &m1).unwrap();
        assert_eq!(pending.hello.caller, *caller.public());
        assert_eq!(pending.hello.secret, "a".repeat(64));
        let m2 = pending.accept(sessions, "cred".into()).unwrap();
        let client = initiator.finish(&m2).unwrap();
        let id = client.id.clone();
        (client, id)
    }
    fn request(body: &[u8]) -> Inner {
        Inner {
            method: "POST".into(),
            path: "/api/v1/mesh/nodes/x/messages".into(),
            body: body.to_vec(),
        }
    }
    #[test]
    fn round_trip_including_large_bodies() {
        let (a, b, _root) = pair();
        let mut sessions = Sessions::default();
        let (client, id) = session(&a, &b, &mut sessions);
        for size in [0usize, 10, CHUNK - 3, CHUNK * 3 + 7] {
            let body: Vec<u8> = (0..size).map(|i| i as u8).collect();
            let (nonce, frame) = client.seal_request(&request(&body)).unwrap();
            assert!(
                !frame.windows(16).any(|w| size >= 16 && w == &body[..16]),
                "plaintext must not appear on the wire"
            );
            let server = sessions.get(&id).unwrap();
            let (got, inner) = server.open_request(&frame).unwrap();
            assert_eq!((got, inner.body.as_slice()), (nonce, body.as_slice()));
            let response = server.seal_response(got, 201, &body).unwrap();
            let (status, back) = client.open_response(nonce, &response).unwrap();
            assert_eq!((status, back), (201, body));
        }
    }
    #[test]
    fn replay_tampering_and_misbinding_are_rejected() {
        let (a, b, _root) = pair();
        let mut sessions = Sessions::default();
        let (client, id) = session(&a, &b, &mut sessions);
        let (nonce, frame) = client.seal_request(&request(b"{}")).unwrap();
        let server = sessions.get(&id).unwrap();
        server.open_request(&frame).unwrap();
        assert!(server.open_request(&frame).is_err(), "replay must fail");
        let (_, frame) = client.seal_request(&request(b"{\"x\":1}")).unwrap();
        let mut tampered = frame.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(
            server.open_request(&tampered).is_err(),
            "tampering must fail"
        );
        // Authentication failure must not burn the nonce: the genuine frame still works.
        let (second, _) = server.open_request(&frame).unwrap();
        // Responses are bound to their request nonce.
        let response = server.seal_response(second, 200, b"{}").unwrap();
        assert!(client.open_response(nonce, &response).is_err());
        assert!(client.open_response(second, &response).is_ok());
        // Chunk truncation is detected.
        let (_, big) = client
            .seal_request(&request(&vec![7u8; CHUNK * 2]))
            .unwrap();
        let mut truncated = big.clone();
        truncated[8..12].copy_from_slice(&1u32.to_be_bytes());
        let first_len = u32::from_be_bytes(truncated[12..16].try_into().unwrap()) as usize;
        truncated.truncate(16 + first_len);
        assert!(server.open_request(&truncated).is_err());
        // The inner path is restricted to the mesh API.
        let (_, nested) = client
            .seal_request(&Inner {
                method: "POST".into(),
                path: "/api/v1/mesh/nodes/x/e2e/call".into(),
                body: vec![],
            })
            .unwrap();
        assert!(server.open_request(&nested).is_err());
    }
    #[test]
    fn wrong_keys_cannot_complete_or_read_a_session() {
        let (a, b, _root) = pair();
        let root = tempfile::tempdir().unwrap();
        let mallory = Identity::create_new(&root.path().join("peers.json")).unwrap();
        // Message 1 for B cannot be opened by another node.
        let (_, m1) = Initiator::start(&a, b.public(), &"a".repeat(64)).unwrap();
        assert!(respond(&mallory, &m1).is_err());
        // A caller cannot claim someone else's identity with its own static key.
        let mut forged = mallory.public().clone();
        forged.noise_key = a.public().noise_key.clone();
        let mut sessions = Sessions::default();
        let (initiator, m1) = Initiator::start(&a, b.public(), &"a".repeat(64)).unwrap();
        let pending = respond(&b, &m1).unwrap();
        let m2 = pending.accept(&mut sessions, "c".into()).unwrap();
        // A different initiator cannot finish B's response.
        let (other, _) = Initiator::start(&mallory, b.public(), &"a".repeat(64)).unwrap();
        assert!(other.finish(&m2).is_err());
        let client = initiator.finish(&m2).unwrap();
        // A second, independent session has unrelated keys.
        let (client2, id2) = session(&a, &b, &mut sessions);
        let (_, frame) = client.seal_request(&request(b"{}")).unwrap();
        assert!(sessions.get(&id2).unwrap().open_request(&frame).is_err());
        drop(client2);
        assert!(forged.verify().is_err());
    }
    #[test]
    fn stream_proofs_are_path_bound_and_single_use() {
        let (a, b, _root) = pair();
        let mut sessions = Sessions::default();
        let (client, id) = session(&a, &b, &mut sessions);
        let proof = client
            .stream_proof("/api/v1/mesh/nodes/x/desktop/ws")
            .unwrap();
        let server = sessions.get(&id).unwrap();
        assert!(
            server
                .open_stream_proof(&proof, "/api/v1/mesh/nodes/x/terminal/ws")
                .is_err()
        );
        server
            .open_stream_proof(&proof, "/api/v1/mesh/nodes/x/desktop/ws")
            .unwrap();
        assert!(
            server
                .open_stream_proof(&proof, "/api/v1/mesh/nodes/x/desktop/ws")
                .is_err()
        );
    }
}
