//! In-memory transport for tests and for running the app without radios.
//!
//! A [`MockNetwork`] is a shared "room". Each [`MockTransport`] joined to it is
//! one phone. Range is controlled explicitly so tests can model people walking
//! in and out of each other's range.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use super::{InboundFrame, PeerAddress, Transport, TransportError, TransportKind};

/// Roughly a few Wi-Fi Direct packets. Small enough to exercise frame splitting.
pub const MOCK_MAX_FRAME_BYTES: usize = 16 * 1024;

#[derive(Debug, Default)]
struct Room {
    inboxes: HashMap<PeerAddress, VecDeque<InboundFrame>>,
    /// Unordered pairs of addresses currently in range of each other.
    links: BTreeSet<(PeerAddress, PeerAddress)>,
    frames_sent: usize,
}

fn link(a: &PeerAddress, b: &PeerAddress) -> (PeerAddress, PeerAddress) {
    if a <= b {
        (a.clone(), b.clone())
    } else {
        (b.clone(), a.clone())
    }
}

#[derive(Debug, Clone, Default)]
pub struct MockNetwork {
    room: Arc<Mutex<Room>>,
}

impl MockNetwork {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn join(&self, address: impl Into<PeerAddress>) -> MockTransport {
        let address = address.into();
        self.lock().inboxes.entry(address.clone()).or_default();
        MockTransport {
            address,
            network: self.clone(),
            available: true,
        }
    }

    pub fn set_in_range(&self, a: &PeerAddress, b: &PeerAddress, in_range: bool) {
        let mut room = self.lock();
        if in_range {
            room.links.insert(link(a, b));
        } else {
            room.links.remove(&link(a, b));
        }
    }

    /// Total frames sent across the network; handy for asserting that sync is
    /// not chattier than expected.
    pub fn frames_sent(&self) -> usize {
        self.lock().frames_sent
    }

    fn lock(&self) -> MutexGuard<'_, Room> {
        // A poisoned lock only happens if a test already panicked.
        self.room
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Debug)]
pub struct MockTransport {
    address: PeerAddress,
    network: MockNetwork,
    available: bool,
}

impl MockTransport {
    /// Simulate the user switching Bluetooth off.
    pub fn set_available(&mut self, available: bool) {
        self.available = available;
    }
}

impl Transport for MockTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Mock
    }

    fn local_address(&self) -> PeerAddress {
        self.address.clone()
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn peers(&mut self) -> Result<Vec<PeerAddress>, TransportError> {
        if !self.available {
            return Err(TransportError::Unavailable);
        }
        let room = self.network.lock();
        Ok(room
            .links
            .iter()
            .filter_map(|(a, b)| {
                if *a == self.address {
                    Some(b.clone())
                } else if *b == self.address {
                    Some(a.clone())
                } else {
                    None
                }
            })
            .collect())
    }

    fn send(&mut self, to: &PeerAddress, frame: Vec<u8>) -> Result<(), TransportError> {
        if !self.available {
            return Err(TransportError::Unavailable);
        }
        if frame.len() > MOCK_MAX_FRAME_BYTES {
            return Err(TransportError::FrameTooLarge {
                size: frame.len(),
                limit: MOCK_MAX_FRAME_BYTES,
            });
        }
        let mut room = self.network.lock();
        if !room.links.contains(&link(&self.address, to)) {
            return Err(TransportError::PeerUnreachable(to.clone()));
        }
        let from = self.address.clone();
        room.inboxes
            .get_mut(to)
            .ok_or_else(|| TransportError::PeerUnreachable(to.clone()))?
            .push_back(InboundFrame { from, bytes: frame });
        room.frames_sent += 1;
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<InboundFrame>, TransportError> {
        let mut room = self.network.lock();
        Ok(room
            .inboxes
            .get_mut(&self.address)
            .map(|q| q.drain(..).collect())
            .unwrap_or_default())
    }

    fn max_frame_bytes(&self) -> usize {
        MOCK_MAX_FRAME_BYTES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivers_only_between_peers_in_range() {
        let net = MockNetwork::new();
        let mut a = net.join("mom");
        let mut b = net.join("dad");
        let (addr_a, addr_b) = (a.local_address(), b.local_address());

        assert_eq!(
            a.send(&addr_b, b"hi".to_vec()),
            Err(TransportError::PeerUnreachable(addr_b.clone()))
        );

        net.set_in_range(&addr_a, &addr_b, true);
        assert_eq!(a.peers().unwrap(), vec![addr_b.clone()]);
        a.send(&addr_b, b"hi".to_vec()).unwrap();

        let inbox = b.poll().unwrap();
        assert_eq!(
            inbox,
            vec![InboundFrame {
                from: addr_a,
                bytes: b"hi".to_vec()
            }]
        );
        assert!(b.poll().unwrap().is_empty());
    }

    #[test]
    fn unavailable_radio_refuses_to_work() {
        let net = MockNetwork::new();
        let mut a = net.join("mom");
        a.set_available(false);
        assert_eq!(a.peers(), Err(TransportError::Unavailable));
    }
}
