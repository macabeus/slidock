import { cleanup } from '@testing-library/react';
import { afterEach, vi } from 'vitest';

// jsdom has no object URLs: count them so tests can check every one is revoked.
export const objectUrls = new Map<string, Blob | MediaSource>();
let next = 0;
URL.createObjectURL = vi.fn((object: Blob | MediaSource) => {
  const url = `blob:test/${++next}`;
  objectUrls.set(url, object);
  return url;
});
URL.revokeObjectURL = vi.fn((url: string) => {
  objectUrls.delete(url);
});

// Node-environment tests (`@vitest-environment node`) share this file and have no DOM.
if (typeof window !== 'undefined') {
  // jsdom lays nothing out, so it has no ResizeObserver; the player's stage observes its box with one.
  window.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  afterEach(() => {
    cleanup();
    window.history.replaceState(null, '', '/');
    document.documentElement.removeAttribute('data-theme');
    try {
      localStorage.clear();
    } catch {
      // jsdom may run without storage.
    }
  });
}
