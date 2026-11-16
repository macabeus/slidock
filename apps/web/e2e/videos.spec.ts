// The download's video compression on the video probe (`probes/out/video-probe.pptx`, `cargo xtask probes-videos`):
// the zip with "Compress videos" on is smaller; each movie the compiler plans is an H.264 MP4 of its planned size,
// its movie boxes first, with an AAC sound track where the original has sound (the AAC one carried over, the PCM one
// re-encoded) and none where it has none or never plays it (a volume of 0); the movie muted in its every use, already
// under the bitrate, is its video copied as it is without its sound; the clip with sound, muted in one use and heard
// in the other, keeps it; the movie already small and the one with alpha stay as they are; and the deck, unpacked and
// opened from file://, plays each video, its frames alike those of the uncompressed download at the same times. The
// choice is remembered across a reload.
import { type Page, expect, test } from '@playwright/test';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { GOOGLE_OFF, OUT, REPO } from './support/decks';
import { ssim, zipEntries } from './support/images';
import { watch } from './support/player';
import { meanDifference, movieOf, videoFrames } from './support/videos';

const DECK = join(REPO, 'probes/out/video-probe.pptx');

function sha(name: string): string {
  return createHash('sha256')
    .update(readFileSync(join(REPO, 'probes/videos/media', name)))
    .digest('hex');
}

/** The re-encoded movies: their planned size and whether they have sound. */
const PLANNED: Record<string, { name: string; target: [number, number]; sound: boolean }> = {
  [sha('sound.mp4')]: { name: 'sound.mp4', target: [960, 540], sound: true },
  // Stored 960 × 540 under a quarter turn: re-encoded upright.
  [sha('turned.mp4')]: { name: 'turned.mp4', target: [360, 640], sound: false },
  // QuickTime with PCM sound, drawn twice: the larger use.
  [sha('pcm.mov')]: { name: 'pcm.mov', target: [768, 432], sound: true },
  // Drawn larger than it is: its own size, its frames encoded as decoded.
  [sha('own.mp4')]: { name: 'own.mp4', target: [640, 360], sound: false },
  // At a volume of 0: its sound left out.
  [sha('quiet.mp4')]: { name: 'quiet.mp4', target: [768, 432], sound: false },
};

/** Muted in its every use and already under the bitrate: its video copied as it is, without its sound. */
const COPIED = sha('muted.mp4');

/** The movies that stay as they are: already small, and HEVC with alpha. */
const KEPT = [sha('tiny.mp4'), sha('alpha.mov')];

/** The frames compared, in seconds. */
const TIMES = [0.3, 0.9, 1.5];

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
  const saved = page.waitForEvent('download', { timeout: 240_000 });
  await dialog.getByRole('button', { name: 'Build and save' }).click();
  const path = join(OUT, 'videos', browserName, name);
  await (await saved).saveAs(path);
  await expect(dialog.getByRole('button', { name: 'Save again' })).toBeVisible();
  return path;
}

test('videos: a compressed download is smaller, re-encodes each planned movie to H.264 at its size, and plays alike', async ({
  page,
  browser,
  browserName,
}) => {
  test.setTimeout(420_000);
  const problems = watch(page);
  await openDeck(page);
  await page.getByRole('button', { name: 'Download zip' }).click();
  const dialog = page.getByRole('dialog', { name: 'Download zip' });
  const toggle = dialog.getByRole('switch', { name: 'Compress videos' });
  await expect(toggle).not.toBeChecked();
  const plainZip = await download(page, 'plain.zip', browserName);

  await toggle.check();
  await expect(dialog.getByRole('radio', { name: /High/ }).last()).toBeChecked();
  const compressedZip = await download(page, 'compressed.zip', browserName);
  await expect(dialog.getByRole('heading', { name: 'Compressed videos' })).toBeVisible();
  await expect(
    dialog.getByText(/5 of 8 videos re-encoded, 1 with their sound re-encoded to AAC, 1 copied as it is\./),
  ).toBeVisible();
  await expect(dialog.getByText(/^Sound removed from 2 videos muted in every use: [\d.]+ KB saved\.$/)).toBeVisible();

  const plain = zipEntries(plainZip);
  const compressed = zipEntries(compressedZip);
  const size = (entries: Map<string, Buffer>) => [...entries.values()].reduce((sum, bytes) => sum + bytes.length, 0);
  expect(size(compressed)).toBeLessThan(size(plain) * 0.6);

  for (const [hash, { name, target, sound }] of Object.entries(PLANNED)) {
    const bytes = compressed.get(`media/${hash}.mp4`);
    expect(bytes, name).toBeDefined();
    const original = [...plain.entries()].find(([path]) => path.startsWith(`media/${hash}.`))!;
    expect(bytes!.length, name).toBeLessThan(original[1].length);
    if (original[0] !== `media/${hash}.mp4`) {
      expect(compressed.has(original[0]), name).toBe(false);
    }
    const { boxes, movie } = await movieOf(bytes!);
    // Fast start: the movie boxes before the media data.
    expect(boxes.slice(0, 3), name).toEqual(['ftyp', 'moov', 'mdat']);
    const videos = movie.tracks.filter((track) => track.handler === 'vide');
    expect(
      videos.map((track) => [track.format, track.visual?.width, track.visual?.height]),
      name,
    ).toEqual([['avc1', ...target]]);
    const sounds = movie.tracks.filter((track) => track.handler === 'soun');
    expect(
      sounds.map((track) => [track.format, track.audio?.objectType]),
      name,
    ).toEqual(sound ? [['mp4a', 0x40]] : []);
    expect(movie.tracks, name).toHaveLength(sound ? 2 : 1);
    // As long as the original (2 s, or 1.6 s for the PCM movie).
    expect(Math.abs(movie.duration / movie.timescale - (name === 'pcm.mov' ? 1.6 : 2)), name).toBeLessThan(0.1);
  }
  {
    const bytes = compressed.get(`media/${COPIED}.mp4`)!;
    const original = plain.get(`media/${COPIED}.mp4`)!;
    expect(bytes, 'muted.mp4').toBeDefined();
    expect(bytes.length).toBeLessThan(original.length);
    const { boxes, movie } = await movieOf(bytes);
    const { movie: source } = await movieOf(original);
    expect(boxes.slice(0, 3)).toEqual(['ftyp', 'moov', 'mdat']);
    expect(movie.tracks.map((track) => track.handler)).toEqual(['vide']);
    const [video] = movie.tracks;
    const from = source.tracks.find((track) => track.handler === 'vide')!;
    // Its description, frames and their bytes as the original's.
    expect(Buffer.from(video!.visual!.entry).equals(Buffer.from(from.visual!.entry))).toBe(true);
    expect(video!.samples.map(({ cts, sync, size }) => [cts, sync, size])).toEqual(
      from.samples.map(({ cts, sync, size }) => [cts, sync, size]),
    );
    // Its edit list, in each movie's timescale.
    const edits = (edits: typeof from.edits, timescale: number) =>
      edits.map(({ duration, mediaTime, rate }) => [duration / timescale, mediaTime, rate]);
    expect(edits(video!.edits, movie.timescale)).toEqual(edits(from.edits, source.timescale));
    for (const index of [0, 1, video!.samples.length - 1]) {
      const [a, b] = [video!.samples[index]!, from.samples[index]!];
      expect(bytes.subarray(a.offset, a.offset + a.size).equals(original.subarray(b.offset, b.offset + b.size))).toBe(
        true,
      );
    }
    expect(Math.abs(movie.duration / movie.timescale - source.duration / source.timescale)).toBeLessThan(0.01);
  }
  for (const hash of KEPT) {
    const [path, bytes] = [...plain.entries()].find(([entry]) => entry.startsWith(`media/${hash}.`))!;
    expect(compressed.get(path)?.equals(bytes), path).toBe(true);
  }
  // The slides name the new files, and not the QuickTime original.
  const index = compressed.get('index.html')!.toString('utf8');
  for (const hash of [...Object.keys(PLANNED), COPIED]) {
    expect(index.includes(`media/${hash}.mp4`), hash).toBe(true);
  }
  expect(index.includes(`media/${sha('pcm.mov')}.mov`)).toBe(false);
  await dialog.getByText(/2 kept as they are/).click();
  await expect(dialog.getByText(/alpha channel/)).toBeVisible();

  // Opened from file://, each video loads, plays, and shows frames alike the uncompressed deck's.
  const before = await videoFrames(browser, plainZip, join(OUT, 'videos', browserName, 'plain'), TIMES);
  const after = await videoFrames(browser, compressedZip, join(OUT, 'videos', browserName, 'compressed'), TIMES);
  expect([...before.problems, ...after.problems]).toEqual([]);
  const names: Record<string, string> = {
    ...Object.fromEntries(Object.entries(PLANNED).map(([h, { name }]) => [h, name])),
    [COPIED]: 'muted.mp4',
  };
  for (const hash of Object.keys(names)) {
    const original = before.videos.get(`media/${hash}`);
    const encoded = after.videos.get(`media/${hash}`);
    expect(original && encoded, hash).toBeTruthy();
    expect([original!.error, encoded!.error], hash).toEqual([null, null]);
    expect(encoded!.plays, hash).toBe(true);
    expect(Math.abs(encoded!.duration - original!.duration), hash).toBeLessThan(0.1);
    TIMES.forEach((time, index) => {
      const alike = ssim(original!.frames[index]!, encoded!.frames[index]!);
      const difference = meanDifference(original!.frames[index]!, encoded!.frames[index]!);
      expect(alike, `${names[hash]} at ${time} s: SSIM ${alike}`).toBeGreaterThan(hash === COPIED ? 0.99 : 0.9);
      expect(difference, `${names[hash]} at ${time} s: mean difference ${difference}`).toBeLessThan(8);
    });
  }

  // The choice is remembered.
  await page.reload();
  await expect(page.getByTestId('wasm-status')).toHaveText(/^slidock-wasm \S+$/, { timeout: 60_000 });
  await page.locator('input[type="file"]').setInputFiles(DECK);
  await expect(page.getByRole('toolbar', { name: 'Deck' })).toBeVisible({ timeout: 120_000 });
  await page.getByRole('button', { name: 'Download zip' }).click();
  await expect(toggle).toBeChecked();
  expect(problems).toEqual([]);
});
