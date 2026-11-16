// A line fading in (the motion probes' `ft-line-fade`, compiled with the CLI): a straight line's box has no height,
// and its stroke and arrowhead overflow it. A fade must fade what overflows with the rest: WebKit composites an
// opacity animation of an empty box apart from its overflow, which then paints at full opacity unless the box has an
// extent of its own (slidock-html's transparent box shadow). Each pixel of the line, mid-fade,
// is the mix of the slide's background and the line's colour at the element's opacity; settled, the line is the
// still render.
import { type Page, expect, test } from '@playwright/test';
import type { Deck } from '@slidock/types';
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

const LABEL = 'line: fade in on click';
/** The probe line's colour (`2F6FDF`). */
const LINE = [47, 111, 223] as const;
/** Points of the line in its slide, in pt: inside its stroke (overflowing the box by up to 3 pt) and its arrowhead. On
 * the harness's 960 px host the 960 pt stage (1280 px) is scaled by 3/4, so a point is a pixel. */
const POINTS = { stroke: [400, 298], arrowhead: [870, 300] } as const;

async function pixel(page: Page, image: string, [x, y]: readonly [number, number]): Promise<number[]> {
  return page.evaluate(
    async ([b64, px, py]) => {
      const bitmap = await createImageBitmap(await (await fetch(`data:image/png;base64,${b64}`)).blob());
      const ctx = new OffscreenCanvas(bitmap.width, bitmap.height).getContext('2d');
      ctx?.drawImage(bitmap, 0, 0);
      const d = ctx?.getImageData(Math.round(px), Math.round(py), 1, 1).data;
      return [d?.[0] ?? -1, d?.[1] ?? -1, d?.[2] ?? -1];
    },
    [image, x, y] as const,
  );
}

const mix = (from: readonly number[], to: readonly number[], t: number) =>
  from.map((v, i) => v + ((to[i] ?? 0) - v) * t);
const near = (actual: readonly number[], expected: readonly number[], tolerance: number) =>
  actual.every((v, i) => Math.abs(v - (expected[i] ?? 0)) <= tolerance);

for (const format of ['pptx', 'odp'] as const) {
  test(`motion-probe.${format}: a line fading in fades its overflowing stroke and arrowhead`, async ({ page }) => {
    const dir = compile(binary, `probes/out/motion-probe.${format}`, `lines-${format}`);
    const { deck, slides } = await readDeckPage(page, readFileSync(join(dir, 'index.html'), 'utf8'));
    const index = deck.slides.findIndex((s) => slides[s.id]?.includes(`aria-label="${LABEL}"`));
    expect(index, 'the line slide').toBeGreaterThan(0);
    const slide = deck.slides[index] as Deck['slides'][number];
    const target = slide.timeline[0]?.target.element ?? '';
    expect(slides[slide.id], 'a box of zero height').toMatch(
      new RegExp(`data-el="${target}"[^>]*height:0pt;box-shadow:0 0 0 1pt transparent`),
    );
    await open(page, { deck, slides });
    await page.evaluate((at) => (window as unknown as Harness).sd.player.goTo(at, 0), index);
    await settle(page);
    const background = await pixel(page, await shot(page), POINTS.arrowhead);

    await page.evaluate(() => {
      const { sd } = window as unknown as Harness;
      sd.player.next();
      sd.pause();
    });
    for (const fraction of [0.25, 0.5, 0.75]) {
      await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), fraction);
      await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
      const opacity = Number(
        await page.evaluate(
          (id) => getComputedStyle((window as unknown as Harness).sd.target(id) as Element).opacity,
          target,
        ),
      );
      expect(opacity, `the line's opacity at ${fraction}`).toBeGreaterThan(0.05);
      expect(opacity, `the line's opacity at ${fraction}`).toBeLessThan(0.95);
      const frame = await shot(page);
      for (const [name, at] of Object.entries(POINTS)) {
        const actual = await pixel(page, frame, at);
        const expected = mix(background, LINE, opacity);
        expect(near(actual, expected, 6), `${name} at ${fraction}: ${actual} ≈ ${expected.map(Math.round)}`).toBe(true);
      }
    }

    await page.evaluate(() => (window as unknown as Harness).sd.resume());
    await settle(page);
    const settled = await shot(page);
    expect(near(await pixel(page, settled, POINTS.arrowhead), LINE, 3), 'built: the arrowhead').toBe(true);
    expect(await diff(page, settled, await staticShot(page, index, 1)), 'built: the still render').toBe(0);
  });
}
