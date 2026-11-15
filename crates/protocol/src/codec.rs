//! Byte encoding for envelopes and sync frames.
//!
//! JSON for now: readable while debugging and trivially inspected from the
//! React Native side. Binary fields (keys, signatures, sealed bodies) are
//! base64. All encode/decode goes through this module so switching to a
//! compact binary format (CBOR / postcard) for BLE is a one-file change,
//! gated by [`crate::PROTOCOL_VERSION`].

use thiserror::Error;

use crate::envelope::{Envelope, SealedEnvelope};
use crate::sync_message::SyncMessage;

/// Hard upper bound on any decoded frame. Checked before parsing so a
/// misbehaving peer cannot make us allocate arbitrarily.
pub const MAX_FRAME_BYTES: usize = 256 * 1024;

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("frame is {0} bytes, larger than the {MAX_FRAME_BYTES} byte limit")]
    TooLarge(usize),
    #[error("malformed frame: {0}")]
    Malformed(#[from] serde_json::Error),
}

pub fn encode_envelope(envelope: &Envelope) -> Result<Vec<u8>, CodecError> {
    Ok(serde_json::to_vec(envelope)?)
}

pub fn decode_envelope(bytes: &[u8]) -> Result<Envelope, CodecError> {
    check_len(bytes)?;
    Ok(serde_json::from_slice(bytes)?)
}

pub fn encode_sealed(envelope: &SealedEnvelope) -> Result<Vec<u8>, CodecError> {
    Ok(serde_json::to_vec(envelope)?)
}

pub fn decode_sealed(bytes: &[u8]) -> Result<SealedEnvelope, CodecError> {
    check_len(bytes)?;
    Ok(serde_json::from_slice(bytes)?)
}

pub fn encode_message(message: &SyncMessage) -> Result<Vec<u8>, CodecError> {
    Ok(serde_json::to_vec(message)?)
}

pub fn decode_message(bytes: &[u8]) -> Result<SyncMessage, CodecError> {
    check_len(bytes)?;
    Ok(serde_json::from_slice(bytes)?)
}

fn check_len(bytes: &[u8]) -> Result<(), CodecError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(CodecError::TooLarge(bytes.len()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{Audience, NewEnvelope, Payload, PriorityClass, Scope};
    use crate::ids::{DeviceId, EnvelopeId, HouseholdId, NeighborhoodId, PersonId, RendezvousId};
    use crate::model::{AlertCategory, CheckIn, CheckInState, GeoPoint, RendezvousPoint};
    use crate::summary::IdSummary;
    use crate::sync_message::Hello;

    fn envelope(audience: Audience, payload: Payload) -> Envelope {
        Envelope::new(NewEnvelope {
            audience,
            author: PersonId::new(),
            origin_device: DeviceId::new(),
            lamport: 7,
            now: 1_700_000_000_000,
            payload,
        })
    }

    fn household() -> Audience {
        Audience::Household {
            household_id: HouseholdId::new(),
        }
    }

    #[test]
    fn check_in_envelope_round_trips() {
        let original = envelope(
            household(),
            Payload::CheckIn(CheckIn {
                person_id: PersonId::new(),
                state: CheckInState::OnMyWay,
                note: Some("Leaving work now".into()),
                rendezvous_id: Some(RendezvousId::new()),
                recorded_at: 1_700_000_000_000,
            }),
        );
        let decoded = decode_envelope(&encode_envelope(&original).unwrap()).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn rendezvous_and_neighborhood_envelopes_round_trip() {
        let household_id = HouseholdId::new();
        let point = envelope(
            Audience::Household { household_id },
            Payload::RendezvousUpsert(RendezvousPoint {
                id: RendezvousId::new(),
                household_id,
                name: "Grandma's house".into(),
                details: "Side door, key under the planter".into(),
                location: Some(GeoPoint::from_degrees(47.6062, -122.3321)),
                when_to_go: "If we can't reach each other for 2 hours".into(),
                rank: 1,
            }),
        );
        let alert = envelope(
            Audience::Neighborhood {
                neighborhood_id: NeighborhoodId::new(),
            },
            Payload::Alert {
                category: AlertCategory::Power,
                text: "Transformer blew on Elm".into(),
                area: Some("Elm St, 400 block".into()),
            },
        );
        for original in [point, alert] {
            assert_eq!(
                decode_envelope(&encode_envelope(&original).unwrap()).unwrap(),
                original
            );
        }
    }

    #[test]
    fn wire_format_uses_stable_snake_case_tags() {
        let env = envelope(household(), Payload::Note { text: "hi".into() });
        let json: serde_json::Value =
            serde_json::from_slice(&encode_envelope(&env).unwrap()).unwrap();
        assert_eq!(json["audience"]["scope"], "household");
        assert_eq!(json["payload"]["type"], "note");
        assert_eq!(json["payload"]["data"]["text"], "hi");
        assert!(json.get("signature").is_none());
    }

    fn sealed() -> SealedEnvelope {
        SealedEnvelope {
            version: crate::PROTOCOL_VERSION,
            id: EnvelopeId::new(),
            scope: Scope::Neighborhood,
            route_tag: vec![1; 16],
            expires_at: 99,
            hops_remaining: 3,
            priority_class: PriorityClass::Normal,
            body: vec![0xAB; 512],
        }
    }

    #[test]
    fn sealed_envelopes_use_base64_bodies() {
        let s = sealed();
        let bytes = encode_sealed(&s).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(json["body"].is_string());
        assert!(
            bytes.len() < 900,
            "512-byte body encoded to {} bytes",
            bytes.len()
        );
        assert_eq!(decode_sealed(&bytes).unwrap(), s);
    }

    #[test]
    fn sync_messages_round_trip() {
        let s = sealed();
        let messages = vec![
            SyncMessage::Hello(Hello {
                protocol_version: crate::PROTOCOL_VERSION,
                device_id: DeviceId::new(),
                have: IdSummary::build([&s.id], 1, 3),
            }),
            SyncMessage::Deliver { envelopes: vec![s] },
            SyncMessage::Done,
            SyncMessage::Reject {
                reason: "unsupported protocol version".into(),
            },
        ];
        for message in messages {
            assert_eq!(
                decode_message(&encode_message(&message).unwrap()).unwrap(),
                message
            );
        }
    }

    #[test]
    fn rejects_malformed_and_oversized_frames() {
        assert!(matches!(
            decode_envelope(b"{not json"),
            Err(CodecError::Malformed(_))
        ));
        let huge = vec![b' '; MAX_FRAME_BYTES + 1];
        assert!(matches!(
            decode_message(&huge),
            Err(CodecError::TooLarge(_))
        ));
    }

    #[test]
    fn signing_bytes_cover_everything_but_the_signature() {
        let mut env = envelope(household(), Payload::Note { text: "hi".into() });
        let before = env.signing_bytes();
        env.signature = Some(crate::Signature(vec![1, 2, 3]));
        assert_eq!(env.signing_bytes(), before);
        env.lamport += 1;
        assert_ne!(env.signing_bytes(), before);
    }
}
