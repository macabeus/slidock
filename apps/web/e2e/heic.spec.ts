// The HEIC probe deck (`probes/out/heic-probe.pptx`: five Apple-encoded photos, a 3 × 2 grid tagged Display P3, the
// grid turned a quarter by its `irot`, a single picture, the single picture mirrored top to bottom by an `imir`, and a
// 61 × 45 picture cut to a 40 × 30 rectangle by its `clap`) in the app. A browser that decodes HEIC (WebKit, with its
// own image decoder; Chrome, `SLIDOCK_E2E_CHROME=1`, with WebCodecs) shows each photo at its plan's size, each tile
// in its place and turned, and downloads the zip the compiler writes for the same decoded files. One that cannot
// (Playwright's Chromium, with no HEVC decoder) keeps each photo's marker, and its diagnostic says what to do.
import { expect, test } from '@playwright/test';
import { join } from 'node:path';

import { GOOGLE_OFF, OUT, REPO } from './support/decks';
import { digest } from './support/files';
import { decodedPhotos, wasmZip, zipNames } from './support/heic';
import { watch } from './support/player';

const DECK = join(REPO, 'probes/out/heic-probe.pptx');

type Rgb = [number, number, number];
const RED: Rgb = [0xd2, 0x3c, 0x3c];
const GREEN: Rgb = [0x3c, 0xa0, 0x3c];
const BLUE: Rgb = [0x3c, 0x50, 0xc8];
const YELLOW: Rgb = [0xe6, 0xc8, 0x3c];
const PURPLE: Rgb = [0x8c, 0x3c, 0xb4];
const TEAL: Rgb = [0x3c, 0xb4, 0xb4];
const WHITE: Rgb = [255, 255, 255];

/** Each photo's size and colours at points of it (`probes/heif/make-media.sh` draws them). */
const PHOTOS: { size: [number, number]; points: [number, number, Rgb][] }[] = [
  {
    size: [1100, 600],
    points: [
      [256, 256, RED],
      [768, 256, GREEN],
      [1062, 256, BLUE],
      [256, 556, YELLOW],
      [768, 556, PURPLE],
      [1062, 556, TEAL],
      [100, 40, WHITE],
    ],
  },
  {
    // Turned a quarter anticlockwise: (x, y) of the grid is (y, 1100 − x).
    size: [600, 1100],
    points: [
      [256, 844, RED],
      [256, 332, GREEN],
      [256, 38, BLUE],
      [556, 844, YELLOW],
      [556, 332, PURPLE],
      [556, 38, TEAL],
      [40, 1000, WHITE],
    ],
  },
  {
    size: [64, 48],
    points: [
      [24, 16, RED],
      [48, 12, GREEN],
      [16, 36, BLUE],
      [48, 36, YELLOW],
    ],
  },
  {
    // Mirrored about the horizontal axis (`imir` axis 0): (x, y) of the single picture is (x, 47 − y).
    size: [64, 48],
    points: [
      [24, 30, RED],
      [48, 36, GREEN],
      [16, 12, BLUE],
      [48, 12, YELLOW],
      [10, 40, WHITE],
    ],
  },
  {
    // The 40 × 30 rectangle from (4, 3) of the quadrants: (x, y) of the picture is (x − 4, y − 3).
    size: [40, 30],
    points: [
      [14, 14, RED],
      [34, 10, GREEN],
      [14, 26, BLUE],
      [34, 26, YELLOW],
      [6, 3, WHITE],
    ],
  },
];

test('heic: a browser that decodes HEIC shows the photos; another keeps their markers and says what to do', async ({
  page,
  browserName,
}, testInfo) => {
  test.setTimeout(180_000);
  const problems = watch(page);
  await page.addInitScript(({ key, value }) => {
    try {
      localStorage.setItem(key, value);
    } catch {
      // A frame with an opaque origin has no storage.
    }
  }, GOOGLE_OFF);
  await page.goto('/');
  await expect(page.getByTestId('wasm-status')).toHaveText(/^slidock-wasm \S+$/, { timeout: 60_000 });

  // What this browser can do, asked the way the app's decoder asks.
  const capable = await page.evaluate(async () => {
    const VideoDecoderClass = (globalThis as { VideoDecoder?: typeof VideoDecoder }).VideoDecoder;
    const webcodecs = VideoDecoderClass
      ? await VideoDecoderClass.isConfigSupported({ codec: 'hvc1.3.E.L90.B0' }).then(
          (support) => support.supported === true,
          () => false,
        )
      : false;
    return { webcodecs };
  });
  const decodes = browserName === 'webkit' || testInfo.project.name === 'chrome' || capable.webcodecs;
  if (browserName === 'chromium' && testInfo.project.name === 'chromium') {
    // Playwright's Chromium is an open-source build: no HEVC.
    expect(capable.webcodecs).toBe(false);
  }

  await page.evaluate(() => {
    const seen: string[] = [];
    (window as unknown as { __sdSteps: string[] }).__sdSteps = seen;
    new MutationObserver(() => {
      for (const item of Array.from(document.querySelectorAll('[aria-label="Conversion"] li'))) {
        const text = item.textContent ?? '';
        if (!seen.includes(text)) {
          seen.push(text);
        }
      }
    }).observe(document.body, { subtree: true, childList: true, characterData: true });
  });
  await page.locator('input[type="file"]').setInputFiles(DECK);
  await expect(page.getByRole('toolbar', { name: 'Deck' })).toBeVisible({ timeout: 120_000 });
  const steps = await page.evaluate(() => (window as unknown as { __sdSteps: string[] }).__sdSteps);
  expect(
    steps.some((step) => step.startsWith('Decoding 5 HEIC photos')),
    steps.join(' | '),
  ).toBe(true);

  const layer = page.getByTestId('stage').locator('.sd-player > .sd-frame > .sd-layer[data-sd-current]');
  const shown = await layer.evaluate(async (element) => {
    const root = element.shadowRoot;
    const images = Array.from(root?.querySelectorAll('img') ?? []);
    await Promise.all(images.map((img) => img.decode().catch(() => {})));
    return {
      markers: (root?.textContent ?? '').split('Image not included').length - 1,
      images: images.map((img) => ({ width: img.naturalWidth, height: img.naturalHeight, src: img.currentSrc })),
    };
  });

  await page.getByRole('tab', { name: /Diagnostics/ }).click();
  const panel = page.locator('#bottom-panel-diagnostics');
  if (decodes) {
    expect(shown.markers).toBe(0);
    expect(shown.images.map(({ width, height }) => [width, height])).toEqual(PHOTOS.map(({ size }) => size));
    // Each photo's colours where its picture drew them: the tiles in place, the turned grid turned.
    const colours = await layer.evaluate(async (element, photos) => {
      const images = Array.from(element.shadowRoot?.querySelectorAll('img') ?? []);
      return images.map((img, index) => {
        const canvas = document.createElement('canvas');
        canvas.width = img.naturalWidth;
        canvas.height = img.naturalHeight;
        const context = canvas.getContext('2d');
        context?.drawImage(img, 0, 0);
        return (photos[index]?.points ?? []).map(([x, y]) =>
          Array.from(context?.getImageData(x, y, 1, 1).data ?? []).slice(0, 3),
        );
      });
    }, PHOTOS);
    for (const [index, photo] of PHOTOS.entries()) {
      for (const [point, [x, y, expected]] of photo.points.entries()) {
        const found = colours[index]?.[point] ?? [];
        const off = Math.max(...expected.map((value, channel) => Math.abs(value - (found[channel] ?? -999))));
        expect(off, `photo ${index} at ${x},${y}: ${found} for ${expected}`).toBeLessThanOrEqual(24);
      }
    }
    await expect(panel).toContainText('MID5608');
    await expect(panel).not.toContainText('MID5601');

    // The zip holds the decoded photos, and is the one the compiler writes for the same files.
    await page.getByRole('button', { name: 'Download zip' }).click();
    const dialog = page.getByRole('dialog', { name: 'Download zip' });
    const download = page.waitForEvent('download');
    await dialog.getByRole('button', { name: 'Build and save' }).click();
    const saved = join(OUT, 'downloads', testInfo.project.name, 'heic-probe.zip');
    await (await download).saveAs(saved);
    const photos = zipNames(saved).filter((name) => name.endsWith('.jpg'));
    expect(photos).toHaveLength(PHOTOS.length);
    const cli = join(OUT, 'heic', testInfo.project.name);
    const reference = wasmZip(DECK, [], join(cli, 'without'));
    const decoded = decodedPhotos(saved, reference, join(cli, 'photos'));
    expect(decoded.map(({ file }) => file.split('/').pop())).toEqual(photos.map((name) => name.split('/').pop()));
    const [app, wasm] = await Promise.all([digest(saved), digest(wasmZip(DECK, decoded, join(cli, 'with')))]);
    expect(app, 'the app zip equals the compiler zip with the same decoded photos').toEqual(wasm);
  } else {
    expect(shown.images).toEqual([]);
    expect(shown.markers).toBe(PHOTOS.length);
    await expect(panel).toContainText(
      'which this browser cannot decode: open the deck in Chrome or Safari, or export it to PPTX from Keynote',
    );
    await expect(panel).not.toContainText('MID5608');
  }
  testInfo.annotations.push({ type: 'heic', description: decodes ? 'decoded' : 'kept omitted' });
  expect(problems, 'console errors, page errors and CSP violations').toEqual([]);
});
