//! Frames exchanged between two nearby devices during a sync session.
//!
//! The exchange is push-based and symmetric, and happens between **any** two
//! Family Beacon phones, not just members of the same household: every phone
//! relays sealed envelopes for everyone.
//!
//! 1. Each side sends [`SyncMessage::Hello`] with a compact summary of the
//!    envelope ids it holds.
//! 2. Each side sends what the other is missing in one or more
//!    [`SyncMessage::Deliver`] frames.
//! 3. Each side sends [`SyncMessage::Done`].
//!
//! No request/response round trip is needed, which matters on links that may
//! only stay up for a few seconds while two people walk past each other.

use serde::{Deserialize, Serialize};

use crate::envelope::SealedEnvelope;
use crate::ids::DeviceId;
use crate::summary::IdSummary;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol_version: u16,
    pub device_id: DeviceId,
    pub have: IdSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncMessage {
    Hello(Hello),
    Deliver {
        envelopes: Vec<SealedEnvelope>,
    },
    Done,
    /// Sent instead of continuing when the peer speaks an incompatible
    /// protocol version.
    Reject {
        reason: String,
    },
}
