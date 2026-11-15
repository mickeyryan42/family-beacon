/**
 * TypeScript mirror of the wire types in `crates/protocol` (protocol v2).
 *
 * The Rust sync core is the long-term owner of this logic; it will be exposed
 * to React Native through a native module (see docs/architecture.md). Until
 * that bridge exists, the app runs this mirror so the UI can be built against
 * real data shapes. Field names and tags match the Rust serde output exactly,
 * and `fixtures/sample-household.json` is checked by a Rust contract test.
 *
 * What the mirror does NOT do: real cryptography. The Rust core seals every
 * envelope and verifies every signature; here, "can this phone read it?" is
 * decided by the envelope's audience, which is the same rule the encryption
 * enforces. See docs/neighborhood-network.md.
 */

export type Uuid = string;
export type UnixMillis = number;

export const PROTOCOL_VERSION = 2;

const HOUR_MS = 60 * 60 * 1000;
export const DAY_MS = 24 * HOUR_MS;

export type PersonRole = 'adult' | 'child' | 'caregiver';

export interface Person {
  id: Uuid;
  display_name: string;
  role: PersonRole;
}

export interface Household {
  id: Uuid;
  name: string;
  members: Person[];
}

export type CheckInState = 'safe' | 'on_my_way' | 'at_rendezvous' | 'need_help';

export interface CheckIn {
  person_id: Uuid;
  state: CheckInState;
  note?: string;
  rendezvous_id?: Uuid;
  recorded_at: UnixMillis;
}

export interface GeoPoint {
  lat_e7: number;
  lon_e7: number;
}

export interface RendezvousPoint {
  id: Uuid;
  household_id: Uuid;
  name: string;
  details: string;
  location?: GeoPoint;
  when_to_go: string;
  rank: number;
}

// --- neighborhoods ---

export interface Neighborhood {
  id: Uuid;
  name: string;
  /** Approximate area in words. Never coordinates. */
  area: string;
  founder: Uuid;
  created_at: UnixMillis;
}

export interface MembershipCert {
  neighborhood_id: Uuid;
  member: Uuid;
  display_name: string;
  /** Absent only for the founder. */
  invited_by?: Uuid;
  invited_at: UnixMillis;
}

export type AlertCategory = 'power' | 'water' | 'road' | 'medical' | 'fire' | 'other';
export type RemovalReason = 'spam' | 'abuse' | 'left_area' | 'other';

export type Scope = 'household' | 'neighborhood' | 'direct';

export type Audience =
  | {scope: 'household'; household_id: Uuid}
  | {scope: 'neighborhood'; neighborhood_id: Uuid}
  | {scope: 'direct'; recipient: Uuid};

export type Payload =
  // household
  | {type: 'check_in'; data: CheckIn}
  | {type: 'rendezvous_upsert'; data: RendezvousPoint}
  | {type: 'rendezvous_remove'; data: {id: Uuid}}
  | {type: 'note'; data: {text: string}}
  // neighborhood
  | {type: 'alert'; data: {category: AlertCategory; text: string; area?: string}}
  | {type: 'offer'; data: {text: string}}
  | {type: 'need'; data: {text: string}}
  | {type: 'household_status'; data: {household_name: string; all_safe: boolean; text?: string}}
  | {type: 'withdraw'; data: {envelope_id: Uuid}}
  | {type: 'member_joined'; data: MembershipCert}
  | {type: 'member_removed'; data: {member: Uuid; reason: RemovalReason; cascade: boolean}}
  | {type: 'steward_added'; data: {member: Uuid}}
  // direct
  | {type: 'message'; data: {text: string}}
  | {type: 'wellness_request'; data: {text?: string}}
  | {type: 'wellness_reply'; data: {state: CheckInState; text?: string}};

export type PayloadType = Payload['type'];

export interface Envelope {
  version: number;
  id: Uuid;
  audience: Audience;
  author: Uuid;
  origin_device: Uuid;
  created_at: UnixMillis;
  lamport: number;
  expires_at: UnixMillis;
  payload: Payload;
  signature?: string;
}

const HOUSEHOLD_TYPES: PayloadType[] = [
  'check_in',
  'rendezvous_upsert',
  'rendezvous_remove',
  'note',
];
const DIRECT_TYPES: PayloadType[] = ['message', 'wellness_request', 'wellness_reply'];
const MEMBERSHIP_TYPES: PayloadType[] = ['member_joined', 'member_removed', 'steward_added'];

/** The one scope each payload may be sent in (`Payload::scope` in Rust). */
export function payloadScope(payload: Payload): Scope {
  if (HOUSEHOLD_TYPES.includes(payload.type)) {
    return 'household';
  }
  return DIRECT_TYPES.includes(payload.type) ? 'direct' : 'neighborhood';
}

export function isMembershipEvent(payload: Payload): boolean {
  return MEMBERSHIP_TYPES.includes(payload.type);
}

/** Relays beyond the author (`Scope::default_hops`). */
export function defaultHops(scope: Scope): number {
  return scope === 'household' ? 12 : 16;
}

/** Same TTLs as `Payload::default_ttl_ms` in Rust. */
export function defaultTtlMs(payload: Payload): number {
  switch (payload.type) {
    case 'check_in':
    case 'note':
    case 'message':
    case 'wellness_request':
    case 'wellness_reply':
      return 2 * DAY_MS;
    case 'rendezvous_upsert':
    case 'rendezvous_remove':
      return 90 * DAY_MS;
    case 'member_joined':
    case 'member_removed':
    case 'steward_added':
      return 365 * DAY_MS;
    case 'alert':
    case 'offer':
    case 'need':
    case 'household_status':
    case 'withdraw':
      return DAY_MS;
  }
}

export type PriorityClass = 'low' | 'normal' | 'urgent';

/** Coarse priority relays see (`Payload::priority` mapped to a class). */
export function priorityClass(payload: Payload): PriorityClass {
  switch (payload.type) {
    case 'check_in':
      return payload.data.state === 'need_help' ? 'urgent' : 'normal';
    case 'wellness_reply':
      return payload.data.state === 'need_help' ? 'urgent' : 'normal';
    case 'alert':
      return payload.data.category === 'medical' || payload.data.category === 'fire'
        ? 'urgent'
        : 'normal';
    case 'note':
    case 'withdraw':
      return 'low';
    default:
      return 'normal';
  }
}

export function isExpired(envelope: {expires_at: UnixMillis}, now: UnixMillis): boolean {
  return now >= envelope.expires_at;
}

/**
 * Development-only UUIDv4. Hermes has no `crypto.randomUUID`, and pulling in a
 * native RNG module is not worth it for mock data. Production IDs will be
 * generated by the Rust core from the OS random source.
 */
/* eslint-disable no-bitwise -- setting UUID version and variant bits */
export function devUuid(): Uuid {
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, c => {
    const r = (Math.random() * 16) | 0;
    const v = c === 'x' ? r : (r & 0x3) | 0x8;
    return v.toString(16);
  });
}
/* eslint-enable no-bitwise */

export interface NewEnvelope {
  audience: Audience;
  author: Uuid;
  origin_device: Uuid;
  lamport: number;
  now: UnixMillis;
  payload: Payload;
}

export function createEnvelope(input: NewEnvelope): Envelope {
  return {
    version: PROTOCOL_VERSION,
    id: devUuid(),
    audience: input.audience,
    author: input.author,
    origin_device: input.origin_device,
    created_at: input.now,
    lamport: input.lamport,
    expires_at: input.now + defaultTtlMs(input.payload),
    payload: input.payload,
  };
}

export const LIMITS = {
  maxNoteChars: 280,
  maxNameChars: 80,
  maxClockSkewMs: 15 * 60 * 1000,
} as const;

/** What a reading device knows about its circles (`Membership` in Rust). */
export interface ReaderContext {
  localPersonId: Uuid;
  household: Household;
  inNeighborhood(id: Uuid): boolean;
  isNeighborhoodMember(id: Uuid, person: Uuid): boolean;
  /** In our household or any neighborhood we're in. */
  knows(person: Uuid): boolean;
}

function required(text: string, max: number): string | null {
  if (!text.trim()) {
    return 'empty';
  }
  return text.length > max ? 'too long' : null;
}

function optional(text: string | undefined, max: number): string | null {
  return text && text.length > max ? 'too long' : null;
}

/**
 * Structural checks, mirroring `validate_envelope` in Rust. Returns a short
 * reason, or null when the envelope is acceptable.
 */
export function validateEnvelope(
  envelope: Envelope,
  ctx: ReaderContext,
  now: UnixMillis,
): string | null {
  const {household} = ctx;
  if (envelope.version !== PROTOCOL_VERSION) {
    return `unsupported protocol version ${envelope.version}`;
  }
  if (payloadScope(envelope.payload) !== envelope.audience.scope) {
    return 'payload is not allowed in this scope';
  }

  const a = envelope.audience;
  const p = envelope.payload;
  if (a.scope === 'household') {
    const isMember = (id: Uuid) => household.members.some(m => m.id === id);
    if (a.household_id !== household.id) {
      return 'envelope belongs to a different household';
    }
    if (!isMember(envelope.author)) {
      return 'author is not a member of this household';
    }
    if (p.type === 'check_in' && !isMember(p.data.person_id)) {
      return 'check-in subject is not a member of this household';
    }
  } else if (a.scope === 'neighborhood') {
    if (!ctx.inNeighborhood(a.neighborhood_id)) {
      return 'not in that neighborhood';
    }
    if (p.type === 'member_joined') {
      if (p.data.neighborhood_id !== a.neighborhood_id || p.data.member !== envelope.author) {
        return 'membership event does not match its neighborhood or author';
      }
    } else if (!ctx.isNeighborhoodMember(a.neighborhood_id, envelope.author)) {
      return 'author is not a member of the neighborhood';
    }
  } else {
    if (a.recipient !== ctx.localPersonId) {
      return 'direct message is addressed to someone else';
    }
    if (!ctx.knows(envelope.author)) {
      return 'direct message from someone outside every circle we share';
    }
  }

  if (envelope.expires_at <= envelope.created_at) {
    return 'envelope expires before it was created';
  }
  if (isExpired(envelope, now)) {
    return 'envelope has expired';
  }
  if (envelope.created_at > now + LIMITS.maxClockSkewMs) {
    return 'envelope timestamp is too far in the future';
  }

  const note = LIMITS.maxNoteChars;
  const name = LIMITS.maxNameChars;
  switch (p.type) {
    case 'check_in':
      return optional(p.data.note, note);
    case 'rendezvous_upsert':
      if (p.data.household_id !== household.id) {
        return 'rendezvous belongs to a different household';
      }
      return required(p.data.name, name);
    case 'note':
    case 'offer':
    case 'need':
    case 'message':
      return required(p.data.text, note);
    case 'alert':
      return required(p.data.text, note) ?? optional(p.data.area, name);
    case 'household_status':
      return required(p.data.household_name, name) ?? optional(p.data.text, note);
    case 'wellness_request':
    case 'wellness_reply':
      return optional(p.data.text, note);
    case 'member_joined':
      return required(p.data.display_name, name);
    case 'rendezvous_remove':
    case 'withdraw':
    case 'member_removed':
    case 'steward_added':
      return null;
  }
}
