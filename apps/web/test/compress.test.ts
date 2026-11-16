import type { PlannedImage } from '@slidock/types';
import { describe, expect, it } from 'vitest';

import {
  type BitmapLike,
  type CanvasLike,
  type CompressEnvironment,
  type ContextLike,
  type DecodeOptions,
  ENCODER_QUALITY,
  appliesOrientation,
  compressImage,
  scaleTo,
} from '../src/media/compress';
import {
  type CompressPort,
  type CompressReply,
  type CompressRequest,
  type Timers,
  workerCount,
  workerImageCompressor,
} from '../src/media/compressor';

class FakeBitmap implements BitmapLike {
  closed = false;
  constructor(
    readonly width: number,
    readonly height: number,
  ) {}
  close() {
    this.closed = true;
  }
}

interface Browser {
  /** Encodes WebP (else its canvas returns a PNG for it, as Safari's does). */
  webp?: boolean;
  /** Applies EXIF orientation: the probe decodes 1 × 2. */
  turns?: boolean;
  /** Honours `resizeWidth`/`resizeHeight`. */
  resizes?: boolean;
  /** The alpha of every pixel the canvases read. */
  alpha?: number;
  /** The size of each encoded file, by type. */
  sizes?: Partial<Record<string, number>>;
  /** The size a whole decode has, when not the plan's. */
  decodes?: [number, number];
}

function fakeBrowser(options: Browser = {}) {
  const decodes: { options: DecodeOptions | undefined; probe: boolean }[] = [];
  const draws: { from: [number, number]; to: [number, number] }[] = [];
  const encodes: { type: string; quality: number | undefined; size: [number, number] }[] = [];
  const canvases: CanvasLike[] = [];
  let plan: PlannedImage | null = null;
  const env: CompressEnvironment = {
    decode: (file, decodeOptions) => {
      const probe = file.type === 'image/jpeg' && file.size === 321;
      decodes.push({ options: decodeOptions, probe });
      if (probe) {
        return Promise.resolve(options.turns === false ? new FakeBitmap(2, 1) : new FakeBitmap(1, 2));
      }
      if (decodeOptions?.resizeWidth !== undefined && options.resizes !== false) {
        return Promise.resolve(new FakeBitmap(decodeOptions.resizeWidth, decodeOptions.resizeHeight!));
      }
      const [w, h] = options.decodes ?? [plan!.width, plan!.height];
      return Promise.resolve(new FakeBitmap(w, h));
    },
    canvas: (width, height) => {
      const canvas: CanvasLike = {
        width,
        height,
        getContext: () => {
          const context: ContextLike = {
            imageSmoothingEnabled: false,
            imageSmoothingQuality: 'low',
            drawImage: (image, _x, _y, w, h) => draws.push({ from: [image.width, image.height], to: [w, h] }),
            getImageData: (_x, _y, w, h) => ({
              data: Uint8ClampedArray.from({ length: w * h * 4 }, (_, i) => (i % 4 === 3 ? (options.alpha ?? 255) : 0)),
            }),
          };
          return context;
        },
        convertToBlob: ({ type, quality }) => {
          const written = type === 'image/webp' && options.webp === false ? 'image/png' : type;
          encodes.push({ type: written, quality, size: [canvas.width, canvas.height] });
          return Promise.resolve(new Blob([new Uint8Array(options.sizes?.[written] ?? 10)], { type: written }));
        },
      };
      canvases.push(canvas);
      return canvas;
    },
    now: () => 0,
  };
  return {
    env,
    decodes,
    draws,
    encodes,
    canvases,
    job(planned: Partial<PlannedImage> = {}, quality = ENCODER_QUALITY.high) {
      plan = {
        path: 'media/a.png',
        mime: 'image/png',
        bytes: 1000,
        width: 400,
        height: 300,
        orientation: 1,
        targetWidth: 100,
        targetHeight: 75,
        ...planned,
      };
      return { plan, file: new Blob([new Uint8Array(1000)], { type: plan.mime }), quality };
    },
  };
}

describe('compressImage', () => {
  it('decodes at the planned size and encodes WebP at the chosen quality', async () => {
    const browser = fakeBrowser();
    const outcome = await compressImage(browser.job({}, ENCODER_QUALITY.medium), browser.env);
    expect(outcome).toMatchObject({ kind: 'compressed', mime: 'image/webp' });
    expect(browser.decodes).toEqual([{ options: { resizeWidth: 100, resizeHeight: 75 }, probe: false }]);
    expect(browser.encodes).toEqual([{ type: 'image/webp', quality: 0.8, size: [100, 75] }]);
    // Every canvas is released.
    expect(browser.canvases.every((canvas) => canvas.width === 0)).toBe(true);
  });

  it('scales on a canvas, halving at most per step, where the decode does not resize', async () => {
    const browser = fakeBrowser({ resizes: false });
    const outcome = await compressImage(browser.job(), browser.env);
    expect(outcome.kind).toBe('compressed');
    expect(browser.draws.map(({ to }) => to)).toEqual([
      [200, 150],
      [100, 75],
    ]);
  });

  it('writes JPEG for an opaque picture where the canvas encodes no WebP', async () => {
    const browser = fakeBrowser({ webp: false });
    const outcome = await compressImage(browser.job(), browser.env);
    expect(outcome).toMatchObject({ kind: 'compressed', mime: 'image/jpeg' });
    expect(browser.encodes.map(({ type }) => type)).toEqual(['image/png', 'image/jpeg']);
  });

  it('writes PNG for a scaled picture with transparency, and keeps one at its own size', async () => {
    const browser = fakeBrowser({ webp: false, alpha: 128 });
    expect(await compressImage(browser.job(), browser.env)).toMatchObject({ kind: 'compressed', mime: 'image/png' });
    const unscaled = fakeBrowser({ webp: false, alpha: 128 });
    const outcome = await compressImage(unscaled.job({ targetWidth: 400, targetHeight: 300 }), unscaled.env);
    expect(outcome).toMatchObject({ kind: 'kept' });
    expect(outcome.kind === 'kept' && outcome.reason).toMatch(/no WebP/);
  });

  it('keeps the original when the new file is not smaller', async () => {
    const browser = fakeBrowser({ sizes: { 'image/webp': 1000 } });
    const outcome = await compressImage(browser.job(), browser.env);
    expect(outcome).toMatchObject({ kind: 'kept' });
    expect(outcome.kind === 'kept' && outcome.reason).toMatch(/not smaller/);
    const smaller = fakeBrowser({ sizes: { 'image/webp': 999 } });
    expect((await compressImage(smaller.job(), smaller.env)).kind).toBe('compressed');
  });

  it('turns a picture by its EXIF orientation where the browser does, decoding it whole', async () => {
    const browser = fakeBrowser();
    const job = browser.job({
      mime: 'image/jpeg',
      orientation: 6,
      width: 300,
      height: 400,
      targetWidth: 75,
      targetHeight: 100,
    });
    const outcome = await compressImage(job, browser.env);
    expect(outcome).toMatchObject({ kind: 'compressed' });
    expect(browser.decodes.map(({ options, probe }) => [probe, options])).toEqual([
      [true, undefined],
      [false, undefined],
    ]);
    expect(browser.encodes.at(-1)?.size).toEqual([75, 100]);
  });

  it('keeps a turned picture where the browser ignores EXIF orientation, or decodes it at another size', async () => {
    const ignores = fakeBrowser({ turns: false });
    const job = ignores.job({
      mime: 'image/jpeg',
      orientation: 6,
      width: 300,
      height: 400,
      targetWidth: 75,
      targetHeight: 100,
    });
    const outcome = await compressImage(job, ignores.env);
    expect(outcome.kind === 'kept' && outcome.reason).toMatch(/EXIF orientation/);
    expect(ignores.encodes).toEqual([]);

    const sideways = fakeBrowser({ decodes: [400, 300] });
    const turned = sideways.job({ mime: 'image/jpeg', orientation: 6, width: 300, height: 400 });
    const kept = await compressImage(turned, sideways.env);
    expect(kept.kind === 'kept' && kept.reason).toMatch(/400 × 300, not the 300 × 400/);
  });

  it('asks the browser about EXIF orientation once', async () => {
    const browser = fakeBrowser();
    expect(await appliesOrientation(browser.env)).toBe(true);
    expect(await appliesOrientation(browser.env)).toBe(true);
    expect(browser.decodes.filter(({ probe }) => probe)).toHaveLength(1);
    expect(await appliesOrientation(fakeBrowser({ turns: false }).env)).toBe(false);
  });

  it('halves from a large picture to a small one without skipping a step', () => {
    const browser = fakeBrowser();
    const canvas = scaleTo(new FakeBitmap(1000, 10), 120, 1, browser.env);
    expect([canvas.width, canvas.height]).toEqual([120, 1]);
    expect(browser.draws.map(({ to }) => to)).toEqual([
      [500, 5],
      [250, 3],
      [125, 2],
      [120, 1],
    ]);
    // Each step but the last is released once the next one has drawn it.
    expect(browser.canvases.slice(0, -1).every((step) => step.width === 0)).toBe(true);
  });
});

class FakePort implements CompressPort {
  onmessage: ((event: MessageEvent<CompressReply>) => void) | null = null;
  onerror: ((event: ErrorEvent) => void) | null = null;
  sent: CompressRequest[] = [];
  terminated = false;
  postMessage(message: CompressRequest) {
    this.sent.push(message);
  }
  terminate() {
    this.terminated = true;
  }
  reply(index: number, bytes = 4) {
    const { id } = this.sent[index]!;
    this.onmessage?.({
      data: { id, outcome: { kind: 'compressed', bytes: new ArrayBuffer(bytes), mime: 'image/webp', elapsed: 1 } },
    } as MessageEvent<CompressReply>);
  }
}

function manualTimers() {
  const timers: { run: () => void; ms: number }[] = [];
  const api: Timers = {
    setTimeout: (run, ms) => {
      const timer = { run, ms };
      timers.push(timer);
      return timer;
    },
    clearTimeout: (timer) => {
      const index = timers.indexOf(timer as (typeof timers)[number]);
      if (index >= 0) {
        timers.splice(index, 1);
      }
    },
  };
  return { api, timers };
}

describe('workerImageCompressor', () => {
  const job = (path: string) => ({
    plan: {
      path,
      mime: 'image/png',
      bytes: 10,
      width: 2,
      height: 2,
      orientation: 1,
      targetWidth: 1,
      targetHeight: 1,
    },
    file: new Blob([]),
    quality: 0.9,
  });

  it('runs one picture per worker, at most the pool size at once, in the order asked', async () => {
    const ports: FakePort[] = [];
    const compressor = workerImageCompressor(() => {
      const port = new FakePort();
      ports.push(port);
      return port;
    }, 2);
    const outcomes = ['a', 'b', 'c'].map((path) => compressor.compress(job(path)));
    expect(ports).toHaveLength(2);
    expect(ports.map((port) => port.sent.map(({ job }) => job.plan.path))).toEqual([['a'], ['b']]);
    ports[1]!.reply(0);
    await outcomes[1];
    // The free worker takes the next picture.
    expect(ports[1]!.sent.map(({ job }) => job.plan.path)).toEqual(['b', 'c']);
    ports[0]!.reply(0);
    ports[1]!.reply(1);
    expect((await Promise.all(outcomes)).map(({ kind }) => kind)).toEqual(['compressed', 'compressed', 'compressed']);
  });

  it('keeps a picture that takes too long and stops its worker; dispose keeps the rest', async () => {
    const { api, timers } = manualTimers();
    const ports: FakePort[] = [];
    const compressor = workerImageCompressor(
      () => {
        const port = new FakePort();
        ports.push(port);
        return port;
      },
      1,
      1000,
      api,
    );
    const slow = compressor.compress(job('a'));
    const next = compressor.compress(job('b'));
    timers[0]!.run();
    expect(await slow).toMatchObject({ kind: 'kept', reason: 're-encoding took longer than 1 s' });
    expect(ports[0]!.terminated).toBe(true);
    // The next picture starts another worker.
    expect(ports).toHaveLength(2);
    const queued = compressor.compress(job('c'));
    compressor.dispose();
    expect(await next).toMatchObject({ kind: 'kept', reason: 'the picture workers were stopped' });
    expect(await queued).toMatchObject({ kind: 'kept' });
    expect(ports[1]!.terminated).toBe(true);
    expect(await compressor.compress(job('d'))).toMatchObject({ kind: 'kept' });
  });

  it('uses one worker fewer than the cores, at least one and at most four', () => {
    expect(workerCount(undefined)).toBe(1);
    expect(workerCount(1)).toBe(1);
    expect(workerCount(4)).toBe(3);
    expect(workerCount(16)).toBe(4);
  });
});
