import AsyncStorage from '@react-native-async-storage/async-storage';

import {sampleSnapshot} from '../src/data/sampleData';
import {STORAGE_KEYS, StorageRepository} from '../src/data/storageRepository';
import {BeaconStore} from '../src/state/beaconStore';

const NOW = 1_800_000_000_000;

beforeEach(() => AsyncStorage.clear());

describe('StorageRepository', () => {
  it('survives a restart: profile, envelopes, carried traffic and local state', async () => {
    const sample = sampleSnapshot(NOW);
    const first = new StorageRepository();
    expect(await first.load()).toBeNull();

    await first.saveHousehold(sample.household, sample.device);
    const added = await first.addEnvelopes(sample.envelopes);
    expect(added).toHaveLength(sample.envelopes.length);
    expect(await first.addEnvelopes(sample.envelopes)).toHaveLength(0);
    await first.addCarried([sample.envelopes[0]]);
    await first.saveLocalState({
      neighborhoods: [],
      blocked: ['someone'],
      relayEnabled: false,
      issuedInvites: [],
      lamport: 7,
    });

    // A fresh instance reads only what reached storage.
    const loaded = await new StorageRepository().load();
    expect(loaded?.household).toEqual(sample.household);
    expect(loaded?.device).toEqual(sample.device);
    expect(loaded?.envelopes.map(e => e.id).sort()).toEqual(sample.envelopes.map(e => e.id).sort());
    expect(loaded?.carried).toHaveLength(1);
    expect(loaded?.local.blocked).toEqual(['someone']);
    expect(loaded?.local.relayEnabled).toBe(false);
    expect(loaded?.local.lamport).toBe(7);
  });

  it('prunes expired envelopes durably and clears everything', async () => {
    const sample = sampleSnapshot(NOW);
    const repo = new StorageRepository();
    await repo.saveHousehold(sample.household, sample.device);
    await repo.addEnvelopes(sample.envelopes);
    await repo.addCarried([sample.envelopes[0]]);

    const latest = Math.max(...sample.envelopes.map(e => e.expires_at));
    expect(await repo.pruneExpired(latest)).toBe(sample.envelopes.length + 1);
    const reloaded = await new StorageRepository().load();
    expect(reloaded?.envelopes).toHaveLength(0);
    expect(reloaded?.carried).toHaveLength(0);

    await repo.clear();
    expect(await new StorageRepository().load()).toBeNull();
    expect(await AsyncStorage.getAllKeys()).toHaveLength(0);
  });

  it('saves bursts of writes in order', async () => {
    const sample = sampleSnapshot(NOW);
    const repo = new StorageRepository();
    await repo.saveHousehold(sample.household, sample.device);
    await Promise.all(sample.envelopes.map(e => repo.addEnvelopes([e])));
    const reloaded = await new StorageRepository().load();
    expect(reloaded?.envelopes).toHaveLength(sample.envelopes.length);
  });

  it('reports damaged data instead of silently starting over', async () => {
    await AsyncStorage.setItem(STORAGE_KEYS.profile, '{not json');
    await expect(new StorageRepository().load()).rejects.toThrow(/damaged/);
  });

  it('backs a BeaconStore across an app restart', async () => {
    const first = new BeaconStore(new StorageRepository(), () => NOW, 'mock');
    await first.createHousehold('The Test family', 'Alex');
    const me = first.getSnapshot().device!.personId;
    await first.checkIn(me, 'safe', 'Home');
    await first.createNeighborhood('Maple Court', 'Maple Ct');
    first.dispose();

    const second = new BeaconStore(new StorageRepository(), () => NOW + 60_000, 'mock');
    await second.init();
    const snap = second.getSnapshot();
    second.dispose();
    expect(snap.phase).toBe('ready');
    expect(snap.household?.name).toBe('The Test family');
    expect(snap.view.checkIns[me].state).toBe('safe');
    expect(snap.neighborhoods.map(n => n.info.name)).toEqual(['Maple Court']);
  });
});
