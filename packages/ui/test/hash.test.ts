import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { formatHash, intParam, parseHash, useHashParams, writeHash } from '../src/hash';

afterEach(() => {
  window.history.replaceState(null, '', '/');
});

describe('hash adapter', () => {
  it('round-trips params, printing empty values as bare keys', () => {
    const params = parseHash('#slide=3&step=1&presenter');
    expect(params.get('slide')).toBe('3');
    expect(params.get('presenter')).toBe('');
    expect(formatHash(params)).toBe('#slide=3&step=1&presenter');
    expect(formatHash(parseHash('a%20b=c%26d'))).toBe('#a%20b=c%26d');
    expect(formatHash(new URLSearchParams())).toBe('');
  });

  it('merges updates, clears null keys, and replaces or pushes', () => {
    const start = window.history.length;
    writeHash({ slide: '2', step: '0' });
    expect(location.hash).toBe('#slide=2&step=0');
    expect(window.history.length).toBe(start);
    writeHash({ step: null }, 'push');
    expect(location.hash).toBe('#slide=2');
    expect(window.history.length).toBe(start + 1);
    writeHash({ slide: null });
    expect(location.hash).toBe('');
  });

  it('writes nothing when the fragment does not change', () => {
    writeHash({ slide: '4' }, 'push');
    const length = window.history.length;
    writeHash({ slide: '4' }, 'push');
    expect(window.history.length).toBe(length);
  });

  it('re-renders subscribers on writes and on hashchange', () => {
    const { result } = renderHook(() => useHashParams());
    expect(result.current.get('slide')).toBeNull();
    act(() => writeHash({ slide: '5' }));
    expect(result.current.get('slide')).toBe('5');
    act(() => {
      window.history.replaceState(null, '', '#slide=7');
      window.dispatchEvent(new HashChangeEvent('hashchange'));
    });
    expect(result.current.get('slide')).toBe('7');
  });

  it('reads positive integers only', () => {
    const params = parseHash('a=3&b=-1&c=x&d=1.5&e=');
    expect(intParam(params, 'a')).toBe(3);
    for (const key of ['b', 'c', 'd', 'e', 'missing']) {
      expect(intParam(params, key)).toBeNull();
    }
  });
});
