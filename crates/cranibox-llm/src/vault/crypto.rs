//! RFC 9180 base-mode HPKE wraps and per-record XChaCha20-Poly1305.
use super::*;
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use hpke::{
    Deserializable, Kem as KemTrait, OpModeR, OpModeS, Serializable, aead::ChaCha20Poly1305,
    kdf::HkdfSha256, kem::X25519HkdfSha256,
};
use rand08::RngCore;
use zeroize::Zeroizing;
type Kem = X25519HkdfSha256;
const INFO: &[u8] = b"cranibox-dek-v1\0";
fn err() -> StoreError {
    other("vault authentication failed")
}
fn info(id: &str) -> Vec<u8> {
    [INFO, id.as_bytes()].concat()
}
pub fn wrap(public: &str, id: &str, secret: &[u8]) -> Result<String, StoreError> {
    let bytes = hex::decode(public).map_err(|_| err())?;
    let key = <Kem as KemTrait>::PublicKey::from_bytes(&bytes).map_err(|_| err())?;
    let (enc, mut sender) = hpke::setup_sender::<ChaCha20Poly1305, HkdfSha256, Kem, _>(
        &OpModeS::Base,
        &key,
        &info(id),
        &mut rand08::rngs::OsRng,
    )
    .map_err(|_| err())?;
    let sealed = sender.seal(secret, id.as_bytes()).map_err(|_| err())?;
    Ok(B64.encode([enc.to_bytes().as_slice(), &sealed].concat()))
}
pub fn unwrap(private: &[u8], id: &str, sealed: &str) -> Result<Zeroizing<Vec<u8>>, StoreError> {
    let bytes = B64.decode(sealed).map_err(|_| err())?;
    if bytes.len() < 48 {
        return Err(err());
    }
    let key = <Kem as KemTrait>::PrivateKey::from_bytes(private).map_err(|_| err())?;
    let enc = <Kem as KemTrait>::EncappedKey::from_bytes(&bytes[..32]).map_err(|_| err())?;
    let mut receiver = hpke::setup_receiver::<ChaCha20Poly1305, HkdfSha256, Kem>(
        &OpModeR::Base,
        &key,
        &enc,
        &info(id),
    )
    .map_err(|_| err())?;
    Ok(Zeroizing::new(
        receiver
            .open(&bytes[32..], id.as_bytes())
            .map_err(|_| err())?,
    ))
}
fn aad(a: &Account) -> Vec<u8> {
    [
        a.id.as_bytes(),
        &a.version.to_be_bytes(),
        &a.generation.to_be_bytes(),
    ]
    .concat()
}
pub fn seal(a: &mut Account, secret: &[u8], roster: &Roster) -> Result<(), StoreError> {
    let mut dek = Zeroizing::new([0u8; 32]);
    let mut nonce = [0u8; 24];
    rand08::rngs::OsRng.fill_bytes(dek.as_mut());
    rand08::rngs::OsRng.fill_bytes(&mut nonce);
    let cipher = XChaCha20Poly1305::new_from_slice(dek.as_ref()).map_err(|_| err())?;
    let sealed = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: secret,
                aad: &aad(a),
            },
        )
        .map_err(|_| err())?;
    a.sealed = B64.encode([nonce.as_slice(), &sealed].concat());
    a.wraps.clear();
    for member in &roster.members {
        if a.sync == SyncPolicy::Synced || member.node_id == a.home_node {
            a.wraps.insert(
                member.node_id.clone(),
                wrap(&member.x25519_pub, &a.id, dek.as_ref())?,
            );
        }
    }
    if !a.wraps.contains_key(&a.home_node) {
        return Err(other("account home is not a roster member"));
    }
    Ok(())
}
pub fn open(a: &Account, keys: &VaultKeys) -> Result<Zeroizing<Vec<u8>>, StoreError> {
    let wrap = a.wraps.get(&keys.node).ok_or(StoreError::NoSecret)?;
    let dek = unwrap(keys.encryption.expose(), &a.id, wrap)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&dek).map_err(|_| err())?;
    let sealed = B64.decode(&a.sealed).map_err(|_| err())?;
    if sealed.len() < 40 {
        return Err(err());
    }
    Ok(Zeroizing::new(
        cipher
            .decrypt(
                XNonce::from_slice(&sealed[..24]),
                Payload {
                    msg: &sealed[24..],
                    aad: &aad(a),
                },
            )
            .map_err(|_| err())?,
    ))
}
