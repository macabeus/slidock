// The download's picture compression on the photo probe (`probes/out/photo-probe.pptx`, `cargo xtask
// probes-photos`): the zip with "Compress images" on is smaller, each planned picture is re-encoded at its planned
// size (WebP where the browser encodes it, else JPEG or PNG), the animated GIF and the SVG stay as they are, and the
// deck, unpacked and opened from file://, draws the same slide: every element in the same box, the screenshot alike
// (SSIM). The choice is remembered across a reload, and a single-file download inlines the re-encoded pictures.
import { type Browser, type Page, expect, test } from '@playwright/test';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import { GOOGLE_OFF, OUT, REPO } from './support/decks';
import { imageHeader, ssim, unpack, zipEntries } from './support/images';
import { CURRENT, settle, watch } from './support/player';

const DECK = join(REPO, 'probes/out/photo-probe.pptx');

/** The planned pictures, by their source's SHA-256: the size each shows at and the size planned for it. */
const PLANNED: Record<string, { shows: [number, number]; target: [number, number] }> = {
  // photo.jpg, drawn twice: the larger use.
  c346272aba1999d5f8df1267a1cea7d538c5b4404b6c8071ddeeea0f3286803f: { shows: [2400, 1600], target: [1440, 960] },
  // crop.png, a quarter cut off each side.
  '0b8ef0814ac38d387c5debad6b89fe19f6d6221fb7b710cb0453251f7216af34': { shows: [1000, 750], target: [800, 600] },
  // logo.png, with transparency.
  '72f80e7fccfa06a9cdc47d3721e86a161dd04d4801e598114493624300cecc2e': { shows: [1200, 1200], target: [480, 480] },
  // turned.jpg: stored 1600 × 1200, EXIF orientation 6.
  '6bc569313d42f3376e6906bdc32eec605820e233e86f12320325eb08143cb341': { shows: [1200, 1600], target: [600, 800] },
  // fill.png, the ellipse's fill.
  d5f35262f0b718fe76419b2d0b71b5fe864d1a0ad61c4ea085d8b63fd33162bf: { shows: [800, 800], target: [320, 320] },
  // bg.jpg, the background: re-encoded at its own size.
  '27a12f9ab690df787ce8eceaf2bc5c4398cf6db4302332be5be95b56e7badff9': { shows: [3000, 1688], target: [3000, 1688] },
};

const EXTENSIONS: Record<string, string> = { 'image/webp': 'webp', 'image/jpeg': 'jpg', 'image/png': 'png' };

async function openDeck(page: Page) {
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
}

/** Builds the zip the dialog is set to and saves it as `name`. */
async function download(page: Page, name: string, browserName: string): Promise<string> {
  const dialog = page.getByRole('dialog', { name: 'Download zip' });
  const saved = page.waitForEvent('download', { timeout: 120_000 });
  await dialog.getByRole('button', { name: 'Build and save' }).click();
  const path = join(OUT, 'images', browserName, name);
  await (await saved).saveAs(path);
  await expect(dialog.getByRole('button', { name: 'Save again' })).toBeVisible();
  return path;
}

/** The deck of `zip`, unpacked and opened from file://: its slide's screenshot and every element's box. */
async function drawn(browser: Browser, zip: string, dir: string) {
  unpack(zip, dir);
  const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
  const problems = watch(page);
  await page.goto(pathToFileURL(join(dir, 'index.html')).href);
  const layer = page.locator(CURRENT);
  await expect(layer).toBeVisible();
  // Every picture has loaded and decoded.
  await layer.evaluate(async (element) => {
    const root = element.shadowRoot!;
    const images = Array.from(root.querySelectorAll('img'));
    await Promise.all(images.map((image) => image.decode().catch(() => undefined)));
    const missing = images.filter((image) => !image.complete || image.naturalWidth === 0);
    if (missing.length > 0) {
      throw new Error(`not loaded: ${missing.map((image) => image.src.slice(0, 80)).join(', ')}`);
    }
  });
  await settle(page);
  await page.waitForTimeout(300);
  const boxes = await layer.evaluate((element) =>
    Array.from(element.shadowRoot!.querySelectorAll('[data-el]'), (child) => {
      const box = child.getBoundingClientRect();
      return `${child.getAttribute('data-el')} ${box.x.toFixed(2)} ${box.y.toFixed(2)} ${box.width.toFixed(2)} ${box.height.toFixed(2)}`;
    }),
  );
  const shot = await layer.screenshot();
  await page.close();
  return { shot, boxes, problems };
}

test('images: a compressed download is smaller, re-encodes each planned picture at its size, and draws the same', async ({
  page,
  browser,
  browserName,
}) => {
  test.setTimeout(300_000);
  const problems = watch(page);
  await openDeck(page);
  await page.getByRole('button', { name: 'Download zip' }).click();
  const dialog = page.getByRole('dialog', { name: 'Download zip' });
  const toggle = dialog.getByRole('switch', { name: 'Compress images' });
  await expect(toggle).not.toBeChecked();
  const plainZip = await download(page, 'plain.zip', browserName);

  await toggle.check();
  await expect(dialog.getByRole('radio', { name: /High/ })).toBeChecked();
  const compressedZip = await download(page, 'compressed.zip', browserName);
  await expect(dialog.getByRole('heading', { name: 'Compressed images' })).toBeVisible();
  // Chromium encodes WebP; WebKit's canvas encodes none, and its JPEG of a picture already compressed harder can be
  // larger than the original, which is then kept.
  await expect(
    dialog.getByText(browserName === 'chromium' ? /6 of 6 pictures re-encoded/ : /[1-6] of 6 pictures re-encoded/),
  ).toBeVisible();

  const plain = zipEntries(plainZip);
  const compressed = zipEntries(compressedZip);
  const size = (entries: Map<string, Buffer>) => [...entries.values()].reduce((sum, bytes) => sum + bytes.length, 0);
  expect(size(compressed)).toBeLessThan(size(plain) * (browserName === 'chromium' ? 0.5 : 0.7));

  const types = new Set<string>();
  const kept: string[] = [];
  for (const [sha, { target }] of Object.entries(PLANNED)) {
    const found = [...compressed.keys()].filter((name) => name.startsWith(`media/${sha}.`));
    expect(found, sha).toHaveLength(1);
    const name = found[0]!;
    const bytes = compressed.get(name)!;
    const original = [...plain.entries()].find(([path]) => path.startsWith(`media/${sha}.`))!;
    if (name === original[0] && bytes.equals(original[1])) {
      kept.push(name);
      continue;
    }
    const header = imageHeader(bytes);
    expect(header, name).not.toBeNull();
    expect([header!.width, header!.height], name).toEqual(target);
    expect(name).toBe(`media/${sha}.${EXTENSIONS[header!.type]}`);
    types.add(header!.type);
    expect(bytes.length, sha).toBeLessThan(original[1].length);
  }
  // The opaque PNGs and the turned photo are re-encoded in every browser.
  for (const sha of [
    '0b8ef0814ac38d387c5debad6b89fe19f6d6221fb7b710cb0453251f7216af34',
    'd5f35262f0b718fe76419b2d0b71b5fe864d1a0ad61c4ea085d8b63fd33162bf',
    '6bc569313d42f3376e6906bdc32eec605820e233e86f12320325eb08143cb341',
  ]) {
    expect(
      kept.some((name) => name.includes(sha)),
      sha,
    ).toBe(false);
  }
  if (browserName === 'chromium') {
    expect([...types]).toEqual(['image/webp']);
    expect(kept).toEqual([]);
  }
  // The animated GIF and the SVG are the original files.
  for (const [name, bytes] of plain) {
    if (/\.(gif|svg)$/.test(name)) {
      expect(compressed.get(name)?.equals(bytes), name).toBe(true);
    }
  }
  const index = compressed.get('index.html')!.toString('utf8');
  for (const sha of Object.keys(PLANNED)) {
    const original = [...plain.keys()].find((name) => name.startsWith(`media/${sha}.`))!;
    const renamed = [...compressed.keys()].find((name) => name.startsWith(`media/${sha}.`))!;
    if (original !== renamed) {
      expect(index.includes(original), original).toBe(false);
    }
    expect(index.includes(renamed), renamed).toBe(true);
  }

  // Opened from file://, both decks draw every element in the same box, and the slide alike.
  const before = await drawn(browser, plainZip, join(OUT, 'images', browserName, 'plain'));
  const after = await drawn(browser, compressedZip, join(OUT, 'images', browserName, 'compressed'));
  expect(after.boxes).toEqual(before.boxes);
  expect(before.boxes.length).toBeGreaterThan(5);
  const alike = ssim(before.shot, after.shot);
  expect(alike, `SSIM ${alike}`).toBeGreaterThan(0.97);
  expect([...before.problems, ...after.problems]).toEqual([]);

  // The choice is remembered.
  await page.reload();
  await expect(page.getByTestId('wasm-status')).toHaveText(/^slidock-wasm \S+$/, { timeout: 60_000 });
  await page.locator('input[type="file"]').setInputFiles(DECK);
  await expect(page.getByRole('toolbar', { name: 'Deck' })).toBeVisible({ timeout: 120_000 });
  await page.getByRole('button', { name: 'Download zip' }).click();
  await expect(toggle).toBeChecked();

  // A single file inlines the re-encoded pictures: no media file, each one a data: URI of its new type.
  await dialog.getByRole('checkbox', { name: /Single HTML file/ }).check();
  await dialog.getByRole('radio', { name: /Small/ }).check();
  const singleZip = await download(page, 'single.zip', browserName);
  const single = zipEntries(singleZip);
  expect([...single.keys()]).toEqual(['index.html', 'LICENSES.txt']);
  const inlined = single.get('index.html')!.toString('utf8');
  const uris = [...inlined.matchAll(/data:(image\/[a-z+]+);base64,/g)].map((match) => match[1]);
  // The photo is drawn twice; the GIF and the SVG keep their types.
  expect(uris.filter((type) => type === 'image/gif' || type === 'image/svg+xml')).toHaveLength(2);
  expect(uris.length).toBe(Object.keys(PLANNED).length + 1 + 2);
  if (browserName === 'chromium') {
    expect(uris.filter((type) => type === 'image/webp')).toHaveLength(Object.keys(PLANNED).length + 1);
  }
  const singleDrawn = await drawn(browser, singleZip, join(OUT, 'images', browserName, 'single'));
  expect(singleDrawn.boxes).toEqual(before.boxes);
  expect(ssim(before.shot, singleDrawn.shot)).toBeGreaterThan(0.95);
  expect(problems).toEqual([]);
});
