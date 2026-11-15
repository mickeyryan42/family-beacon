/**
 * Placeholder for the real radio transports.
 *
 * BLE (discovery and small updates) and Wi-Fi Direct / Multipeer (bulk
 * catch-up) will live in native Swift/Kotlin modules that feed frames into the
 * Rust sync core. See docs/transports.md. Until then this reports itself as
 * unavailable so the UI can show an honest state instead of pretending.
 */
import type {SyncTransport, TransportKind, TransportListener, TransportStatus} from './transport';

export class NativeTransportStub implements SyncTransport {
  readonly label: string;

  constructor(readonly kind: Exclude<TransportKind, 'mock'>) {
    this.label = kind === 'ble' ? 'Bluetooth LE' : 'Wi-Fi Direct';
  }

  status(): TransportStatus {
    return {state: 'unavailable', reason: `${this.label} support is not built yet`};
  }

  start(listener: TransportListener): void {
    listener.onPeersChanged([]);
  }

  stop(): void {}
}
