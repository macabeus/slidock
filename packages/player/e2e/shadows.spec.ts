// Printed shadows through the player: a filter effect adds to the element's printed
// `filter: drop-shadow`, so a bold flash, a blur track and a held blur keep its shadow, and a by-paragraph build of a
// box-shadowed shape lifts its box with its one shadow.
import { type Page, expect, test } from '@playwright/test';
import type { DeckStep } from '@slidock/types';

import { deckSlide, fixture, step } from './support/deck';
import { type Harness, open, settle, shot, staticShot } from './support/harness';

/** A 160 x 80 pt shape whose hard black shadow falls 40 pt below it, and a box-shadowed text box. */
function slideHtml(id: string): string {
  return (
    `<main data-slide="${id}" style="position:relative;width:720pt;height:405pt;overflow:hidden">` +
    `<div data-sd-bg style="position:absolute;inset:0;background:#ffffff"></div>` +
    `<div data-el="${id}.glass" data-kind="shape" style="position:absolute;left:60pt;top:60pt;width:160pt;height:80pt;` +
    `background-color:#2563eb80;filter:drop-shadow(0pt 40pt 0pt #000000)"></div>` +
    `<div data-el="${id}.card" data-kind="shape" data-sd-text style="position:absolute;left:360pt;top:60pt;width:240pt;` +
    `height:80pt;background-color:#16a34a;color:#ffffff;font:20pt/1.2 sans-serif;white-space:pre-wrap;` +
    `box-shadow:0pt 40pt 0pt #00000080"><p data-p="0">One</p><p data-p="1">Two</p></div>` +
    `</main>`
  );
}

/** Points to px. */
const px = (pt: number) => Math.round(pt * (4 / 3));
/** Inside the glass's shadow, below the shape: 40 pt below its centre band. */
const GLASS_SHADOW = { x: px(140), y: px(165) };
/** Inside the card's shadow, below the card. */
const CARD_SHADOW = { x: px(480), y: px(165) };

async function pixel(page: Page, image: string, at: { x: number; y: number }): Promise<number[]> {
  return page.evaluate(
    async ([b64, x, y]) => {
      const bitmap = await createImageBitmap(await (await fetch(`data:image/png;base64,${b64}`)).blob());
      const ctx = new OffscreenCanvas(bitmap.width, bitmap.height).getContext('2d');
      if (ctx === null) {
        return [];
      }
      ctx.drawImage(bitmap, 0, 0);
      return Array.from(ctx.getImageData(x, y, 1, 1).data);
    },
    [image, at.x, at.y] as const,
  );
}

async function at(page: Page, fraction: number): Promise<string> {
  await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), fraction);
  return shot(page);
}

async function playing(page: Page, timeline: DeckStep[]): Promise<void> {
  await open(page, fixture([deckSlide('s1', { timeline })], { s1: slideHtml('s1') }));
  const started = await page.evaluate(() => {
    const { sd } = window as unknown as Harness;
    sd.player.next();
    return sd.pause();
  });
  expect(started).toBeGreaterThan(0);
}

const emphasis = (element: string, overrides: Record<string, unknown>): DeckStep =>
  step(element, { kind: 'emphasis', fallback: 'none', ...overrides });

const darkest = (rgba: number[]) => Math.max(rgba[0] ?? 255, rgba[1] ?? 255, rgba[2] ?? 255);

/** The glass's shadow as the static render shows it: a 50 % fill casts a 50 % shadow (127 over white). */
async function stillShadow(page: Page): Promise<number> {
  const still = darkest(await pixel(page, await staticShot(page, 0, 0), GLASS_SHADOW));
  expect(still).toBeLessThan(150);
  return still;
}

test('a bold flash keeps the printed drop shadow', async ({ page }) => {
  await playing(page, [emphasis('s1.glass', { effect: 'boldFlash', params: { repeat: 1 } })]);
  const still = await stillShadow(page);
  // The flash's own drop shadows follow the printed one, so they thicken the shadow too: it may darken, never fade.
  for (const fraction of [0.25, 0.5, 0.75]) {
    const now = darkest(await pixel(page, await at(page, fraction), GLASS_SHADOW));
    expect(now, `${fraction}`).toBeLessThanOrEqual(still + 8);
  }
});

test('a blur track blurs the shape with its shadow, and the held blur keeps it', async ({ page }) => {
  const blur = {
    property: 'blur',
    beginMs: 0,
    durationMs: 1000,
    frames: [
      { offset: 0, value: 0 },
      { offset: 1, value: 2 },
    ],
  };
  await playing(page, [emphasis('s1.glass', { effect: 'behaviors', params: [blur] })]);
  const still = await stillShadow(page);
  const mid = darkest(await pixel(page, await at(page, 0.5), GLASS_SHADOW));
  expect(Math.abs(mid - still)).toBeLessThanOrEqual(8);
  await page.evaluate(() => (window as unknown as Harness).sd.resume());
  await settle(page);
  const held = darkest(await pixel(page, await shot(page), GLASS_SHADOW));
  expect(Math.abs(held - still)).toBeLessThanOrEqual(8);
});

test('a by-paragraph build lifts a box shadow with its box: one shadow, at every frame', async ({ page }) => {
  await playing(page, [step('s1.card', { kind: 'entrance', effect: 'fade', delivery: 'byParagraph' })]);
  const after = await pixel(page, await staticShot(page, 0, 1), CARD_SHADOW);
  // One 50 % black shadow over white: 128 per channel, where two would give 64.
  expect(darkest(after)).toBeGreaterThan(110);
  expect(darkest(after)).toBeLessThan(150);
  for (const fraction of [0.5, 1]) {
    expect(darkest(await pixel(page, await at(page, fraction), CARD_SHADOW)), `${fraction}`).toBeGreaterThan(110);
  }
  await page.evaluate(() => (window as unknown as Harness).sd.resume());
  await settle(page);
  expect(await pixel(page, await shot(page), CARD_SHADOW)).toEqual(after);
});
