import { describe, expect, it } from 'vitest';

import { DEFAULT_STATE, explore, historyFor, navigate, readState, stateUpdates } from '../src/state/url';

const read = (hash: string) => readState(new URLSearchParams(hash));

describe('URL state', () => {
  it('reads an empty fragment as the defaults', () => {
    expect(read('')).toEqual(DEFAULT_STATE);
  });

  it('reads every key', () => {
    const state = read(
      'tab=explorer&run=abc&base=baseline&fe=pptx,odp&oc=match&deck=demo&code=PX1,IR2&feat=element.chart&q=kn&fx=fade&ref=1&step=steps&sort=ssim&dir=desc&row=a:b:pptx&img=onion&ps=regressed',
    );
    expect(state).toEqual({
      tab: 'explorer',
      run: 'abc',
      base: 'baseline',
      fe: ['pptx', 'odp'],
      oc: ['match'],
      deck: 'demo',
      code: ['PX1', 'IR2'],
      feat: ['element.chart'],
      q: 'kn',
      fx: 'fade',
      ref: true,
      step: 'steps',
      sort: 'ssim',
      dir: 'desc',
      row: 'a:b:pptx',
      img: 'onion',
      ps: ['regressed'],
    });
  });

  it('reads malformed closed values as their defaults, and keeps unknown filter values (they match nothing)', () => {
    const state = read('tab=nope&sort=size&img=zoom&step=x&dir=up&oc=weird,match,match&ref=yes');
    expect(state.tab).toBe('overview');
    expect(state.sort).toBe('id');
    expect(state.img).toBe('side');
    expect(state.step).toBe('all');
    expect(state.dir).toBe('asc');
    expect(state.oc).toEqual(['weird', 'match']);
    expect(state.ref).toBe(false);
  });

  it('clears keys set back to their defaults and round-trips the rest', () => {
    const updates = stateUpdates({
      tab: 'overview',
      run: 'latest',
      fe: [],
      ref: false,
      q: '',
      row: null,
      sort: 'ssim',
    });
    expect(updates).toEqual({ tab: null, run: null, fe: null, ref: null, q: null, row: null, sort: 'ssim' });
    const change = { tab: 'parity' as const, fe: ['keynote'], ref: true, row: 'x:y:odp#s1', ps: ['missing'] };
    const params = new URLSearchParams();
    for (const [key, value] of Object.entries(stateUpdates(change))) {
      if (value !== null) {
        params.set(key, value);
      }
    }
    expect(readState(params)).toEqual({ ...DEFAULT_STATE, ...change });
  });

  it('pushes a history entry for a tab, run, base or opened row, and replaces it for everything else', () => {
    const state = { ...DEFAULT_STATE, row: 'a:b:pptx' };
    expect(historyFor(state, { tab: 'explorer' })).toBe('push');
    expect(historyFor(state, { run: 'baseline' })).toBe('push');
    expect(historyFor(state, { base: 'k' })).toBe('push');
    expect(historyFor(state, { row: 'c:d:odp' })).toBe('push');
    expect(historyFor(state, { row: null })).toBe('replace');
    expect(historyFor(state, { tab: 'overview' })).toBe('replace');
    expect(historyFor(state, { fe: ['odp'], sort: 'de', img: 'slider' })).toBe('replace');
  });

  it('writes the fragment with those history rules', () => {
    const start = history.length;
    navigate(DEFAULT_STATE, { tab: 'explorer' });
    expect(location.hash).toBe('#tab=explorer');
    expect(history.length).toBe(start + 1);
    navigate(read(location.hash), { fe: ['pptx', 'odp'] });
    expect(location.hash).toBe('#tab=explorer&fe=pptx%2Codp');
    expect(history.length).toBe(start + 1);
    navigate(read(location.hash), { row: 'demo:s1:pptx#t@500' });
    expect(history.length).toBe(start + 2);
    expect(read(location.hash).row).toBe('demo:s1:pptx#t@500');
  });

  it('opens the explorer on a preset alone: other filters and the drawer cleared', () => {
    expect(explore({ fe: ['odp'], oc: ['close'], ref: true })).toEqual({
      fe: ['odp'],
      oc: ['close'],
      deck: null,
      code: [],
      feat: [],
      q: '',
      fx: null,
      ref: true,
      step: 'all',
      tab: 'explorer',
      row: null,
    });
  });
});
