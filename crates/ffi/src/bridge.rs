//! The radio seam: a [`Transport`] fed and drained by native code.
//!
//! Swift/Kotlin own the radios (BLE now, Wi-Fi Direct / Multipeer later).
//! They report which peers are in range, push in frames as they arrive, and
//! after each sync round take the frames to transmit. Rust never blocks on
//! I/O and never starts a thread, which keeps battery behavior in the
//! platform's hands (see docs/transports.md).

use beacon_sync_core::transport::{
    InboundFrame, PeerAddress, Transport, TransportError, TransportKind,
};

/// BLE-sized frames by default; the native side can raise it per transport.
pub(crate) const DEFAULT_MAX_FRAME_BYTES: usize = 8 * 1024;
/// Frames waiting in either direction are capped so a stalled radio or a
/// chatty peer can't grow memory without bound.
pub(crate) const MAX_QUEUED_FRAMES: usize = 512;

#[derive(Debug)]
pub(crate) struct BridgeTransport {
    address: PeerAddress,
    available: bool,
    peers: Vec<PeerAddress>,
    inbound: Vec<InboundFrame>,
    outbound: Vec<(PeerAddress, Vec<u8>)>,
    max_frame_bytes: usize,
}

impl BridgeTransport {
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            address: PeerAddress(address.into()),
            available: false,
            peers: Vec::new(),
            inbound: Vec::new(),
            outbound: Vec::new(),
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
        }
    }

    pub fn set_available(&mut self, available: bool) {
        self.available = available;
        if !available {
            self.peers.clear();
        }
    }

    pub fn set_peers(&mut self, peers: Vec<String>) {
        self.peers = peers.into_iter().map(PeerAddress).collect();
        self.peers.sort();
        self.peers.dedup();
    }

    pub fn set_max_frame_bytes(&mut self, bytes: usize) {
        self.max_frame_bytes = bytes.max(1);
    }

    /// Returns false (dropping the frame) when the inbound queue is full.
    pub fn push_inbound(&mut self, from: String, bytes: Vec<u8>) -> bool {
        if self.inbound.len() >= MAX_QUEUED_FRAMES {
            return false;
        }
        self.inbound.push(InboundFrame {
            from: PeerAddress(from),
            bytes,
        });
        true
    }

    pub fn pending_outbound(&self) -> usize {
        self.outbound.len()
    }

    pub fn take_outbound(&mut self) -> Vec<(PeerAddress, Vec<u8>)> {
        std::mem::take(&mut self.outbound)
    }
}

impl Transport for BridgeTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::BluetoothLe
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
        Ok(self.peers.clone())
    }

    fn send(&mut self, to: &PeerAddress, frame: Vec<u8>) -> Result<(), TransportError> {
        if frame.len() > self.max_frame_bytes {
            return Err(TransportError::FrameTooLarge {
                size: frame.len(),
                limit: self.max_frame_bytes,
            });
        }
        // A peer may have just left; sessions already tolerate that.
        if !self.available || !self.peers.contains(to) || self.outbound.len() >= MAX_QUEUED_FRAMES {
            return Err(TransportError::PeerUnreachable(to.clone()));
        }
        self.outbound.push((to.clone(), frame));
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<InboundFrame>, TransportError> {
        Ok(std::mem::take(&mut self.inbound))
    }

    fn max_frame_bytes(&self) -> usize {
        self.max_frame_bytes
    }
}
