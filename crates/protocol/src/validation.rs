//! Structural validation.
//!
//! Two levels:
//!
//! * [`validate_sealed`]: checks any relay can do on the routing header,
//!   without being able to read the body.
//! * [`validate_envelope`]: checks a reader does after opening: right scope,
//!   right audience, author belongs to it, sane timestamps and sizes.
//!
//! Signature checks happen in `sync-core`, which knows the authors' keys.

use thiserror::Error;

use crate::envelope::{Audience, Envelope, Payload, SealedEnvelope};
use crate::ids::{NeighborhoodId, PersonId};
use crate::model::{Household, UnixMillis};
use crate::{codec, PROTOCOL_VERSION};

/// Largest sealed body any scope can produce: an ephemeral key (direct only),
/// a nonce, a 4 KiB padded plaintext and an AEAD tag.
pub const MAX_SEALED_BODY: usize = 32 + 24 + 4096 + 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidationLimits {
    /// Max encoded inner envelope size. Must fit the largest padding bucket.
    pub max_envelope_bytes: usize,
    pub max_note_chars: usize,
    pub max_name_chars: usize,
    /// How far in the future `created_at` may be. Generous because phones
    /// without network time drift, and rejecting an urgent check-in over a few
    /// minutes of skew would be the wrong trade-off.
    pub max_clock_skew_ms: u64,
}

impl Default for ValidationLimits {
    fn default() -> Self {
        Self {
            max_envelope_bytes: 4000,
            max_note_chars: 280,
            max_name_chars: 80,
            max_clock_skew_ms: 15 * 60 * 1000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u16),
    #[error("payload is not allowed in this scope")]
    ScopeMismatch,
    #[error("envelope belongs to a different household")]
    WrongHousehold,
    #[error("author is not a member of this household")]
    AuthorNotMember,
    #[error("check-in subject is not a member of this household")]
    SubjectNotMember,
    #[error("this device is not in that neighborhood")]
    NotInNeighborhood,
    #[error("author is not a member of the neighborhood")]
    NotNeighborhoodMember,
    #[error("membership event does not match its neighborhood or author")]
    BadMembershipEvent,
    #[error("direct message is addressed to someone else")]
    NotForMe,
    #[error("direct message from someone outside every circle we share")]
    UnknownSender,
    #[error("envelope has expired")]
    Expired,
    #[error("envelope expires before it was created")]
    InvalidExpiry,
    #[error("envelope timestamp is too far in the future")]
    FromTheFuture,
    #[error("envelope is {0} bytes, over the size limit")]
    TooLarge(usize),
    #[error("text field is too long")]
    TextTooLong,
    #[error("required text field is empty")]
    EmptyText,
    #[error("rendezvous belongs to a different household")]
    RendezvousWrongHousehold,
    #[error("invalid coordinates")]
    InvalidLocation,
    #[error("key has the wrong length")]
    BadKey,
    #[error("routing header is malformed")]
    BadHeader,
}

/// What a reading device knows about the circles it belongs to.
pub trait Membership {
    fn local_person(&self) -> PersonId;
    fn household(&self) -> &Household;
    /// Whether this device has joined the neighborhood at all.
    fn in_neighborhood(&self, neighborhood: &NeighborhoodId) -> bool;
    fn is_neighborhood_member(&self, neighborhood: &NeighborhoodId, person: &PersonId) -> bool;
    /// Whether `person` is in our household or any neighborhood we're in.
    fn knows(&self, person: &PersonId) -> bool;
}

/// Checks a relay can make without reading the body.
pub fn validate_sealed(sealed: &SealedEnvelope, now: UnixMillis) -> Result<(), ValidationError> {
    if sealed.version != PROTOCOL_VERSION {
        return Err(ValidationError::UnsupportedVersion(sealed.version));
    }
    if sealed.is_expired(now) {
        return Err(ValidationError::Expired);
    }
    if sealed.route_tag.len() != 16 || sealed.body.is_empty() {
        return Err(ValidationError::BadHeader);
    }
    if sealed.body.len() > MAX_SEALED_BODY {
        return Err(ValidationError::TooLarge(sealed.body.len()));
    }
    Ok(())
}

pub fn validate_envelope(
    envelope: &Envelope,
    membership: &dyn Membership,
    now: UnixMillis,
    limits: &ValidationLimits,
) -> Result<(), ValidationError> {
    if envelope.version != PROTOCOL_VERSION {
        return Err(ValidationError::UnsupportedVersion(envelope.version));
    }
    if envelope.payload.scope() != envelope.audience.scope() {
        return Err(ValidationError::ScopeMismatch);
    }
    check_audience(envelope, membership)?;

    if envelope.expires_at <= envelope.created_at {
        return Err(ValidationError::InvalidExpiry);
    }
    if envelope.is_expired(now) {
        return Err(ValidationError::Expired);
    }
    if envelope.created_at > now.saturating_add(limits.max_clock_skew_ms) {
        return Err(ValidationError::FromTheFuture);
    }
    let size = codec::encode_envelope(envelope)
        .map(|b| b.len())
        .unwrap_or(usize::MAX);
    if size > limits.max_envelope_bytes {
        return Err(ValidationError::TooLarge(size));
    }
    validate_payload(envelope, membership.household(), limits)
}

fn check_audience(envelope: &Envelope, m: &dyn Membership) -> Result<(), ValidationError> {
    match envelope.audience {
        Audience::Household { household_id } => {
            let household = m.household();
            if household_id != household.id {
                return Err(ValidationError::WrongHousehold);
            }
            if !household.is_member(&envelope.author) {
                return Err(ValidationError::AuthorNotMember);
            }
            if let Payload::CheckIn(c) = &envelope.payload {
                if !household.is_member(&c.person_id) {
                    return Err(ValidationError::SubjectNotMember);
                }
            }
        }
        Audience::Neighborhood { neighborhood_id } => {
            if !m.in_neighborhood(&neighborhood_id) {
                return Err(ValidationError::NotInNeighborhood);
            }
            match &envelope.payload {
                // A joiner announces their own certificate; whether the
                // invite itself is valid is decided by the membership rules.
                Payload::MemberJoined(cert) => {
                    if cert.neighborhood_id != neighborhood_id || cert.member != envelope.author {
                        return Err(ValidationError::BadMembershipEvent);
                    }
                }
                _ => {
                    if !m.is_neighborhood_member(&neighborhood_id, &envelope.author) {
                        return Err(ValidationError::NotNeighborhoodMember);
                    }
                }
            }
        }
        Audience::Direct { recipient } => {
            if recipient != m.local_person() {
                return Err(ValidationError::NotForMe);
            }
            if !m.knows(&envelope.author) {
                return Err(ValidationError::UnknownSender);
            }
        }
    }
    Ok(())
}

fn validate_payload(
    envelope: &Envelope,
    household: &Household,
    limits: &ValidationLimits,
) -> Result<(), ValidationError> {
    let note = limits.max_note_chars;
    let name = limits.max_name_chars;
    match &envelope.payload {
        Payload::CheckIn(check_in) => {
            if let Some(n) = &check_in.note {
                check_len(n, note)?;
            }
        }
        Payload::RendezvousUpsert(point) => {
            if point.household_id != household.id {
                return Err(ValidationError::RendezvousWrongHousehold);
            }
            check_required(&point.name, name)?;
            check_len(&point.details, note)?;
            check_len(&point.when_to_go, note)?;
            if point.location.is_some_and(|loc| !loc.is_valid()) {
                return Err(ValidationError::InvalidLocation);
            }
        }
        Payload::RendezvousRemove { .. } | Payload::Withdraw { .. } => {}
        Payload::Note { text }
        | Payload::Offer { text }
        | Payload::Need { text }
        | Payload::Message { text } => check_required(text, note)?,
        Payload::Alert { text, area, .. } => {
            check_required(text, note)?;
            if let Some(a) = area {
                check_len(a, name)?;
            }
        }
        Payload::HouseholdStatus {
            household_name,
            text,
            ..
        } => {
            check_required(household_name, name)?;
            if let Some(t) = text {
                check_len(t, note)?;
            }
        }
        Payload::WellnessRequest { text } | Payload::WellnessReply { text, .. } => {
            if let Some(t) = text {
                check_len(t, note)?;
            }
        }
        Payload::MemberJoined(cert) => {
            check_required(&cert.display_name, name)?;
            if cert.identity_key.len() != 32 || cert.encryption_key.len() != 32 {
                return Err(ValidationError::BadKey);
            }
        }
        Payload::MemberRemoved { .. } | Payload::StewardAdded { .. } => {}
        Payload::KeyRotation { key, .. } => {
            if key.len() != 32 {
                return Err(ValidationError::BadKey);
            }
        }
    }
    Ok(())
}

fn check_required(text: &str, max_chars: usize) -> Result<(), ValidationError> {
    if text.trim().is_empty() {
        return Err(ValidationError::EmptyText);
    }
    check_len(text, max_chars)
}

fn check_len(text: &str, max_chars: usize) -> Result<(), ValidationError> {
    if text.chars().count() > max_chars {
        return Err(ValidationError::TextTooLong);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::NewEnvelope;
    use crate::ids::{DeviceId, HouseholdId, RendezvousId};
    use crate::model::{
        AlertCategory, CheckIn, CheckInState, GeoPoint, MembershipCert, Person, PersonRole,
        RendezvousPoint,
    };

    const NOW: UnixMillis = 1_700_000_000_000;

    struct Fixture {
        household: Household,
        parent: PersonId,
        child: PersonId,
        neighborhood: NeighborhoodId,
        neighbor: PersonId,
    }

    impl Membership for Fixture {
        fn local_person(&self) -> PersonId {
            self.parent
        }
        fn household(&self) -> &Household {
            &self.household
        }
        fn in_neighborhood(&self, n: &NeighborhoodId) -> bool {
            *n == self.neighborhood
        }
        fn is_neighborhood_member(&self, n: &NeighborhoodId, p: &PersonId) -> bool {
            *n == self.neighborhood && (*p == self.neighbor || *p == self.parent)
        }
        fn knows(&self, p: &PersonId) -> bool {
            self.household.is_member(p) || *p == self.neighbor
        }
    }

    fn person(id: PersonId, name: &str, role: PersonRole) -> Person {
        Person {
            id,
            display_name: name.into(),
            role,
            public_key: None,
            encryption_key: None,
        }
    }

    fn fixture() -> Fixture {
        let parent = PersonId::new();
        let child = PersonId::new();
        Fixture {
            household: Household {
                id: HouseholdId::new(),
                name: "The Rivera family".into(),
                members: vec![
                    person(parent, "Ana", PersonRole::Adult),
                    person(child, "Leo", PersonRole::Child),
                ],
            },
            parent,
            child,
            neighborhood: NeighborhoodId::new(),
            neighbor: PersonId::new(),
        }
    }

    fn envelope(audience: Audience, author: PersonId, payload: Payload) -> Envelope {
        Envelope::new(NewEnvelope {
            audience,
            author,
            origin_device: DeviceId::new(),
            lamport: 1,
            now: NOW,
            payload,
        })
    }

    fn home(f: &Fixture) -> Audience {
        Audience::Household {
            household_id: f.household.id,
        }
    }

    fn hood(f: &Fixture) -> Audience {
        Audience::Neighborhood {
            neighborhood_id: f.neighborhood,
        }
    }

    fn check_in(person_id: PersonId) -> Payload {
        Payload::CheckIn(CheckIn {
            person_id,
            state: CheckInState::Safe,
            note: None,
            rendezvous_id: None,
            recorded_at: NOW,
        })
    }

    fn validate(f: &Fixture, env: &Envelope) -> Result<(), ValidationError> {
        validate_envelope(env, f, NOW, &ValidationLimits::default())
    }

    #[test]
    fn accepts_a_valid_check_in_on_behalf_of_a_child() {
        let f = fixture();
        assert_eq!(
            validate(&f, &envelope(home(&f), f.parent, check_in(f.child))),
            Ok(())
        );
    }

    #[test]
    fn rejects_unknown_protocol_version() {
        let f = fixture();
        let mut env = envelope(home(&f), f.parent, check_in(f.parent));
        env.version = 99;
        assert_eq!(
            validate(&f, &env),
            Err(ValidationError::UnsupportedVersion(99))
        );
    }

    #[test]
    fn payloads_must_match_their_scope() {
        let f = fixture();
        // A family check-in may never be sent to the neighborhood.
        let leaked = envelope(hood(&f), f.parent, check_in(f.parent));
        assert_eq!(validate(&f, &leaked), Err(ValidationError::ScopeMismatch));

        let alert = Payload::Alert {
            category: AlertCategory::Power,
            text: "Out".into(),
            area: None,
        };
        assert_eq!(
            validate(&f, &envelope(home(&f), f.parent, alert)),
            Err(ValidationError::ScopeMismatch)
        );
    }

    #[test]
    fn rejects_household_envelopes_from_outside_the_circle() {
        let f = fixture();
        let mut other = envelope(home(&f), f.parent, check_in(f.parent));
        other.audience = Audience::Household {
            household_id: HouseholdId::new(),
        };
        assert_eq!(validate(&f, &other), Err(ValidationError::WrongHousehold));

        let stranger = envelope(home(&f), PersonId::new(), check_in(f.parent));
        assert_eq!(
            validate(&f, &stranger),
            Err(ValidationError::AuthorNotMember)
        );

        let about_stranger = envelope(home(&f), f.parent, check_in(PersonId::new()));
        assert_eq!(
            validate(&f, &about_stranger),
            Err(ValidationError::SubjectNotMember)
        );
    }

    #[test]
    fn neighborhood_posts_need_a_member_author() {
        let f = fixture();
        let post = Payload::Offer {
            text: "Generator at #12".into(),
        };
        assert_eq!(
            validate(&f, &envelope(hood(&f), f.neighbor, post.clone())),
            Ok(())
        );
        assert_eq!(
            validate(&f, &envelope(hood(&f), PersonId::new(), post.clone())),
            Err(ValidationError::NotNeighborhoodMember)
        );
        let elsewhere = Audience::Neighborhood {
            neighborhood_id: NeighborhoodId::new(),
        };
        assert_eq!(
            validate(&f, &envelope(elsewhere, f.neighbor, post)),
            Err(ValidationError::NotInNeighborhood)
        );
    }

    #[test]
    fn member_joined_must_be_announced_by_the_member() {
        let f = fixture();
        let newcomer = PersonId::new();
        let cert = MembershipCert {
            neighborhood_id: f.neighborhood,
            member: newcomer,
            display_name: "Dana".into(),
            identity_key: vec![1; 32],
            encryption_key: vec![2; 32],
            invited_by: Some(f.neighbor),
            invited_at: NOW,
            signature: vec![],
        };
        let joined = Payload::MemberJoined(cert);
        assert_eq!(
            validate(&f, &envelope(hood(&f), newcomer, joined.clone())),
            Ok(())
        );
        assert_eq!(
            validate(&f, &envelope(hood(&f), f.neighbor, joined)),
            Err(ValidationError::BadMembershipEvent)
        );
    }

    #[test]
    fn direct_messages_must_be_for_me_from_someone_i_know() {
        let f = fixture();
        let to_me = Audience::Direct {
            recipient: f.parent,
        };
        let msg = Payload::Message {
            text: "Are you home?".into(),
        };
        assert_eq!(
            validate(&f, &envelope(to_me, f.neighbor, msg.clone())),
            Ok(())
        );
        assert_eq!(
            validate(&f, &envelope(to_me, PersonId::new(), msg.clone())),
            Err(ValidationError::UnknownSender)
        );
        let to_other = Audience::Direct {
            recipient: f.neighbor,
        };
        assert_eq!(
            validate(&f, &envelope(to_other, f.parent, msg)),
            Err(ValidationError::NotForMe)
        );
    }

    #[test]
    fn rejects_expired_and_future_envelopes() {
        let f = fixture();
        let env = envelope(home(&f), f.parent, check_in(f.parent));
        let limits = ValidationLimits::default();
        assert_eq!(
            validate_envelope(&env, &f, env.expires_at, &limits),
            Err(ValidationError::Expired)
        );

        let mut future = env.clone();
        future.created_at = NOW + limits.max_clock_skew_ms + 1;
        future.expires_at = future.created_at + 1_000;
        assert_eq!(validate(&f, &future), Err(ValidationError::FromTheFuture));

        let mut slightly_ahead = env;
        slightly_ahead.created_at = NOW + 60_000;
        assert_eq!(validate(&f, &slightly_ahead), Ok(()));
    }

    #[test]
    fn rejects_bad_text_fields() {
        let f = fixture();
        let empty = envelope(home(&f), f.parent, Payload::Note { text: "   ".into() });
        assert_eq!(validate(&f, &empty), Err(ValidationError::EmptyText));
        let long = envelope(
            home(&f),
            f.parent,
            Payload::Note {
                text: "a".repeat(281),
            },
        );
        assert_eq!(validate(&f, &long), Err(ValidationError::TextTooLong));
    }

    #[test]
    fn validates_rendezvous_points() {
        let f = fixture();
        let point = RendezvousPoint {
            id: RendezvousId::new(),
            household_id: f.household.id,
            name: "Library steps".into(),
            details: String::new(),
            location: Some(GeoPoint::from_degrees(40.7532, -73.9822)),
            when_to_go: String::new(),
            rank: 1,
        };
        let ok = envelope(home(&f), f.parent, Payload::RendezvousUpsert(point.clone()));
        assert_eq!(validate(&f, &ok), Ok(()));

        let mut bad_location = point.clone();
        bad_location.location = Some(GeoPoint {
            lat_e7: 950_000_000,
            lon_e7: 0,
        });
        let env = envelope(home(&f), f.parent, Payload::RendezvousUpsert(bad_location));
        assert_eq!(validate(&f, &env), Err(ValidationError::InvalidLocation));

        let mut foreign = point;
        foreign.household_id = HouseholdId::new();
        let env = envelope(home(&f), f.parent, Payload::RendezvousUpsert(foreign));
        assert_eq!(
            validate(&f, &env),
            Err(ValidationError::RendezvousWrongHousehold)
        );
    }

    #[test]
    fn rejects_oversized_envelopes() {
        let f = fixture();
        let limits = ValidationLimits {
            max_envelope_bytes: 200,
            ..Default::default()
        };
        let env = envelope(
            home(&f),
            f.parent,
            Payload::Note {
                text: "x".repeat(200),
            },
        );
        assert!(matches!(
            validate_envelope(&env, &f, NOW, &limits),
            Err(ValidationError::TooLarge(_))
        ));
    }

    #[test]
    fn sealed_header_checks() {
        use crate::envelope::{PriorityClass, Scope};
        let mut sealed = SealedEnvelope {
            version: PROTOCOL_VERSION,
            id: crate::ids::EnvelopeId::new(),
            scope: Scope::Direct,
            route_tag: vec![0; 16],
            expires_at: NOW + 1,
            hops_remaining: 1,
            priority_class: PriorityClass::Normal,
            body: vec![0; 600],
        };
        assert_eq!(validate_sealed(&sealed, NOW), Ok(()));
        assert_eq!(
            validate_sealed(&sealed, NOW + 1),
            Err(ValidationError::Expired)
        );
        sealed.body = vec![0; MAX_SEALED_BODY + 1];
        assert!(matches!(
            validate_sealed(&sealed, NOW),
            Err(ValidationError::TooLarge(_))
        ));
        sealed.body = vec![0; 10];
        sealed.route_tag = vec![0; 3];
        assert_eq!(
            validate_sealed(&sealed, NOW),
            Err(ValidationError::BadHeader)
        );
    }
}
