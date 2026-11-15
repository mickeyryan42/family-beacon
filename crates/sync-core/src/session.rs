//! Per-peer sync session bookkeeping.
//!
//! A session is one encounter with one nearby device: both sides say hello,
//! push what the other is missing, and say done. See
//! [`beacon_protocol::SyncMessage`] for the exchange itself.

use beacon_protocol::{
    encode_message, CodecError, DeviceId, SealedEnvelope, SyncMessage, UnixMillis,
};
use log::warn;

use crate::transport::PeerAddress;

/// Give up on a half-finished exchange after this long. Links on the move are
/// short-lived; a stale session would otherwise block a fresh one.
pub const SESSION_TIMEOUT_MS: u64 = 2 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    /// We said hello and are waiting for the peer's hello.
    AwaitingHello,
    /// Hellos exchanged; deliveries in flight.
    Exchanging,
    /// Both sides sent `Done`.
    Complete,
    /// Peer is not in our circle or speaks another protocol version.
    Rejected,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SessionStats {
    pub sent: usize,
    pub received: usize,
    pub duplicates: usize,
    pub rejected: usize,
}

#[derive(Debug, Clone)]
pub struct SyncSession {
    pub peer: PeerAddress,
    pub remote_device: Option<DeviceId>,
    pub started_at: UnixMillis,
    pub stats: SessionStats,
    sent_done: bool,
    received_done: bool,
    rejected: bool,
}

impl SyncSession {
    pub fn new(peer: PeerAddress, now: UnixMillis) -> Self {
        Self {
            peer,
            remote_device: None,
            started_at: now,
            stats: SessionStats::default(),
            sent_done: false,
            received_done: false,
            rejected: false,
        }
    }

    pub fn phase(&self) -> SessionPhase {
        if self.rejected {
            SessionPhase::Rejected
        } else if self.sent_done && self.received_done {
            SessionPhase::Complete
        } else if self.remote_device.is_some() {
            SessionPhase::Exchanging
        } else {
            SessionPhase::AwaitingHello
        }
    }

    pub fn is_finished(&self) -> bool {
        matches!(
            self.phase(),
            SessionPhase::Complete | SessionPhase::Rejected
        )
    }

    pub fn is_timed_out(&self, now: UnixMillis) -> bool {
        now.saturating_sub(self.started_at) > SESSION_TIMEOUT_MS
    }

    pub fn mark_hello_received(&mut self, device: DeviceId) {
        self.remote_device = Some(device);
    }

    pub fn mark_sent_done(&mut self) {
        self.sent_done = true;
    }

    pub fn mark_received_done(&mut self) {
        self.received_done = true;
    }

    pub fn mark_rejected(&mut self) {
        self.rejected = true;
    }
}

/// Split envelopes into as few `Deliver` frames as fit under `max_frame_bytes`.
///
/// An envelope that cannot fit in a frame on its own is skipped (and logged)
/// rather than failing the whole session. Sealed bodies are capped well below
/// every transport's frame limit, so this only guards against bugs.
pub fn pack_deliveries(
    envelopes: Vec<SealedEnvelope>,
    max_frame_bytes: usize,
) -> Result<Vec<Vec<u8>>, CodecError> {
    let mut frames = Vec::new();
    let mut batch: Vec<SealedEnvelope> = Vec::new();
    let mut batch_frame: Option<Vec<u8>> = None;

    for envelope in envelopes {
        batch.push(envelope);
        let candidate = encode_message(&SyncMessage::Deliver {
            envelopes: batch.clone(),
        })?;
        if candidate.len() <= max_frame_bytes {
            batch_frame = Some(candidate);
            continue;
        }

        // Did not fit: flush what we had, start a new batch with this one.
        let Some(overflow) = batch.pop() else {
            continue;
        };
        if let Some(frame) = batch_frame.take() {
            frames.push(frame);
        }
        batch.clear();

        let alone = encode_message(&SyncMessage::Deliver {
            envelopes: vec![overflow.clone()],
        })?;
        if alone.len() <= max_frame_bytes {
            batch.push(overflow);
            batch_frame = Some(alone);
        } else {
            warn!(
                "envelope {} is too large for a single frame; skipping",
                overflow.id
            );
        }
    }

    if let Some(frame) = batch_frame {
        frames.push(frame);
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use beacon_protocol::{decode_message, EnvelopeId, PriorityClass, Scope};

    fn sealed(body: usize) -> SealedEnvelope {
        SealedEnvelope {
            version: beacon_protocol::PROTOCOL_VERSION,
            id: EnvelopeId::new(),
            scope: Scope::Household,
            route_tag: vec![0; 16],
            expires_at: 1,
            hops_remaining: 1,
            priority_class: PriorityClass::Normal,
            body: vec![7; body],
        }
    }

    #[test]
    fn phases_progress_from_hello_to_complete() {
        let mut s = SyncSession::new("dad".into(), 0);
        assert_eq!(s.phase(), SessionPhase::AwaitingHello);
        s.mark_hello_received(DeviceId::new());
        assert_eq!(s.phase(), SessionPhase::Exchanging);
        s.mark_sent_done();
        assert!(!s.is_finished());
        s.mark_received_done();
        assert_eq!(s.phase(), SessionPhase::Complete);
    }

    #[test]
    fn rejection_finishes_the_session() {
        let mut s = SyncSession::new("stranger".into(), 0);
        s.mark_rejected();
        assert!(s.is_finished());
        assert!(s.is_timed_out(SESSION_TIMEOUT_MS + 1));
    }

    #[test]
    fn packs_envelopes_into_frames_under_the_limit() {
        let envelopes: Vec<_> = (0..20).map(|_| sealed(100)).collect();
        let one = encode_message(&SyncMessage::Deliver {
            envelopes: vec![envelopes[0].clone()],
        })
        .unwrap();
        let limit = one.len() * 3;

        let frames = pack_deliveries(envelopes.clone(), limit).unwrap();
        assert!(frames.len() > 1);
        let mut seen = vec![];
        for frame in &frames {
            assert!(frame.len() <= limit);
            match decode_message(frame).unwrap() {
                SyncMessage::Deliver { envelopes } => seen.extend(envelopes),
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(seen, envelopes);
    }

    #[test]
    fn skips_envelopes_that_can_never_fit() {
        let small = sealed(10);
        let limit = encode_message(&SyncMessage::Deliver {
            envelopes: vec![small.clone()],
        })
        .unwrap()
        .len()
            + 50;
        let frames = pack_deliveries(vec![small, sealed(5_000)], limit).unwrap();
        assert_eq!(frames.len(), 1);
        match decode_message(&frames[0]).unwrap() {
            SyncMessage::Deliver { envelopes } => assert_eq!(envelopes.len(), 1),
            other => panic!("unexpected {other:?}"),
        }
    }
}
