//! Family Beacon cryptography.
//!
//! Small, opinionated wrappers around audited RustCrypto / dalek crates. Nothing
//! here does I/O or keeps state; callers own their keys.
//!
//! | Need | Primitive |
//! | --- | --- |
//! | Who wrote this? | Ed25519 signatures ([`IdentityKeypair`]) |
//! | Readable by a group (household, neighborhood) | XChaCha20-Poly1305 with a shared [`SymmetricKey`] |
//! | Readable by one person | X25519 + HKDF-SHA256 + XChaCha20-Poly1305 ([`seal_to`]) |
//! | Can I read this envelope? (without decrypting) | HMAC-SHA256 [`route_tag`] |
//! | Hide exact message sizes from relays | [`pad`] to fixed buckets |

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use ed25519_dalek::{Signer, Verifier};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use thiserror::Error;
use zeroize::Zeroizing;

pub const KEY_LEN: usize = 32;
pub const SIGNATURE_LEN: usize = 64;
pub const ROUTE_TAG_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const AEAD_TAG_LEN: usize = 16;

/// Sealed sizes are padded up to one of these, so relays can't tell a one-word
/// note from a long alert. The 4-byte length prefix counts toward the bucket.
pub const PAD_BUCKETS: [usize; 3] = [512, 1024, 4096];
/// Largest plaintext that fits in the biggest bucket.
pub const MAX_PADDED_PLAINTEXT: usize = PAD_BUCKETS[2] - 4;
/// Largest sealed body any scope can produce (direct adds an ephemeral key).
pub const MAX_SEALED_BODY: usize = KEY_LEN + NONCE_LEN + PAD_BUCKETS[2] + AEAD_TAG_LEN;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CryptoError {
    #[error("key or signature has the wrong length")]
    BadLength,
    #[error("signature does not verify")]
    BadSignature,
    #[error("could not decrypt: wrong key or tampered data")]
    DecryptFailed,
    #[error("message too large to seal ({0} bytes)")]
    TooLarge(usize),
    #[error("padding is malformed")]
    BadPadding,
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    OsRng.fill_bytes(&mut out);
    out
}

fn to_array(bytes: &[u8]) -> Result<[u8; KEY_LEN], CryptoError> {
    bytes.try_into().map_err(|_| CryptoError::BadLength)
}

// --- identity (signing) ---

/// A person's long-term signing key. On device this lives in the OS keystore.
pub struct IdentityKeypair(ed25519_dalek::SigningKey);

impl IdentityKeypair {
    pub fn generate() -> Self {
        Self(ed25519_dalek::SigningKey::generate(&mut OsRng))
    }

    pub fn from_secret_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        Ok(Self(ed25519_dalek::SigningKey::from_bytes(&to_array(
            bytes,
        )?)))
    }

    pub fn secret_bytes(&self) -> Zeroizing<[u8; KEY_LEN]> {
        Zeroizing::new(self.0.to_bytes())
    }

    pub fn public_bytes(&self) -> [u8; KEY_LEN] {
        self.0.verifying_key().to_bytes()
    }

    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.0.sign(message).to_bytes().to_vec()
    }
}

impl std::fmt::Debug for IdentityKeypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "IdentityKeypair(public={:02x?})",
            &self.public_bytes()[..4]
        )
    }
}

pub fn verify(public_key: &[u8], message: &[u8], signature: &[u8]) -> Result<(), CryptoError> {
    let key = ed25519_dalek::VerifyingKey::from_bytes(&to_array(public_key)?)
        .map_err(|_| CryptoError::BadLength)?;
    let sig_bytes: [u8; SIGNATURE_LEN] =
        signature.try_into().map_err(|_| CryptoError::BadLength)?;
    key.verify(message, &ed25519_dalek::Signature::from_bytes(&sig_bytes))
        .map_err(|_| CryptoError::BadSignature)
}

// --- encryption to one person ---

/// A person's long-term key for receiving direct messages.
pub struct EncryptionKeypair(x25519_dalek::StaticSecret);

impl EncryptionKeypair {
    pub fn generate() -> Self {
        Self(x25519_dalek::StaticSecret::random_from_rng(OsRng))
    }

    pub fn from_secret_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        Ok(Self(x25519_dalek::StaticSecret::from(to_array(bytes)?)))
    }

    pub fn secret_bytes(&self) -> Zeroizing<[u8; KEY_LEN]> {
        Zeroizing::new(self.0.to_bytes())
    }

    pub fn public_bytes(&self) -> [u8; KEY_LEN] {
        x25519_dalek::PublicKey::from(&self.0).to_bytes()
    }
}

impl std::fmt::Debug for EncryptionKeypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "EncryptionKeypair(public={:02x?})",
            &self.public_bytes()[..4]
        )
    }
}

// --- group encryption ---

/// Shared key for a household or one neighborhood epoch.
#[derive(Clone, PartialEq, Eq)]
pub struct SymmetricKey(Zeroizing<[u8; KEY_LEN]>);

impl SymmetricKey {
    pub fn generate() -> Self {
        Self(Zeroizing::new(random_bytes()))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        Ok(Self(Zeroizing::new(to_array(bytes)?)))
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl std::fmt::Debug for SymmetricKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SymmetricKey(..)")
    }
}

fn aead_seal(key: &[u8; KEY_LEN], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce: [u8; NONCE_LEN] = random_bytes();
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        // Encryption only fails for plaintexts larger than ~256 GiB.
        .unwrap_or_default();
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    out
}

fn aead_open(key: &[u8; KEY_LEN], aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() < NONCE_LEN + AEAD_TAG_LEN {
        return Err(CryptoError::DecryptFailed);
    }
    let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
    XChaCha20Poly1305::new(key.into())
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| CryptoError::DecryptFailed)
}

/// Encrypt for everyone holding `key`. `aad` is authenticated but not hidden.
pub fn seal(key: &SymmetricKey, aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    aead_seal(key.as_bytes(), aad, plaintext)
}

pub fn open(key: &SymmetricKey, aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
    aead_open(key.as_bytes(), aad, sealed)
}

const DIRECT_INFO: &[u8] = b"family-beacon direct v1";

fn direct_key(shared: &[u8; KEY_LEN], ephemeral: &[u8], recipient: &[u8]) -> [u8; KEY_LEN] {
    let mut salt = Vec::with_capacity(2 * KEY_LEN);
    salt.extend_from_slice(ephemeral);
    salt.extend_from_slice(recipient);
    let mut key = [0u8; KEY_LEN];
    // Expanding 32 bytes from HKDF-SHA256 cannot fail.
    let _ = Hkdf::<Sha256>::new(Some(&salt), shared).expand(DIRECT_INFO, &mut key);
    key
}

/// Encrypt so that only the holder of `recipient_public`'s secret can read it.
/// Output: ephemeral public key || nonce || ciphertext.
pub fn seal_to(
    recipient_public: &[u8],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let recipient = x25519_dalek::PublicKey::from(to_array(recipient_public)?);
    let ephemeral = x25519_dalek::EphemeralSecret::random_from_rng(OsRng);
    let ephemeral_public = x25519_dalek::PublicKey::from(&ephemeral);
    let shared = Zeroizing::new(ephemeral.diffie_hellman(&recipient).to_bytes());
    let key = Zeroizing::new(direct_key(
        &shared,
        ephemeral_public.as_bytes(),
        recipient.as_bytes(),
    ));

    let mut out = ephemeral_public.as_bytes().to_vec();
    out.extend_from_slice(&aead_seal(&key, aad, plaintext));
    Ok(out)
}

pub fn open_from(
    recipient: &EncryptionKeypair,
    aad: &[u8],
    sealed: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() < KEY_LEN {
        return Err(CryptoError::DecryptFailed);
    }
    let (ephemeral, rest) = sealed.split_at(KEY_LEN);
    let ephemeral = x25519_dalek::PublicKey::from(to_array(ephemeral)?);
    let shared = Zeroizing::new(recipient.0.diffie_hellman(&ephemeral).to_bytes());
    let key = Zeroizing::new(direct_key(
        &shared,
        ephemeral.as_bytes(),
        &recipient.public_bytes(),
    ));
    aead_open(&key, aad, rest)
}

// --- routing ---

/// A 16-byte tag that lets holders of `key_material` recognize an envelope as
/// theirs without trial decryption. `context` is per-envelope (its id and
/// expiry), so tags never repeat and outsiders can't link envelopes together.
pub fn route_tag(key_material: &[u8], label: &[u8], context: &[u8]) -> [u8; ROUTE_TAG_LEN] {
    // HMAC accepts keys of any length, so the error branch is unreachable.
    let Ok(mut mac) = <Hmac<Sha256> as Mac>::new_from_slice(key_material) else {
        return [0u8; ROUTE_TAG_LEN];
    };
    mac.update(label);
    mac.update(context);
    let full = mac.finalize().into_bytes();
    let mut tag = [0u8; ROUTE_TAG_LEN];
    tag.copy_from_slice(&full[..ROUTE_TAG_LEN]);
    tag
}

// --- padding ---

pub fn pad(plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let needed = plaintext.len() + 4;
    let bucket = PAD_BUCKETS
        .iter()
        .copied()
        .find(|b| *b >= needed)
        .ok_or(CryptoError::TooLarge(plaintext.len()))?;
    let mut out = Vec::with_capacity(bucket);
    out.extend_from_slice(&(plaintext.len() as u32).to_le_bytes());
    out.extend_from_slice(plaintext);
    out.resize(bucket, 0);
    Ok(out)
}

pub fn unpad(padded: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let (len, rest) = padded.split_at_checked(4).ok_or(CryptoError::BadPadding)?;
    let len = u32::from_le_bytes(len.try_into().map_err(|_| CryptoError::BadPadding)?) as usize;
    rest.get(..len)
        .map(<[u8]>::to_vec)
        .ok_or(CryptoError::BadPadding)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_verify_and_reject_tampering() {
        let id = IdentityKeypair::generate();
        let sig = id.sign(b"I'm safe");
        assert_eq!(verify(&id.public_bytes(), b"I'm safe", &sig), Ok(()));
        assert_eq!(
            verify(&id.public_bytes(), b"I'm not safe", &sig),
            Err(CryptoError::BadSignature)
        );
        let other = IdentityKeypair::generate();
        assert_eq!(
            verify(&other.public_bytes(), b"I'm safe", &sig),
            Err(CryptoError::BadSignature)
        );
    }

    #[test]
    fn identity_round_trips_through_secret_bytes() {
        let id = IdentityKeypair::generate();
        let restored = IdentityKeypair::from_secret_bytes(&*id.secret_bytes()).unwrap();
        assert_eq!(restored.public_bytes(), id.public_bytes());
    }

    #[test]
    fn group_seal_needs_the_key_and_the_same_aad() {
        let key = SymmetricKey::generate();
        let sealed = seal(&key, b"header", b"Power is out on Elm St");
        assert_eq!(
            open(&key, b"header", &sealed).unwrap(),
            b"Power is out on Elm St"
        );
        assert_eq!(
            open(&key, b"other header", &sealed),
            Err(CryptoError::DecryptFailed)
        );
        assert_eq!(
            open(&SymmetricKey::generate(), b"header", &sealed),
            Err(CryptoError::DecryptFailed)
        );
    }

    #[test]
    fn direct_seal_is_readable_only_by_the_recipient() {
        let dana = EncryptionKeypair::generate();
        let eve = EncryptionKeypair::generate();
        let sealed = seal_to(&dana.public_bytes(), b"h", b"Are you OK?").unwrap();
        assert_eq!(open_from(&dana, b"h", &sealed).unwrap(), b"Are you OK?");
        assert_eq!(
            open_from(&eve, b"h", &sealed),
            Err(CryptoError::DecryptFailed)
        );
    }

    #[test]
    fn route_tags_depend_on_key_and_context() {
        let a = route_tag(b"key-a", b"household", b"envelope-1");
        assert_eq!(a, route_tag(b"key-a", b"household", b"envelope-1"));
        assert_ne!(a, route_tag(b"key-b", b"household", b"envelope-1"));
        assert_ne!(a, route_tag(b"key-a", b"household", b"envelope-2"));
        assert_ne!(a, route_tag(b"key-a", b"neighborhood", b"envelope-1"));
    }

    #[test]
    fn padding_hides_length_within_a_bucket() {
        let short = pad(b"ok").unwrap();
        let longer = pad(&[7u8; 300]).unwrap();
        assert_eq!(short.len(), 512);
        assert_eq!(longer.len(), 512);
        assert_eq!(pad(&[0u8; 600]).unwrap().len(), 1024);
        assert_eq!(unpad(&short).unwrap(), b"ok");
        assert!(matches!(pad(&[0u8; 5000]), Err(CryptoError::TooLarge(_))));
        assert_eq!(unpad(&[1, 0]), Err(CryptoError::BadPadding));
    }
}
