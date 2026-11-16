// Re-encodes a deck's picture smaller for a download (an `imagePlan` entry), in the browser, off the main thread
// (`compress-worker.ts`). The compiler planned the size (`targetWidth` × `targetHeight`: the size the picture shows
// at, or smaller, never larger) and checks the file it is handed; this decodes, scales and encodes:
//
// - Decoding applies the file's EXIF orientation (`imageOrientation: 'from-image'`, as an `<img>` does), so the
//   pixels are stored as they show and the file needs no orientation of its own. A picture the plan says is turned
//   is only re-encoded where this browser turns a probe JPEG as its EXIF says, and its decoded size must be the size
//   it shows at; otherwise it is kept.
// - Scaling: the browser's own resize as it decodes (`resizeQuality: 'high'`), else a canvas, halving at most per step
//   so no source pixel is skipped.
// - Encoding: WebP at the chosen quality, which keeps transparency. A browser whose canvas does not encode WebP
//   (Safari: `convertToBlob` then returns a PNG, so the blob's type is checked) writes JPEG for an opaque picture and
//   PNG for one with transparency, the PNG only when it is scaled down.
// - The original is kept when the new file is not smaller.
//
// Colour: the canvas is sRGB and decoding converts into it, so a picture in another space (an iPhone's or a Mac
// display's Display P3) keeps its colours where sRGB has them and loses only the most saturated ones; the file is
// untagged, which browsers read as sRGB. The environment is injected so tests can fake the browser.
import type { PlannedImage } from '@slidock/types';

/** The quality choices of the download dialog. */
export type ImageQuality = 'high' | 'medium' | 'small';

export const IMAGE_QUALITIES: readonly ImageQuality[] = ['high', 'medium', 'small'];

/** The encoder quality of each choice (WebP and JPEG, 0–1). */
export const ENCODER_QUALITY: Readonly<Record<ImageQuality, number>> = { high: 0.9, medium: 0.8, small: 0.65 };

/** A picture to re-encode: the plan's entry, its file and the encoder quality. */
export interface CompressJob {
  plan: PlannedImage;
  file: Blob;
  quality: number;
}

/** What became of a picture. */
export type CompressOutcome =
  | {
      kind: 'compressed';
      bytes: ArrayBuffer;
      mime: 'image/webp' | 'image/jpeg' | 'image/png';
      /** In milliseconds, in the worker. */
      elapsed: number;
    }
  | { kind: 'kept'; reason: string; elapsed: number };

export interface BitmapLike {
  readonly width: number;
  readonly height: number;
  close(): void;
}

export interface ContextLike {
  imageSmoothingEnabled: boolean;
  imageSmoothingQuality: ImageSmoothingQuality;
  drawImage(image: BitmapLike | CanvasLike, dx: number, dy: number, dw: number, dh: number): void;
  getImageData(sx: number, sy: number, sw: number, sh: number): { data: ArrayLike<number> };
}

export interface CanvasLike {
  /** Set to 0 to release the canvas's pixels. */
  width: number;
  height: number;
  getContext(kind: '2d'): ContextLike | null;
  convertToBlob(options: { type: string; quality?: number }): Promise<Blob>;
}

/** How a decode is asked for: the browser's resize, when given. */
export interface DecodeOptions {
  resizeWidth?: number;
  resizeHeight?: number;
}

/** The browser a re-encode runs in: a worker's globals, or fakes. */
export interface CompressEnvironment {
  /** `createImageBitmap(blob, { imageOrientation: 'from-image', ... })`. */
  decode(file: Blob, options?: DecodeOptions): Promise<BitmapLike>;
  canvas(width: number, height: number): CanvasLike;
  now(): number;
}

/** The environment of a worker (or a page) of this browser. */
export function browserCompressEnvironment(scope: typeof globalThis = globalThis): CompressEnvironment {
  return {
    decode: (file, options = {}) =>
      scope.createImageBitmap(file, {
        imageOrientation: 'from-image',
        ...(options.resizeWidth !== undefined
          ? { resizeWidth: options.resizeWidth, resizeHeight: options.resizeHeight, resizeQuality: 'high' as const }
          : {}),
      }),
    canvas: (width, height) => new OffscreenCanvas(width, height) as unknown as CanvasLike,
    now: () => performance.now(),
  };
}

/** A 2 × 1 JPEG whose EXIF orientation (6) turns it a quarter: a browser that applies it decodes 1 × 2. */
const TURNED_PROBE =
  '/9j/4QAiRXhpZgAATU0AKgAAAAgAAQESAAMAAAABAAYAAAAAAAD/4AAQSkZJRgABAQAAAQABAAD/2wBDABALDA4MChAODQ4SERATGCgaGBYWGDEjJR0oOjM9PDkzODdASFxOQERXRTc4UG1RV19iZ2hnPk1xeXBkeFxlZ2P/2wBDARESEhgVGC8aGi9jQjhCY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2P/wAARCAABAAIDAREAAhEBAxEB/8QAFAABAAAAAAAAAAAAAAAAAAAABf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/xAAVAQEBAAAAAAAAAAAAAAAAAAAFBv/EABQRAQAAAAAAAAAAAAAAAAAAAAD/2gAMAwEAAhEDEQA/ABRa8f/Z';

function probeFile(): Blob {
  const bytes = Uint8Array.from(atob(TURNED_PROBE), (char) => char.charCodeAt(0));
  return new Blob([bytes], { type: 'image/jpeg' });
}

/** Whether this browser's decode turns a JPEG as its EXIF orientation says (asked once per environment). */
const turns = new WeakMap<CompressEnvironment, Promise<boolean>>();

export function appliesOrientation(env: CompressEnvironment): Promise<boolean> {
  let known = turns.get(env);
  if (!known) {
    known = env.decode(probeFile()).then(
      (bitmap) => {
        const turned = bitmap.width === 1 && bitmap.height === 2;
        bitmap.close();
        return turned;
      },
      () => false,
    );
    turns.set(env, known);
  }
  return known;
}

function context(canvas: CanvasLike): ContextLike {
  const context = canvas.getContext('2d');
  if (!context) {
    throw new Error('the browser gave no 2D canvas');
  }
  context.imageSmoothingEnabled = true;
  context.imageSmoothingQuality = 'high';
  return context;
}

/**
 * `source` drawn on a canvas of `width` × `height`, halving at most per step: one draw that skips source pixels
 * aliases. Each intermediate canvas is released once the next step has drawn it.
 */
export function scaleTo(source: BitmapLike, width: number, height: number, env: CompressEnvironment): CanvasLike {
  let current: BitmapLike | CanvasLike = source;
  let [w, h] = [source.width, source.height];
  for (;;) {
    const next: [number, number] = [Math.max(width, Math.ceil(w / 2)), Math.max(height, Math.ceil(h / 2))];
    const canvas = env.canvas(next[0], next[1]);
    context(canvas).drawImage(current, 0, 0, next[0], next[1]);
    if (current !== source) {
      (current as CanvasLike).width = 0;
    }
    current = canvas;
    [w, h] = next;
    if (w === width && h === height) {
      return canvas;
    }
  }
}

/** Whether every pixel of `canvas` is opaque. */
function opaque(canvas: CanvasLike): boolean {
  const { data } = context(canvas).getImageData(0, 0, canvas.width, canvas.height);
  for (let at = 3; at < data.length; at += 4) {
    if (data[at] !== 255) {
      return false;
    }
  }
  return true;
}

/** The picture's encode: WebP, else JPEG when opaque, else PNG when scaled down; `null` when none fits. */
async function encode(canvas: CanvasLike, job: CompressJob, scaled: boolean): Promise<Blob | null> {
  const webp = await canvas.convertToBlob({ type: 'image/webp', quality: job.quality });
  if (webp.type === 'image/webp') {
    return webp;
  }
  if (job.plan.mime === 'image/jpeg' || opaque(canvas)) {
    const jpeg = await canvas.convertToBlob({ type: 'image/jpeg', quality: job.quality });
    return jpeg.type === 'image/jpeg' ? jpeg : null;
  }
  if (!scaled) {
    return null;
  }
  const png = await canvas.convertToBlob({ type: 'image/png' });
  return png.type === 'image/png' ? png : null;
}

/** The picture at its planned size, decoded as it shows. */
async function decoded(job: CompressJob, env: CompressEnvironment): Promise<BitmapLike | string> {
  const { plan, file } = job;
  const target = { resizeWidth: plan.targetWidth, resizeHeight: plan.targetHeight };
  if (plan.orientation === 1 && (plan.targetWidth !== plan.width || plan.targetHeight !== plan.height)) {
    const resized = await env.decode(file, target).catch(() => null);
    if (resized && resized.width === plan.targetWidth && resized.height === plan.targetHeight) {
      return resized;
    }
    resized?.close();
  }
  if (plan.orientation !== 1 && !(await appliesOrientation(env))) {
    return `this browser does not turn a picture by its EXIF orientation (${plan.orientation})`;
  }
  const whole = await env.decode(file);
  if (whole.width !== plan.width || whole.height !== plan.height) {
    whole.close();
    return `decoded at ${whole.width} × ${whole.height}, not the ${plan.width} × ${plan.height} it shows at`;
  }
  return whole;
}

/** Re-encodes `job`'s picture at its planned size, or says why the original stays. */
export async function compressImage(job: CompressJob, env: CompressEnvironment): Promise<CompressOutcome> {
  const start = env.now();
  const kept = (reason: string): CompressOutcome => ({ kind: 'kept', reason, elapsed: env.now() - start });
  const { plan } = job;
  let bitmap: BitmapLike | string;
  try {
    bitmap = await decoded(job, env);
  } catch (error) {
    return kept(`the browser cannot decode it (${String(error)})`);
  }
  if (typeof bitmap === 'string') {
    return kept(bitmap);
  }
  let canvas: CanvasLike;
  try {
    canvas =
      bitmap.width === plan.targetWidth && bitmap.height === plan.targetHeight
        ? scaleOnce(bitmap, env)
        : scaleTo(bitmap, plan.targetWidth, plan.targetHeight, env);
  } finally {
    bitmap.close();
  }
  try {
    const scaled = plan.targetWidth !== plan.width || plan.targetHeight !== plan.height;
    const file = await encode(canvas, job, scaled);
    if (file === null) {
      return kept('this browser encodes no WebP, and the picture has transparency at its own size');
    }
    if (file.size >= plan.bytes) {
      return kept(`the new file (${file.size} bytes) is not smaller than the original (${plan.bytes})`);
    }
    return {
      kind: 'compressed',
      bytes: await file.arrayBuffer(),
      mime: file.type as 'image/webp' | 'image/jpeg' | 'image/png',
      elapsed: env.now() - start,
    };
  } finally {
    canvas.width = 0;
  }
}

/** A bitmap already at its size, on a canvas of its own to encode. */
function scaleOnce(bitmap: BitmapLike, env: CompressEnvironment): CanvasLike {
  const canvas = env.canvas(bitmap.width, bitmap.height);
  context(canvas).drawImage(bitmap, 0, 0, bitmap.width, bitmap.height);
  return canvas;
}
