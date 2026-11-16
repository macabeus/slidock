// The deck page of a CLI zip, opened from file:// with this build's player: P opens the presenter view
// (`#presenter=<session>`), whose next-click preview shows the next build step, then the next slide, as the deck
// window moves. The probe deck gets two click builds on its first slide (the JSON is data, outside the page's CSP).
import { type Page, expect, test } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import { cli, compile, embedPlayer, withDeck } from './support/cli';
import { diff } from './support/harness';

test.describe.configure({ timeout: 10 * 60_000 });

let page0: { url: string; first: string; second: string } | null = null;

test.beforeAll(() => {
  test.setTimeout(30 * 60_000);
  const dir = compile(cli(), 'probes/out/motion-probe.pptx', 'zip-presenter');
  const html = readFileSync(join(dir, 'index.html'), 'utf8');
  let first = '';
  let second = '';
  const edited = withDeck(embedPlayer(html), (deck) => {
    const slides = deck.slides as { id: string; timeline: unknown[] }[];
    const slide = slides[0];
    if (slide === undefined) {
      throw new Error('the probe deck has no slides');
    }
    first = slides[0]?.id ?? '';
    second = slides[1]?.id ?? '';
    const build = (element: string, effect: Record<string, unknown>) => ({
      target: { element, paragraphs: null },
      trigger: 'click',
      kind: 'entrance',
      delivery: 'all',
      delayMs: 0,
      durationMs: 400,
      native: 'test:build',
      fallback: 'dissolve',
      ...effect,
    });
    slide.timeline = [
      build(`${slide.id}.2`, { effect: 'fade' }),
      build(`${slide.id}.3`, { effect: 'flyIn', params: { dir: 'u', distance: 'offSlide' } }),
    ];
  });
  writeFileSync(join(dir, 'index.html'), edited);
  page0 = { url: pathToFileURL(join(dir, 'index.html')).href, first, second };
});

/** Waits until nothing animates on the deck page (a key during a build finishes it instead of advancing). */
async function idle(page: Page): Promise<void> {
  await page.waitForFunction(
    () =>
      [...document.querySelectorAll('.sd-layer')].every((layer) =>
        (layer.shadowRoot?.getAnimations() ?? []).every((a) => a.playState !== 'running'),
      ) && document.getAnimations().every((a) => a.playState !== 'running'),
  );
}

/** The presenter's preview against a still render of (slide, step) drawn over it at the same place. */
async function previewMatches(presenter: Page, slide: number, step: number): Promise<number> {
  const preview = presenter.locator('.sd-presenter .sd-next > div');
  await presenter.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const box = await preview.boundingBox();
  if (box === null) {
    throw new Error('no preview');
  }
  const a = (await presenter.screenshot({ clip: box })).toString('base64');
  await presenter.evaluate(
    ([s, t, at]) => {
      const host = document.createElement('div');
      host.id = 'expected';
      host.style.cssText = `position:fixed;left:${at.x}px;top:${at.y}px;width:${at.width}px;height:${at.height}px;background:#000;z-index:20`;
      document.body.append(host);
      const api = (
        window as unknown as {
          Slidock: {
            inlineDeck(d: Document): unknown;
            renderStatic(source: unknown, i: number, step: number, host: HTMLElement): unknown;
          };
        }
      ).Slidock;
      api.renderStatic(api.inlineDeck(document), s, t, host);
    },
    [slide, step, box] as const,
  );
  await presenter.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const b = (await presenter.screenshot({ clip: box })).toString('base64');
  await presenter.evaluate(() => document.getElementById('expected')?.remove());
  // `diff` compares in the page it is given; the harness is not loaded here.
  await presenter.evaluate(DIFF);
  return diff(presenter, a, b, 16);
}

// The harness's pixel comparison, for a page the harness did not set up; images load as `data:` images, which the
// deck page's CSP allows (it refuses `fetch`).
const DIFF = `window.sd = window.sd || { async diff(a, b, tolerance) {
  const load = async (b64) => {
    const image = new Image();
    image.src = 'data:image/png;base64,' + b64;
    await image.decode();
    return createImageBitmap(image);
  };
  const [x, y] = await Promise.all([load(a), load(b)]);
  if (x.width !== y.width || x.height !== y.height) return -1;
  const canvas = new OffscreenCanvas(x.width, x.height);
  const ctx = canvas.getContext('2d');
  ctx.drawImage(x, 0, 0);
  const da = ctx.getImageData(0, 0, x.width, x.height).data;
  ctx.clearRect(0, 0, x.width, x.height);
  ctx.drawImage(y, 0, 0);
  const db = ctx.getImageData(0, 0, x.width, x.height).data;
  let count = 0;
  for (let i = 0; i < da.length; i += 4) {
    const d = Math.max(Math.abs(da[i] - db[i]), Math.abs(da[i + 1] - db[i + 1]), Math.abs(da[i + 2] - db[i + 2]));
    if (d > tolerance) count++;
  }
  this.lastDiff = { count };
  return count;
} };`;

test('the zip presenter view previews the next build step, then the next slide', async ({ page }) => {
  if (page0 === null) {
    throw new Error('no deck');
  }
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') {
      errors.push(message.text());
    }
  });
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.goto(page0.url);
  await expect(page.locator('.sd-player')).toBeVisible();
  const popup = page.waitForEvent('popup');
  await page.keyboard.press('p');
  const presenter = await popup;
  await presenter.setViewportSize({ width: 1400, height: 900 });
  await expect(presenter).toHaveURL(/#presenter=[\w-]+$/);
  const label = presenter.locator('.sd-presenter .sd-next .sd-label').first();
  const next = presenter.locator('.sd-presenter .sd-next');
  await expect(label).toHaveText('Next: build 1 of 2');
  await expect(next).toHaveAttribute('data-sd-next', 'build');
  expect(await previewMatches(presenter, 0, 1)).toBe(0);
  expect(await previewMatches(presenter, 0, 0), 'not the current step').toBeGreaterThan(0);
  // The deck window drives: the presenter view follows and previews the click after.
  await idle(page);
  await page.keyboard.press('ArrowRight');
  await expect(label).toHaveText('Next: build 2 of 2');
  expect(await previewMatches(presenter, 0, 2)).toBe(0);
  await idle(page);
  await page.keyboard.press('ArrowRight');
  await expect(label).toHaveText('Next: slide 2');
  await expect(next).toHaveAttribute('data-sd-next', 'slide');
  expect(await previewMatches(presenter, 1, 0)).toBe(0);
  expect(errors).toEqual([]);
});
