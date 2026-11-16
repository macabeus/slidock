// Videos in the suite: what a movie file holds (read with the app's own demuxer), and the frames a downloaded deck's
// videos show when a browser plays them.
import type { Browser } from '@playwright/test';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import { type Movie, readMovie } from '../../src/media/movie';
import { unpack } from './images';
import { CURRENT, watch } from './player';
import { decode } from './png';

/** A movie file's top-level boxes in order, and its tracks. */
export async function movieOf(bytes: Buffer): Promise<{ boxes: string[]; movie: Movie }> {
  const boxes: string[] = [];
  for (let at = 0; at + 8 <= bytes.length;) {
    let size = bytes.readUInt32BE(at);
    if (size === 1) {
      size = Number(bytes.readBigUInt64BE(at + 8));
    } else if (size === 0) {
      size = bytes.length - at;
    }
    boxes.push(bytes.toString('latin1', at + 4, at + 8));
    at += Math.max(8, size);
  }
  return { boxes, movie: await readMovie(new Blob([new Uint8Array(bytes)])) };
}

/** One video's frames at the times asked (screenshots of its element), its length, and whether it played on. */
export interface Frames {
  duration: number;
  frames: Buffer[];
  plays: boolean;
  /** Why it would not load (this browser decodes no HEVC, say); its frames are then none. */
  error: string | null;
}

/**
 * The deck of `zip`, unpacked into `dir` and opened from file://: each video of its first slide by the file it plays
 * (`media/<sha256>`, its extension dropped), loaded, played a moment, then sought to each time and its element
 * captured as the page shows it.
 */
export async function videoFrames(
  browser: Browser,
  zip: string,
  dir: string,
  times: number[],
): Promise<{ videos: Map<string, Frames>; problems: string[] }> {
  unpack(zip, dir);
  const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
  const problems = watch(page);
  await page.goto(pathToFileURL(join(dir, 'index.html')).href);
  const layer = page.locator(CURRENT);
  await layer.waitFor();
  const videos = layer.locator('video');
  const out = new Map<string, Frames>();
  for (let index = 0; index < (await videos.count()); index++) {
    const video = videos.nth(index);
    const facts = await video.evaluate(async (element: HTMLVideoElement) => {
      const src = element.getAttribute('src') ?? '';
      element.muted = true;
      element.preload = 'auto';
      element.load();
      const loaded = await new Promise<string | null>((resolve) => {
        element.addEventListener('loadeddata', () => resolve(null), { once: true });
        element.addEventListener('error', () => resolve(`error ${element.error?.code}: ${element.error?.message}`), {
          once: true,
        });
      });
      if (loaded !== null) {
        return { src, duration: NaN, plays: false, error: loaded };
      }
      await element.play();
      await new Promise((resolve) => setTimeout(resolve, 300));
      const plays = element.currentTime > 0;
      element.pause();
      return { src, duration: element.duration, plays, error: null };
    });
    const frames: Buffer[] = [];
    for (const time of facts.error === null ? times : []) {
      await video.evaluate(async (element: HTMLVideoElement, at) => {
        element.currentTime = at;
        await new Promise((resolve) => element.addEventListener('seeked', resolve, { once: true }));
        await new Promise((resolve) => setTimeout(resolve, 100));
      }, time);
      frames.push(await video.screenshot());
    }
    out.set(facts.src.replace(/\.[a-z0-9]+$/, ''), {
      duration: facts.duration,
      frames,
      plays: facts.plays,
      error: facts.error,
    });
  }
  await page.close();
  return { videos: out, problems };
}

/** The mean absolute difference of two screenshots' colour channels. */
export function meanDifference(a: Buffer, b: Buffer): number {
  const [x, y] = [decode(a), decode(b)];
  let total = 0;
  for (let i = 0; i < x.rgb.length; i++) {
    total += Math.abs((x.rgb[i] ?? 0) - (y.rgb[i] ?? 0));
  }
  return total / x.rgb.length;
}
