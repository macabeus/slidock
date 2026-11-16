// Builds of a shape that paints a box of its own (a fill, a border) around its text. Delivered
// by paragraph, word or character, the box builds with the text: hidden at the stage's first frame, in with the
// unit that starts first, out with the one that starts last. A background build builds the box alone. A text colour
// change starts each run of the text from that run's own colour.
import { type Page, expect, test } from '@playwright/test';
import type { DeckStep } from '@slidock/types';

import { deckSlide, fixture, slideHtml, step } from './support/deck';
import { type Harness, diff, open, settle, shot, staticShot } from './support/harness';

const AA = 96;
const SPECKS = 256;
const MOVED = 16;
/**
 * A still state after motion, per channel: Chromium may raster the rounded, bordered corner of a shape that was
 * composited one level apart, whole-element builds included.
 */
const STILL = 1;

/** The fixture slide with a filled, bordered shape holding two paragraphs, the second with a white run. */
function labelled(id: string, fill = '#7c3aed', border = '4pt solid #111827'): string {
  return slideHtml(id).replace(
    '</main>',
    `<div data-el="${id}.label" data-kind="shape" data-sd-text style="position:absolute;left:380pt;top:250pt;` +
      `width:300pt;height:110pt;box-sizing:border-box;padding:6pt;background:${fill};border:${border};` +
      'border-radius:8pt;color:#fde68a;font:20pt/1.2 sans-serif;white-space:pre-wrap">' +
      '<p data-p="0">Boxed words</p><p data-p="1">and a <span style="color:#ffffff">white</span> run</p></div></main>',
  );
}

/** The label's border box on the stage, in px. */
const LABEL = { x: 380 * (4 / 3), y: 250 * (4 / 3), w: 300 * (4 / 3), h: 110 * (4 / 3) };

async function at(page: Page, fraction: number): Promise<string> {
  await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), fraction);
  return shot(page);
}

/**
 * Pixels of the label's box that differ by more than `tolerance` between two frames, sampled where no text is (its
 * border and a fill band). A box on a composited layer may antialias its rounded corners anew.
 */
async function boxPixels(page: Page, a: string, b: string, tolerance: number): Promise<number> {
  return page.evaluate(
    async ([x, y, box, t]) => {
      const load = async (b64: string) => createImageBitmap(await (await fetch(`data:image/png;base64,${b64}`)).blob());
      const [p, q] = await Promise.all([load(x), load(y)]);
      const ctx = new OffscreenCanvas(p.width, p.height).getContext('2d');
      if (ctx === null) {
        return -1;
      }
      const read = (image: ImageBitmap) => {
        ctx.drawImage(image, 0, 0);
        // The bottom band of the box: below the text, inside the fill and across the border.
        return ctx.getImageData(Math.round(box.x), Math.round(box.y + box.h - 14), Math.round(box.w), 12).data;
      };
      const da = read(p);
      const db = read(q);
      let count = 0;
      for (let i = 0; i < da.length; i += 4) {
        const d = Math.max(
          Math.abs((da[i] ?? 0) - (db[i] ?? 0)),
          Math.abs((da[i + 1] ?? 0) - (db[i + 1] ?? 0)),
          Math.abs((da[i + 2] ?? 0) - (db[i + 2] ?? 0)),
        );
        if (d > t) {
          count++;
        }
      }
      return count;
    },
    [a, b, LABEL, tolerance] as const,
  );
}

interface Case {
  readonly name: string;
  readonly step: Partial<DeckStep> & Record<string, unknown>;
  /** The label's fill (default violet) and border (default 4pt, dark). */
  readonly fill?: string;
  readonly border?: string;
}

const CASES: readonly Case[] = [
  { name: 'fade by character', step: { kind: 'entrance', delivery: 'byChar', effect: 'fade' } },
  { name: 'fade by word', step: { kind: 'entrance', delivery: 'byWord', effect: 'fade' } },
  {
    name: 'fly in by paragraph',
    step: { kind: 'entrance', delivery: 'byParagraph', effect: 'flyIn', params: { dir: 'u', distance: 'slight' } },
  },
  {
    name: 'wipe by word, backwards',
    step: {
      kind: 'entrance',
      delivery: 'byWord',
      effect: 'wipe',
      params: { dir: 'r', soft: false },
      stagger: { order: 'backward', perUnit: { ms: 150 } },
    },
  },
  { name: 'fade out by character', step: { kind: 'exit', delivery: 'byChar', effect: 'fade' } },
  // Black (`rgb(0, 0, 0)`) is a fill, not a transparent colour, with no border to lift either.
  {
    name: 'fade out by character, black',
    step: { kind: 'exit', delivery: 'byChar', effect: 'fade' },
    fill: '#000000',
    border: 'none',
  },
  {
    name: 'fly out by word',
    step: { kind: 'exit', delivery: 'byWord', effect: 'flyIn', params: { dir: 'd', distance: 'slight' } },
  },
  {
    name: 'background fade',
    step: { kind: 'entrance', effect: 'fade', target: { element: 's1.label', paragraphs: null, background: true } },
  },
  {
    name: 'background wipe out',
    step: {
      kind: 'exit',
      effect: 'wipe',
      params: { dir: 'l', soft: false },
      target: { element: 's1.label', paragraphs: null, background: true },
    },
  },
];

for (const c of CASES) {
  test(`a boxed shape builds its box with its text: ${c.name}`, async ({ page }) => {
    const data = fixture([deckSlide('s1', { timeline: [step('s1.label', { fallback: 'dissolve', ...c.step })] })], {
      s1: labelled('s1', c.fill, c.border),
    });
    await open(page, data);
    const before = await staticShot(page, 0, 0);
    const after = await staticShot(page, 0, 1);
    expect(await boxPixels(page, before, after, MOVED), 'the step changes the box').toBeGreaterThan(1000);
    const started = await page.evaluate(() => {
      const { sd } = window as unknown as Harness;
      sd.player.next();
      return sd.pause();
    });
    expect(started).toBeGreaterThan(0);
    const first = await at(page, 0);
    const middle = await at(page, 0.5);
    const last = await at(page, 1);
    expect(await diff(page, first, before, AA), 'first frame = state before').toBeLessThanOrEqual(SPECKS);
    expect(await boxPixels(page, first, before, AA), 'the box as before at the first frame').toBe(0);
    expect(await diff(page, last, after, AA), 'last frame = state after').toBeLessThanOrEqual(SPECKS);
    expect(await boxPixels(page, last, after, AA), 'the box as after at the last frame').toBe(0);
    expect(await diff(page, middle, before, MOVED), 'middle frame moved').toBeGreaterThan(0);
    await page.evaluate(() => (window as unknown as Harness).sd.resume());
    await settle(page);
    expect(await diff(page, await shot(page), after, STILL), 'settled end state = renderStatic').toBe(0);
    // Moving back cuts to the state before, the box put back on its shape.
    await page.evaluate(() => (window as unknown as Harness).sd.player.previous());
    await settle(page);
    expect(await diff(page, await shot(page), before, STILL), 'back = state before').toBe(0);
    const lifted = await page.evaluate(() => {
      const label = (window as unknown as Harness).sd.target('s1.label');
      return label === null ? null : label.querySelectorAll('[data-sd-boxlayer]').length;
    });
    expect(lifted).toBe(0);
  });
}

// A box of zero width or height carries a transparent shadow as its extent: it paints no box,
// so a background build lifts none and leaves the extent on the element; a shadow in a colour is a box and lifts.
for (const [shadow, lifts] of [
  ['0 0 0 1pt transparent', false],
  ['0 0 0 1pt #dc2626', true],
] as const) {
  test(`a background build lifts the box of a shape whose shadow is ${shadow}: ${lifts}`, async ({ page }) => {
    const data = fixture(
      [
        deckSlide('s1', {
          timeline: [
            step('s1.label', {
              kind: 'entrance',
              effect: 'fade',
              target: { element: 's1.label', paragraphs: null, background: true },
            }),
          ],
        }),
      ],
      { s1: labelled('s1', 'transparent', 'none').replace('border-radius:8pt;', `box-shadow:${shadow};`) },
    );
    await open(page, data);
    await page.evaluate(() => {
      const { sd } = window as unknown as Harness;
      sd.player.next();
      sd.pause();
    });
    await at(page, 0.5);
    const state = await page.evaluate(() => {
      const label = (window as unknown as Harness).sd.target('s1.label');
      return label === null
        ? null
        : { layers: label.querySelectorAll('[data-sd-boxlayer]').length, shadow: getComputedStyle(label).boxShadow };
    });
    expect(state?.layers).toBe(lifts ? 1 : 0);
    expect(state?.shadow === 'none').toBe(lifts);
  });
}

test('reduced motion: a boxed shape built by character appears with its box', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const data = fixture(
    [deckSlide('s1', { timeline: [step('s1.label', { kind: 'entrance', delivery: 'byChar', effect: 'fade' })] })],
    { s1: labelled('s1') },
  );
  await open(page, data);
  const after = await staticShot(page, 0, 1);
  await page.evaluate(() => (window as unknown as Harness).sd.player.next());
  await settle(page);
  expect(await diff(page, await shot(page), after, STILL)).toBe(0);
});

test('a text colour change starts each run from its own colour', async ({ page }) => {
  const data = fixture(
    [
      deckSlide('s1', {
        timeline: [
          step('s1.label', {
            kind: 'emphasis',
            fallback: 'none',
            effect: 'colorChange',
            params: { property: 'text', to: '#dc2626' },
          }),
        ],
      }),
    ],
    { s1: labelled('s1') },
  );
  await open(page, data);
  const before = await staticShot(page, 0, 0);
  const after = await staticShot(page, 0, 1);
  await page.evaluate(() => {
    const { sd } = window as unknown as Harness;
    sd.player.next();
    return sd.pause();
  });
  const first = await at(page, 0);
  expect(
    await diff(page, first, before, AA),
    'first frame = state before (the white run still white)',
  ).toBeLessThanOrEqual(SPECKS);
  const runs = await page.evaluate(() => {
    const label = (window as unknown as Harness).sd.target('s1.label');
    const span = label?.querySelector('span');
    return span === null || span === undefined ? null : getComputedStyle(span).color;
  });
  expect(runs).toBe('rgb(255, 255, 255)');
  expect(await diff(page, await at(page, 1), after, AA), 'last frame = state after').toBeLessThanOrEqual(SPECKS);
  await page.evaluate(() => (window as unknown as Harness).sd.resume());
  await settle(page);
  expect(await diff(page, await shot(page), after, STILL)).toBe(0);
});
