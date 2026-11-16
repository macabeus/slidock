import { describe, expect, it } from 'vitest';

import { formatBytes, zipName } from '../src/deck/download';

describe('zipName', () => {
  it('names the zip after the deck, without characters file systems refuse', () => {
    expect(zipName('Quarterly review')).toBe('Quarterly review.slidock.zip');
    expect(zipName('a/b:c*d?"e"<f>|g\\h')).toBe('a b c d e f g h.slidock.zip');
    expect(zipName('tab\there\u0001')).toBe('tab here.slidock.zip');
    expect(zipName('   ')).toBe('deck.slidock.zip');
    expect(zipName('x'.repeat(300))).toBe(`${'x'.repeat(120)}.slidock.zip`);
  });
});

describe('formatBytes', () => {
  it('prints powers of 1000', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(999)).toBe('999 B');
    expect(formatBytes(1200)).toBe('1.2 KB');
    expect(formatBytes(48_213)).toBe('48 KB');
    expect(formatBytes(3_400_000)).toBe('3.4 MB');
    expect(formatBytes(2_000_000_000_000)).toBe('2000 GB');
  });
});
