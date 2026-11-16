// Hostile deck JSON in a real browser: fields outside the contract fail closed with an RT diagnostic, never throw,
// and never leave the player unable to move forward; a slide's units stay bounded however wide its ranges are.
import { type Page, expect, test } from '@playwright/test';
import type { DeckStep, DeckTransition } from '@slidock/types';

import { deckSlide, fixture, step, transition } from './support/deck';
import { type Harness, open, settle } from './support/harness';

function pageErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(String(error)));
  return errors;
}

async function next(page: Page): Promise<void> {
  await page.evaluate(() => (window as unknown as Harness).sd.player.next());
  await settle(page);
}

const position = (page: Page) => page.evaluate(() => (window as unknown as Harness).sd.player.position);
const codes = (page: Page) =>
  page.evaluate(() => (window as unknown as Harness).sd.diagnostics.map((d) => `${d.code} ${d.message}`));

test('an out-of-contract transition fallback cuts, reports it, and the player goes on', async ({ page }) => {
  const errors = pageErrors(page);
  const bad = transition({ fallback: 'cover', effect: 'vortex' } as unknown as Partial<DeckTransition>);
  await open(page, fixture([deckSlide('s1'), deckSlide('s2', { transition: bad }), deckSlide('s3')]));
  await next(page);
  await next(page);
  expect(await position(page)).toMatchObject({ slide: 2, step: 0 });
  expect(errors).toEqual([]);
  expect(await codes(page)).toContain(
    'RT0006 slide s2 transition: unknown transition vortex, unknown fallback, played none',
  );
});

test('steps with no target, an unknown trigger or kind are dropped with RT0008', async ({ page }) => {
  const errors = pageErrors(page);
  const timeline = [
    { ...step('s2.box'), target: null } as unknown as DeckStep,
    step('s2.box', { trigger: 'onMediaEnd', effect: 'fade' } as unknown as Partial<DeckStep>),
    step('s2.text', { kind: 'teleport', effect: 'fade' } as unknown as Partial<DeckStep>),
    step('s2.text', { effect: 'fade' }),
  ];
  await open(page, fixture([deckSlide('s1'), deckSlide('s2', { timeline }), deckSlide('s3')]));
  await next(page);
  // The box's steps were dropped, so it shows from the start; the text waits for its one click.
  const box = await page.evaluate(() => (window as unknown as Harness).sd.style('s2.box'));
  const text = await page.evaluate(() => (window as unknown as Harness).sd.style('s2.text'));
  expect(box?.visibility).toBe('visible');
  expect(text?.visibility).toBe('hidden');
  await next(page);
  expect(await position(page)).toMatchObject({ slide: 1, step: 1 });
  await next(page);
  expect(await position(page)).toMatchObject({ slide: 2, step: 0 });
  expect(errors).toEqual([]);
  expect(await codes(page)).toEqual([
    'RT0008 slide s2 step 0: its target names no element; dropped',
    'RT0008 slide s2 step 1: its trigger is not in the contract (an interactive trigger); dropped',
    'RT0008 slide s2 step 2: its kind is not in the contract; dropped',
  ]);
});

test('huge paragraph ranges stay within the slide bound: fast to open and to build', async ({ page }) => {
  const errors = pageErrors(page);
  const timeline = Array.from({ length: 2_000 }, (_, i) =>
    step(`s2.el${i}`, { target: { element: `s2.el${i}`, paragraphs: [0, 9_999] }, effect: 'fade' }),
  );
  const started = Date.now();
  await open(page, fixture([deckSlide('s1'), deckSlide('s2', { timeline })]));
  const opened = Date.now() - started;
  const blocked = await page.evaluate(() => {
    const t = performance.now();
    (window as unknown as Harness).sd.player.next();
    return performance.now() - t;
  });
  await settle(page);
  expect(opened).toBeLessThan(3_000);
  expect(blocked).toBeLessThan(500);
  expect(errors).toEqual([]);
  expect(await codes(page)).toContain(
    'RT0008 slide s2 1999 steps from step 1: past the 10000 build units a slide addresses; dropped',
  );
});
