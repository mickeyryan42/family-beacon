//! Turning readable envelopes into sealed ones, and back.
//!
//! * Household and neighborhood envelopes are encrypted with the group's
//!   symmetric key.
//! * Direct envelopes are encrypted to the recipient's X25519 key.
//! * The plaintext is padded to a fixed bucket, so relays learn only a coarse
//!   size.
//! * The route tag is an HMAC over the envelope's id and expiry, keyed with
//!   the group key (or the recipient's public key). Holders of the key can
//!   recognize their envelopes without trial-decrypting everything; to anyone
//!   else every tag looks random and unrelated to every other tag.
//!
//! The routing header (id, scope, expiry) is bound to the ciphertext as
//! associated data, so a relay can't move a body under a different header.
//! `hops_remaining` and `priority_class` are deliberately *not* bound: relays
//! need to lower them.

use beacon_crypto::{
    open, open_from, pad, route_tag, seal, seal_to, unpad, EncryptionKeypair, SymmetricKey,
};
use beacon_protocol::{
    decode_envelope, encode_envelope, Envelope, SealedEnvelope, ValidationError, PROTOCOL_VERSION,
};

use crate::error::SyncError;

/// Who an envelope is being sealed for.
pub enum SealFor<'a> {
    Group(&'a SymmetricKey),
    /// The recipient's X25519 public key.
    Person(&'a [u8]),
}

pub fn seal_envelope(envelope: &Envelope, to: SealFor<'_>) -> Result<SealedEnvelope, SyncError> {
    let scope = envelope.scope();
    let plaintext = pad(&encode_envelope(envelope)?)?;
    let aad = SealedEnvelope::associated_data(&envelope.id, scope, envelope.expires_at);
    let ctx = SealedEnvelope::tag_context(&envelope.id, envelope.expires_at);
    let (body, tag) = match to {
        SealFor::Group(key) => (
            seal(key, &aad, &plaintext),
            route_tag(key.as_bytes(), scope.label(), &ctx),
        ),
        SealFor::Person(public) => (
            seal_to(public, &aad, &plaintext)?,
            route_tag(public, scope.label(), &ctx),
        ),
    };
    Ok(SealedEnvelope {
        version: PROTOCOL_VERSION,
        id: envelope.id,
        scope,
        route_tag: tag.to_vec(),
        expires_at: envelope.expires_at,
        hops_remaining: scope.default_hops(),
        priority_class: envelope.priority().into(),
        body,
    })
}

fn expected_tag(sealed: &SealedEnvelope, key_material: &[u8]) -> [u8; 16] {
    route_tag(
        key_material,
        sealed.scope.label(),
        &SealedEnvelope::tag_context(&sealed.id, sealed.expires_at),
    )
}

/// Is this envelope for holders of `key`?
pub fn is_for_group(sealed: &SealedEnvelope, key: &SymmetricKey) -> bool {
    sealed.route_tag == expected_tag(sealed, key.as_bytes())
}

/// Is this envelope a direct message to the owner of `encryption_public`?
pub fn is_for_person(sealed: &SealedEnvelope, encryption_public: &[u8]) -> bool {
    sealed.route_tag == expected_tag(sealed, encryption_public)
}

fn finish(sealed: &SealedEnvelope, padded: Vec<u8>) -> Result<Envelope, SyncError> {
    let envelope = decode_envelope(&unpad(&padded)?)?;
    // The header is authenticated, but check the inner copy agrees with it so
    // an author can't send one header and a different body.
    if envelope.id != sealed.id
        || envelope.expires_at != sealed.expires_at
        || envelope.scope() != sealed.scope
    {
        return Err(ValidationError::BadHeader.into());
    }
    Ok(envelope)
}

pub fn open_group(sealed: &SealedEnvelope, key: &SymmetricKey) -> Result<Envelope, SyncError> {
    let aad = SealedEnvelope::associated_data(&sealed.id, sealed.scope, sealed.expires_at);
    finish(sealed, open(key, &aad, &sealed.body)?)
}

pub fn open_direct(sealed: &SealedEnvelope, me: &EncryptionKeypair) -> Result<Envelope, SyncError> {
    let aad = SealedEnvelope::associated_data(&sealed.id, sealed.scope, sealed.expires_at);
    finish(sealed, open_from(me, &aad, &sealed.body)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use beacon_protocol::{Audience, DeviceId, HouseholdId, NewEnvelope, Payload, PersonId, Scope};

    fn note(audience: Audience, text: &str) -> Envelope {
        Envelope::new(NewEnvelope {
            audience,
            author: PersonId::new(),
            origin_device: DeviceId::new(),
            lamport: 1,
            now: 1_000,
            payload: Payload::Note { text: text.into() },
        })
    }

    #[test]
    fn group_envelopes_round_trip_and_hide_their_content() {
        let key = SymmetricKey::generate();
        let env = note(
            Audience::Household {
                household_id: HouseholdId::new(),
            },
            "Phones charged",
        );
        let sealed = seal_envelope(&env, SealFor::Group(&key)).unwrap();

        assert_eq!(sealed.scope, Scope::Household);
        assert_eq!(sealed.hops_remaining, 12);
        assert!(!String::from_utf8_lossy(&sealed.body).contains("Phones"));
        assert!(is_for_group(&sealed, &key));
        assert!(!is_for_group(&sealed, &SymmetricKey::generate()));
        assert_eq!(open_group(&sealed, &key).unwrap(), env);
    }

    #[test]
    fn short_and_long_messages_look_the_same_size() {
        let key = SymmetricKey::generate();
        let audience = Audience::Household {
            household_id: HouseholdId::new(),
        };
        let a = seal_envelope(&note(audience, "ok"), SealFor::Group(&key)).unwrap();
        let b = seal_envelope(&note(audience, &"x".repeat(150)), SealFor::Group(&key)).unwrap();
        assert_eq!(a.body.len(), b.body.len());
    }

    #[test]
    fn direct_envelopes_open_only_for_the_recipient() {
        let dana = EncryptionKeypair::generate();
        let eve = EncryptionKeypair::generate();
        let env = Envelope::new(NewEnvelope {
            audience: Audience::Direct {
                recipient: PersonId::new(),
            },
            author: PersonId::new(),
            origin_device: DeviceId::new(),
            lamport: 1,
            now: 1_000,
            payload: Payload::Message {
                text: "Are you home?".into(),
            },
        });
        let sealed = seal_envelope(&env, SealFor::Person(&dana.public_bytes())).unwrap();
        assert!(is_for_person(&sealed, &dana.public_bytes()));
        assert!(!is_for_person(&sealed, &eve.public_bytes()));
        assert_eq!(open_direct(&sealed, &dana).unwrap(), env);
        assert!(open_direct(&sealed, &eve).is_err());
    }

    #[test]
    fn tampered_headers_are_rejected() {
        let key = SymmetricKey::generate();
        let env = note(
            Audience::Household {
                household_id: HouseholdId::new(),
            },
            "hi",
        );
        let mut sealed = seal_envelope(&env, SealFor::Group(&key)).unwrap();
        sealed.expires_at += 1;
        assert!(open_group(&sealed, &key).is_err());

        // Relays may lower hops and priority without breaking the seal.
        let mut relayed = seal_envelope(&env, SealFor::Group(&key)).unwrap();
        relayed.hops_remaining = 1;
        relayed.priority_class = beacon_protocol::PriorityClass::Low;
        assert!(open_group(&relayed, &key).is_ok());
    }
}
