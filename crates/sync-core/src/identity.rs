//! This device's keys, and checking other people's signatures.
//!
//! See `docs/security.md`. On a phone, the secret halves live in the OS
//! keystore (Secure Enclave / Android Keystore) and never leave it; here they
//! are plain values so the engine can be tested end to end.

use beacon_crypto::{verify, CryptoError, EncryptionKeypair, IdentityKeypair};
use beacon_protocol::{Envelope, Person, PersonId, PersonRole, Signature};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdentityError {
    #[error("envelope is missing a required signature")]
    MissingSignature,
    #[error("signature does not match the author's key")]
    BadSignature,
    #[error("no key on record for the author")]
    UnknownKey,
}

impl From<CryptoError> for IdentityError {
    fn from(_: CryptoError) -> Self {
        IdentityError::BadSignature
    }
}

/// The person using this device, with their keys.
#[derive(Debug)]
pub struct LocalIdentity {
    pub person_id: PersonId,
    pub display_name: String,
    pub signing: IdentityKeypair,
    pub encryption: EncryptionKeypair,
}

impl LocalIdentity {
    pub fn generate(display_name: impl Into<String>) -> Self {
        Self {
            person_id: PersonId::new(),
            display_name: display_name.into(),
            signing: IdentityKeypair::generate(),
            encryption: EncryptionKeypair::generate(),
        }
    }

    /// How this person appears in a household roster.
    pub fn as_person(&self, role: PersonRole) -> Person {
        Person {
            id: self.person_id,
            display_name: self.display_name.clone(),
            role,
            public_key: Some(self.signing.public_bytes().to_vec()),
            encryption_key: Some(self.encryption.public_bytes().to_vec()),
        }
    }

    pub fn sign(&self, envelope: &mut Envelope) {
        envelope.signature = Some(Signature(self.signing.sign(&envelope.signing_bytes())));
    }
}

/// Check that `envelope` was signed by the holder of `identity_key`.
pub fn verify_envelope(envelope: &Envelope, identity_key: &[u8]) -> Result<(), IdentityError> {
    let signature = envelope
        .signature
        .as_ref()
        .ok_or(IdentityError::MissingSignature)?;
    Ok(verify(
        identity_key,
        &envelope.signing_bytes(),
        &signature.0,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use beacon_protocol::{Audience, DeviceId, HouseholdId, NewEnvelope, Payload};

    fn note() -> Envelope {
        Envelope::new(NewEnvelope {
            audience: Audience::Household {
                household_id: HouseholdId::new(),
            },
            author: PersonId::new(),
            origin_device: DeviceId::new(),
            lamport: 1,
            now: 0,
            payload: Payload::Note { text: "hi".into() },
        })
    }

    #[test]
    fn signed_envelopes_verify_and_tampering_is_caught() {
        let me = LocalIdentity::generate("Ana");
        let mut env = note();
        assert_eq!(
            verify_envelope(&env, &me.signing.public_bytes()),
            Err(IdentityError::MissingSignature)
        );

        me.sign(&mut env);
        assert_eq!(verify_envelope(&env, &me.signing.public_bytes()), Ok(()));

        env.payload = Payload::Note {
            text: "changed".into(),
        };
        assert_eq!(
            verify_envelope(&env, &me.signing.public_bytes()),
            Err(IdentityError::BadSignature)
        );
    }
}
