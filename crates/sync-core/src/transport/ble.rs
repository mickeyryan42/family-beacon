//! Bluetooth LE transport (stub).
//!
//! Planned design (see `docs/transports.md`):
//!
//! * Advertise a Family Beacon service UUID plus a short rotating household
//!   token, so only members recognise each other and passers-by can't track a
//!   fixed identifier.
//! * Use OS-managed low-duty-cycle scanning (iOS background BLE, Android
//!   `SCAN_MODE_LOW_POWER`) rather than continuous scans.
//! * Exchange frames over a GATT characteristic, chunked to the negotiated MTU.
//!
//! The radio work lives in native Swift/Kotlin modules; this type will wrap
//! the frames they buffer. Until then every operation reports
//! [`TransportError::NotImplemented`].

use super::{InboundFrame, PeerAddress, Transport, TransportError, TransportKind};

/// Room for the largest sealed envelope (a padded 4 KiB body, base64, plus
/// header), sent as several GATT writes at the negotiated MTU.
pub const BLE_MAX_FRAME_BYTES: usize = 8 * 1024;

#[derive(Debug, Default)]
pub struct BleTransport;

impl Transport for BleTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::BluetoothLe
    }

    fn local_address(&self) -> PeerAddress {
        PeerAddress("ble:unassigned".into())
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
        BLE_MAX_FRAME_BYTES
    }
}
