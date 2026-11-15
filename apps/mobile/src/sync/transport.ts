/**
 * Transport seam for the app.
 *
 * Mirrors the shape of the Rust `Transport` trait at a higher level: a
 * transport tells us when a household phone comes into range and lets us swap
 * envelopes with it. Everything else (what to send, validation, conflict
 * rules) is the sync service's job, so BLE, Wi-Fi Direct and the mock can be
 * swapped without touching it.
 */
import type {Envelope, Uuid} from './protocol';

export type TransportKind = 'mock' | 'ble' | 'wifi-direct';

export interface NearbyPeer {
  /** Transport-scoped address. Not an identity: envelopes carry that. */
  address: string;
  /** Who we believe owns the phone, once their hello is seen. */
  personId?: Uuid;
  label: string;
}

export interface Encounter {
  peer: NearbyPeer;
  /**
   * Send what the peer is missing; resolve with what they sent us.
   * `outbound` may be empty. Implementations must not throw for a peer that
   * walked away mid-exchange; they resolve with whatever arrived.
   */
  exchange(outbound: Envelope[], localHave: ReadonlySet<Uuid>): Promise<Envelope[]>;
  /** IDs the peer already holds, from its hello. */
  peerHas: ReadonlySet<Uuid>;
}

export interface TransportListener {
  onPeersChanged(peers: NearbyPeer[]): void;
  onEncounter(encounter: Encounter): void;
}

export type TransportStatus =
  | {state: 'running'}
  | {state: 'stopped'}
  | {state: 'unavailable'; reason: string};

export interface SyncTransport {
  readonly kind: TransportKind;
  readonly label: string;
  status(): TransportStatus;
  start(listener: TransportListener): void;
  stop(): void;
}
