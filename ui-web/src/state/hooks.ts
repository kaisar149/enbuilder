import { useSyncExternalStore } from 'react';
import type { Store } from 'wb-client';

/** Subscribes a component to a store; `select` must return a stable reference for unchanged data (pick a slice, do not build new objects). */
export function useStore<S, T>(store: Store<S>, select: (s: S) => T): T {
  return useSyncExternalStore(store.subscribe, () => select(store.getState()));
}
