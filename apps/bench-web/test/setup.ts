import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';

// Node-environment tests (`@vitest-environment node`) share this file and have no DOM.
if (typeof window !== 'undefined') {
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
