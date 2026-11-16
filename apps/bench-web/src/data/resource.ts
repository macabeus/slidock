import { createContext, useContext, useEffect, useSyncExternalStore } from 'react';

import type { BenchSource } from './source';

export type Resource<T> =
  | { readonly state: 'loading' }
  | { readonly state: 'ready'; readonly value: T }
  | { readonly state: 'error'; readonly error: Error };

const LOADING: Resource<never> = { state: 'loading' };

/** Loads each key once and keeps the outcome; readers re-render when a load settles. */
export class ResourceCache {
  #entries = new Map<string, Resource<unknown>>();
  #listeners = new Set<() => void>();

  /** Starts loading `key` unless it is loaded or loading. */
  ensure(key: string, load: () => Promise<unknown>) {
    if (this.#entries.has(key)) {
      return;
    }
    this.#entries.set(key, LOADING);
    load().then(
      (value) => this.#settle(key, { state: 'ready', value }),
      (error: unknown) =>
        this.#settle(key, { state: 'error', error: error instanceof Error ? error : new Error(String(error)) }),
    );
  }

  peek(key: string): Resource<unknown> {
    return this.#entries.get(key) ?? LOADING;
  }

  subscribe = (listener: () => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  #settle(key: string, resource: Resource<unknown>) {
    this.#entries.set(key, resource);
    for (const listener of this.#listeners) {
      listener();
    }
  }
}

export interface BenchData {
  source: BenchSource;
  cache: ResourceCache;
}

export const BenchDataContext = createContext<BenchData | null>(null);

export function useBenchData(): BenchData {
  const value = useContext(BenchDataContext);
  if (!value) {
    throw new Error('useBenchData needs a <BenchDataContext.Provider>');
  }
  return value;
}

/** A value read through the source, by a key naming it; `null` key reads nothing. */
export function useResource<T>(key: string | null, load: (source: BenchSource) => Promise<T>): Resource<T> {
  const { source, cache } = useBenchData();
  const resource = useSyncExternalStore(cache.subscribe, () => (key === null ? LOADING : cache.peek(key)));
  useEffect(() => {
    if (key !== null) {
      cache.ensure(key, () => load(source));
    }
    // The key names what is loaded; `load` is recreated on every render and left out.
  }, [key, cache, source]);
  return resource as Resource<T>;
}
