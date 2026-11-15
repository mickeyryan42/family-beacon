import {buildView} from '../src/sync/householdView';
import {createEnvelope, type Envelope, type Payload} from '../src/sync/protocol';

const HOUSEHOLD = 'h-1';
const PERSON = 'p-1';
const NOW = 1_700_000_000_000;

function env(lamport: number, at: number, payload: Payload): Envelope {
  return createEnvelope({
    audience: {scope: 'household', household_id: HOUSEHOLD},
    author: PERSON,
    origin_device: 'd-1',
    lamport,
    now: at,
    payload,
  });
}

const checkIn = (lamport: number, at: number, state: 'safe' | 'on_my_way') =>
  env(lamport, at, {type: 'check_in', data: {person_id: PERSON, state, recorded_at: at}});

const point = (id: string, name: string, lamport: number) =>
  env(lamport, NOW, {
    type: 'rendezvous_upsert',
    data: {id, household_id: HOUSEHOLD, name, details: '', when_to_go: '', rank: 1},
  });

describe('buildView', () => {
  it('keeps the most recently reported check-in, whatever the arrival order', () => {
    const earlier = checkIn(5, NOW, 'on_my_way');
    const later = checkIn(1, NOW + 1000, 'safe');
    for (const order of [
      [earlier, later],
      [later, earlier],
    ]) {
      expect(buildView(order, NOW + 2000).checkIns[PERSON].state).toBe('safe');
    }
  });

  it('applies the causally later edit to a meeting point', () => {
    const a = point('r-1', 'Park gate', 1);
    const b = point('r-1', 'Library steps', 2);
    expect(buildView([b, a], NOW).rendezvous.map(p => p.name)).toEqual(['Library steps']);
  });

  it('does not resurrect a removed point when an older copy arrives late', () => {
    const created = point('r-1', 'Park gate', 1);
    const removed = env(2, NOW, {type: 'rendezvous_remove', data: {id: 'r-1'}});
    expect(buildView([removed, created], NOW).rendezvous).toEqual([]);
  });

  it('drops expired envelopes and de-duplicates notes', () => {
    const note = env(1, NOW, {type: 'note', data: {text: 'Power is out'}});
    expect(buildView([note, note], NOW).notes).toHaveLength(1);
    expect(buildView([note], note.expires_at).notes).toHaveLength(0);
  });
});
