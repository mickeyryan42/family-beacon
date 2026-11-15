//! Envelopes: the unit of store-and-forward.
//!
//! Every change someone makes (a check-in, a neighborhood alert, a private
//! message) becomes one immutable [`Envelope`]. Before it leaves the device it
//! is sealed into a [`SealedEnvelope`]: a small routing header any phone can
//! read, plus an encrypted body only the intended audience can open. Any
//! Family Beacon phone relays sealed envelopes, so reach grows with every
//! install while readability stays with the audience.

use serde::{Deserialize, Serialize};

use crate::ids::{DeviceId, EnvelopeId, HouseholdId, NeighborhoodId, PersonId, RendezvousId};
use crate::model::{
    AlertCategory, CheckIn, CheckInState, MembershipCert, RemovalReason, RendezvousPoint,
    UnixMillis, DAY_MS,
};
use crate::PROTOCOL_VERSION;

/// Who can read an envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Household,
    Neighborhood,
    Direct,
}

impl Scope {
    /// Relays beyond the author. Higher than a household-only mesh needs,
    /// because any phone may now carry traffic; per-scope budgets in
    /// `sync-core` keep that from flooding the mesh.
    pub fn default_hops(self) -> u8 {
        match self {
            Scope::Household => 12,
            Scope::Neighborhood | Scope::Direct => 16,
        }
    }

    pub fn label(self) -> &'static [u8] {
        match self {
            Scope::Household => b"household",
            Scope::Neighborhood => b"neighborhood",
            Scope::Direct => b"direct",
        }
    }
}

/// The exact audience of an envelope: which household, which neighborhood, or
/// which person. Inside the sealed body, so relays never see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum Audience {
    Household { household_id: HouseholdId },
    Neighborhood { neighborhood_id: NeighborhoodId },
    Direct { recipient: PersonId },
}

impl Audience {
    pub fn scope(&self) -> Scope {
        match self {
            Audience::Household { .. } => Scope::Household,
            Audience::Neighborhood { .. } => Scope::Neighborhood,
            Audience::Direct { .. } => Scope::Direct,
        }
    }
}

/// Detached Ed25519 signature over [`Envelope::signing_bytes`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Signature(#[serde(with = "crate::b64")] pub Vec<u8>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Payload {
    // --- household ---
    CheckIn(CheckIn),
    RendezvousUpsert(RendezvousPoint),
    RendezvousRemove {
        id: RendezvousId,
    },
    /// A short free-text note to the family ("Phones are charged").
    Note {
        text: String,
    },

    // --- neighborhood ---
    Alert {
        category: AlertCategory,
        text: String,
        /// Coarse location in words ("Elm St, 400 block"). Never GPS.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        area: Option<String>,
    },
    Offer {
        text: String,
    },
    Need {
        text: String,
    },
    /// Opt-in, household-level status. Never includes individual members.
    HouseholdStatus {
        household_name: String,
        all_safe: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    /// Retract one of the author's own earlier posts.
    Withdraw {
        envelope_id: EnvelopeId,
    },
    /// Broadcast by a new member when they accept an invite.
    MemberJoined(MembershipCert),
    MemberRemoved {
        member: PersonId,
        reason: RemovalReason,
        cascade: bool,
    },
    StewardAdded {
        member: PersonId,
    },

    // --- direct ---
    Message {
        text: String,
    },
    WellnessRequest {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    WellnessReply {
        state: CheckInState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    /// New neighborhood key after a removal, sealed to each remaining member.
    KeyRotation {
        neighborhood_id: NeighborhoodId,
        epoch: u32,
        #[serde(with = "crate::b64")]
        key: Vec<u8>,
    },
}

/// Fine-grained priority, used for ordering and eviction on devices that can
/// read the envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    Low = 0,
    Normal = 1,
    High = 2,
    Urgent = 3,
}

/// Coarse priority visible to relays. Deliberately only three levels so the
/// header reveals as little as possible about the content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriorityClass {
    Low,
    Normal,
    Urgent,
}

impl From<Priority> for PriorityClass {
    fn from(p: Priority) -> Self {
        match p {
            Priority::Urgent => PriorityClass::Urgent,
            Priority::High | Priority::Normal => PriorityClass::Normal,
            Priority::Low => PriorityClass::Low,
        }
    }
}

impl Payload {
    /// The one scope this payload may be sent in.
    pub fn scope(&self) -> Scope {
        match self {
            Payload::CheckIn(_)
            | Payload::RendezvousUpsert(_)
            | Payload::RendezvousRemove { .. }
            | Payload::Note { .. } => Scope::Household,
            Payload::Alert { .. }
            | Payload::Offer { .. }
            | Payload::Need { .. }
            | Payload::HouseholdStatus { .. }
            | Payload::Withdraw { .. }
            | Payload::MemberJoined(_)
            | Payload::MemberRemoved { .. }
            | Payload::StewardAdded { .. } => Scope::Neighborhood,
            Payload::Message { .. }
            | Payload::WellnessRequest { .. }
            | Payload::WellnessReply { .. }
            | Payload::KeyRotation { .. } => Scope::Direct,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Payload::CheckIn(_) => "check_in",
            Payload::RendezvousUpsert(_) => "rendezvous_upsert",
            Payload::RendezvousRemove { .. } => "rendezvous_remove",
            Payload::Note { .. } => "note",
            Payload::Alert { .. } => "alert",
            Payload::Offer { .. } => "offer",
            Payload::Need { .. } => "need",
            Payload::HouseholdStatus { .. } => "household_status",
            Payload::Withdraw { .. } => "withdraw",
            Payload::MemberJoined(_) => "member_joined",
            Payload::MemberRemoved { .. } => "member_removed",
            Payload::StewardAdded { .. } => "steward_added",
            Payload::Message { .. } => "message",
            Payload::WellnessRequest { .. } => "wellness_request",
            Payload::WellnessReply { .. } => "wellness_reply",
            Payload::KeyRotation { .. } => "key_rotation",
        }
    }

    /// Membership events shape who is in a neighborhood and must outlive
    /// ordinary posts.
    pub fn is_membership_event(&self) -> bool {
        matches!(
            self,
            Payload::MemberJoined(_) | Payload::MemberRemoved { .. } | Payload::StewardAdded { .. }
        )
    }

    pub fn priority(&self) -> Priority {
        match self {
            Payload::CheckIn(c) if c.state == CheckInState::NeedHelp => Priority::Urgent,
            Payload::WellnessReply {
                state: CheckInState::NeedHelp,
                ..
            } => Priority::Urgent,
            Payload::Alert { category, .. } if category.is_urgent() => Priority::Urgent,
            Payload::CheckIn(_)
            | Payload::Alert { .. }
            | Payload::WellnessRequest { .. }
            | Payload::WellnessReply { .. }
            | Payload::KeyRotation { .. }
            | Payload::MemberRemoved { .. } => Priority::High,
            Payload::RendezvousUpsert(_)
            | Payload::RendezvousRemove { .. }
            | Payload::Offer { .. }
            | Payload::Need { .. }
            | Payload::HouseholdStatus { .. }
            | Payload::Message { .. }
            | Payload::MemberJoined(_)
            | Payload::StewardAdded { .. } => Priority::Normal,
            Payload::Note { .. } | Payload::Withdraw { .. } => Priority::Low,
        }
    }

    /// How long this kind of update stays useful.
    pub fn default_ttl_ms(&self) -> u64 {
        match self {
            Payload::CheckIn(_) | Payload::Note { .. } => 2 * DAY_MS,
            Payload::RendezvousUpsert(_) | Payload::RendezvousRemove { .. } => 90 * DAY_MS,
            Payload::Alert { .. }
            | Payload::Offer { .. }
            | Payload::Need { .. }
            | Payload::HouseholdStatus { .. }
            | Payload::Withdraw { .. } => DAY_MS,
            Payload::MemberJoined(_)
            | Payload::MemberRemoved { .. }
            | Payload::StewardAdded { .. } => 365 * DAY_MS,
            Payload::Message { .. }
            | Payload::WellnessRequest { .. }
            | Payload::WellnessReply { .. } => 2 * DAY_MS,
            Payload::KeyRotation { .. } => 30 * DAY_MS,
        }
    }
}

/// The readable envelope. Only the audience ever sees this form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub version: u16,
    pub id: EnvelopeId,
    pub audience: Audience,
    pub author: PersonId,
    pub origin_device: DeviceId,
    pub created_at: UnixMillis,
    /// Lamport timestamp from the authoring device. Gives a causal order that
    /// does not depend on phone clocks being correct.
    pub lamport: u64,
    pub expires_at: UnixMillis,
    pub payload: Payload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
}

/// Inputs for building a new envelope on the authoring device.
#[derive(Debug, Clone)]
pub struct NewEnvelope {
    pub audience: Audience,
    pub author: PersonId,
    pub origin_device: DeviceId,
    pub lamport: u64,
    pub now: UnixMillis,
    pub payload: Payload,
}

impl Envelope {
    pub fn new(input: NewEnvelope) -> Self {
        let expires_at = input.now.saturating_add(input.payload.default_ttl_ms());
        Self {
            version: PROTOCOL_VERSION,
            id: EnvelopeId::new(),
            audience: input.audience,
            author: input.author,
            origin_device: input.origin_device,
            created_at: input.now,
            lamport: input.lamport,
            expires_at,
            payload: input.payload,
            signature: None,
        }
    }

    pub fn scope(&self) -> Scope {
        self.audience.scope()
    }

    pub fn is_expired(&self, now: UnixMillis) -> bool {
        now >= self.expires_at
    }

    pub fn priority(&self) -> Priority {
        self.payload.priority()
    }

    /// Canonical bytes covered by the author's signature: everything except
    /// the signature itself. serde_json output is deterministic here because
    /// every field is a struct field or enum (no maps).
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut unsigned = self.clone();
        unsigned.signature = None;
        serde_json::to_vec(&unsigned).unwrap_or_default()
    }
}

/// What every phone sees and relays. The body is ciphertext; see
/// `sync-core/src/sealing.rs` for how it is produced and opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedEnvelope {
    pub version: u16,
    /// Same id as the inner envelope; used for de-duplication mesh-wide.
    pub id: EnvelopeId,
    pub scope: Scope,
    /// Lets the audience recognize the envelope without trial decryption.
    #[serde(with = "crate::b64")]
    pub route_tag: Vec<u8>,
    pub expires_at: UnixMillis,
    /// Decremented by each relay. Outside the ciphertext so relays can change it.
    pub hops_remaining: u8,
    pub priority_class: PriorityClass,
    #[serde(with = "crate::b64")]
    pub body: Vec<u8>,
}

impl SealedEnvelope {
    pub fn is_expired(&self, now: UnixMillis) -> bool {
        now >= self.expires_at
    }

    /// Bytes authenticated (but not hidden) by the body's encryption. Excludes
    /// `hops_remaining` and `priority_class`, which relays may lower.
    pub fn associated_data(id: &EnvelopeId, scope: Scope, expires_at: UnixMillis) -> Vec<u8> {
        let mut aad = Vec::with_capacity(16 + 1 + 8);
        aad.extend_from_slice(id.0.as_bytes());
        aad.push(scope as u8);
        aad.extend_from_slice(&expires_at.to_le_bytes());
        aad
    }

    /// Per-envelope context mixed into the route tag.
    pub fn tag_context(id: &EnvelopeId, expires_at: UnixMillis) -> Vec<u8> {
        let mut ctx = id.0.as_bytes().to_vec();
        ctx.extend_from_slice(&expires_at.to_le_bytes());
        ctx
    }

    /// The copy a relay forwards: same envelope, one fewer hop.
    pub fn for_relay(&self) -> Option<SealedEnvelope> {
        let hops = self.hops_remaining.checked_sub(1)?;
        Some(SealedEnvelope {
            hops_remaining: hops,
            ..self.clone()
        })
    }

    /// Approximate size on the wire, for budgets.
    pub fn approx_bytes(&self) -> usize {
        self.body.len() * 4 / 3 + 160
    }
}
