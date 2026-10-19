//! End-to-end encryption for WebSocket data frames.
//!
//! The caller picks a fresh 32-byte stream secret and sends it inside the stream proof,
//! which is itself sealed by the Noise session, so only the target learns it. Each
//! direction then uses its own ChaCha20-Poly1305 key derived from the secret and the
//! request path. Every data message carries one AEAD frame; nonces are implicit
//! per-direction counters, so a replayed, dropped, reordered or modified frame fails
//! authentication and closes the stream. Control frames (ping, pong, close) carry no
//! application data and are not encrypted.
use anyhow::{Result, ensure};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use sha2::{Digest, Sha256};

/// Bytes added to every encrypted message (kind byte + Poly1305 tag).
pub const OVERHEAD: usize = 1 + 16;
const TEXT: u8 = 0;
const BINARY: u8 = 1;

/// A stream secret shared by both ends of one WebSocket.
#[derive(Clone)]
pub struct Secret([u8; 32]);
impl Secret {
    pub fn random() -> Self {
        let mut value = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rng(), &mut value);
        Self(value)
    }
    pub fn to_hex(&self) -> String {
        super::hex(&self.0)
    }
    pub fn from_hex(value: &str) -> Result<Self> {
        let bytes = super::decode_hex(value)?;
        ensure!(bytes.len() == 32, "invalid stream secret");
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(Self(out))
    }
    fn key(&self, direction: &[u8], path: &str) -> ChaCha20Poly1305 {
        let mut hash = Sha256::new();
        hash.update(b"cranibox-stream-v1\0");
        hash.update(direction);
        hash.update([0]);
        hash.update(self.0);
        hash.update(path.as_bytes());
        ChaCha20Poly1305::new(Key::from_slice(&hash.finalize()))
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

/// A decrypted data message.
#[derive(Debug, PartialEq, Eq)]
pub enum Data {
    Text(String),
    Binary(Vec<u8>),
}

struct Direction {
    cipher: ChaCha20Poly1305,
    counter: u64,
}
impl Direction {
    fn nonce(&mut self) -> Result<Nonce> {
        let mut nonce = [0u8; 12];
        nonce[4..].copy_from_slice(&self.counter.to_be_bytes());
        self.counter = self
            .counter
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("stream nonce space exhausted"))?;
        Ok(*Nonce::from_slice(&nonce))
    }
}

/// Per-stream encryption state for one end.
pub struct Cipher {
    send: Direction,
    receive: Direction,
}
impl Cipher {
    fn new(secret: &Secret, path: &str, caller: bool) -> Self {
        let (out, back) = (b"caller-to-target" as &[u8], b"target-to-caller" as &[u8]);
        let (send, receive) = if caller { (out, back) } else { (back, out) };
        Self {
            send: Direction {
                cipher: secret.key(send, path),
                counter: 0,
            },
            receive: Direction {
                cipher: secret.key(receive, path),
                counter: 0,
            },
        }
    }
    pub fn caller(secret: &Secret, path: &str) -> Self {
        Self::new(secret, path, true)
    }
    pub fn target(secret: &Secret, path: &str) -> Self {
        Self::new(secret, path, false)
    }
    pub fn seal(&mut self, data: &Data) -> Result<Vec<u8>> {
        let (kind, bytes) = match data {
            Data::Text(text) => (TEXT, text.as_bytes()),
            Data::Binary(bytes) => (BINARY, bytes.as_slice()),
        };
        let mut plaintext = Vec::with_capacity(bytes.len() + 1);
        plaintext.push(kind);
        plaintext.extend_from_slice(bytes);
        let nonce = self.send.nonce()?;
        self.send
            .cipher
            .encrypt(&nonce, Payload { msg: &plaintext, aad: b"" })
            .map_err(|_| anyhow::anyhow!("stream encryption failed"))
    }
    /// Decrypt the next frame. Any failure is fatal for the stream: the counter has
    /// advanced, so the caller must close instead of retrying.
    pub fn open(&mut self, frame: &[u8]) -> Result<Data> {
        let nonce = self.receive.nonce()?;
        let mut plaintext = self
            .receive
            .cipher
            .decrypt(&nonce, Payload { msg: frame, aad: b"" })
            .map_err(|_| anyhow::anyhow!("encrypted stream frame failed authentication"))?;
        ensure!(!plaintext.is_empty(), "invalid encrypted stream frame");
        let kind = plaintext.remove(0);
        match kind {
            TEXT => Ok(Data::Text(String::from_utf8(plaintext)?)),
            BINARY => Ok(Data::Binary(plaintext)),
            _ => anyhow::bail!("invalid encrypted stream frame"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PATH: &str = "/api/v1/mesh/nodes/x/terminal/ws";
    fn pair() -> (Cipher, Cipher) {
        let secret = Secret::random();
        (Cipher::caller(&secret, PATH), Cipher::target(&secret, PATH))
    }
    #[test]
    fn round_trip_both_directions_without_plaintext_on_the_wire() {
        let (mut caller, mut target) = pair();
        for i in 0..5 {
            let text = format!("{{\"type\":\"input\",\"data\":\"secret-{i}\"}}");
            let frame = caller.seal(&Data::Text(text.clone())).unwrap();
            assert!(!String::from_utf8_lossy(&frame).contains("secret"));
            assert_eq!(target.open(&frame).unwrap(), Data::Text(text));
            let bytes = vec![i as u8; 1000];
            let frame = target.seal(&Data::Binary(bytes.clone())).unwrap();
            assert_eq!(frame.len(), bytes.len() + OVERHEAD);
            assert_eq!(caller.open(&frame).unwrap(), Data::Binary(bytes));
        }
    }
    #[test]
    fn tamper_replay_reorder_reflection_and_wrong_key_are_rejected() {
        let (mut caller, mut target) = pair();
        let first = caller.seal(&Data::Text("one".into())).unwrap();
        let second = caller.seal(&Data::Text("two".into())).unwrap();
        // Tamper: any flipped bit fails, and the stream is then unusable.
        let mut flipped = first.clone();
        flipped[3] ^= 1;
        assert!(target.open(&flipped).is_err());
        // Reorder: delivering the second frame first fails.
        let (mut caller2, mut target2) = pair();
        let a = caller2.seal(&Data::Text("a".into())).unwrap();
        let b = caller2.seal(&Data::Text("b".into())).unwrap();
        assert!(target2.open(&b).is_err());
        let _ = a;
        // Replay: a frame accepted once is not accepted again.
        let (mut caller3, mut target3) = pair();
        let once = caller3.seal(&Data::Binary(vec![1, 2, 3])).unwrap();
        target3.open(&once).unwrap();
        assert!(target3.open(&once).is_err());
        // Reflection: a caller frame sent back to the caller fails (direction keys).
        let (mut caller4, _) = pair();
        let mine = caller4.seal(&Data::Text("echo".into())).unwrap();
        assert!(caller4.open(&mine).is_err());
        // Wrong key / other path.
        let secret = Secret::random();
        let mut sender = Cipher::caller(&secret, PATH);
        let frame = sender.seal(&Data::Text("x".into())).unwrap();
        assert!(Cipher::target(&Secret::random(), PATH).open(&frame).is_err());
        assert!(
            Cipher::target(&secret, "/api/v1/mesh/nodes/x/desktop/ws")
                .open(&frame)
                .is_err()
        );
        assert!(Cipher::target(&secret, PATH).open(&frame).is_ok());
        let _ = (first, second);
    }
    #[test]
    fn secrets_round_trip_as_hex() {
        let secret = Secret::random();
        assert_eq!(Secret::from_hex(&secret.to_hex()).unwrap().0, secret.0);
        assert!(Secret::from_hex("abcd").is_err());
        assert_eq!(format!("{secret:?}"), "Secret(..)");
    }
}
