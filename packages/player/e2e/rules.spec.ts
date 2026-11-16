// The motion probe decks' cases of three rules, compiled with the CLI and played by this build of the
// player: a switch (`set`) shows its value from its begin and not before, and holds it; an appear shown for a while
// (LibreOffice's Flash Once, PowerPoint's entr.11) shows its target, then hides it when its duration ends; a held
// tier-2 scale repeated until the end of the slide (LibreOffice's blast) loops while the slide shows, through the next
// click, and the click that leaves the slide cuts it to its held value. Each settled frame equals `renderStatic`.
import { type Page, expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { cli, compile, readDeckPage } from './support/cli';
import { type Harness, diff, open, settle, shot, staticShot } from './support/harness';

test.describe.configure({ mode: 'serial', timeout: 5 * 60_000 });

let binary = '';
test.beforeAll(() => {
  test.setTimeout(30 * 60_000);
  binary = cli();
});

/** Each format's slides of the three cases, and the step (its click) each case plays. */
const CASES = {
  pptx: { bold: ['pp-415', 4], flash: ['pp-400', 0], blast: ['pp-452', 1] },
  odp: { bold: ['od-s126', 4], flash: ['od-s110', 0], blast: ['od-s164', 1] },
} as const;

/** Cuts to `id` at `step` and plays its next click, paused at its start; resolves with the step's target. */
async function playPaused(page: Page, id: string, step: number): Promise<{ slide: number; target: string }> {
  return page.evaluate(
    ([slideId, at]) => {
      const { sd } = window as unknown as Harness;
      const deck = (
        sd.player as unknown as { deck: { slides: { id: string; timeline: { target: { element: string } }[] }[] } }
      ).deck;
      const slide = deck.slides.findIndex((s) => s.id === slideId);
      sd.player.goTo(slide, at);
      return { slide, target: deck.slides[slide]?.timeline[at]?.target.element ?? '' };
    },
    [id, step] as const,
  );
}

async function computed(page: Page, id: string, selector: string | null, property: string): Promise<string> {
  return page.evaluate(
    ([target, sel, prop]) => {
      const root = (window as unknown as Harness).sd.target(target);
      const el = sel === null ? root : (root?.querySelector(sel) ?? null);
      return el === null ? '' : getComputedStyle(el).getPropertyValue(prop).trim();
    },
    [id, selector, property] as const,
  );
}

async function next(page: Page): Promise<void> {
  await page.evaluate(() => {
    const { sd } = window as unknown as Harness;
    sd.player.next();
    sd.pause();
  });
}

/** Whether an endless animation runs on `id`. */
async function looping(page: Page, id: string): Promise<boolean> {
  return page.evaluate(
    (target) =>
      ((window as unknown as Harness).sd.target(target)?.getAnimations() ?? []).some(
        (a) => a.playState === 'running' && a.effect?.getComputedTiming().endTime === Infinity,
      ),
    id,
  );
}

async function seek(page: Page, fraction: number): Promise<void> {
  await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), fraction);
}

for (const format of ['pptx', 'odp'] as const) {
  test(`motion-probe.${format}: a switch, Flash Once and a held repeat play as their rules say`, async ({ page }) => {
    const dir = compile(binary, `probes/out/motion-probe.${format}`, `rules-${format}`);
    await open(page, await readDeckPage(page, readFileSync(join(dir, 'index.html'), 'utf8')));
    const cases = CASES[format];

    await test.step('bold set', async () => {
      const [id, step] = cases.bold;
      const { slide, target } = await playPaused(page, id, step);
      await settle(page);
      const reader = '[style*="var(--sd-bold"]';
      expect(await computed(page, target, reader, 'font-weight'), 'before the click').toBe('400');
      const before = await shot(page);
      await next(page);
      await seek(page, 0.01);
      expect(await computed(page, target, reader, 'font-weight'), 'just after its begin').toBe('700');
      expect(await diff(page, await shot(page), before), 'the switch shows').toBeGreaterThan(0);
      await page.evaluate(() => (window as unknown as Harness).sd.resume());
      await settle(page);
      expect(await computed(page, target, reader, 'font-weight'), 'held').toBe('700');
      expect(await diff(page, await shot(page), await staticShot(page, slide, step + 1))).toBe(0);
    });

    await test.step('Flash Once', async () => {
      const [id, step] = cases.flash;
      const { slide, target } = await playPaused(page, id, step);
      await settle(page);
      const shown = async () =>
        `${await computed(page, target, null, 'visibility')} ${await computed(page, target, null, 'opacity')}`;
      expect(await shown(), 'before the click').toBe('hidden 1');
      await next(page);
      for (const [fraction, expected] of [
        [0, 'visible 1'],
        [0.5, 'visible 1'],
        [0.98, 'visible 1'],
        [1, 'visible 0'],
      ] as const) {
        await seek(page, fraction);
        expect(await shown(), `at ${fraction}`).toBe(expected);
      }
      await page.evaluate(() => (window as unknown as Harness).sd.resume());
      await settle(page);
      expect(await shown(), 'settled').toBe('hidden 1');
      expect(await diff(page, await shot(page), await staticShot(page, slide, step + 1))).toBe(0);
    });

    await test.step('blast', async () => {
      const [id, step] = cases.blast;
      const { slide, target } = await playPaused(page, id, step);
      await settle(page);
      await next(page);
      // It loops while the slide shows: a frame of its second round is the same frame of its first.
      const frames: string[] = [];
      for (const ms of [250, 750]) {
        await page.evaluate((at) => (window as unknown as Harness).sd.seekMs(at), ms);
        frames.push(await computed(page, target, null, 'transform'));
      }
      expect(frames[1], 'a round later').toBe(frames[0]);
      expect(frames[0]).not.toBe('matrix(1.1, 0, 0, 1.1, 0, 0)');
      await page.evaluate(() => (window as unknown as Harness).sd.resume());
      await settle(page);
      expect(await looping(page, target), 'still looping once its group settled').toBe(true);
      // The next click plays the slide's next group, and it loops on.
      await page.evaluate(() => (window as unknown as Harness).sd.player.next());
      await settle(page);
      expect(await looping(page, target), 'still looping through the next click').toBe(true);
      // The click that leaves the slide cuts it: it holds where its one round ends.
      const cut = await page.evaluate(
        ([slideId, el]) => {
          (window as unknown as Harness).sd.player.next();
          const layer = document.querySelector(`.sd-layer[data-sd-layer="${slideId}"]`);
          const element = [...(layer?.shadowRoot?.querySelectorAll('[data-el]') ?? [])].find(
            (e) => e.getAttribute('data-el') === el,
          );
          return element === undefined ? '' : getComputedStyle(element).transform;
        },
        [id, target] as const,
      );
      expect(cut, 'cut by the click that leaves the slide').toBe('matrix(1.1, 0, 0, 1.1, 0, 0)');
      await settle(page);
      await page.evaluate(([at, s]) => (window as unknown as Harness).sd.player.goTo(at, s), [
        slide,
        step + 1,
      ] as const);
      await settle(page);
      expect(await computed(page, target, null, 'transform'), 'held').toBe(cut);
      expect(await diff(page, await shot(page), await staticShot(page, slide, step + 1))).toBe(0);
    });
  });
}
