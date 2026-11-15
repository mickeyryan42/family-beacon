//! Family Beacon wire protocol.
//!
//! Everything that crosses a device boundary is defined here: identifiers,
//! domain records, the readable [`Envelope`] and the [`SealedEnvelope`] relays
//! carry, the [`SyncMessage`] frames exchanged during a sync session, and the
//! codec used to turn them into bytes.
//!
//! This crate deliberately has no I/O, no clocks, no storage and no
//! cryptography. It can be shared by the on-device sync core, test tools, and
//! any future optional relay without dragging in platform code.

mod b64;
pub mod codec;
pub mod envelope;
pub mod ids;
pub mod model;
pub mod summary;
pub mod sync_message;
pub mod validation;

pub use codec::{
    decode_envelope, decode_message, decode_sealed, encode_envelope, encode_message, encode_sealed,
    CodecError,
};
pub use envelope::{
    Audience, Envelope, NewEnvelope, Payload, Priority, PriorityClass, Scope, SealedEnvelope,
    Signature,
};
pub use ids::{DeviceId, EnvelopeId, HouseholdId, NeighborhoodId, PersonId, RendezvousId};
pub use model::{
    AlertCategory, CheckIn, CheckInState, GeoPoint, Household, MembershipCert, Neighborhood,
    Person, PersonRole, RemovalReason, RendezvousPoint, UnixMillis, DAY_MS,
};
pub use summary::IdSummary;
pub use sync_message::{Hello, SyncMessage};
pub use validation::{
    validate_envelope, validate_sealed, Membership, ValidationError, ValidationLimits,
};

/// Current wire protocol version. Bump on any breaking change to
/// [`Envelope`], [`SealedEnvelope`] or [`SyncMessage`]; peers reject versions
/// they do not know.
///
/// v2: scopes, sealed relay, neighborhoods, direct messages, summary hellos.
pub const PROTOCOL_VERSION: u16 = 2;
