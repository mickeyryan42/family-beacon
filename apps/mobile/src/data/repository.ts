/**
 * Local persistence boundary.
 *
 * Everything the app knows lives on the device. The repository is the only
 * code that touches storage, so the backend (in-memory for tests, key-value
 * storage on device today, SQLite via the Rust core later; schema:
 * crates/sync-core/migrations/) can change without touching screens or the
 * sync service.
 *
 * Envelopes are the source of truth; views are always re-derived from them.
 * Alongside them the repository keeps the small amount of local-only state
 * that envelopes can't reconstruct: which neighborhoods this phone joined,
 * who is blocked here, the relay setting, sealed traffic carried for others,
 * and invites signed today.
 */
import type {Envelope, Household, Neighborhood, Uuid} from '../sync/protocol';

export interface DeviceProfile {
  deviceId: Uuid;
  personId: Uuid;
}

export interface IssuedInvite {
  neighborhood: Uuid;
  member: Uuid;
  at: number;
}

/** Per-phone state that is never sent to anyone. */
export interface LocalState {
  neighborhoods: Neighborhood[];
  blocked: Uuid[];
  relayEnabled: boolean;
  issuedInvites: IssuedInvite[];
  /**
   * Highest Lamport clock seen. Kept so the clock never runs backwards when
   * old envelopes expire and are pruned.
   */
  lamport: number;
}

export const DEFAULT_LOCAL_STATE: LocalState = {
  neighborhoods: [],
  blocked: [],
  relayEnabled: true,
  issuedInvites: [],
  lamport: 0,
};

export interface LocalSnapshot {
  household: Household;
  device: DeviceProfile;
  /** Envelopes this phone can read, including its own. */
  envelopes: Envelope[];
  /** Traffic carried for others; unreadable here. */
  carried: Envelope[];
  local: LocalState;
}

export interface Repository {
  load(): Promise<LocalSnapshot | null>;
  saveHousehold(household: Household, device: DeviceProfile): Promise<void>;
  /** Insert envelopes, ignoring ones already stored. Returns the newly added ones. */
  addEnvelopes(envelopes: Envelope[]): Promise<Envelope[]>;
  /** Insert relay envelopes, ignoring ones already carried. */
  addCarried(envelopes: Envelope[]): Promise<void>;
  clearCarried(): Promise<void>;
  saveLocalState(state: LocalState): Promise<void>;
  /** Drop expired envelopes (readable and carried). Returns how many were removed. */
  pruneExpired(now: number): Promise<number>;
  clear(): Promise<void>;
}

export class MemoryRepository implements Repository {
  private household: Household | null = null;
  private device: DeviceProfile | null = null;
  private envelopes = new Map<Uuid, Envelope>();
  private carried = new Map<Uuid, Envelope>();
  private local: LocalState = DEFAULT_LOCAL_STATE;

  async load(): Promise<LocalSnapshot | null> {
    if (!this.household || !this.device) {
      return null;
    }
    return {
      household: this.household,
      device: this.device,
      envelopes: [...this.envelopes.values()],
      carried: [...this.carried.values()],
      local: this.local,
    };
  }

  async saveHousehold(household: Household, device: DeviceProfile): Promise<void> {
    this.household = household;
    this.device = device;
  }

  async addEnvelopes(envelopes: Envelope[]): Promise<Envelope[]> {
    return insertNew(this.envelopes, envelopes);
  }

  async addCarried(envelopes: Envelope[]): Promise<void> {
    insertNew(this.carried, envelopes);
  }

  async clearCarried(): Promise<void> {
    this.carried.clear();
  }

  async saveLocalState(state: LocalState): Promise<void> {
    this.local = state;
  }

  async pruneExpired(now: number): Promise<number> {
    return pruneMap(this.envelopes, now) + pruneMap(this.carried, now);
  }

  async clear(): Promise<void> {
    this.household = null;
    this.device = null;
    this.envelopes.clear();
    this.carried.clear();
    this.local = DEFAULT_LOCAL_STATE;
  }
}

export function insertNew(into: Map<Uuid, Envelope>, envelopes: Envelope[]): Envelope[] {
  const added: Envelope[] = [];
  for (const env of envelopes) {
    if (!into.has(env.id)) {
      into.set(env.id, env);
      added.push(env);
    }
  }
  return added;
}

export function pruneMap(map: Map<Uuid, Envelope>, now: number): number {
  let removed = 0;
  for (const [id, env] of map) {
    if (now >= env.expires_at) {
      map.delete(id);
      removed += 1;
    }
  }
  return removed;
}
