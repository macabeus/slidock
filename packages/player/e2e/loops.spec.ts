// The motion lane's rule for an endless repeat, played by its own page script
// (`crates/slidock-bench/src/motion_lane/motion.js`) on the probe decks' repeat slide (LibreOffice's blast repeated
// until the end of the slide, then a teeter until the next click): a batch counts one round of an endless repeat, the
// repeat loops at the times the lane pauses inside the batch, and the batch's end is a click that cuts it (`drive.rs`
// `pauses`). That end frame and the settled stage paint exactly as the player's still render of the stage, its held
// end, and not as the end of the repeat's first round.
import { type Page, expect, test } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import { REPO, cli, compile, embedPlayer } from './support/cli';

const CASES = [
  { format: 'pptx', slide: 'pp-452' },
  { format: 'odp', slide: 'od-s164' },
] as const;

const SHOT = { caret: 'initial', animations: 'allow' } as const;
const MOTION_JS = readFileSync(join(REPO, 'crates/slidock-bench/src/motion_lane/motion.js'), 'utf8');

interface Animation {
  readonly delay: number;
  readonly duration: number;
  readonly direction: string | null;
  readonly end: number | null;
}
interface Report {
  readonly batch: { readonly durationMs: number; readonly animations: readonly Animation[] } | null;
}
interface Lane {
  boot(): unknown;
  pending(): Promise<Report>;
  cut(slide: number, step: number): Promise<Report>;
  next(): Promise<Report>;
  seek(ms: number, cut: boolean): Promise<unknown>;
  still(slide: number | null, step: number): Promise<unknown>;
  finish(): Promise<Report>;
}

test.describe.configure({ mode: 'serial', timeout: 10 * 60_000 });

const pages = new Map<string, string>();
test.beforeAll(() => {
  test.setTimeout(30 * 60_000);
  const binary = cli();
  for (const { format } of CASES) {
    const dir = compile(binary, `probes/out/motion-probe.${format}`, `loops-${format}`);
    const page = join(dir, 'index.html');
    writeFileSync(page, embedPlayer(readFileSync(page, 'utf8')));
    pages.set(format, pathToFileURL(page).href);
  }
});

/** A batch's length as the lane counts it (`drive.rs` `batch_length`): one round of each endless repeat. */
function length(batch: NonNullable<Report['batch']>): number {
  const ends = batch.animations.map(
    (a) => a.end ?? a.delay + a.duration * (a.direction === 'alternate' || a.direction === 'alternate-reverse' ? 2 : 1),
  );
  return Math.max(batch.durationMs, ...ends);
}

/** The iteration each endless repeat of the shown slides is in now (0 is its first round). */
function iterations(page: Page): Promise<(number | null)[]> {
  return page.evaluate(() =>
    [...document.querySelectorAll('.sd-layer')]
      .flatMap((layer) => layer.shadowRoot?.getAnimations() ?? [])
      .filter((a) => a.effect?.getTiming().iterations === Infinity)
      .map((a) => a.effect?.getComputedTiming().currentIteration ?? null),
  );
}

/**
 * Plays a move as the lane does: each batch paused at its quarters, then at its end with the repeats cut; returns
 * each batch's end frame and how many endless repeats it played. A batch with an endless repeat is also paused a round
 * and a half into each of them, where each must be in its second round (iteration 1): the lane's pauses past an
 * endless step's first round, in a batch a longer step makes longer, see it loop.
 */
async function play(page: Page, report: Report): Promise<{ ends: Buffer[]; endless: number }> {
  const ends: Buffer[] = [];
  let endless = 0;
  for (
    let batch = report.batch;
    batch !== null;
    batch = (await page.evaluate(() => (window as unknown as { __sdMotion: Lane }).__sdMotion.finish())).batch
  ) {
    const span = length(batch);
    const loops = batch.animations.filter((a) => a.end === null).length;
    endless += loops;
    for (const at of [0, 0.25, 0.5, 0.75].map((f) => f * span)) {
      await page.evaluate(([ms]) => (window as unknown as { __sdMotion: Lane }).__sdMotion.seek(ms, false), [
        at,
      ] as const);
    }
    if (loops > 0) {
      const later = Math.max(
        ...batch.animations
          .filter((a) => a.end === null)
          .map((a) => a.delay + a.duration * 1.5 * (a.direction?.startsWith('alternate') === true ? 2 : 1)),
      );
      await page.evaluate(([ms]) => (window as unknown as { __sdMotion: Lane }).__sdMotion.seek(ms, false), [
        later,
      ] as const);
      const now = await iterations(page);
      expect(now.length, 'the endless repeats on the stage').toBe(loops);
      expect(
        now.every((i) => i !== null && i >= 1),
        `a round and a half in, each loops: ${JSON.stringify(now)}`,
      ).toBe(true);
    }
    await page.evaluate(([ms, cut]) => (window as unknown as { __sdMotion: Lane }).__sdMotion.seek(ms, cut), [
      span,
      loops > 0,
    ] as const);
    ends.push(await page.screenshot(SHOT));
  }
  return { ends, endless };
}

for (const c of CASES) {
  test(`${c.slide}: an endless repeat played by the motion lane ends and settles as its still render`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 720 });
    await page.addInitScript(MOTION_JS);
    await page.goto(pages.get(c.format) ?? '');
    const { index, clicks, before } = await page.evaluate((slideId) => {
      const api = (
        window as unknown as {
          Slidock: { inlineDeck(d: Document): { deck: { slides: unknown[] } }; stepCount(slide: unknown): number };
        }
      ).Slidock;
      const slides = api.inlineDeck(document).deck.slides;
      const i = slides.findIndex((slide) => (slide as { id: string }).id === slideId);
      return { index: i, clicks: api.stepCount(slides[i]), before: api.stepCount(slides[i - 1]) };
    }, c.slide);
    await page.evaluate(() => (window as unknown as { __sdMotion: Lane }).__sdMotion.boot());
    await play(page, await page.evaluate(() => (window as unknown as { __sdMotion: Lane }).__sdMotion.pending()));
    await page.evaluate(([i, step]) => (window as unknown as { __sdMotion: Lane }).__sdMotion.cut(i, step), [
      index - 1,
      before,
    ] as const);
    let looped = 0;
    for (let step = 0; step <= clicks; step++) {
      const { ends, endless } = await play(
        page,
        await page.evaluate(() => (window as unknown as { __sdMotion: Lane }).__sdMotion.next()),
      );
      looped += endless;
      const settled = await page.screenshot(SHOT);
      await page.evaluate(([i, at]) => (window as unknown as { __sdMotion: Lane }).__sdMotion.still(i, at), [
        index,
        step,
      ] as const);
      const still = await page.screenshot(SHOT);
      await page.evaluate(() => (window as unknown as { __sdMotion: Lane }).__sdMotion.still(null, 0));
      expect(settled.equals(still), `${c.slide} step ${step}: settled = still`).toBe(true);
      const end = ends.at(-1);
      if (end !== undefined && endless > 0) {
        expect(end.equals(still), `${c.slide} step ${step}: the cut end = still`).toBe(true);
      }
    }
    // The blast and the teeter.
    expect(looped).toBe(2);
  });
}
