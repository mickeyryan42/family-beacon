/**
 * Neighborhood membership, feed and direct-message folds.
 *
 * Mirrors `crates/sync-core/src/neighborhood.rs` and `feed.rs`: the roster is
 * recomputed from every membership event, so every phone with the same
 * events agrees on who is in, whatever order they arrived in. The Rust core
 * additionally verifies every certificate's signature; the mirror trusts the
 * audience check instead (see protocol.ts).
 */
import {
  DAY_MS,
  type AlertCategory,
  type CheckInState,
  type Envelope,
  type MembershipCert,
  type Neighborhood,
  type RemovalReason,
  type UnixMillis,
  type Uuid,
} from './protocol';

export const MAX_NEIGHBORHOODS = 3;
export const INVITES_PER_DAY = 10;
export const POSTS_PER_DAY = 20;
export const DIRECT_PER_DAY = 30;
export const SOFT_MEMBER_CAP = 500;

export interface Roster {
  members: Map<Uuid, MembershipCert>;
  stewards: Set<Uuid>;
  removed: Map<Uuid, RemovalReason | 'cascade'>;
}

/** Recompute who is in a neighborhood from its membership events. */
export function buildRoster(info: Neighborhood, events: readonly Envelope[]): Roster {
  const founder = info.founder;
  const certs: MembershipCert[] = [];
  const grants: [Uuid, Uuid][] = [];
  const removals: {by: Uuid; member: Uuid; reason: RemovalReason; cascade: boolean; at: number}[] =
    [];
  for (const e of events) {
    const p = e.payload;
    if (p.type === 'member_joined') {
      certs.push(p.data);
    } else if (p.type === 'steward_added') {
      grants.push([e.author, p.data.member]);
    } else if (p.type === 'member_removed') {
      removals.push({by: e.author, ...p.data, at: e.created_at});
    }
  }

  const stewards = new Set<Uuid>([founder]);
  grants.filter(([by]) => by === founder).forEach(([, m]) => stewards.add(m));

  // Removals by stewards, never of the founder, with optional cascade.
  const removedAt = new Map<Uuid, number>();
  const reasons = new Map<Uuid, RemovalReason | 'cascade'>();
  const mark = (p: Uuid, at: number, why: RemovalReason | 'cascade') => {
    removedAt.set(p, Math.min(removedAt.get(p) ?? at, at));
    if (!reasons.has(p) || why !== 'cascade') {
      reasons.set(p, why);
    }
  };
  for (const r of removals) {
    if (!stewards.has(r.by) || r.member === founder) {
      continue;
    }
    mark(r.member, r.at, r.reason);
    if (r.cascade) {
      const frontier = [r.member];
      const seen = new Set(frontier);
      while (frontier.length) {
        const p = frontier.pop()!;
        for (const c of certs.filter(x => x.invited_by === p)) {
          if (c.member !== founder && !seen.has(c.member)) {
            seen.add(c.member);
            mark(c.member, r.at, 'cascade');
            frontier.push(c.member);
          }
        }
      }
    }
  }

  // Invites beyond an inviter's daily allowance don't count.
  const overLimit = new Set<string>();
  const byInviter = new Map<Uuid, MembershipCert[]>();
  certs.forEach(c => {
    if (c.invited_by) {
      byInviter.set(c.invited_by, [...(byInviter.get(c.invited_by) ?? []), c]);
    }
  });
  byInviter.forEach((list, inviter) => {
    list.sort((a, b) => a.invited_at - b.invited_at || (a.member < b.member ? -1 : 1));
    list.forEach((c, i) => {
      const earlier = list.slice(0, i).filter(x => c.invited_at - x.invited_at < DAY_MS).length;
      if (earlier >= INVITES_PER_DAY) {
        overLimit.add(`${inviter}:${c.member}`);
      }
    });
  });

  // Valid invite chains, regardless of later removals; a removed member's
  // invites after their removal don't count.
  const valid = new Map<Uuid, MembershipCert>();
  const founderCert = certs.find(c => c.member === founder && !c.invited_by);
  if (founderCert) {
    valid.set(founder, founderCert);
  }
  const candidates = [...certs].sort(
    (a, b) => a.invited_at - b.invited_at || (a.member < b.member ? -1 : 1),
  );
  let grew = true;
  while (grew) {
    grew = false;
    for (const c of candidates) {
      const inviter = c.invited_by;
      if (!inviter || valid.has(c.member) || overLimit.has(`${inviter}:${c.member}`)) {
        continue;
      }
      const inviterRemoved = removedAt.get(inviter);
      if (inviterRemoved !== undefined && inviterRemoved <= c.invited_at) {
        continue;
      }
      if (valid.has(inviter)) {
        valid.set(c.member, c);
        grew = true;
      }
    }
  }

  removedAt.forEach((_, p) => valid.delete(p));
  [...stewards].forEach(s => {
    if (!valid.has(s)) {
      stewards.delete(s);
    }
  });
  return {members: valid, stewards, removed: reasons};
}

/** Who invited whom, from this person back to the founder. */
export function inviteChain(roster: Roster, person: Uuid): Uuid[] {
  const chain = [person];
  let current = roster.members.get(person)?.invited_by;
  while (current && !chain.includes(current)) {
    chain.push(current);
    current = roster.members.get(current)?.invited_by;
  }
  return chain;
}

/** Members `person` invited, directly or indirectly. */
export function invitedSubtree(roster: Roster, person: Uuid): Uuid[] {
  const out: Uuid[] = [];
  const frontier = [person];
  while (frontier.length) {
    const p = frontier.pop()!;
    roster.members.forEach(c => {
      if (c.invited_by === p && !out.includes(c.member)) {
        out.push(c.member);
        frontier.push(c.member);
      }
    });
  }
  return out;
}

// --- feed ---

export type PostKind =
  | {kind: 'alert'; category: AlertCategory; text: string; area?: string}
  | {kind: 'offer'; text: string}
  | {kind: 'need'; text: string}
  | {kind: 'household_status'; household_name: string; all_safe: boolean; text?: string};

export type Post = PostKind & {envelopeId: Uuid; author: Uuid; createdAt: UnixMillis};

/** Visible posts, newest first. `visible` hides removed and blocked authors. */
export function buildFeed(
  envelopes: readonly Envelope[],
  now: UnixMillis,
  visible: (author: Uuid) => boolean,
): Post[] {
  const posts: Post[] = [];
  const withdrawn = new Map<Uuid, Uuid[]>();
  for (const e of envelopes) {
    if (now >= e.expires_at) {
      continue;
    }
    const base = {envelopeId: e.id, author: e.author, createdAt: e.created_at};
    const p = e.payload;
    switch (p.type) {
      case 'alert':
        posts.push({...base, kind: 'alert', ...p.data});
        break;
      case 'offer':
      case 'need':
        posts.push({...base, kind: p.type, text: p.data.text});
        break;
      case 'household_status':
        posts.push({...base, kind: 'household_status', ...p.data});
        break;
      case 'withdraw':
        withdrawn.set(p.data.envelope_id, [...(withdrawn.get(p.data.envelope_id) ?? []), e.author]);
        break;
    }
  }
  return posts
    .filter(post => !withdrawn.get(post.envelopeId)?.includes(post.author) && visible(post.author))
    .sort((a, b) => b.createdAt - a.createdAt || (a.envelopeId < b.envelopeId ? 1 : -1));
}

// --- direct messages ---

export type DirectKind =
  | {kind: 'message'; text: string}
  | {kind: 'wellness_request'; text?: string}
  | {kind: 'wellness_reply'; state: CheckInState; text?: string};

export type DirectMessage = DirectKind & {
  envelopeId: Uuid;
  from: Uuid;
  to: Uuid;
  createdAt: UnixMillis;
};

export function buildInbox(envelopes: readonly Envelope[], now: UnixMillis): DirectMessage[] {
  const out: DirectMessage[] = [];
  for (const e of envelopes) {
    if (e.audience.scope !== 'direct' || now >= e.expires_at) {
      continue;
    }
    const base = {
      envelopeId: e.id,
      from: e.author,
      to: e.audience.recipient,
      createdAt: e.created_at,
    };
    const p = e.payload;
    if (p.type === 'message') {
      out.push({...base, kind: 'message', text: p.data.text});
    } else if (p.type === 'wellness_request') {
      out.push({...base, kind: 'wellness_request', text: p.data.text});
    } else if (p.type === 'wellness_reply') {
      out.push({...base, kind: 'wellness_reply', state: p.data.state, text: p.data.text});
    }
  }
  return out.sort((a, b) => a.createdAt - b.createdAt);
}

/** Count of envelopes `author` created today that match `pred`. */
export function usedToday(
  envelopes: readonly Envelope[],
  author: Uuid,
  now: UnixMillis,
  pred: (e: Envelope) => boolean,
): number {
  const day = Math.floor(now / DAY_MS);
  return envelopes.filter(
    e => e.author === author && Math.floor(e.created_at / DAY_MS) === day && pred(e),
  ).length;
}
