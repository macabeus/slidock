// The probe decks as the CLI writes them (their fonts shipped), opened from `file://` with this build's player and
// driven by the motion lane's own page script (`crates/slidock-bench/src/motion_lane/motion.js`): each position of a
// slide with font switches, reached by playing its steps as the lane does (each batch paused at its start, held at
// marks along the way, then finished, then the still render shown over it and taken away), paints exactly as a cut
// to the same position, which is how the shell lane walks
// the zip a user downloads. A stage that leaves pixels of an earlier frame fails it.
import { type Page, expect, test } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import { REPO, cli, compile, embedPlayer } from './support/cli';

/** Slides whose steps switch a synthetic bold (the decks ship Liberation Sans without its bold face). */
const CASES = [
  { format: 'pptx', slide: 'pp-419' },
  { format: 'odp', slide: 'od-s130' },
] as const;

/** Where the lane holds a batch, as thousandths of its span (`drive.rs` `TRANSITION_PM`, `STEP_PM`). */
const MARKS = [0, 250, 500, 750, 1000];
/** How far from a switch's begin the lane holds a batch to see it before and after (`drive.rs` `SWITCH_MS`). */
const SWITCH_MS = 1;

/**
 * The lane's screenshots take the page as it is: Playwright's default hides the caret with an injected style, whose
 * restyle repaints the slide and would cover a stale pixel.
 */
const SHOT = { caret: 'initial', animations: 'allow' } as const;

const MOTION_JS = readFileSync(join(REPO, 'crates/slidock-bench/src/motion_lane/motion.js'), 'utf8');

interface Report {
  readonly batch: { readonly durationMs: number } | null;
  readonly position: { readonly slide: number; readonly step: number };
}

interface Lane {
  boot(): unknown;
  pending(): Promise<Report>;
  cut(slide: number, step: number): Promise<Report>;
  next(): Promise<Report>;
  seek(ms: number): Promise<unknown>;
  still(slide: number | null, step: number): Promise<unknown>;
  finish(): Promise<Report>;
}

test.describe.configure({ mode: 'serial', timeout: 10 * 60_000 });

const pages = new Map<string, string>();
test.beforeAll(() => {
  test.setTimeout(30 * 60_000);
  const binary = cli();
  for (const { format } of CASES) {
    const dir = compile(binary, `probes/out/motion-probe.${format}`, `settled-${format}`);
    const page = join(dir, 'index.html');
    writeFileSync(page, embedPlayer(readFileSync(page, 'utf8')));
    pages.set(format, pathToFileURL(page).href);
  }
});

/** Opens the deck under the lane's script; resolves with the slide's index and its clicks, and the clicks before. */
async function boot(page: Page, url: string, id: string): Promise<{ index: number; clicks: number; before: number }> {
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.addInitScript(MOTION_JS);
  await page.goto(url);
  const found = await page.evaluate((slideId) => {
    const api = (window as unknown as { Slidock: { inlineDeck(d: Document): { deck: { slides: unknown[] } } } })
      .Slidock;
    const count = (window as unknown as { Slidock: { stepCount(slide: unknown): number } }).Slidock.stepCount;
    const slides = api.inlineDeck(document).deck.slides;
    const index = slides.findIndex((slide) => (slide as { id: string }).id === slideId);
    return { index, clicks: count(slides[index]), before: count(slides[index - 1]) };
  }, id);
  await page.evaluate(() => (window as unknown as { __sdMotion: Lane }).__sdMotion.boot());
  await play(page, () => (window as unknown as { __sdMotion: Lane }).__sdMotion.pending());
  return found;
}

/** Runs a move, then each batch it plays held at the lane's marks and finished, until the move is over. */
async function play(page: Page, move: () => Promise<Report>): Promise<void> {
  let report = await page.evaluate(move);
  while (report.batch !== null) {
    const span = report.batch.durationMs;
    // The lane sees a switch just before its begin (`drive.rs` `switch_pauses`): a batch held before its start.
    for (const ms of [-SWITCH_MS, ...MARKS.map((mark) => (span * mark) / 1000)]) {
      await page.evaluate((at) => (window as unknown as { __sdMotion: Lane }).__sdMotion.seek(at), ms);
      await page.screenshot(SHOT);
    }
    report = await page.evaluate(() => (window as unknown as { __sdMotion: Lane }).__sdMotion.finish());
  }
}

for (const c of CASES) {
  test(`${c.slide}: every position played as the motion lane plays it paints as its cut`, async ({ browser }) => {
    const url = pages.get(c.format) ?? '';
    const played = await browser.newPage();
    const { index, clicks, before } = await boot(played, url, c.slide);
    expect(index).toBeGreaterThan(0);
    expect(clicks).toBeGreaterThan(0);
    // Arriving from the slide before, through its transition, as the lane's walk does.
    await played.evaluate(([i, step]) => (window as unknown as { __sdMotion: Lane }).__sdMotion.cut(i, step), [
      index - 1,
      before,
    ] as const);
    const shots: Buffer[] = [];
    for (let step = 0; step <= clicks; step++) {
      await play(played, () => (window as unknown as { __sdMotion: Lane }).__sdMotion.next());
      shots.push(await played.screenshot(SHOT));
      // The lane then shoots the player's still render of the stage over it, and takes it away.
      await played.evaluate(([i, at]) => (window as unknown as { __sdMotion: Lane }).__sdMotion.still(i, at), [
        index,
        step,
      ] as const);
      await played.screenshot(SHOT);
      await played.evaluate(() => (window as unknown as { __sdMotion: Lane }).__sdMotion.still(null, 0));
    }
    const cut = await browser.newPage();
    await boot(cut, url, c.slide);
    for (let step = 0; step <= clicks; step++) {
      await cut.evaluate(([i, at]) => (window as unknown as { __sdMotion: Lane }).__sdMotion.cut(i, at), [
        index,
        step,
      ] as const);
      expect((await cut.screenshot(SHOT)).equals(shots[step] ?? Buffer.alloc(0)), `${c.slide} step ${step}`).toBe(true);
    }
  });
}
