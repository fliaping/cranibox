//! Persistent node identity.
//!
//! Every node owns an Ed25519 signing key and a separate X25519 Noise static key. The
//! public signing key derives new NodeIDs (`node-` + 128-bit key hash, the same shape as
//! the legacy random IDs so every existing validator keeps working), signs the Noise key
//! binding, and signs target-issued grant records. Legacy random NodeIDs keep their value
//! and are bound to a key by the same self-signature, so existing routes and grants keep
//! working; peers pin the key on first contact either way.
//! Private key material stays in `node-identity.json` (0600) and is never exported.
use crate::{config, protocol::valid_node_id};
use anyhow::{Context, Result, ensure};
use data_encoding::BASE32_NOPAD;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const FILE: &str = "node-identity.json";
/// NodeIDs (legacy random and key-derived) share this prefix.
pub const PREFIX: &str = "node-";
pub(crate) const NOISE_PARAMS: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn unhex<const N: usize>(value: &str) -> Result<[u8; N]> {
    ensure!(
        value.len() == N * 2 && value.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid hexadecimal key material"
    );
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16)?;
    }
    Ok(out)
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// `node-` + the first 128 bits (32 hex) of SHA-256 over the Ed25519 public key.
pub fn derive_node_id(signing_key: &[u8; 32]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"cranibox-node-id-v1\0");
    hasher.update(signing_key);
    format!("{PREFIX}{}", hex(&hasher.finalize()[..16]))
}

/// Public, self-signed identity that any node may publish and pin.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PublicIdentity {
    pub version: u32,
    pub node_id: String,
    /// Ed25519 public key, hexadecimal.
    pub signing_key: String,
    /// X25519 Noise static public key, hexadecimal.
    pub noise_key: String,
    /// Ed25519 signature over the identity binding, hexadecimal.
    pub signature: String,
}
impl PublicIdentity {
    fn binding(node_id: &str, signing: &[u8; 32], noise: &[u8; 32]) -> Vec<u8> {
        let mut message = b"cranibox-identity-v1\0".to_vec();
        message.extend_from_slice(node_id.as_bytes());
        message.push(0);
        message.extend_from_slice(signing);
        message.extend_from_slice(noise);
        message
    }
    pub fn verifying_key(&self) -> Result<VerifyingKey> {
        VerifyingKey::from_bytes(&unhex::<32>(&self.signing_key)?).context("invalid signing key")
    }
    pub fn noise_bytes(&self) -> Result<[u8; 32]> {
        unhex::<32>(&self.noise_key)
    }
    /// True when the NodeID is derived from the signing key (self-certifying).
    pub fn derived(&self) -> bool {
        unhex::<32>(&self.signing_key).is_ok_and(|key| derive_node_id(&key) == self.node_id)
    }
    pub fn verify(&self) -> Result<()> {
        ensure!(
            self.version == 1 && valid_node_id(&self.node_id),
            "unsupported node identity"
        );
        let signing = unhex::<32>(&self.signing_key)?;
        let noise = unhex::<32>(&self.noise_key)?;
        let signature = Signature::from_bytes(&unhex::<64>(&self.signature)?);
        self.verifying_key()?
            .verify(&Self::binding(&self.node_id, &signing, &noise), &signature)
            .context("node identity signature is invalid")
    }
    /// Short human-readable key fingerprint for lists and logs (not the pairing code).
    pub fn fingerprint(&self) -> String {
        let digest = Sha256::digest(self.signing_key.as_bytes());
        let text = BASE32_NOPAD.encode(&digest[..10]).to_ascii_lowercase();
        format!(
            "{}-{}-{}-{}",
            &text[0..4],
            &text[4..8],
            &text[8..12],
            &text[12..16]
        )
    }
    pub fn same_keys(&self, other: &Self) -> bool {
        self.node_id == other.node_id
            && self.signing_key == other.signing_key
            && self.noise_key == other.noise_key
    }
}

/// Short authentication string both owners compare before marking a pairing verified.
/// It commits to both NodeIDs and all four public keys, independent of who initiated.
pub fn pairing_code(a: &PublicIdentity, b: &PublicIdentity) -> String {
    let mut parts = [a, b];
    parts.sort_by(|x, y| x.signing_key.cmp(&y.signing_key));
    let mut hasher = Sha256::new();
    hasher.update(b"cranibox-pairing-code-v1\0");
    for part in parts {
        for field in [&part.node_id, &part.signing_key, &part.noise_key] {
            hasher.update(field.as_bytes());
            hasher.update([0]);
        }
    }
    let digest = hasher.finalize();
    // 40 bits rendered as 12 decimal digits; an attacker gets one online guess per pairing.
    let value = u64::from_be_bytes([
        0, 0, 0, digest[0], digest[1], digest[2], digest[3], digest[4],
    ]) % 1_000_000_000_000;
    let text = format!("{value:012}");
    format!("{}-{}-{}", &text[0..4], &text[4..8], &text[8..12])
}
pub fn normalize_code(code: &str) -> String {
    code.chars().filter(char::is_ascii_digit).collect()
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Secret {
    version: u32,
    node_id: String,
    signing_seed: String,
    noise_private: String,
    noise_public: String,
    created_at: u64,
}

/// Private identity. Only the owning node process loads it.
pub struct Identity {
    signing: SigningKey,
    noise_private: [u8; 32],
    public: PublicIdentity,
}
impl Identity {
    fn assemble(
        node_id: String,
        seed: [u8; 32],
        noise_private: [u8; 32],
        noise_public: [u8; 32],
    ) -> Result<Self> {
        ensure!(valid_node_id(&node_id), "invalid node ID");
        let signing = SigningKey::from_bytes(&seed);
        let key = signing.verifying_key().to_bytes();
        let signature = signing.sign(&PublicIdentity::binding(&node_id, &key, &noise_public));
        let public = PublicIdentity {
            version: 1,
            node_id,
            signing_key: hex(&key),
            noise_key: hex(&noise_public),
            signature: hex(&signature.to_bytes()),
        };
        public.verify()?;
        Ok(Self {
            signing,
            noise_private,
            public,
        })
    }
    fn generate(node_id: Option<&str>) -> Result<Self> {
        let mut seed = [0u8; 32];
        rand::rng().fill_bytes(&mut seed);
        let keypair = snow::Builder::new(NOISE_PARAMS.parse()?).generate_keypair()?;
        let noise_private: [u8; 32] = keypair
            .private
            .as_slice()
            .try_into()
            .context("invalid Noise key")?;
        let noise_public: [u8; 32] = keypair
            .public
            .as_slice()
            .try_into()
            .context("invalid Noise key")?;
        let node_id = match node_id {
            Some(id) => id.to_owned(),
            None => derive_node_id(&SigningKey::from_bytes(&seed).verifying_key().to_bytes()),
        };
        Self::assemble(node_id, seed, noise_private, noise_public)
    }
    pub fn public(&self) -> &PublicIdentity {
        &self.public
    }
    pub fn node_id(&self) -> &str {
        &self.public.node_id
    }
    pub fn noise_private(&self) -> &[u8; 32] {
        &self.noise_private
    }
    pub fn sign(&self, message: &[u8]) -> String {
        hex(&self.signing.sign(message).to_bytes())
    }
    fn write(&self, path: &Path) -> Result<()> {
        let secret = Secret {
            version: 1,
            node_id: self.public.node_id.clone(),
            signing_seed: hex(&self.signing.to_bytes()),
            noise_private: hex(&self.noise_private),
            noise_public: self.public.noise_key.clone(),
            created_at: now(),
        };
        // Never replace an existing key: a mismatch must stay visible for repair.
        config::write_private(path, &serde_json::to_vec_pretty(&secret)?, false)
    }
    pub fn load(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path).context("read node identity")?;
        ensure!(
            file.metadata()?.len() <= 4096,
            "node identity file too large"
        );
        let secret: Secret = serde_json::from_reader(file).context("invalid node identity file")?;
        ensure!(secret.version == 1, "unsupported node identity file");
        Self::assemble(
            secret.node_id,
            unhex::<32>(&secret.signing_seed)?,
            unhex::<32>(&secret.noise_private)?,
            unhex::<32>(&secret.noise_public)?,
        )
    }
    /// Load the identity bound to `node_id`, creating a key for a legacy NodeID once.
    /// Callers hold the peer configuration lock.
    pub fn ensure(config_path: &Path, node_id: &str) -> Result<Self> {
        let path = path_for(config_path);
        if path.try_exists()? {
            let identity = Self::load(&path)?;
            ensure!(
                identity.node_id() == node_id,
                "node identity file belongs to another NodeID; existing data was preserved"
            );
            return Ok(identity);
        }
        let identity = Self::generate(Some(node_id))?;
        identity.write(&path)?;
        Ok(identity)
    }
    /// A brand new node: create the key first and derive its NodeID from it.
    pub fn create_new(config_path: &Path) -> Result<Self> {
        let path = path_for(config_path);
        if path.try_exists()? {
            // A previous attempt created the key but not the configuration.
            return Self::load(&path);
        }
        let identity = Self::generate(None)?;
        identity.write(&path)?;
        Ok(identity)
    }
}

/// `peers.json` uses `node-identity.json`; other configuration names get their own file.
pub fn path_for(config_path: &Path) -> PathBuf {
    match config_path.file_name().and_then(|n| n.to_str()) {
        Some("peers.json") | None => config_path.with_file_name(FILE),
        Some(name) => config_path.with_file_name(format!("{name}.identity")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn derived_identity_is_self_certifying_and_tamper_evident() {
        let identity = Identity::generate(None).unwrap();
        let public = identity.public().clone();
        assert!(public.node_id.starts_with(PREFIX) && public.node_id.len() == 37);
        assert!(valid_node_id(&public.node_id) && public.derived());
        public.verify().unwrap();
        let other = Identity::generate(None).unwrap();
        // Swapping in another node's keys breaks the self-signature over the NodeID.
        let mut forged = public.clone();
        forged.signing_key = other.public().signing_key.clone();
        forged.signature = other.public().signature.clone();
        assert!(forged.verify().is_err());
        // Replacing only the Noise key breaks the signature.
        let mut forged = public.clone();
        forged.noise_key = other.public().noise_key.clone();
        assert!(forged.verify().is_err());
    }
    #[test]
    fn legacy_ids_keep_their_value_and_are_bound_by_signature() {
        let identity = Identity::generate(Some("node-0123abcd")).unwrap();
        assert_eq!(identity.node_id(), "node-0123abcd");
        assert!(!identity.public().derived());
        identity.public().verify().unwrap();
        let mut forged = identity.public().clone();
        forged.node_id = "node-other".into();
        assert!(
            forged.verify().is_err(),
            "the NodeID is covered by the signature"
        );
    }
    #[test]
    fn persistence_never_replaces_a_key() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("peers.json");
        let first = Identity::ensure(&config, "node-legacy").unwrap();
        let second = Identity::ensure(&config, "node-legacy").unwrap();
        assert_eq!(first.public(), second.public());
        assert!(Identity::ensure(&config, "node-another").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path_for(&config))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o077,
                0,
                "private key must not be group/world readable"
            );
        }
    }
    #[test]
    fn pairing_code_is_symmetric_and_key_bound() {
        let a = Identity::generate(None).unwrap();
        let b = Identity::generate(None).unwrap();
        let c = Identity::generate(None).unwrap();
        let ab = pairing_code(a.public(), b.public());
        assert_eq!(ab, pairing_code(b.public(), a.public()));
        assert_ne!(ab, pairing_code(a.public(), c.public()));
        assert_eq!(normalize_code(&ab).len(), 12);
    }
}
