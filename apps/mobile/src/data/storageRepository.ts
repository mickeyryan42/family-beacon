/**
 * Durable repository backed by on-device key-value storage (AsyncStorage).
 *
 * An interim backend: once the Rust core is bridged in, its SQLite
 * `EnvelopeStore` replaces this. Until then this keeps everything across app
 * restarts, entirely on the phone.
 *
 * Layout: one JSON value per collection under a versioned prefix. Envelope
 * counts stay small (everything expires, and queues are budgeted), so writing
 * a whole collection at a time is cheap and keeps each write atomic.
 * Writes go through an in-memory cache and are serialized, so a fast burst
 * of updates can't save out of order.
 */
import AsyncStorage from '@react-native-async-storage/async-storage';

import type {Envelope, Household, Uuid} from '../sync/protocol';
import {
  DEFAULT_LOCAL_STATE,
  insertNew,
  pruneMap,
  type DeviceProfile,
  type LocalSnapshot,
  type LocalState,
  type Repository,
} from './repository';

/** The subset of AsyncStorage used here, so tests can supply their own. */
export interface KeyValueStorage {
  multiGet(keys: readonly string[]): Promise<readonly (readonly [string, string | null])[]>;
  multiSet(pairs: [string, string][]): Promise<void>;
  multiRemove(keys: readonly string[]): Promise<void>;
}

const PREFIX = 'family-beacon:v1:';
export const STORAGE_KEYS = {
  profile: `${PREFIX}profile`,
  envelopes: `${PREFIX}envelopes`,
  carried: `${PREFIX}carried`,
  local: `${PREFIX}local`,
} as const;
const ALL_KEYS = Object.values(STORAGE_KEYS);

interface Profile {
  household: Household;
  device: DeviceProfile;
}

interface Cache {
  profile: Profile | null;
  envelopes: Map<Uuid, Envelope>;
  carried: Map<Uuid, Envelope>;
  local: LocalState;
}

export class StorageRepository implements Repository {
  private cache: Cache | null = null;
  private loading: Promise<Cache> | null = null;
  private writes: Promise<void> = Promise.resolve();

  constructor(private readonly storage: KeyValueStorage = AsyncStorage as KeyValueStorage) {}

  async load(): Promise<LocalSnapshot | null> {
    const c = await this.read();
    if (!c.profile) {
      return null;
    }
    return {
      ...c.profile,
      envelopes: [...c.envelopes.values()],
      carried: [...c.carried.values()],
      local: c.local,
    };
  }

  async saveHousehold(household: Household, device: DeviceProfile): Promise<void> {
    const c = await this.read();
    c.profile = {household, device};
    await this.write(['profile']);
  }

  async addEnvelopes(envelopes: Envelope[]): Promise<Envelope[]> {
    const c = await this.read();
    const added = insertNew(c.envelopes, envelopes);
    if (added.length) {
      await this.write(['envelopes']);
    }
    return added;
  }

  async addCarried(envelopes: Envelope[]): Promise<void> {
    const c = await this.read();
    if (insertNew(c.carried, envelopes).length) {
      await this.write(['carried']);
    }
  }

  async clearCarried(): Promise<void> {
    const c = await this.read();
    if (c.carried.size) {
      c.carried.clear();
      await this.write(['carried']);
    }
  }

  async saveLocalState(state: LocalState): Promise<void> {
    const c = await this.read();
    c.local = state;
    await this.write(['local']);
  }

  async pruneExpired(now: number): Promise<number> {
    const c = await this.read();
    const fromEnvelopes = pruneMap(c.envelopes, now);
    const fromCarried = pruneMap(c.carried, now);
    const changed: (keyof Cache)[] = [];
    if (fromEnvelopes) {
      changed.push('envelopes');
    }
    if (fromCarried) {
      changed.push('carried');
    }
    if (changed.length) {
      await this.write(changed);
    }
    return fromEnvelopes + fromCarried;
  }

  async clear(): Promise<void> {
    this.cache = emptyCache();
    await this.enqueue(() => this.storage.multiRemove(ALL_KEYS));
  }

  private read(): Promise<Cache> {
    if (this.cache) {
      return Promise.resolve(this.cache);
    }
    this.loading ??= this.storage.multiGet(ALL_KEYS).then(
      pairs => {
        const raw = Object.fromEntries(pairs);
        this.cache = {
          profile: parse<Profile | null>(raw[STORAGE_KEYS.profile], null),
          envelopes: byId(parse<Envelope[]>(raw[STORAGE_KEYS.envelopes], [])),
          carried: byId(parse<Envelope[]>(raw[STORAGE_KEYS.carried], [])),
          local: {...DEFAULT_LOCAL_STATE, ...parse(raw[STORAGE_KEYS.local], {})},
        };
        return this.cache;
      },
      e => {
        // Let a later call retry instead of caching the failure.
        this.loading = null;
        throw e;
      },
    );
    return this.loading;
  }

  /** Snapshot the named collections now, save them in order. */
  private write(parts: (keyof Cache)[]): Promise<void> {
    const c = this.cache!;
    const pairs = parts.map((part): [string, string] => {
      const value = part === 'envelopes' || part === 'carried' ? [...c[part].values()] : c[part];
      return [STORAGE_KEYS[part], JSON.stringify(value)];
    });
    return this.enqueue(() => this.storage.multiSet(pairs));
  }

  private enqueue(op: () => Promise<void>): Promise<void> {
    const next = this.writes.then(op);
    // Keep the chain alive after a failure; the caller still sees the error.
    this.writes = next.catch(() => undefined);
    return next;
  }
}

function emptyCache(): Cache {
  return {
    profile: null,
    envelopes: new Map(),
    carried: new Map(),
    local: DEFAULT_LOCAL_STATE,
  };
}

function byId(envelopes: Envelope[]): Map<Uuid, Envelope> {
  return new Map(envelopes.map(e => [e.id, e]));
}

function parse<T>(raw: string | null | undefined, fallback: T): T {
  if (raw == null) {
    return fallback;
  }
  try {
    return JSON.parse(raw) as T;
  } catch {
    throw new Error('Saved data on this phone is damaged and could not be read');
  }
}
