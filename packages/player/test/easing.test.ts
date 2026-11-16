import { describe, expect, it, vi } from 'vitest';

import { type Curve, EASE_IN_OUT, LINEAR, easeKeyframes, evaluate, readEasing } from '../src/effects/easing';

// A browser that reads `linear()`: the probe is made once per window.
vi.stubGlobal('CSS', { supports: () => true });

/** Where a list of keyframes stands at time `t` (0 to 1), as progress through their offsets: what WAAPI computes. */
function progressAt(keyframes: Keyframe[], t: number, offsets: readonly number[]): number {
  let i = keyframes.length - 2;
  while (i > 0 && (keyframes[i]?.offset ?? 0) > t) {
    i--;
  }
  const [t0, t1] = [keyframes[i]?.offset ?? 0, keyframes[i + 1]?.offset ?? 1];
  const [p0, p1] = [offsets[i] ?? 0, offsets[i + 1] ?? 1];
  const local = t1 > t0 ? (t - t0) / (t1 - t0) : 1;
  return p0 + (p1 - p0) * linearAt(String(keyframes[i]?.easing ?? 'linear'), local);
}

/** A `linear()` easing (with an input percentage on every point) at `x`. */
function linearAt(easing: string, x: number): number {
  if (easing === 'linear') {
    return x;
  }
  const points = /^linear\((.*)\)$/
    .exec(easing)?.[1]
    ?.split(',')
    .map((point) => point.trim().split(' '))
    .map(([y, at]) => [Number.parseFloat(at ?? '') / 100, Number(y)] as const);
  if (points === undefined) {
    throw new Error(`not a linear() easing: ${easing}`);
  }
  for (let i = 1; i < points.length; i++) {
    const [x0, y0] = points[i - 1] ?? [0, 0];
    const [x1, y1] = points[i] ?? [1, 1];
    if (x <= x1) {
      return x1 > x0 ? y0 + ((y1 - y0) * (x - x0)) / (x1 - x0) : y1;
    }
  }
  return points.at(-1)?.[1] ?? 1;
}

const ACCEL = readEasing({ accelDecel: { accel: 0.3, decel: 0.5 } }) as Curve;

describe('easeKeyframes', () => {
  it('puts the curve on the first of two keyframes', () => {
    expect(easeKeyframes([{ opacity: 0 }, { opacity: 1 }], EASE_IN_OUT)).toEqual([
      { opacity: 0, easing: 'cubic-bezier(0.42, 0, 0.58, 1)' },
      { opacity: 1 },
    ]);
  });

  it('leaves keyframes alone for a linear curve', () => {
    const keyframes = [{ opacity: 0 }, { offset: 0.3, opacity: 1 }, { opacity: 0 }];
    expect(easeKeyframes(keyframes, LINEAR)).toBe(keyframes);
  });

  it.each([
    ['ease-in-out', EASE_IN_OUT],
    ['an accelerate and decelerate', ACCEL],
  ])('re-times more keyframes so that they play %s as the effect would', (_, curve) => {
    const offsets = [0, 0.25, 0.5, 0.5, 1];
    const keyframes = offsets.map((offset, i) => ({ offset, opacity: i / 4 }));
    const eased = easeKeyframes(keyframes, curve);
    expect(eased).not.toBeNull();
    const out = eased ?? [];
    expect(out.map((frame) => frame.offset)).toEqual(
      out.map((frame) => frame.offset).sort((a, b) => (a ?? 0) - (b ?? 0)),
    );
    expect(out[0]?.offset).toBe(0);
    expect(out.at(-1)?.offset).toBe(1);
    for (let t = 0; t <= 1; t += 1 / 128) {
      expect(progressAt(out, t, offsets)).toBeCloseTo(evaluate(curve, t), 3);
    }
  });

  it('keeps a jump where it was: duplicate offsets stay together', () => {
    const eased = easeKeyframes(
      [
        { offset: 0, opacity: 0 },
        { offset: 0.5, opacity: 0 },
        { offset: 0.5, opacity: 1 },
        { offset: 1, opacity: 1 },
      ],
      EASE_IN_OUT,
    );
    expect(eased?.map((frame) => frame.offset)).toEqual([0, 0.5, 0.5, 1]);
    expect(eased?.[1]?.easing).toBeUndefined();
  });

  it('reads missing offsets as WAAPI spaces them while the given ones sit on the even grid', () => {
    // A paint run's list: its start value at offset 0, then the step's own keyframes (WAAPI: 0, 0.5, 1).
    const eased = easeKeyframes([{ offset: 0, opacity: 0 }, { opacity: 0.5 }, { opacity: 1 }], EASE_IN_OUT) ?? [];
    expect(eased).toHaveLength(3);
    for (let t = 0; t <= 1; t += 1 / 128) {
      expect(progressAt(eased, t, [0, 0.5, 1])).toBeCloseTo(evaluate(EASE_IN_OUT, t), 3);
    }
  });

  it('leaves a list alone whose given offsets are off the even grid', () => {
    // WAAPI reads these offsets as 0, 0.8, 0.9, 1; an even spread would read 0, 0.8, 2/3, 1, which goes down.
    expect(
      easeKeyframes([{ opacity: 0 }, { offset: 0.8, opacity: 1 }, { opacity: 0.5 }, { opacity: 0 }], EASE_IN_OUT),
    ).toBeNull();
    expect(
      easeKeyframes([{ opacity: 0 }, { offset: 0.6, opacity: 1 }, { offset: 0.4 }, { opacity: 0 }], EASE_IN_OUT),
    ).toBeNull();
  });

  it('leaves the curve on the effect where the keyframes cannot carry it', () => {
    const three = [{ opacity: 0 }, { opacity: 0.5 }, { opacity: 1 }];
    expect(easeKeyframes([{ offset: 0.2, opacity: 0 }, { opacity: 1 }], EASE_IN_OUT)).toBeNull();
    expect(easeKeyframes([{ opacity: 0 }, { opacity: 1, offset: 0.8 }], EASE_IN_OUT)).toBeNull();
    expect(easeKeyframes([{ opacity: 0, easing: 'steps(1, end)' }, { opacity: 1 }], EASE_IN_OUT)).toBeNull();
    expect(easeKeyframes([{ opacity: 0 }], EASE_IN_OUT)).toBeNull();
    const overshoot = readEasing({ bezier: [0.3, -0.5, 0.7, 1.5] }) as Curve;
    expect(easeKeyframes(three, overshoot)).toBeNull();
    expect(easeKeyframes(three, EASE_IN_OUT)).not.toBeNull();
  });
});
