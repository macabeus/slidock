import { useMemo, useSyncExternalStore } from 'react';

// URL state in the fragment (`#slide=3&step=1`): it never reaches a server and needs no SPA fallback on static hosts.
// A view change pushes a history entry; position and filter changes replace the current one.

export type HistoryMode = 'push' | 'replace';

const listeners = new Set<() => void>();

function notify() {
  for (const listener of listeners) {
    listener();
  }
}

let installed = false;

function install() {
  if (installed || typeof window === 'undefined') {
    return;
  }
  installed = true;
  window.addEventListener('hashchange', notify);
  window.addEventListener('popstate', notify);
}

/** Parses a fragment (`#a=1&b`, with or without the `#`). A key without `=` reads as `''`. */
export function parseHash(hash: string): URLSearchParams {
  return new URLSearchParams(hash.startsWith('#') ? hash.slice(1) : hash);
}

/** Prints params as a fragment, `''` for none. Keys with an empty value print bare (`#presenter`). */
export function formatHash(params: URLSearchParams): string {
  const parts: string[] = [];
  for (const [key, value] of params) {
    const name = encodeURIComponent(key);
    parts.push(value === '' ? name : `${name}=${encodeURIComponent(value)}`);
  }
  return parts.length === 0 ? '' : `#${parts.join('&')}`;
}

export function readHash(): URLSearchParams {
  return parseHash(typeof location === 'undefined' ? '' : location.hash);
}

/**
 * Sets or clears (`null`) fragment keys and keeps the others. Pushes or replaces a history entry; nothing is written
 * when the fragment does not change.
 */
export function writeHash(updates: Record<string, string | null>, history: HistoryMode = 'replace') {
  const params = readHash();
  for (const [key, value] of Object.entries(updates)) {
    if (value === null) {
      params.delete(key);
    } else {
      params.set(key, value);
    }
  }
  const next = formatHash(params);
  if (next === location.hash || (next === '' && location.hash === '#')) {
    return;
  }
  const url = `${location.pathname}${location.search}${next}`;
  if (history === 'push') {
    window.history.pushState(window.history.state, '', url);
  } else {
    window.history.replaceState(window.history.state, '', url);
  }
  notify();
}

export function subscribeHash(listener: () => void): () => void {
  install();
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function snapshot(): string {
  return typeof location === 'undefined' ? '' : location.hash;
}

/** The fragment's params, re-rendering when they change (navigation, back/forward or `writeHash`). */
export function useHashParams(): URLSearchParams {
  const hash = useSyncExternalStore(subscribeHash, snapshot, () => '');
  return useMemo(() => parseHash(hash), [hash]);
}

/** A positive integer param, or `null` when absent or malformed. */
export function intParam(params: URLSearchParams, key: string): number | null {
  const raw = params.get(key);
  if (raw === null || !/^\d+$/.test(raw)) {
    return null;
  }
  const value = Number(raw);
  return Number.isSafeInteger(value) ? value : null;
}
