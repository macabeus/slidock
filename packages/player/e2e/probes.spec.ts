// The motion probe decks (probes/out/motion-probe.{pptx,odp}) compiled with the CLI and played through by this
// build of the player: after every click (each build and each transition, played and fast-forwarded, an endless repeat
// cut as a click cuts it), the stage must equal `renderStatic` of the position it reached, pixel for pixel, and no
// animation may touch a property outside the allowed set. Steps and transitions the frontends give only as opaque ids
// play their declared fallback, and still end at the static truth. The fonts are left out: the live and the still
// render draw the same fallback faces. Once a media step has played, the medium shows a frame of its own rather than
// its poster (the static truth before playback), so the media boxes are masked in both shots.
import { expect, test } from '@playwright/test';
import type { Deck } from '@slidock/types';
import { readFileSync } from 'node:fs';
import { extname, join } from 'node:path';

import { cli, compile, readDeckPage } from './support/cli';
import { type Harness, diff, disallowedKeys, open, shot, staticShot } from './support/harness';

const MEDIA = 'https://deck.slidock.test/';
const TYPES: Record<string, string> = {
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.gif': 'image/gif',
  '.svg': 'image/svg+xml',
  '.webp': 'image/webp',
  '.mp4': 'video/mp4',
  '.mp3': 'audio/mpeg',
  '.wav': 'audio/wav',
};

test.describe.configure({ mode: 'serial', timeout: 15 * 60_000 });

let binary = '';
test.beforeAll(() => {
  test.setTimeout(30 * 60_000);
  binary = cli();
});

for (const format of ['pptx', 'odp'] as const) {
  test(`every motion-probe.${format} position played equals renderStatic`, async ({ page }) => {
    const dir = compile(binary, `probes/out/motion-probe.${format}`, `probe-${format}`);
    await page.route(`${MEDIA}**`, async (route) => {
      const path = decodeURIComponent(new URL(route.request().url()).pathname).replace(/^\/+/, '');
      if (!/^media\/[\w.-]+$/.test(path)) {
        await route.abort();
        return;
      }
      await route.fulfill({
        body: readFileSync(join(dir, path)),
        contentType: TYPES[extname(path).toLowerCase()] ?? 'application/octet-stream',
      });
    });
    const { deck: typed, slides } = await readDeckPage(page, readFileSync(join(dir, 'index.html'), 'utf8'));
    expect(typed.slides.length).toBeGreaterThan(100);
    // The walk visits every position by clicking, so a slide advances on a click and never on a timer; the advance
    // suites play the timers.
    for (const slide of typed.slides) {
      if (slide.transition !== null) {
        slide.transition = { ...slide.transition, advance: null };
      }
    }
    await open(page, { deck: typed, slides, mediaBase: MEDIA });

    const seen = new Set<string>();
    let positions = 0;
    for (;;) {
      const at = await page.evaluate(() => (window as unknown as Harness).sd.player.position);
      const key = `${at.slide}.${at.step}`;
      expect(seen.has(key), `position ${key} reached twice`).toBe(false);
      seen.add(key);
      positions++;
      const played = at.step > 0 && (typed.slides[at.slide]?.timeline ?? []).some((step) => step.kind === 'media');
      const mask = played ? [page.locator('video, audio')] : [];
      // An endless repeat loops while its slide shows. The walk cuts it, as the click that ends it does, to hold the
      // end of its round, the still render's state.
      await page.evaluate(() => (window as unknown as Harness).sd.cut());
      const expected = await staticShot(page, at.slide, at.step, mask);
      const id = typed.slides[at.slide]?.id ?? '';
      expect(await diff(page, await shot(page, mask), expected), `slide ${id} step ${at.step}`).toBe(0);
      const moved = await page.evaluate(async (from) => {
        const { sd } = window as unknown as Harness;
        sd.player.next();
        const to = sd.player.position;
        if (to.slide === from.slide && to.step === from.step) {
          return false;
        }
        const target = (sd.player as unknown as { deck: Deck }).deck.slides[to.slide]?.id;
        // Fast-forward every animation the move starts (the transition, then the automatic group) until it rests.
        let quiet = 0;
        for (let frame = 0; frame < 2_000 && quiet < 3; frame++) {
          if (sd.count() > 0) {
            sd.pause();
            sd.resume();
            quiet = 0;
          } else {
            const current = document.querySelector('.sd-layer[data-sd-current]');
            const resting =
              current?.getAttribute('data-sd-layer') === target &&
              document.querySelector('.sd-layer[data-sd-incoming]') === null;
            quiet = resting ? quiet + 1 : 0;
          }
          await new Promise((done) => requestAnimationFrame(done));
        }
        return quiet >= 3;
      }, at);
      if (!moved) {
        break;
      }
      expect(await disallowedKeys(page)).toEqual([]);
    }
    const shown = typed.slides.filter((slide) => !slide.hidden).length;
    const clicks = typed.slides
      .filter((slide) => !slide.hidden)
      .reduce((sum, slide) => sum + slide.timeline.filter((s) => s.trigger === 'click').length, 0);
    expect(positions).toBe(shown + clicks);
    const codes = await page.evaluate(() => (window as unknown as Harness).sd.diagnostics.map((d) => d.code));
    test.info().annotations.push({
      type: 'probe',
      description: `${format}: ${positions} positions, ${typed.slides.filter((s) => s.transition !== null).length} transitions, diagnostics ${JSON.stringify([...new Set(codes)])}`,
    });
  });
}
