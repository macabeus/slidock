// Stagger: a step delivered by word starts each word `perUnit` after the slot before it, in the
// step's order: forward, backward, from the centre, from the edges, or a seeded random order that is the same on every
// replay. Each engine plays the delays (each unit's own animation), the units show in that order, and the group ends
// on the static render.
import { type Page, expect, test } from '@playwright/test';
import type { StaggerOrder } from '@slidock/types';

import { deckSlide, fixture, slideHtml, step } from './support/deck';
import { type Harness, diff, open, settle, shot, staticShot } from './support/harness';

const STILL = 1;
const PER_UNIT = 200;
const DURATION = 400;

/** The fixture slide with a one-line text box of six words. */
function words(id: string): string {
  return slideHtml(id).replace(
    '</main>',
    `<div data-el="${id}.words" data-kind="shape" data-sd-text style="position:absolute;left:40pt;top:300pt;` +
      'width:640pt;height:60pt;color:#0f172a;font:28pt/1.2 sans-serif;white-space:pre-wrap">' +
      '<p data-p="0">alpha bravo charlie delta echo foxtrot</p></div></main>',
  );
}

/** Each order's stagger slot per word, left to right (the slots `staggerLevels` gives six units). */
const CASES: readonly { readonly order: StaggerOrder; readonly slots: readonly number[] }[] = [
  { order: 'forward', slots: [0, 1, 2, 3, 4, 5] },
  { order: 'backward', slots: [5, 4, 3, 2, 1, 0] },
  { order: 'fromCenter', slots: [2, 1, 0, 0, 1, 2] },
  { order: 'fromEdges', slots: [0, 1, 2, 2, 1, 0] },
  // mulberry32 from seed 7, a Fisher–Yates shuffle: the same on every replay and in every engine.
  { order: { random: 7 }, slots: [5, 1, 2, 3, 0, 4] },
];

/** Each word unit's text, its animation's delay and its opacity now, left to right. */
async function units(page: Page): Promise<{ text: string; delay: number; opacity: number }[]> {
  return page.evaluate(() => {
    const root = (window as unknown as Harness).sd.target('s1.words');
    return Array.from(root?.querySelectorAll('[data-sd-unit]') ?? [], (unit) => ({
      text: unit.textContent ?? '',
      delay: Math.min(...unit.getAnimations().map((a) => Number(a.effect?.getTiming().delay ?? 0))),
      opacity: Number(getComputedStyle(unit).opacity),
    }));
  });
}

async function seekMs(page: Page, ms: number): Promise<void> {
  await page.evaluate((t) => {
    const { sd } = window as unknown as Harness;
    sd.seek(t / sd.seek(0));
  }, ms);
}

for (const c of CASES) {
  const name = typeof c.order === 'string' ? c.order : `random(${c.order.random})`;
  test(`a word build staggers its words ${name}`, async ({ page }) => {
    const timeline = [
      step('s1.words', {
        delivery: 'byWord',
        effect: 'fade',
        durationMs: DURATION,
        stagger: { perUnit: { ms: PER_UNIT }, order: c.order },
      }),
    ];
    await open(page, fixture([deckSlide('s1', { timeline })], { s1: words('s1') }));
    const after = await staticShot(page, 0, 1);
    const started = await page.evaluate(() => {
      const { sd } = window as unknown as Harness;
      sd.player.next();
      return sd.pause();
    });
    expect(started).toBeGreaterThan(0);
    const played = await units(page);
    expect(played.map((u) => u.text.trim())).toEqual(['alpha', 'bravo', 'charlie', 'delta', 'echo', 'foxtrot']);
    expect(played.map((u) => u.delay)).toEqual(c.slots.map((slot) => slot * PER_UNIT));
    const total = await page.evaluate(() => (window as unknown as Harness).sd.seek(0));
    expect(total).toBe(Math.max(...c.slots) * PER_UNIT + DURATION);
    // Half way through the first slot's fade: its words half shown, every later slot not yet started.
    await seekMs(page, DURATION / 2);
    const middle = await units(page);
    middle.forEach((unit, i) => {
      const expected = Math.min(1, Math.max(0, (DURATION / 2 - (c.slots[i] ?? 0) * PER_UNIT) / DURATION));
      expect(unit.opacity, `${unit.text} at slot ${c.slots[i]}`).toBeCloseTo(expected, 1);
    });
    // The first slot's words paint where the others do not yet.
    const frame = await shot(page);
    const hidden = await staticShot(page, 0, 0);
    const changed = await page.evaluate(
      async ([a, b, slots]) => {
        const load = async (b64: string) =>
          createImageBitmap(await (await fetch(`data:image/png;base64,${b64}`)).blob());
        const [x, y] = await Promise.all([load(a), load(b)]);
        const ctx = new OffscreenCanvas(x.width, x.height).getContext('2d');
        const root = (window as unknown as Harness).sd.target('s1.words');
        if (ctx === null || root === null) {
          return [];
        }
        const read = (image: ImageBitmap) => {
          ctx.drawImage(image, 0, 0);
          return ctx.getImageData(0, 0, x.width, x.height).data;
        };
        const fa = read(x);
        const fb = read(y);
        return Array.from(root.querySelectorAll('[data-sd-unit]'), (unit, i) => {
          const box = unit.getBoundingClientRect();
          let count = 0;
          for (let py = Math.floor(box.top); py < Math.ceil(box.bottom); py++) {
            for (let px = Math.floor(box.left); px < Math.ceil(box.right); px++) {
              const k = (py * x.width + px) * 4;
              if (Math.abs((fa[k] ?? 0) - (fb[k] ?? 0)) > 32) {
                count++;
              }
            }
          }
          return { slot: slots[i] ?? -1, count };
        });
      },
      [frame, hidden, c.slots] as const,
    );
    for (const { slot, count } of changed) {
      if (slot === 0) {
        expect(count, 'a first-slot word shows').toBeGreaterThan(20);
      } else {
        expect(count, `a slot-${slot} word is still hidden`).toBe(0);
      }
    }
    await page.evaluate(() => (window as unknown as Harness).sd.resume());
    await settle(page);
    expect(await diff(page, await shot(page), after, STILL), 'settled = renderStatic').toBe(0);
  });
}

test('a random stagger order is the same on every replay', async ({ page }) => {
  const timeline = [
    step('s1.words', {
      delivery: 'byWord',
      effect: 'fade',
      durationMs: DURATION,
      stagger: { perUnit: { ms: PER_UNIT }, order: { random: 4242 } },
    }),
  ];
  const data = fixture([deckSlide('s1', { timeline })], { s1: words('s1') });
  await open(page, data);
  const replay = async () => {
    await page.evaluate(() => {
      const { sd } = window as unknown as Harness;
      sd.player.goTo(0, 0);
    });
    await settle(page);
    await page.evaluate(() => {
      const { sd } = window as unknown as Harness;
      sd.player.next();
      sd.pause();
    });
    const delays = (await units(page)).map((u) => u.delay / PER_UNIT);
    await page.evaluate(() => (window as unknown as Harness).sd.resume());
    await settle(page);
    return delays;
  };
  const first = await replay();
  expect(await replay()).toEqual(first);
  expect([...first].sort()).toEqual([0, 1, 2, 3, 4, 5]);
  expect(first).not.toEqual([0, 1, 2, 3, 4, 5]);
});
