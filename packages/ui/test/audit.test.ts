import { describe, expect, it } from 'vitest';

import { tabKeyFor } from '../src/audit';

describe('tabKeyFor', () => {
  it('moves with Alt+Tab in WebKit on macOS, whose Tab skips buttons and links', () => {
    expect(tabKeyFor('webkit', 'darwin', false)).toBe('Alt+Tab');
  });

  it('moves with Tab everywhere else, WebKit in the Linux image included', () => {
    expect(tabKeyFor('webkit', 'darwin', true)).toBe('Tab');
    expect(tabKeyFor('webkit', 'linux', false)).toBe('Tab');
    expect(tabKeyFor('chromium', 'darwin', false)).toBe('Tab');
    expect(tabKeyFor('firefox', 'darwin', false)).toBe('Tab');
  });
});
