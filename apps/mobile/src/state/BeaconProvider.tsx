import React, {createContext, useContext, useEffect, useState, useSyncExternalStore} from 'react';
import {AppState} from 'react-native';
import NetInfo from '@react-native-community/netinfo';

import {BeaconStore, type BeaconSnapshot} from './beaconStore';

const StoreContext = createContext<BeaconStore | null>(null);

export function BeaconProvider({
  children,
  store: injected,
}: {
  children: React.ReactNode;
  /** Tests inject a store with a fixed clock. */
  store?: BeaconStore;
}) {
  const [store] = useState(() => injected ?? new BeaconStore());

  useEffect(() => {
    store.init();
    // Time passed while backgrounded: drop expired updates, refresh "x min ago".
    const sub = AppState.addEventListener('change', state => {
      if (state === 'active') {
        store.refresh();
      }
    });
    return () => {
      sub.remove();
      store.dispose();
    };
  }, [store]);

  return <StoreContext.Provider value={store}>{children}</StoreContext.Provider>;
}

export function useBeaconStore(): BeaconStore {
  const store = useContext(StoreContext);
  if (!store) {
    throw new Error('useBeaconStore must be used inside <BeaconProvider>');
  }
  return store;
}

export function useBeacon(): BeaconSnapshot {
  const store = useBeaconStore();
  return useSyncExternalStore(store.subscribe, store.getSnapshot);
}

export type Connectivity = 'no-signal' | 'online' | 'unknown';

/**
 * Whether the phone has any internet path. Family Beacon never needs one;
 * this only drives the banner so people understand the app is still working.
 */
export function useConnectivity(): Connectivity {
  const {simulateNoSignal} = useBeacon();
  const [state, setState] = useState<Connectivity>('unknown');

  useEffect(
    () =>
      NetInfo.addEventListener(info => {
        if (info.isConnected === null) {
          setState('unknown');
        } else {
          setState(info.isConnected && info.isInternetReachable !== false ? 'online' : 'no-signal');
        }
      }),
    [],
  );

  return simulateNoSignal ? 'no-signal' : state;
}
