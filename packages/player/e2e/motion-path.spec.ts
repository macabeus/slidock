// A Keynote motion path as the deck path carries it: a tier-2 translate track sampled along the path, here the custom
// curve of slide 65 of the Keynote build catalog (build-subset.key), from (0, 0) to (320, 180) slide px over 500 ms. On
// a fixture box, each seeked frame puts the box where the track's frames put it, off the straight line between its
// ends, and the box settles held at the path's end, as the still render shows it.
import { expect, test } from '@playwright/test';
import type { DeckStep } from '@slidock/types';

import { deckSlide, fixture, step } from './support/deck';
import { type Harness, diff, open, settle, shot, staticShot } from './support/harness';

/** The track's frames: offset, then the translation in slide px. */
const FRAMES: readonly (readonly [number, number, number])[] = [
  [0, 0, 0],
  [0.06263, 23.984, 0.835],
  [0.12531, 47.875, 3.305],
  [0.18807, 71.578, 7.356],
  [0.2509, 95, 12.938],
  [0.3138, 118.047, 19.995],
  [0.37675, 140.625, 28.477],
  [0.43969, 162.641, 38.329],
  [0.5026, 184, 49.5],
  [0.56542, 204.609, 61.937],
  [0.6281, 224.375, 75.586],
  [0.69062, 243.203, 90.396],
  [0.75293, 261, 106.313],
  [0.81501, 277.672, 123.284],
  [0.87687, 293.125, 141.258],
  [0.93852, 307.266, 160.181],
  [1, 320, 180],
];

const PATH = step('s1.box', {
  kind: 'emphasis',
  fallback: 'dissolve',
  native: 'keynote:apple:action-motion-path',
  effect: 'behaviors',
  offsetMs: 0,
  durationMs: 500,
  params: [
    {
      property: 'translate',
      beginMs: 0,
      durationMs: 500,
      frames: FRAMES.map(([offset, x, y]) => ({ offset, value: { x, y } })),
    },
  ],
} as Partial<DeckStep>);

/** Where the track puts the box `p` of the way through (linear between frames). */
function along(p: number): [number, number] {
  for (let i = 1; i < FRAMES.length; i++) {
    const [a, ax, ay] = FRAMES[i - 1] ?? [0, 0, 0];
    const [b, bx, by] = FRAMES[i] ?? [1, 0, 0];
    if (p <= b) {
      const k = (p - a) / (b - a);
      return [ax + (bx - ax) * k, ay + (by - ay) * k];
    }
  }
  return [320, 180];
}

test("a Keynote motion path plays along its track and holds the path's end", async ({ page }) => {
  await open(page, fixture([deckSlide('s1', { timeline: [PATH] })]));
  const after = await staticShot(page, 0, 1);
  const corner = () =>
    page.evaluate(() => {
      const box = (window as unknown as Harness).sd.target('s1.box')?.getBoundingClientRect();
      return box === undefined ? [Number.NaN, Number.NaN] : [box.left, box.top];
    });
  const [x0, y0] = (await corner()) as [number, number];
  await page.evaluate(() => {
    const { sd } = window as unknown as Harness;
    sd.player.next();
    sd.pause();
  });
  for (const p of [0, 0.1, 0.25, 0.5, 0.75, 0.9, 1]) {
    await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), p);
    const [x, y] = (await corner()) as [number, number];
    const [ex, ey] = along(p);
    expect(Math.abs(x - x0 - ex), `x at ${p}: ${x - x0} vs ${ex}`).toBeLessThanOrEqual(0.5);
    expect(Math.abs(y - y0 - ey), `y at ${p}: ${y - y0} vs ${ey}`).toBeLessThanOrEqual(0.5);
  }
  // A curve: halfway it is well off the straight line from (0, 0) to (320, 180).
  const [mx, my] = along(0.5);
  expect((180 / 320) * mx - my).toBeGreaterThan(40);
  await page.evaluate(() => (window as unknown as Harness).sd.resume());
  await settle(page);
  const [x, y] = (await corner()) as [number, number];
  expect([x - x0, y - y0]).toEqual([320, 180]);
  expect(await diff(page, await shot(page), after), 'settled = renderStatic').toBe(0);
});
