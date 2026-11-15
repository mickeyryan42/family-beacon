/**
 * Folds envelopes into what the screens show.
 *
 * Mirrors `crates/sync-core/src/conflict.rs` so the UI behaves the same way it
 * will once the Rust core is bridged in:
 *  - check-ins: latest reported time wins per person (then Lamport, then id)
 *  - meeting points: highest Lamport wins per point; removals are tombstones
 *  - notes: append-only, newest first, capped
 *
 * Every device holding the same envelopes derives the same view, regardless of
 * the order they arrived in.
 */
import type {CheckIn, Envelope, RendezvousPoint, UnixMillis, Uuid} from './protocol';

export const MAX_NOTES = 50;

export interface Note {
  envelopeId: Uuid;
  author: Uuid;
  text: string;
  createdAt: UnixMillis;
}

export interface HouseholdView {
  /** Latest check-in per person id. */
  checkIns: Record<Uuid, CheckIn>;
  /** Live meeting points, primary first. */
  rendezvous: RendezvousPoint[];
  /** Newest first. */
  notes: Note[];
}

type Stamp = [number, number, string];

function compareStamp(a: Stamp, b: Stamp): number {
  if (a[0] !== b[0]) {
    return a[0] - b[0];
  }
  if (a[1] !== b[1]) {
    return a[1] - b[1];
  }
  return a[2] < b[2] ? -1 : a[2] > b[2] ? 1 : 0;
}

export function buildView(envelopes: readonly Envelope[], now: UnixMillis): HouseholdView {
  const checkIns = new Map<Uuid, {stamp: Stamp; value: CheckIn}>();
  const points = new Map<Uuid, {stamp: Stamp; value: RendezvousPoint | null}>();
  const notes: Note[] = [];
  const seenNotes = new Set<Uuid>();

  const upsert = <V>(
    map: Map<Uuid, {stamp: Stamp; value: V}>,
    key: Uuid,
    stamp: Stamp,
    value: V,
  ) => {
    const existing = map.get(key);
    if (!existing || compareStamp(existing.stamp, stamp) < 0) {
      map.set(key, {stamp, value});
    }
  };

  for (const env of envelopes) {
    if (now >= env.expires_at || env.audience.scope !== 'household') {
      continue;
    }
    const p = env.payload;
    switch (p.type) {
      case 'check_in':
        upsert(checkIns, p.data.person_id, [p.data.recorded_at, env.lamport, env.id], p.data);
        break;
      case 'rendezvous_upsert':
        upsert(points, p.data.id, [env.lamport, env.created_at, env.id], p.data);
        break;
      case 'rendezvous_remove':
        upsert(points, p.data.id, [env.lamport, env.created_at, env.id], null);
        break;
      case 'note':
        if (!seenNotes.has(env.id)) {
          seenNotes.add(env.id);
          notes.push({
            envelopeId: env.id,
            author: env.author,
            text: p.data.text,
            createdAt: env.created_at,
          });
        }
        break;
    }
  }

  notes.sort((a, b) => b.createdAt - a.createdAt || (a.envelopeId < b.envelopeId ? 1 : -1));

  const rendezvous = [...points.values()]
    .map(entry => entry.value)
    .filter((p): p is RendezvousPoint => p !== null)
    .sort((a, b) => a.rank - b.rank || a.name.localeCompare(b.name));

  return {
    checkIns: Object.fromEntries([...checkIns].map(([id, entry]) => [id, entry.value])),
    rendezvous,
    notes: notes.slice(0, MAX_NOTES),
  };
}
