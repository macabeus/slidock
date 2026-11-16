// The 3D effects drawn on screen. WebKit's screenshot paints a projective transform flat (`a projective transform in
// a screenshot`, effects.spec.ts), so the effect suite measures those middle frames there by the quad the target's
// computed transform projects to. This suite checks that the page really draws that quad: it holds each 3D effect at
// its measured middle frame while the browser records the page (its compositor's frames), and the held frame's
// pixels of the target must cover the projected quad, and cover it better than the same quad without perspective by
// at least half of what a perfect render would (the two quads differ less where the turn is shallow). The effect
// runs on screen before it is held (`pauseRunning`): WebKit's compositor draws an effect's own easing as linear, so a
// curve the player left on the effect would hold a frame other than the one its computed style describes. The
// recording is not always the viewport pixel for pixel (macOS WebKit's frames hold it scaled down by about 2 %, with a
// grey margin): two markers in opposite corners of the viewport place the quads in the frame. A recording whose
// frame shows no markers (macOS WebKit's recorder sometimes captures a viewport offset from the page) says nothing
// about the drawing: it is made again, at most twice, and each retry is reported.
import { type Browser, type Page, expect, test } from '@playwright/test';
import type { BuildEffect } from '@slidock/types';
import { readFileSync } from 'node:fs';

import { ENTRANCES, PROJECTIVE, TRANSITIONS } from './catalog';
import { HEIGHT, WIDTH, deckSlide, fixture, step, transition } from './support/deck';
import { type Harness, type Quads, open, pauseRunning } from './support/harness';

/** The fixture's box (variant a) and the incoming slide's background (variant b). */
const BOX = [37, 99, 235] as const;
const INCOMING_BG = [253, 230, 138] as const;
/** Per channel: the video's compression moves colours this far. */
const TOLERANCE = 48;
/** The least intersection over union with the projected quad. */
const FIT = 0.8;
/** How long the held frame is recorded, ms. */
const HOLD_MS = 1500;
/** The corner markers' colour, size (px) and tolerance per channel: a colour far from any the fixtures paint. */
const MARKER = [255, 0, 255] as const;
const MARK = 8;
const MARKER_TOLERANCE = 96;
/** Recordings one test makes at most. */
const RECORDINGS = 3;

type Box = readonly [number, number, number, number];

interface Case {
  readonly title: string;
  readonly data: ReturnType<typeof fixture>;
  /** The target's `data-el`, or null for the incoming layer. */
  readonly target: string | null;
  readonly fraction: number;
  readonly color: readonly [number, number, number];
}

function fields(effect: BuildEffect): Record<string, unknown> {
  return 'params' in effect ? { effect: effect.effect, params: effect.params } : { effect: effect.effect };
}

function cases(): Case[] {
  const out: Case[] = [];
  for (const c of ENTRANCES) {
    for (const kind of ['entrance', 'exit'] as const) {
      const title = `build ${kind} ${c.name}`;
      if (PROJECTIVE.includes(title)) {
        out.push({
          title,
          data: fixture([
            deckSlide('s1', { timeline: [step('s1.box', { kind, fallback: 'dissolve', ...fields(c.effect) })] }),
          ]),
          target: 's1.box',
          fraction: c.mid?.at ?? c.middle ?? 0.5,
          color: BOX,
        });
      }
    }
  }
  for (const c of TRANSITIONS) {
    const title = `transition ${c.name}`;
    if (PROJECTIVE.includes(title)) {
      out.push({
        title,
        data: fixture([
          deckSlide('s1'),
          deckSlide('s2', { transition: transition({ fallback: 'dissolve', ...fields(c.effect as BuildEffect) }) }),
        ]),
        target: null,
        fraction: c.mid?.at ?? c.middle ?? 0.5,
        color: INCOMING_BG,
      });
    }
  }
  return out;
}

/** Records the page while `c` holds its middle frame; returns the quads there and the video. */
async function record(browser: Browser, c: Case, dir: string): Promise<{ quads: Quads | null; video: Buffer }> {
  const size = { width: WIDTH, height: HEIGHT };
  const context = await browser.newContext({ viewport: size, deviceScaleFactor: 1, recordVideo: { dir, size } });
  const page = await context.newPage();
  await open(page, c.data);
  await page.evaluate(
    ([size, color]) => {
      for (const corner of ['left:0;top:0', 'right:0;bottom:0']) {
        const marker = document.createElement('div');
        marker.style.cssText = `position:fixed;${corner};width:${size}px;height:${size}px;z-index:99;background:rgb(${color})`;
        document.body.append(marker);
      }
    },
    [MARK, MARKER.join(',')] as const,
  );
  await page.evaluate(() => (window as unknown as Harness).sd.player.next());
  await pauseRunning(page);
  await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), c.fraction);
  const quads = await page.evaluate((id) => (window as unknown as Harness).sd.projected(id), c.target);
  await page.waitForTimeout(HOLD_MS);
  const video = page.video();
  expect(video).not.toBeNull();
  await context.close();
  // Saved rather than read in place: a browser on a remote server (Firefox) records there.
  const path = `${dir}/held.webm`;
  await video?.saveAs(path);
  return { quads, video: readFileSync(path) };
}

/**
 * Where the viewport's corner markers sit in a frame: the centroids of the marker's colour in the frame's top-left and
 * bottom-right quarters.
 */
async function markers(page: Page, frame: string): Promise<[[number, number], [number, number]]> {
  return page.evaluate(
    async ([b64, color, tolerance]) => {
      const image = await createImageBitmap(await (await fetch('data:image/png;base64,' + b64)).blob());
      const { width, height } = image;
      const ctx = new OffscreenCanvas(width, height).getContext('2d', { willReadFrequently: true });
      if (ctx === null) {
        throw new Error('no 2D context');
      }
      ctx.drawImage(image, 0, 0);
      const px = ctx.getImageData(0, 0, width, height).data;
      const sums = [
        [0, 0, 0],
        [0, 0, 0],
      ];
      for (let y = 0; y < height; y++) {
        for (let x = 0; x < width; x++) {
          const i = (y * width + x) * 4;
          const near = [0, 1, 2].every((k) => Math.abs((px[i + k] ?? 0) - (color[k] ?? 0)) <= tolerance);
          const quarter = x < width / 2 && y < height / 2 ? 0 : x >= width / 2 && y >= height / 2 ? 1 : -1;
          const sum = sums[quarter];
          if (near && sum !== undefined) {
            sum[0] = (sum[0] ?? 0) + x;
            sum[1] = (sum[1] ?? 0) + y;
            sum[2] = (sum[2] ?? 0) + 1;
          }
        }
      }
      return sums.map(([sx = 0, sy = 0, n = 0]) => (n === 0 ? [NaN, NaN] : [sx / n + 0.5, sy / n + 0.5])) as [
        [number, number],
        [number, number],
      ];
    },
    [frame, MARKER, MARKER_TOLERANCE] as const,
  );
}

/** `quads` (viewport px) placed in a frame whose corner markers sit at `at`, and the viewport's box there. */
function placed(quads: Quads, at: [[number, number], [number, number]]): { quads: Quads; view: Box } {
  const [[x0, y0], [x1, y1]] = at;
  const half = MARK / 2;
  const sx = (x1 - x0) / (WIDTH - MARK);
  const sy = (y1 - y0) / (HEIGHT - MARK);
  const point = (x: number, y: number) => [x0 + (x - half) * sx, y0 + (y - half) * sy] as const;
  const map = (quad: Quads['now']) => quad.map(([x, y]) => point(x, y));
  return {
    quads: { rest: map(quads.rest), now: map(quads.now), flat: map(quads.flat) },
    view: [...point(0, 0), ...point(WIDTH, HEIGHT)],
  };
}

/** The video's last frame, as a PNG (base64), decoded by the browser. */
async function lastFrame(page: Page, video: Buffer): Promise<string> {
  return page.evaluate(async (b64) => {
    const element = document.createElement('video');
    element.muted = true;
    const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    element.src = URL.createObjectURL(new Blob([bytes], { type: 'video/webm' }));
    await new Promise((resolve, reject) => {
      element.onloadeddata = resolve;
      element.onerror = () => reject(new Error(`video: ${element.error?.code}`));
    });
    const end = Number.isFinite(element.duration) ? Math.max(0, element.duration - 0.2) : 1e6;
    await new Promise((resolve) => {
      element.onseeked = resolve;
      element.currentTime = end;
    });
    const canvas = document.createElement('canvas');
    canvas.width = element.videoWidth;
    canvas.height = element.videoHeight;
    canvas.getContext('2d')?.drawImage(element, 0, 0);
    return canvas.toDataURL('image/png').slice('data:image/png;base64,'.length);
  }, video.toString('base64'));
}

for (const c of cases()) {
  test(`${c.title} draws in perspective on screen`, async ({ browser, page }, testInfo) => {
    await open(page, c.data);
    let held: { quads: Quads; frame: string; corners: [[number, number], [number, number]] } | null = null;
    for (let n = 1; n <= RECORDINGS && held === null; n++) {
      const { quads, video } = await record(browser, c, testInfo.outputPath(`video-${n}`));
      expect(quads, 'projective at its middle frame').not.toBeNull();
      const frame = await lastFrame(page, video);
      await testInfo.attach(`held frame ${n}`, { body: Buffer.from(frame, 'base64'), contentType: 'image/png' });
      const corners = await markers(page, frame);
      if (corners.flat().every(Number.isFinite)) {
        held = { quads: quads as Quads, frame, corners };
      } else {
        const description = `recording ${n} of ${RECORDINGS} shows no corner markers (${JSON.stringify(corners)})`;
        testInfo.annotations.push({ type: 're-recorded', description });
        console.warn(`${c.title} [${testInfo.project.name}]: ${description}`);
      }
    }
    expect(held, `the corner markers show in one of ${RECORDINGS} recordings`).not.toBeNull();
    const { quads, frame, corners } = held as NonNullable<typeof held>;
    const { quads: there, view } = placed(quads, corners);
    const fit = await page.evaluate(
      ([f, q, color, tolerance, box]) => (window as unknown as Harness).sd.fit(f, q, color, tolerance, box),
      [frame, there, c.color, TOLERANCE, view] as const,
    );
    const at = JSON.stringify({ ...fit, corners });
    expect(fit.now, `covers the projected quad: ${at}`).toBeGreaterThanOrEqual(FIT);
    expect(fit.now - fit.flat, `fits it better than the flat quad: ${at}`).toBeGreaterThanOrEqual(
      ((1 - fit.quads) * fit.now) / 2,
    );
  });
}
