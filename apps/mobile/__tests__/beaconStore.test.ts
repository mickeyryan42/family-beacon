import {config} from '../src/config';
import {MemoryRepository} from '../src/data/repository';
import {DAY_MS} from '../src/sync/protocol';
import {sampleSnapshot} from '../src/data/sampleData';
import {BeaconStore} from '../src/state/beaconStore';

const NOW = 1_800_000_000_000;

const created: BeaconStore[] = [];

function makeStore() {
  let clock = NOW;
  const store = new BeaconStore(new MemoryRepository(), () => clock, 'mock');
  created.push(store);
  return {store, advance: (ms: number) => (clock += ms)};
}

// Stop every store's transport, even when a test fails before its own
// dispose(); otherwise a live mock timer keeps Jest from exiting.
afterEach(() => {
  created.splice(0).forEach(s => s.dispose());
  jest.useRealTimers();
});

// Let the store's async exchange settle.
const flush = () => new Promise<void>(resolve => setImmediate(() => resolve()));

describe('BeaconStore', () => {
  afterEach(() => jest.useRealTimers());

  it('starts in onboarding when nothing is stored', async () => {
    const {store} = makeStore();
    await store.init();
    expect(store.getSnapshot().phase).toBe('onboarding');
  });

  it('loads the sample household with fresh, unexpired data', async () => {
    const {store} = makeStore();
    await store.init();
    expect((await store.loadSampleHousehold()).ok).toBe(true);

    const snap = store.getSnapshot();
    expect(snap.phase).toBe('ready');
    expect(snap.household?.members.length).toBeGreaterThan(1);
    expect(snap.view.rendezvous.length).toBeGreaterThan(0);
    expect(Object.keys(snap.view.checkIns).length).toBeGreaterThan(0);
    store.dispose();
  });

  it('records a local check-in and rejects invalid input', async () => {
    const {store} = makeStore();
    await store.createHousehold('The Test family', 'Alex');
    const me = store.getSnapshot().device!.personId;

    expect((await store.checkIn(me, 'safe', 'All good')).ok).toBe(true);
    expect(store.getSnapshot().view.checkIns[me].state).toBe('safe');

    const tooLong = await store.postNote('x'.repeat(281));
    expect(tooLong.ok).toBe(false);
    expect(store.getSnapshot().view.notes).toHaveLength(0);
    store.dispose();
  });

  it('exchanges updates with a simulated family phone and does not resend them', async () => {
    jest.useFakeTimers({doNotFake: ['setImmediate']});
    const {store, advance} = makeStore();
    await store.loadSampleHousehold();
    const before = store.getSnapshot().envelopeCount;

    advance(60_000);
    expect(store.simulateEncounter('family')).toBe(true);
    await flush();

    let snap = store.getSnapshot();
    expect(snap.activity).toHaveLength(1);
    expect(snap.activity[0].sent).toBe(before);
    expect(snap.activity[0].received).toBe(1);
    expect(snap.envelopeCount).toBe(before + 1);
    expect(snap.nearby).toHaveLength(1);

    jest.advanceTimersByTime(10_000);
    expect(store.getSnapshot().nearby).toHaveLength(0);

    // Meeting the other simulated phone, then the first again: nothing is resent.
    store.simulateEncounter('family');
    await flush();
    store.simulateEncounter('family');
    await flush();
    snap = store.getSnapshot();
    expect(snap.activity[0].sent).toBe(1);
    store.dispose();
  });

  it('sample data is shifted so the newest update is recent', () => {
    const snap = sampleSnapshot(NOW);
    const newest = Math.max(...snap.envelopes.map(e => e.created_at));
    expect(NOW - newest).toBeLessThan(10 * 60_000);
    expect(snap.envelopes.every(e => e.expires_at > NOW)).toBe(true);
  });
});

describe('BeaconStore neighborhoods', () => {
  async function inSampleNeighborhood() {
    jest.useFakeTimers({doNotFake: ['setImmediate']});
    const {store, advance} = makeStore();
    await store.loadSampleHousehold();
    expect((await store.joinSampleNeighborhood()).ok).toBe(true);
    return {store, advance, hood: store.getSnapshot().neighborhoods[0]};
  }

  afterEach(() => jest.useRealTimers());

  it('joins with a roster and posts with a clear scope', async () => {
    const {store, advance, hood} = await inSampleNeighborhood();
    expect(hood.info.name).toBe('Elm & Oak');
    expect(hood.members.map(m => m.display_name)).toEqual(['Dana Okafor', 'Marcus Webb', 'Sam']);

    expect((await store.postAlert(hood.info.id, 'power', 'Out on Elm', 'Elm St')).ok).toBe(true);
    advance(1_000);
    expect((await store.shareHouseholdStatus(hood.info.id, true)).ok).toBe(true);
    const feed = store.getSnapshot().neighborhoods[0].feed;
    expect(feed.map(p => p.kind)).toEqual(['household_status', 'alert']);
    store.dispose();
  });

  it('receives neighbor posts and "Are you OK?", and answers', async () => {
    const {store, advance, hood} = await inSampleNeighborhood();
    // Two neighbors take turns; "Are you OK?" is the third thing each does.
    for (let i = 0; i < 6; i++) {
      advance(60_000);
      store.simulateEncounter('neighbor');
      await flush();
    }
    const snap = store.getSnapshot();
    expect(snap.neighborhoods[0].feed.length).toBeGreaterThanOrEqual(2);
    const ask = snap.inbox.find(m => m.kind === 'wellness_request');
    expect(ask).toBeDefined();

    expect((await store.replyWellness(ask!.from, 'safe')).ok).toBe(true);
    expect(store.getSnapshot().inbox.some(m => m.kind === 'wellness_reply')).toBe(true);

    // Neighbors never learn family check-ins: they aren't in our household.
    expect(hood.members.some(m => m.member === ask!.from)).toBe(true);
    store.dispose();
  });

  it('carries sealed traffic for a passer-by only while relaying is on', async () => {
    const {store, advance} = await inSampleNeighborhood();
    advance(60_000);
    store.simulateEncounter('passer-by');
    await flush();
    expect(store.getSnapshot().carryingForOthers).toBe(1);
    expect(store.getSnapshot().activity[0].carried).toBe(1);

    store.setRelayEnabled(false);
    expect(store.getSnapshot().carryingForOthers).toBe(0);
    advance(60_000);
    store.simulateEncounter('passer-by');
    await flush();
    expect(store.getSnapshot().carryingForOthers).toBe(0);
    store.dispose();
  });

  it('founders invite, appoint stewards and remove; the cascade is opt-in', async () => {
    const {store} = makeStore();
    await store.loadSampleHousehold();
    await store.createNeighborhood('Maple Court', 'Maple Ct');
    let hood = store.getSnapshot().neighborhoods[0];
    expect(hood.isFounder && hood.isSteward).toBe(true);

    await store.inviteNeighbor(hood.info.id, 'Priya');
    hood = store.getSnapshot().neighborhoods[0];
    const priya = hood.members.find(m => m.display_name === 'Priya')!;
    expect(hood.invitesLeftToday).toBe(9);
    expect(store.chainOf(hood.info.id, priya.member)).toEqual([priya.member, hood.info.founder]);

    expect((await store.addSteward(hood.info.id, priya.member)).ok).toBe(true);
    expect(store.getSnapshot().neighborhoods[0].stewards).toContain(priya.member);

    expect((await store.removeMember(hood.info.id, priya.member, 'left_area', false)).ok).toBe(
      true,
    );
    expect(store.getSnapshot().neighborhoods[0].members.map(m => m.display_name)).toEqual(['Sam']);
    store.dispose();
  });

  it('enforces limits: 3 neighborhoods, 10 invites and 20 posts a day', async () => {
    const {store} = makeStore();
    await store.loadSampleHousehold();
    for (let i = 0; i < 3; i++) {
      expect((await store.createNeighborhood(`Hood ${i}`, '')).ok).toBe(true);
    }
    expect((await store.createNeighborhood('Hood 4', '')).ok).toBe(false);

    const id = store.getSnapshot().neighborhoods[0].info.id;
    for (let i = 0; i < 10; i++) {
      expect((await store.inviteNeighbor(id, `Guest ${i}`)).ok).toBe(true);
    }
    expect((await store.inviteNeighbor(id, 'One more')).ok).toBe(false);

    for (let i = 0; i < 20; i++) {
      expect((await store.postNeed(id, `need ${i}`)).ok).toBe(true);
    }
    expect((await store.postNeed(id, 'too many')).ok).toBe(false);
    store.dispose();
  });

  it('blocking hides a neighbor on this phone only', async () => {
    const {store, advance} = await inSampleNeighborhood();
    advance(60_000);
    store.simulateEncounter('neighbor');
    await flush();
    const author = store.getSnapshot().neighborhoods[0].feed[0].author;
    store.block(author);
    expect(store.getSnapshot().neighborhoods[0].feed.some(p => p.author === author)).toBe(false);
    store.unblock(author);
    expect(store.getSnapshot().neighborhoods[0].feed.some(p => p.author === author)).toBe(true);
    store.dispose();
  });
});

describe('BeaconStore restart', () => {
  afterEach(() => jest.useRealTimers());

  it('restores neighborhoods, blocks, relay setting, carried traffic and invites', async () => {
    jest.useFakeTimers({doNotFake: ['setImmediate']});
    let clock = NOW;
    const repo = new MemoryRepository();
    const first = new BeaconStore(repo, () => clock, 'mock');
    created.push(first);
    await first.loadSampleHousehold();
    await first.joinSampleNeighborhood();
    await first.createNeighborhood('Maple Court', 'Maple Ct');
    const maple = first.getSnapshot().neighborhoods.find(n => n.info.name === 'Maple Court')!;
    await first.inviteNeighbor(maple.info.id, 'Priya');

    clock += 60_000;
    first.simulateEncounter('passer-by');
    await flush();
    clock += 60_000;
    first.simulateEncounter('neighbor');
    await flush();
    const before = first.getSnapshot();
    const author = before.neighborhoods.find(n => n.info.name === 'Elm & Oak')!.feed[0].author;
    await first.block(author);
    expect(before.carryingForOthers).toBe(1);
    first.dispose();

    const second = new BeaconStore(repo, () => clock, 'mock');
    created.push(second);
    await second.init();
    let snap = second.getSnapshot();
    expect(snap.phase).toBe('ready');
    expect(snap.neighborhoods.map(n => n.info.name)).toEqual(['Elm & Oak', 'Maple Court']);
    expect(snap.blocked).toEqual([author]);
    expect(snap.carryingForOthers).toBe(1);
    expect(snap.relayEnabled).toBe(true);
    expect(snap.neighborhoods.find(n => n.info.name === 'Maple Court')!.invitesLeftToday).toBe(9);

    // Turning relaying off forgets carried traffic, and stays off after a restart.
    await second.setRelayEnabled(false);
    second.dispose();
    const third = new BeaconStore(repo, () => clock, 'mock');
    created.push(third);
    await third.init();
    snap = third.getSnapshot();
    expect(snap.relayEnabled).toBe(false);
    expect(snap.carryingForOthers).toBe(0);
  });

  it('reset clears local state as well as the household', async () => {
    const repo = new MemoryRepository();
    const store = new BeaconStore(repo, () => NOW, 'mock');
    created.push(store);
    await store.loadSampleHousehold();
    await store.createNeighborhood('Maple Court', '');
    await store.setRelayEnabled(false);
    await store.reset();
    expect(store.getSnapshot().relayEnabled).toBe(true);
    await store.loadSampleHousehold();
    expect(store.getSnapshot().neighborhoods).toHaveLength(0);
    expect((await repo.load())!.local.relayEnabled).toBe(true);
  });
});

describe('BeaconStore freshness', () => {
  afterEach(() => jest.useRealTimers());

  it('drops expired check-ins on its own as time passes', async () => {
    jest.useFakeTimers({doNotFake: ['setImmediate']});
    let clock = NOW;
    const repo = new MemoryRepository();
    const store = new BeaconStore(repo, () => clock, 'mock');
    created.push(store);
    await store.createHousehold('The Test family', 'Alex');
    const me = store.getSnapshot().device!.personId;
    await store.checkIn(me, 'safe');
    expect(store.getSnapshot().view.checkIns[me]).toBeDefined();

    // No new events at all; the ticker alone must notice the expiry.
    clock += 8 * DAY_MS;
    jest.advanceTimersByTime(config.viewRefreshIntervalMs);
    await flush();
    expect(store.getSnapshot().view.checkIns[me]).toBeUndefined();
    expect(store.getSnapshot().envelopeCount).toBe(0);
    expect((await repo.load())!.envelopes).toHaveLength(0);
  });

  it('never reuses a Lamport clock after old updates are pruned', async () => {
    let clock = NOW;
    const repo = new MemoryRepository();
    const first = new BeaconStore(repo, () => clock, 'mock');
    created.push(first);
    await first.createHousehold('The Test family', 'Alex');
    for (let i = 0; i < 3; i++) {
      await first.postNote(`note ${i}`);
    }
    const highest = Math.max(...(await repo.load())!.envelopes.map(e => e.lamport));
    first.dispose();

    clock += 30 * DAY_MS;
    const second = new BeaconStore(repo, () => clock, 'mock');
    created.push(second);
    await second.init();
    expect(second.getSnapshot().envelopeCount).toBe(0);
    await second.postNote('after the gap');
    const [next] = (await repo.load())!.envelopes;
    expect(next.lamport).toBe(highest + 1);
  });
});
