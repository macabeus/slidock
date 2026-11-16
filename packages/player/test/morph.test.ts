import { describe, expect, it } from 'vitest';

import { beneath } from '../src/effects/morph';

/** The opacity a keyframe list gives at offset w, interpolated linearly as the Web Animations API does. */
function at(keyframes: Keyframe[], w: number): number {
  // Keyframes without offsets spread evenly from 0 to 1.
  const points = keyframes.map(
    (k, i) => [typeof k.offset === 'number' ? k.offset : i / (keyframes.length - 1), Number(k.opacity)] as const,
  );
  for (let i = 1; i < points.length; i++) {
    const [x0, y0] = points[i - 1] ?? [0, 0];
    const [x1, y1] = points[i] ?? [1, 0];
    if (w <= x1) {
      return x1 === x0 ? y1 : y0 + ((y1 - y0) * (w - x0)) / (x1 - x0);
    }
  }
  return points.at(-1)?.[1] ?? 0;
}

describe("an incoming match beneath the outgoing slide's fade", () => {
  // The outgoing slide at 1 − w over the incoming match: what each weighs and what they cover together.
  const mix = (a: number, b: number, w: number) => {
    const top = (1 - w) * a;
    const bottom = at(beneath(a, b), w) * (1 - top);
    return { top, bottom, cover: top + bottom };
  };

  it('weighs the two as a linear mix and covers as one element, at its sample points', () => {
    for (const [a, b] of [
      [1, 1],
      [0.5, 1],
      [1, 0.5],
      [0.3, 0.3],
      [0, 1],
    ] as const) {
      for (const w of [0, 0.25, 0.5, 0.75, 1]) {
        const { top, bottom, cover } = mix(a, b, w);
        expect(top, `outgoing weight ${a}→${b} at ${w}`).toBeCloseTo((1 - w) * a, 6);
        expect(bottom, `incoming weight ${a}→${b} at ${w}`).toBeCloseTo(w * b, 6);
        expect(cover, `coverage ${a}→${b} at ${w}`).toBeCloseTo((1 - w) * a + w * b, 6);
      }
    }
  });

  it('starts hidden and ends at its own opacity', () => {
    const keyframes = beneath(0.4, 0.9);
    expect([at(keyframes, 0), at(keyframes, 1)]).toEqual([0, 0.9]);
  });

  it('stays whole beneath an opaque outgoing match, so nothing shows through either', () => {
    expect(beneath(1, 1).every((k) => k.opacity === 1)).toBe(true);
  });
});
