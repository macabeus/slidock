// Decodes a HEIC photo the compiler cannot draw (a `mediaNeeds` entry) into a JPEG of its plan's output size, in the
// browser, off the main thread (`decode-worker.ts`). Two paths, in order:
// - the browser's own image decoder (`createImageBitmap` of the file), which Safari has: it applies the file's
//   transforms itself (WebKit's does, probed with a `clap` alone, `imir` of either axis, `irot` of a half turn and of
//   a square image), so its bitmap is the plan's output, and one of another size is not taken;
// - WebCodecs (Chrome, and Safari): one `VideoDecoder` per HEVC configuration of the plan, one key chunk per tile,
//   each frame drawn at its place on a canvas of the coded size, then the plan's transforms (crop, quarter turns,
//   mirror) in their order.
// Neither: the photo stays omitted. A 12-megapixel canvas holds about 48 MB, so each one is released once the next
// step has drawn it, and the last once it is encoded. The environment is injected so tests can fake the browser.
//
// Colour: each tile's frames are labelled with the photo's colour space (its `nclx`, or the space its ICC profile
// matches: an iPhone's Display P3), so the browser converts them to the canvas's sRGB as it draws them. A decoder
// configured with a colour space takes it over what the coded pictures state, so a value neither the file's `nclx`
// nor its profile names is the one the pictures' own VUI states, and a default only where that is silent too. A
// profile of another space goes with the JPEG instead, the frames drawn as they are. The browser's own image decoder
// (ImageIO in Safari) converts with the profile itself, and is tried first: it matches what Keynote shows best.
import type { HeicPlan, HeicTransform, HevcDecoderConfig } from '@slidock/types';

/** How a photo was decoded. */
export type DecodePath = 'webcodecs' | 'native';

export interface DecodedPhoto {
  bytes: ArrayBuffer;
  mime: 'image/jpeg';
  width: number;
  height: number;
  path: DecodePath;
  /** In milliseconds. */
  elapsed: number;
}

/** Why a photo could not be decoded: each path's reason. */
export class Undecodable extends Error {
  readonly reasons: readonly string[];

  constructor(reasons: readonly string[]) {
    super(reasons.join('; '));
    this.name = 'Undecodable';
    this.reasons = reasons;
  }
}

/** The JPEG quality of a decoded photo. */
export const JPEG_QUALITY = 0.92;

/** A frame as the decoder hands it out (`VideoFrame`). */
export interface FrameLike {
  readonly timestamp: number;
  readonly displayWidth: number;
  readonly displayHeight: number;
  close(): void;
}

/** What drawing reads (a `VideoFrame`, an `ImageBitmap` or another canvas). */
export type Drawable = FrameLike | ImageLike | CanvasLike;

export interface ImageLike {
  readonly width: number;
  readonly height: number;
  close(): void;
}

export interface ContextLike {
  setTransform(a: number, b: number, c: number, d: number, e: number, f: number): void;
  drawImage(image: Drawable, dx: number, dy: number): void;
}

export interface CanvasLike {
  /** Set to 0 to release the canvas's pixels. */
  width: number;
  height: number;
  getContext(kind: '2d', options?: { colorSpace?: PredefinedColorSpace }): ContextLike | null;
  convertToBlob(options: { type: string; quality?: number }): Promise<Blob>;
}

export interface DecoderLike {
  configure(config: VideoDecoderConfig): void;
  decode(chunk: unknown): void;
  flush(): Promise<void>;
  close(): void;
}

export interface DecoderClass {
  new (init: { output: (frame: FrameLike) => void; error: (error: Error) => void }): DecoderLike;
  isConfigSupported(config: VideoDecoderConfig): Promise<{ supported?: boolean }>;
}

/** The browser a decode runs in: a worker's globals, or fakes. */
export interface DecodeEnvironment {
  VideoDecoder?: DecoderClass;
  makeChunk?: (init: { type: 'key'; timestamp: number; data: Uint8Array }) => unknown;
  createImageBitmap?: (blob: Blob) => Promise<ImageLike>;
  canvas(width: number, height: number): CanvasLike;
  /** The colour space the canvases draw in. */
  colorSpace: PredefinedColorSpace;
  now(): number;
}

/** The environment of a worker (or a page) of this browser. */
export function browserEnvironment(scope: typeof globalThis = globalThis): DecodeEnvironment {
  const decoder = (scope as { VideoDecoder?: DecoderClass }).VideoDecoder;
  const Chunk = (scope as { EncodedVideoChunk?: new (init: object) => unknown }).EncodedVideoChunk;
  const bitmap = scope.createImageBitmap?.bind(scope);
  return {
    ...(decoder && Chunk ? { VideoDecoder: decoder, makeChunk: (init) => new Chunk(init) } : {}),
    ...(bitmap ? { createImageBitmap: (blob: Blob) => bitmap(blob) } : {}),
    canvas: (width, height) => new OffscreenCanvas(width, height) as unknown as CanvasLike,
    colorSpace: 'srgb',
    now: () => performance.now(),
  };
}

function context(canvas: CanvasLike, env: DecodeEnvironment): ContextLike {
  const context = canvas.getContext('2d', { colorSpace: env.colorSpace });
  if (!context) {
    throw new Error('the browser gave no 2D canvas');
  }
  return context;
}

/** The decoder configuration of one HEVC configuration: its codec string and its `hvcC` record. */
export function decoderConfig(config: HevcDecoderConfig, colorSpace?: VideoColorSpaceInit): VideoDecoderConfig {
  return {
    codec: config.codec,
    description: new Uint8Array(config.description),
    ...(colorSpace ? { colorSpace } : {}),
  };
}

// ISO/IEC 23091-2 code points WebCodecs names (TypeScript's DOM library lists fewer than the WebCodecs registry).
const PRIMARIES: Partial<Record<number, string>> = {
  1: 'bt709',
  5: 'bt470bg',
  6: 'smpte170m',
  9: 'bt2020',
  12: 'smpte432',
};
const TRANSFERS: Partial<Record<number, string>> = {
  1: 'bt709',
  6: 'smpte170m',
  8: 'linear',
  13: 'iec61966-2-1',
  16: 'pq',
  18: 'hlg',
};
const MATRICES: Partial<Record<number, string>> = {
  0: 'rgb',
  1: 'bt709',
  5: 'bt470bg',
  6: 'smpte170m',
  9: 'bt2020-ncl',
};

/** An `nclx` code point, unless it is absent or unspecified (2). */
function stated(value: number | null): number | null {
  return value === null || value === 2 ? null : value;
}

/**
 * The colour space the frames of `config` are labelled with, which the browser converts from as it draws them. It
 * replaces what the coded pictures state, so each field is the first that says something of: the primaries and
 * transfer, the `nclx`, the space the ICC profile's colourants and curve match (an iPhone's Display P3), the pictures'
 * VUI (`config.signal`), else sRGB's; the YCbCr matrix and range, the `nclx`, the VUI, else BT.601 full range, as an
 * iPhone writes them.
 */
export function frameColour(plan: HeicPlan, config: HevcDecoderConfig): VideoColorSpaceInit {
  const { primaries, transfer, matrix, fullRange, iccPrimaries, iccTransfer } = plan.colour;
  const { signal } = config;
  const named = (table: Partial<Record<number, string>>, ...values: (number | null)[]): string | undefined => {
    const value = values.find((candidate) => candidate !== null && table[candidate] !== undefined);
    return value === undefined || value === null ? undefined : table[value];
  };
  return {
    primaries: (named(PRIMARIES, stated(primaries), iccPrimaries, signal.primaries) ?? 'bt709') as VideoColorPrimaries,
    transfer: (named(TRANSFERS, stated(transfer), iccTransfer, signal.transfer) ??
      'iec61966-2-1') as VideoTransferCharacteristics,
    matrix: (named(MATRICES, stated(matrix), signal.matrix) ?? 'smpte170m') as VideoMatrixCoefficients,
    fullRange: fullRange ?? signal.fullRange ?? true,
  };
}

/**
 * The ICC profile a WebCodecs decode carries: a profile of no space `frameColour` names (no `nclx` either), whose
 * values are drawn as sRGB's and kept as they are.
 */
export function carriedProfile(plan: HeicPlan): Uint8Array | null {
  const { icc, primaries, transfer, iccPrimaries, iccTransfer } = plan.colour;
  const known = (stated(primaries) ?? iccPrimaries) !== null && (stated(transfer) ?? iccTransfer) !== null;
  return icc === null || known ? null : new Uint8Array(icc);
}

/** The tile's chunk: its ranges of the file, concatenated. */
async function tileData(file: Blob, extents: HeicPlan['tiles'][number]['extents']): Promise<Uint8Array> {
  const parts = await Promise.all(
    extents.map(async ({ offset, length }) => new Uint8Array(await file.slice(offset, offset + length).arrayBuffer())),
  );
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
}

/** Draws every tile of `plan` on a canvas of its coded size with WebCodecs. */
async function decodeTiles(plan: HeicPlan, file: Blob, env: DecodeEnvironment): Promise<CanvasLike> {
  const Decoder = env.VideoDecoder;
  const makeChunk = env.makeChunk;
  if (!Decoder || !makeChunk) {
    throw new Error('this browser has no WebCodecs video decoder');
  }
  for (const config of plan.configs) {
    const support = await Decoder.isConfigSupported(decoderConfig(config));
    if (!support.supported) {
      throw new Error(`this browser's video decoder does not take ${config.codec}`);
    }
  }
  const canvas = env.canvas(plan.codedWidth, plan.codedHeight);
  const draw = context(canvas, env);
  for (const [index, config] of plan.configs.entries()) {
    const tiles = plan.tiles.filter((tile) => tile.config === index);
    // Written by the decoder's callbacks.
    const state: { failure: Error | null; drawn: number } = { failure: null, drawn: 0 };
    const decoder = new Decoder({
      output: (frame) => {
        const tile = tiles[frame.timestamp];
        try {
          if (tile) {
            draw.drawImage(frame, tile.x, tile.y);
            state.drawn += 1;
          }
        } finally {
          frame.close();
        }
      },
      error: (error) => {
        state.failure ??= error;
      },
    });
    try {
      decoder.configure(decoderConfig(config, frameColour(plan, config)));
      for (const [timestamp, tile] of tiles.entries()) {
        decoder.decode(makeChunk({ type: 'key', timestamp, data: await tileData(file, tile.extents) }));
        if (state.failure) {
          break;
        }
      }
      await decoder.flush();
    } catch (error) {
      state.failure ??= error instanceof Error ? error : new Error(String(error));
    } finally {
      try {
        decoder.close();
      } catch {
        // Closed already, after an error.
      }
    }
    if (state.failure) {
      throw new Error(`the video decoder failed on ${config.codec}: ${state.failure.message}`);
    }
    if (state.drawn !== tiles.length) {
      throw new Error(`the video decoder gave ${state.drawn} of ${tiles.length} tiles`);
    }
  }
  return canvas;
}

/** `source` drawn through one transform onto a new canvas. */
function transform(source: CanvasLike, step: HeicTransform, env: DecodeEnvironment): CanvasLike {
  const { width, height } = source;
  switch (step.kind) {
    case 'crop': {
      const canvas = env.canvas(step.width, step.height);
      context(canvas, env).drawImage(source, -step.x, -step.y);
      return canvas;
    }
    case 'rotate': {
      const turns = ((step.turns % 4) + 4) % 4;
      const odd = turns % 2 === 1;
      const canvas = env.canvas(odd ? height : width, odd ? width : height);
      const draw = context(canvas, env);
      // Anticlockwise quarter turns: (x, y) goes to (y, W − x), (W − x, H − y) or (H − y, x).
      const matrices: Record<number, [number, number, number, number, number, number]> = {
        0: [1, 0, 0, 1, 0, 0],
        1: [0, -1, 1, 0, 0, width],
        2: [-1, 0, 0, -1, width, height],
        3: [0, 1, -1, 0, height, 0],
      };
      draw.setTransform(...matrices[turns]!);
      draw.drawImage(source, 0, 0);
      return canvas;
    }
    case 'mirror': {
      const canvas = env.canvas(width, height);
      const draw = context(canvas, env);
      if (step.axis === 'vertical') {
        draw.setTransform(-1, 0, 0, 1, width, 0);
      } else {
        draw.setTransform(1, 0, 0, -1, 0, height);
      }
      draw.drawImage(source, 0, 0);
      return canvas;
    }
  }
}

/** Frees a canvas's pixels now rather than when it is collected. */
function release(canvas: CanvasLike) {
  canvas.width = 0;
  canvas.height = 0;
}

/** The plan's transforms, in order, each step's source released once the next canvas holds it. */
function transformed(canvas: CanvasLike, plan: HeicPlan, env: DecodeEnvironment): CanvasLike {
  let current = canvas;
  for (const step of plan.transforms) {
    const next = transform(current, step, env);
    release(current);
    current = next;
  }
  return current;
}

/** The browser's own decode of the file, which applies the file's transforms: a bitmap of the plan's output size. */
async function decodeNatively(plan: HeicPlan, file: Blob, env: DecodeEnvironment): Promise<CanvasLike> {
  if (!env.createImageBitmap) {
    throw new Error('this browser has no image decoder in workers');
  }
  let bitmap: ImageLike;
  try {
    bitmap = await env.createImageBitmap(file.type === 'image/heic' ? file : file.slice(0, file.size, 'image/heic'));
  } catch (error) {
    throw new Error(`this browser does not decode HEIC images (${error instanceof Error ? error.message : error})`, {
      cause: error,
    });
  }
  try {
    if (bitmap.width !== plan.width || bitmap.height !== plan.height) {
      throw new Error(
        `this browser decoded it at ${bitmap.width} × ${bitmap.height}, not ${plan.width} × ${plan.height}`,
      );
    }
    const canvas = env.canvas(bitmap.width, bitmap.height);
    context(canvas, env).drawImage(bitmap, 0, 0);
    return canvas;
  } finally {
    bitmap.close();
  }
}

const ICC_MARKER = new TextEncoder().encode('ICC_PROFILE\0');
/** The most profile bytes one APP2 segment holds (ICC.1 Annex B). */
const ICC_CHUNK = 65_519;

function isIccSegment(jpeg: Uint8Array, at: number): boolean {
  return jpeg[at + 1] === 0xe2 && ICC_MARKER.every((byte, index) => jpeg[at + 4 + index] === byte);
}

/**
 * `jpeg` with `icc` as its ICC profile (APP2 `ICC_PROFILE` segments, after the JFIF segment if there is one) in
 * place of any it had. The segments up to the first scan are walked; the scan is kept as it is.
 */
export function withIccProfile(jpeg: Uint8Array, icc: Uint8Array): Uint8Array<ArrayBuffer> {
  if (jpeg[0] !== 0xff || jpeg[1] !== 0xd8) {
    throw new Error('the encoded file is not a JPEG');
  }
  const kept: Uint8Array[] = [];
  let jfif: Uint8Array | null = null;
  let at = 2;
  while (at + 4 <= jpeg.length && jpeg[at] === 0xff && jpeg[at + 1] !== 0xda) {
    const length = (jpeg[at + 2]! << 8) | jpeg[at + 3]!;
    const segment = jpeg.subarray(at, at + 2 + length);
    if (jpeg[at + 1] === 0xe0 && jfif === null && kept.length === 0) {
      jfif = segment;
    } else if (!isIccSegment(jpeg, at)) {
      kept.push(segment);
    }
    at += 2 + length;
  }
  const count = Math.ceil(icc.length / ICC_CHUNK);
  if (count > 255) {
    throw new Error('the ICC profile is too large for a JPEG');
  }
  const segments: Uint8Array[] = [];
  for (let index = 0; index < count; index += 1) {
    const data = icc.subarray(index * ICC_CHUNK, (index + 1) * ICC_CHUNK);
    const length = 2 + ICC_MARKER.length + 2 + data.length;
    const segment = new Uint8Array(2 + length);
    segment.set([0xff, 0xe2, length >> 8, length & 0xff]);
    segment.set(ICC_MARKER, 4);
    segment.set([index + 1, count], 4 + ICC_MARKER.length);
    segment.set(data, 6 + ICC_MARKER.length);
    segments.push(segment);
  }
  const parts = [jpeg.subarray(0, 2), ...(jfif ? [jfif] : []), ...segments, ...kept, jpeg.subarray(at)];
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

async function encode(canvas: CanvasLike, plan: HeicPlan, icc: Uint8Array | null): Promise<ArrayBuffer> {
  if (canvas.width !== plan.width || canvas.height !== plan.height) {
    throw new Error(`the decode is ${canvas.width} × ${canvas.height}, not ${plan.width} × ${plan.height}`);
  }
  const blob = await canvas.convertToBlob({ type: 'image/jpeg', quality: JPEG_QUALITY });
  if (blob.type !== 'image/jpeg') {
    throw new Error(`the browser encoded ${blob.type || 'an unknown type'}, not JPEG`);
  }
  const bytes = new Uint8Array(await blob.arrayBuffer());
  return (icc ? withIccProfile(bytes, icc) : bytes).buffer as ArrayBuffer;
}

/** Decodes `file` (the need's source) along `plan`: the browser's image decoder, else WebCodecs. */
export async function decodeHeic(plan: HeicPlan, file: Blob, env: DecodeEnvironment): Promise<DecodedPhoto> {
  const started = env.now();
  const reasons: string[] = [];
  const paths: [DecodePath, () => Promise<CanvasLike>, Uint8Array | null][] = [
    ['native', () => decodeNatively(plan, file, env), null],
    ['webcodecs', async () => transformed(await decodeTiles(plan, file, env), plan, env), carriedProfile(plan)],
  ];
  for (const [path, decode, profile] of paths) {
    try {
      const canvas = await decode();
      let bytes: ArrayBuffer;
      try {
        bytes = await encode(canvas, plan, profile);
      } finally {
        release(canvas);
      }
      return { bytes, mime: 'image/jpeg', width: plan.width, height: plan.height, path, elapsed: env.now() - started };
    } catch (error) {
      reasons.push(error instanceof Error ? error.message : String(error));
    }
  }
  throw new Undecodable(reasons);
}
