//! Core domain records.

use serde::{Deserialize, Serialize};

use crate::ids::{HouseholdId, NeighborhoodId, PersonId, RendezvousId};

/// Milliseconds since the Unix epoch, as reported by the authoring device.
///
/// Phone clocks drift, especially after long periods without a network time
/// source, so wall-clock values are treated as hints, never as the only
/// ordering signal. See `sync-core`'s conflict module.
pub type UnixMillis = u64;

pub const DAY_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonRole {
    Adult,
    Child,
    /// Grandparent, sitter, neighbour: trusted but not part of the core household.
    Caregiver,
}

/// A trusted contact.
///
/// People with a phone have both keys, exchanged in person when they join. A
/// child without a phone has neither; adults check in on their behalf.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    pub id: PersonId,
    pub display_name: String,
    pub role: PersonRole,
    /// Ed25519 key that signs everything this person writes.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::b64::option"
    )]
    pub public_key: Option<Vec<u8>>,
    /// X25519 key others use to send this person direct messages.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::b64::option"
    )]
    pub encryption_key: Option<Vec<u8>>,
}

/// A household is the private trust boundary: its traffic is sealed with the
/// household key, shared only at in-person pairing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Household {
    pub id: HouseholdId,
    pub name: String,
    pub members: Vec<Person>,
}

impl Household {
    pub fn member(&self, id: &PersonId) -> Option<&Person> {
        self.members.iter().find(|p| &p.id == id)
    }

    pub fn is_member(&self, id: &PersonId) -> bool {
        self.member(id).is_some()
    }
}

/// The handful of states a family actually needs during an outage.
/// Kept intentionally small so the UI can offer one-tap answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckInState {
    Safe,
    OnMyWay,
    AtRendezvous,
    NeedHelp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckIn {
    /// Who this check-in is about. Usually the author, but a parent may check
    /// in on behalf of a child who does not have a phone.
    pub person_id: PersonId,
    pub state: CheckInState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rendezvous_id: Option<RendezvousId>,
    /// When the person reported this, per the authoring device's clock.
    pub recorded_at: UnixMillis,
}

/// A coordinate stored as fixed-point degrees * 1e7.
///
/// Integers keep equality, hashing and serialization exact across platforms,
/// which matters when the same record must compare equal on every phone.
/// 1e-7 degrees is roughly 1 cm, far finer than needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GeoPoint {
    pub lat_e7: i32,
    pub lon_e7: i32,
}

impl GeoPoint {
    pub fn from_degrees(lat: f64, lon: f64) -> Self {
        Self {
            lat_e7: (lat * 1e7).round() as i32,
            lon_e7: (lon * 1e7).round() as i32,
        }
    }

    pub fn is_valid(&self) -> bool {
        (-900_000_000..=900_000_000).contains(&self.lat_e7)
            && (-1_800_000_000..=1_800_000_000).contains(&self.lon_e7)
    }
}

/// A pre-agreed place to meet when the family cannot reach each other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RendezvousPoint {
    pub id: RendezvousId,
    pub household_id: HouseholdId,
    pub name: String,
    /// Plain-language directions ("Back gate of Lincoln Elementary").
    #[serde(default)]
    pub details: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<GeoPoint>,
    /// When to go there, in words ("If we can't reach each other for 2 hours").
    #[serde(default)]
    pub when_to_go: String,
    /// 1 = primary meeting point, 2 = first backup, and so on.
    pub rank: u8,
}

// --- neighborhoods ---

/// An invite-only network of neighbors. See docs/neighborhood-network.md.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Neighborhood {
    pub id: NeighborhoodId,
    pub name: String,
    /// Approximate area in words ("Elm & Oak streets"). Never coordinates.
    #[serde(default)]
    pub area: String,
    pub founder: PersonId,
    /// Pins the founder's key so nobody else can forge a founding certificate.
    #[serde(with = "crate::b64")]
    pub founder_identity_key: Vec<u8>,
    pub created_at: UnixMillis,
}

/// Proof that someone was invited into a neighborhood, signed by whoever
/// invited them. The founder's certificate is self-signed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipCert {
    pub neighborhood_id: NeighborhoodId,
    pub member: PersonId,
    pub display_name: String,
    #[serde(with = "crate::b64")]
    pub identity_key: Vec<u8>,
    #[serde(with = "crate::b64")]
    pub encryption_key: Vec<u8>,
    /// `None` only for the founder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invited_by: Option<PersonId>,
    pub invited_at: UnixMillis,
    #[serde(default, with = "crate::b64")]
    pub signature: Vec<u8>,
}

impl MembershipCert {
    /// Canonical bytes covered by the inviter's signature.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut unsigned = self.clone();
        unsigned.signature.clear();
        serde_json::to_vec(&unsigned).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertCategory {
    Power,
    Water,
    Road,
    Medical,
    Fire,
    Other,
}

impl AlertCategory {
    /// Only life-safety alerts may jump the queue.
    pub fn is_urgent(self) -> bool {
        matches!(self, AlertCategory::Medical | AlertCategory::Fire)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovalReason {
    Spam,
    Abuse,
    LeftArea,
    Other,
}
