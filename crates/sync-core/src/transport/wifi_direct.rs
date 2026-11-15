//! Wi-Fi Direct / peer-to-peer Wi-Fi transport (stub).
//!
//! Planned design (see `docs/transports.md`): used opportunistically for bulk
//! catch-up once BLE has established that a household member is nearby, since
//! it is faster but far more power-hungry. Android Wi-Fi Aware / Wi-Fi P2P and
//! Apple's Multipeer Connectivity are the native candidates.
//!
//! Until the native modules exist, every operation reports
//! [`TransportError::NotImplemented`].

use super::{InboundFrame, PeerAddress, Transport, TransportError, TransportKind};

pub const WIFI_DIRECT_MAX_FRAME_BYTES: usize = 64 * 1024;

#[derive(Debug, Default)]
pub struct WifiDirectTransport;

impl Transport for WifiDirectTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::WifiDirect
    }

    fn local_address(&self) -> PeerAddress {
        PeerAddress("wifi-direct:unassigned".into())
    }

    fn is_available(&self) -> bool {
        false
    }

    fn peers(&mut self) -> Result<Vec<PeerAddress>, TransportError> {
        Err(TransportError::NotImplemented)
    }

    fn send(&mut self, _to: &PeerAddress, _frame: Vec<u8>) -> Result<(), TransportError> {
        Err(TransportError::NotImplemented)
    }

    fn poll(&mut self) -> Result<Vec<InboundFrame>, TransportError> {
        Err(TransportError::NotImplemented)
    }

    fn max_frame_bytes(&self) -> usize {
        WIFI_DIRECT_MAX_FRAME_BYTES
    }
}
