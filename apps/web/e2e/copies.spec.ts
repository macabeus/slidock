// The copy of a transparent video (`media/alpha.ts`, `slidock::alternates`) on the video probe deck
// (`probes/out/video-probe.pptx`), whose "Clip with alpha" is HEVC with an alpha layer: opaque on its left half, at
// alpha 96 on its right. WebKit plays it with its alpha, so it makes the WebM copy: the zip holds both files, the
// slide's video offers both, and the deck opened from file:// plays the clip with its transparency in WebKit (the
// original) and in Chromium (the copy), as the two backgrounds it is drawn over show through its right half and not
// its left. Chromium plays no HEVC: it makes no copy, says the clip plays only in Safari and Chrome on a Mac and how
// to fix it, and the deck it writes does not play the clip in Chromium. The opaque H.264 clip on the same slide is the
// check's negative control: no background shows through it.
import { type Browser, type Page, chromium, expect, firefox, test, webkit } from '@playwright/test';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import { FIREFOX_IN_DOCKER } from '../../../packages/player/e2e/support/browserServer';
import { BROWSER_SERVER_PORT, GOOGLE_OFF, OUT, REPO } from './support/decks';
import { unpack, zipEntries } from './support/images';
import { CURRENT, watch } from './support/player';
import { decode } from './support/png';

const DECK = join(REPO, 'probes/out/video-probe.pptx');

function sha(name: string): string {
  return createHash('sha256')
    .update(readFileSync(join(REPO, 'probes/videos/media', name)))
    .digest('hex');
}

const ALPHA = sha('alpha.mov');
const OPAQUE = sha('own.mp4');

async function download(page: Page, path: string): Promise<{ dialog: ReturnType<Page['getByRole']> }> {
  await page.addInitScript(({ key, value }) => {
    try {
      localStorage.setItem(key, value);
    } catch {
      // A frame with an opaque origin has no storage.
    }
  }, GOOGLE_OFF);
  await page.goto('/');
  await expect(page.getByTestId('wasm-status')).toHaveText(/^slidock-wasm \S+$/, { timeout: 60_000 });
  await page.locator('input[type="file"]').setInputFiles(DECK);
  await expect(page.getByRole('toolbar', { name: 'Deck' })).toBeVisible({ timeout: 120_000 });
  await page.getByRole('button', { name: 'Download zip' }).click();
  const dialog = page.getByRole('dialog', { name: 'Download zip' });
  const toggle = dialog.getByRole('switch', { name: 'Play transparent videos in every browser' });
  await expect(toggle).toBeChecked();
  const saved = page.waitForEvent('download', { timeout: 240_000 });
  await dialog.getByRole('button', { name: 'Build and save' }).click();
  await (await saved).saveAs(path);
  await expect(dialog.getByRole('button', { name: 'Save again' })).toBeVisible();
  return { dialog };
}

/** What a clip of the deck's slide does: the file it plays, whether it plays, and what shows through it. */
interface Shown {
  src: string;
  plays: boolean;
  /** The mean difference (0 to 255) between its renders over two backgrounds, on its left and right halves. */
  left: number;
  right: number;
}

/** Plays the clip that names `hash`, stops it at 0.5 s, and renders it over a magenta and a green background. */
async function shown(page: Page, hash: string): Promise<Shown> {
  const video = page.locator(CURRENT).locator(`video:has(source[src*="${hash}"]), video[src*="${hash}"]`);
  await expect(video).toHaveCount(1);
  const state = await video.evaluate(async (element: HTMLVideoElement) => {
    element.muted = true;
    element.preload = 'auto';
    element.load();
    const loaded = await new Promise<boolean>((resolve) => {
      element.addEventListener('loadeddata', () => resolve(true), { once: true });
      element.addEventListener('error', () => resolve(false), { once: true });
      element.querySelector('source:last-of-type')?.addEventListener('error', () => resolve(false), { once: true });
      setTimeout(() => resolve(false), 20_000);
    });
    if (!loaded) {
      return { src: element.currentSrc, plays: false };
    }
    await element.play();
    await new Promise((resolve) => setTimeout(resolve, 300));
    const plays = element.currentTime > 0;
    element.pause();
    element.currentTime = 0.5;
    await new Promise((resolve) => element.addEventListener('seeked', resolve, { once: true }));
    await new Promise((resolve) => setTimeout(resolve, 200));
    return { src: element.currentSrc, plays };
  });
  const render = async (colour: string) => {
    await video.evaluate((element: HTMLVideoElement, background) => {
      element.style.background = background;
    }, colour);
    return decode(await video.screenshot());
  };
  const [a, b] = [await render('#ff00ff'), await render('#00ff00')];
  const half = (from: number, to: number) => {
    let total = 0;
    let count = 0;
    // Inset a few pixels from the edges and the middle.
    for (let y = 4; y < a.height - 4; y += 1) {
      for (let x = Math.round(from * a.width) + 4; x < Math.round(to * a.width) - 4; x += 1) {
        const at = (y * a.width + x) * 3;
        total += (Math.abs(a.rgb[at]! - b.rgb[at]!) + Math.abs(a.rgb[at + 1]! - b.rgb[at + 1]!)) / 2;
        count += 1;
      }
    }
    return total / Math.max(1, count);
  };
  return { ...state, left: half(0, 0.5), right: half(0.5, 1) };
}

/** The unpacked deck at `dir`, opened from file:// in `browser`. */
async function openDeck(browser: Browser, dir: string): Promise<{ page: Page; problems: string[] }> {
  const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
  const problems = watch(page);
  await page.goto(pathToFileURL(join(dir, 'index.html')).href);
  await page.locator(CURRENT).waitFor();
  return { page, problems };
}

test('copies: WebKit makes a WebM copy of the clip with alpha, and the deck plays it with its transparency in WebKit and Chromium', async ({
  page,
  browser,
  browserName,
}) => {
  test.skip(browserName !== 'webkit', 'the browser that plays HEVC with its alpha makes the copy');
  test.setTimeout(300_000);
  const problems = watch(page);
  const zip = join(OUT, 'copies', browserName, 'deck.zip');
  const { dialog } = await download(page, zip);
  await expect(dialog.getByText(/1 of 1 video with a WebM copy for every browser/)).toBeVisible();
  expect(problems).toEqual([]);

  const entries = zipEntries(zip);
  const names = [...entries.keys()];
  const original = `media/${ALPHA}.mov`;
  const copy = `media/${ALPHA}.webm`;
  expect(names.indexOf(copy), 'the copy right after the original').toBe(names.indexOf(original) + 1);
  expect(entries.get(copy)!.subarray(0, 4).toString('hex')).toBe('1a45dfa3');
  const index = entries.get('index.html')!.toString('utf8');
  expect(index).toContain(
    `<source src="${original}" type="video/quicktime; codecs=&quot;hvc1&quot;"><source src="${copy}" type="video/webm; codecs=&quot;vp9&quot;">`,
  );

  const dir = join(OUT, 'copies', browserName, 'deck');
  unpack(zip, dir);
  // Firefox in the pinned image (`SLIDOCK_E2E_FIREFOX=docker`) too: it decodes HEVC, but without its alpha layer, so
  // it must play the copy.
  const engines: [string, Browser][] = [
    ['webkit', browser],
    ['chromium', await chromium.launch()],
    ...(FIREFOX_IN_DOCKER
      ? [
          [
            'firefox',
            await firefox.connect(`ws://127.0.0.1:${BROWSER_SERVER_PORT}/`, { exposeNetwork: '<loopback>' }),
          ] as [string, Browser],
        ]
      : []),
  ];
  for (const [name, engine] of engines) {
    const deck = await openDeck(engine, dir);
    const alpha = await shown(deck.page, ALPHA);
    const opaque = await shown(deck.page, OPAQUE);
    test.info().annotations.push({ type: `shown in ${name}`, description: JSON.stringify({ alpha, opaque }) });
    expect(alpha.src.endsWith(name === 'webkit' ? '.mov' : '.webm'), `${name} plays ${alpha.src}`).toBe(true);
    expect(alpha.plays, name).toBe(true);
    // Its opaque half hides the background; its half at alpha 96 shows it: (1 − 96/255) of the 255 apart.
    expect(alpha.left, `${name}: the opaque half`).toBeLessThan(8);
    expect(alpha.right, `${name}: the transparent half`).toBeGreaterThan(120);
    // Negative control: an opaque clip shows no background.
    expect(opaque.plays, name).toBe(true);
    expect(Math.max(opaque.left, opaque.right), `${name}: the opaque clip`).toBeLessThan(8);
    expect(deck.problems, name).toEqual([]);
    await deck.page.close();
    if (engine !== browser) {
      await engine.close();
    }
  }
});

test('copies: Chromium plays no HEVC, so it makes no copy, says where the clip plays and how to fix it', async ({
  page,
  browser,
  browserName,
}) => {
  test.skip(browserName !== 'chromium', "Playwright's Chromium has no HEVC decoder");
  test.setTimeout(240_000);
  const zip = join(OUT, 'copies', browserName, 'deck.zip');
  const { dialog } = await download(page, zip);
  await expect(dialog.getByText('1 video plays only in Safari and Chrome on a Mac')).toBeVisible();
  await expect(dialog.getByText(/this browser plays no hvc1 video/)).toBeVisible();
  await expect(dialog.getByText(/download the deck again from Safari or Chrome on a Mac/)).toBeVisible();
  const entries = zipEntries(zip);
  expect([...entries.keys()].filter((name) => name.endsWith('.webm'))).toEqual([]);
  expect(entries.get('index.html')!.toString('utf8')).toContain(`<video src="media/${ALPHA}.mov"`);

  // The deck it writes plays the clip in WebKit only; Chromium shows its poster.
  const dir = join(OUT, 'copies', browserName, 'deck');
  unpack(zip, dir);
  const deck = await openDeck(browser, dir);
  const alpha = await shown(deck.page, ALPHA);
  expect(alpha.plays).toBe(false);
  await deck.page.close();
  const other = await webkit.launch();
  const inWebKit = await openDeck(other, dir);
  const played = await shown(inWebKit.page, ALPHA);
  expect([played.plays, played.right > 120, played.left < 8]).toEqual([true, true, true]);
  await other.close();
});
