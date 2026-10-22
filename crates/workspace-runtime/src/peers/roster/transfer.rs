//! Owner key transfer for the SAS-verified pairing coordinator. Never a public mesh endpoint.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use hpke::{
    Deserializable, Kem, OpModeR, OpModeS, Serializable, aead::ChaCha20Poly1305, kdf::HkdfSha256,
    kem::X25519HkdfSha256,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;
type Key = X25519HkdfSha256;
const INFO: &[u8] = b"cranibox-owner-signing-key-v1";
fn hpke_error(e: hpke::HpkeError) -> anyhow::Error {
    anyhow::anyhow!("owner key encryption failed: {e}")
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealedOwnerKey {
    pub owner_pub: String,
    pub recipient: String,
    pub sealed: String,
}
fn aad(owner: &str, recipient: &str) -> Vec<u8> {
    [owner.as_bytes(), b"/", recipient.as_bytes()].concat()
}
impl OwnerKey {
    /// Call only after the recipient identity and encryption key were verified during pairing.
    pub fn seal_for(&self, recipient: &RosterMember) -> Result<SealedOwnerKey> {
        let public = <Key as Kem>::PublicKey::from_bytes(&unhex::<32>(&recipient.x25519_pub)?)
            .map_err(hpke_error)?;
        let (enc, mut sender) = hpke::setup_sender::<ChaCha20Poly1305, HkdfSha256, Key, _>(
            &OpModeS::Base,
            &public,
            INFO,
            &mut rand08::rngs::OsRng,
        )
        .map_err(hpke_error)?;
        let owner_pub = self.public();
        let sealed = self
            .with_seed(|seed| sender.seal(seed, &aad(&owner_pub, &recipient.node_id)))
            .map_err(hpke_error)?;
        Ok(SealedOwnerKey {
            owner_pub,
            recipient: recipient.node_id.clone(),
            sealed: B64.encode([enc.to_bytes().as_slice(), &sealed].concat()),
        })
    }
    /// The expected owner must be SAS-pinned separately; never trust the envelope's owner alone.
    pub fn install_sealed(
        path: &Path,
        expected_owner: &str,
        recipient: &str,
        private: &[u8; 32],
        envelope: &SealedOwnerKey,
    ) -> Result<Self> {
        ensure!(
            envelope.owner_pub == expected_owner,
            "owner changed during pairing"
        );
        ensure!(envelope.recipient == recipient, "wrong owner key recipient");
        ensure!(envelope.sealed.len() <= 256, "owner key envelope too large");
        let bytes = B64.decode(&envelope.sealed)?;
        ensure!(bytes.len() == 80, "invalid owner key envelope");
        let private = <Key as Kem>::PrivateKey::from_bytes(private).map_err(hpke_error)?;
        let enc = <Key as Kem>::EncappedKey::from_bytes(&bytes[..32]).map_err(hpke_error)?;
        let mut receiver = hpke::setup_receiver::<ChaCha20Poly1305, HkdfSha256, Key>(
            &OpModeR::Base,
            &private,
            &enc,
            INFO,
        )
        .map_err(hpke_error)?;
        let seed = Zeroizing::new(
            receiver
                .open(&bytes[32..], &aad(expected_owner, recipient))
                .map_err(hpke_error)?,
        );
        let seed: &[u8; 32] = seed.as_slice().try_into().context("invalid owner seed")?;
        let key = Self(SigningKey::from_bytes(seed));
        ensure!(
            key.public() == expected_owner,
            "owner key fingerprint mismatch"
        );
        match fs::read(path) {
            Ok(existing) => {
                let existing = Zeroizing::new(existing);
                ensure!(existing.as_slice() == seed, "existing owner key differs");
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                crate::config::write_private(path, seed, false)?;
                #[cfg(unix)]
                fs::File::open(path.parent().context("owner key directory")?)?.sync_all()?;
            }
            Err(e) => return Err(e.into()),
        }
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_transfer_is_bound_to_the_verified_owner_and_recipient() {
        let root = tempfile::tempdir().unwrap();
        let owner = OwnerKey::load_or_create(&root.path().join("source")).unwrap();
        let (private, public) = Key::gen_keypair(&mut rand08::rngs::OsRng);
        let member = RosterMember {
            node_id: "recipient".into(),
            signing_key: owner.public(),
            x25519_pub: hex(public.to_bytes().as_slice()),
            role: Role::Node,
            priority: 1,
            is_claude_node: false,
            name: "Laptop".into(),
            added_at: 1,
            identity: None,
        };
        let envelope = owner.seal_for(&member).unwrap();
        let private: [u8; 32] = private.to_bytes().as_slice().try_into().unwrap();
        let path = root.path().join("target");
        assert!(
            OwnerKey::install_sealed(&path, &"00".repeat(32), "recipient", &private, &envelope)
                .is_err()
        );
        assert!(
            OwnerKey::install_sealed(&path, &owner.public(), "guest", &private, &envelope).is_err()
        );
        assert!(
            OwnerKey::install_sealed(&path, &owner.public(), "recipient", &[9; 32], &envelope)
                .is_err()
        );
        assert!(!path.exists());
        let received =
            OwnerKey::install_sealed(&path, &owner.public(), "recipient", &private, &envelope)
                .unwrap();
        assert_eq!(received.public(), owner.public());
        assert_eq!(
            OwnerKey::load_or_create(&path).unwrap().public(),
            owner.public()
        );
        OwnerKey::install_sealed(&path, &owner.public(), "recipient", &private, &envelope).unwrap();
        let mut corrupted = envelope.clone();
        let mut bytes = B64.decode(&corrupted.sealed).unwrap();
        bytes[45] ^= 1;
        corrupted.sealed = B64.encode(bytes);
        assert!(
            OwnerKey::install_sealed(&path, &owner.public(), "recipient", &private, &corrupted)
                .is_err()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
