//! Transport abstraction.
//!
//! A transport moves opaque byte frames between this device and nearby peers.
//! It knows nothing about envelopes, households or sync rules; the engine owns
//! all of that. Keeping the boundary this thin is what lets BLE, Wi-Fi Direct
//! and the in-memory mock be swapped without touching sync logic.
//!
//! The trait is synchronous and poll-based on purpose. Real radios are driven
//! by the OS (CoreBluetooth / Android BLE callbacks) in native code; that code
//! buffers inbound frames and the engine drains them when the app is woken.

mod ble;
mod mock;
mod wifi_direct;

use std::fmt;

use thiserror::Error;

pub use ble::BleTransport;
pub use mock::{MockNetwork, MockTransport};
pub use wifi_direct::WifiDirectTransport;

/// Transport-scoped peer address (a BLE peripheral UUID, a Wi-Fi Direct MAC,
/// a mock name). Not a stable identity: the sync `Hello` carries that.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PeerAddress(pub String);

impl fmt::Display for PeerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for PeerAddress {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportKind {
    Mock,
    BluetoothLe,
    WifiDirect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundFrame {
    pub from: PeerAddress,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransportError {
    #[error("transport is not available on this device")]
    Unavailable,
    #[error("transport is not implemented yet")]
    NotImplemented,
    #[error("peer {0} is out of range")]
    PeerUnreachable(PeerAddress),
    #[error("frame of {size} bytes exceeds transport limit of {limit}")]
    FrameTooLarge { size: usize, limit: usize },
}

pub trait Transport {
    fn kind(&self) -> TransportKind;

    /// Our own address on this transport.
    fn local_address(&self) -> PeerAddress;

    /// Whether the radio is present, permitted and switched on.
    fn is_available(&self) -> bool;

    /// Peers currently in range. Implementations should return cached results
    /// from passive/low-duty scanning rather than starting an active scan.
    fn peers(&mut self) -> Result<Vec<PeerAddress>, TransportError>;

    fn send(&mut self, to: &PeerAddress, frame: Vec<u8>) -> Result<(), TransportError>;

    /// Drain frames received since the last call.
    fn poll(&mut self) -> Result<Vec<InboundFrame>, TransportError>;

    /// Largest frame this transport accepts in one `send`.
    fn max_frame_bytes(&self) -> usize;
}
