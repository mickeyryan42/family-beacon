import {buildFeed, buildRoster, inviteChain, INVITES_PER_DAY} from '../src/sync/neighborhoodView';
import {
  createEnvelope,
  DAY_MS,
  type Envelope,
  type Neighborhood,
  type Payload,
} from '../src/sync/protocol';

const T0 = 1_700_000_000_000;
const info: Neighborhood = {id: 'n', name: 'Elm & Oak', area: '', founder: 'ana', created_at: T0};

function env(author: string, payload: Payload, at = T0 + 1_000): Envelope {
  return createEnvelope({
    audience: {scope: 'neighborhood', neighborhood_id: 'n'},
    author,
    origin_device: 'd',
    lamport: 1,
    now: at,
    payload,
  });
}

const join = (member: string, by: string | undefined, at = T0) =>
  env(member, {
    type: 'member_joined',
    data: {neighborhood_id: 'n', member, display_name: member, invited_by: by, invited_at: at},
  });

const remove = (by: string, member: string, cascade: boolean) =>
  env(by, {type: 'member_removed', data: {member, reason: 'spam', cascade}});

describe('buildRoster', () => {
  it('grows through invite chains in any arrival order', () => {
    const events = [
      join('cleo', 'ben', T0 + 2),
      join('ben', 'ana', T0 + 1),
      join('ana', undefined),
    ];
    const roster = buildRoster(info, events);
    expect([...roster.members.keys()].sort()).toEqual(['ana', 'ben', 'cleo']);
    expect(inviteChain(roster, 'cleo')).toEqual(['cleo', 'ben', 'ana']);
  });

  it('only stewards remove; without cascade, invitees stay', () => {
    const base = [join('ana', undefined), join('ben', 'ana', T0 + 1), join('cleo', 'ben', T0 + 2)];
    expect(buildRoster(info, [...base, remove('cleo', 'ben', false)]).members.has('ben')).toBe(
      true,
    );

    const roster = buildRoster(info, [...base, remove('ana', 'ben', false)]);
    expect(roster.members.has('ben')).toBe(false);
    expect(roster.members.has('cleo')).toBe(true);
    expect(roster.removed.get('ben')).toBe('spam');
  });

  it('cascade removes the invite subtree, never the founder', () => {
    const base = [join('ana', undefined), join('ben', 'ana', T0 + 1), join('cleo', 'ben', T0 + 2)];
    const roster = buildRoster(info, [
      ...base,
      remove('ana', 'ben', true),
      remove('ana', 'ana', true),
    ]);
    expect([...roster.members.keys()]).toEqual(['ana']);
  });

  it('caps invites per inviter per day', () => {
    const events = [join('ana', undefined)];
    for (let i = 0; i < INVITES_PER_DAY + 2; i++) {
      events.push(join(`guest${i}`, 'ana', T0 + 10 + i));
    }
    events.push(join('tomorrow', 'ana', T0 + DAY_MS + 100));
    const roster = buildRoster(info, events);
    expect(roster.members.size).toBe(1 + INVITES_PER_DAY + 1);
    expect(roster.members.has('tomorrow')).toBe(true);
  });
});

describe('buildFeed', () => {
  it('hides withdrawn posts and authors the caller filters out', () => {
    const offer = env('ben', {type: 'offer', data: {text: 'Generator at #12'}});
    const need = env('cleo', {type: 'need', data: {text: 'Ice'}}, T0 + 2_000);
    const forged = env('cleo', {type: 'withdraw', data: {envelope_id: offer.id}});
    expect(buildFeed([offer, need, forged], T0 + 3_000, () => true).map(p => p.kind)).toEqual([
      'need',
      'offer',
    ]);

    const own = env('ben', {type: 'withdraw', data: {envelope_id: offer.id}});
    expect(buildFeed([offer, need, own], T0 + 3_000, a => a !== 'cleo')).toEqual([]);
  });
});
