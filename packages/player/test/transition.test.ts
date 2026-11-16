// @vitest-environment happy-dom
import type { DeckTransition } from '@slidock/types';
import { describe, expect, it, vi } from 'vitest';

import { planTransition, runTransition } from '../src/transition';
import { transition } from './fixtures';

/** The keyframes and timing each layer's `animate` got when `deck` played from one layer into another. */
function played(deck: DeckTransition): { keyframes: Keyframe[]; options: KeyframeAnimationOptions }[] {
  const calls: { keyframes: Keyframe[]; options: KeyframeAnimationOptions }[] = [];
  const layer = (): HTMLElement => {
    const element = document.createElement('div');
    element.animate = vi.fn((keyframes: Keyframe[], options: KeyframeAnimationOptions) => {
      calls.push({ keyframes, options });
      return { finished: Promise.resolve(), cancel: () => undefined } as unknown as Animation;
    }) as unknown as typeof element.animate;
    document.body.append(element);
    return element;
  };
  const stage = {
    out: layer(),
    into: layer(),
    size: { width: 960, height: 540 },
    seed: 1,
    properties: true,
    cells: 64,
  };
  runTransition(stage, planTransition(deck, false)).cleanup();
  return calls;
}

describe('a transition plays the curve its IR names', () => {
  it("over the recipe's own default: a fade is linear unnamed, eased when named", () => {
    const fade = { effect: 'fade', params: { through: 'none' } } as const;
    const [linear] = played(transition({ ...fade, durationMs: 1000 }));
    expect(linear?.keyframes.map((k) => k.easing)).toEqual([undefined, undefined]);
    expect(linear?.options).toMatchObject({ duration: 1000, easing: 'linear' });
    // A Keynote dissolve that stores no curve plays its own, ease-in-out.
    const [eased] = played(transition({ ...fade, durationMs: 1000, easing: 'in-out' }));
    expect(eased?.keyframes[0]?.easing).toBe('cubic-bezier(0.42, 0, 0.58, 1)');
    expect(eased?.options).toMatchObject({ duration: 1000, easing: 'linear' });
  });

  it('as a bezier on a moving recipe, whose default is ease-in-out', () => {
    const cover = { effect: 'cover', params: { dir: 'l' } } as const;
    const unnamed = played(transition({ ...cover, durationMs: 1500 }));
    expect(unnamed.length).toBeGreaterThan(0);
    for (const call of unnamed) {
      expect(call.keyframes[0]?.easing).toBe('cubic-bezier(0.42, 0, 0.58, 1)');
    }
    // Keynote's move in, which stores no curve, as its movie plays it.
    const named = played(transition({ ...cover, durationMs: 1500, easing: { bezier: [0.62, 0.92, 0.62, 0.98] } }));
    expect(named.length).toBe(unnamed.length);
    for (const call of named) {
      expect(call.keyframes[0]?.easing).toBe('cubic-bezier(0.62, 0.92, 0.62, 0.98)');
    }
  });
});
