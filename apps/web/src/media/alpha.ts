// A WebM copy of a video with alpha that only some browsers play (HEVC with an alpha layer: Safari, and Chrome on
// macOS), for every other one (Chromium, Chrome elsewhere, Firefox): VP9 (else VP8) with WebM's alpha
// (`webm-writer.ts`), at the size the compiler plans (`alternatePlan`), which the deck then offers after the original
// (`<source>`s in that order, `docs/format.md` §1).
//
// No browser's `VideoEncoder` keeps alpha, and WebKit's `VideoDecoder` drops HEVC's alpha layer, but both engines draw
// a playing `<video>` with its alpha. So the page seeks a hidden `<video>` of the original to each frame's time (the
// middle of the frame, so a rounding of the time cannot land on its neighbour) and takes the frame shown
// (`new VideoFrame(video)`, its alpha kept); a worker (`alpha-worker.ts`) draws it at the planned size, splits its
// pixels into a colour picture and an alpha picture (BT.709 limited range; the alpha as luma, full range, chroma
// neutral) and encodes each with its own encoder, the same frames key frames in both (every `KEY_FRAME_SECONDS`,
// where the cues point). Every frame keeps its time (ms), so the frame rate, variable ones too, and the length stay.
//
// The copy is not made, and the deck keeps the original alone, when this browser does not play the original, shows
// none of the sampled frames with any transparency (it plays the video without its alpha layer), encodes neither VP9
// nor VP8, or the original has a sound some use may play (no copy carries sound yet). A sound every use mutes
// (`silent`) is never heard: the copy leaves it out. The environments are injected so tests can fake the browser.
import type { PlannedVideo } from '@slidock/types';

import { type Movie, MovieError, type MovieTrack, playedSpan, readMovie } from './movie';
import { type CodecLike, type EncodedLike, type FrameLike, KEY_FRAME_SECONDS, frameRate, sourceBitrate } from './video';
import { type WebmFrame, writeWebm } from './webm-writer';

/** A video to copy: the plan's entry and its file. */
export interface AlphaJob {
  plan: PlannedVideo;
  file: Blob;
}

export interface AlphaProgress {
  frames: number;
}

/** What became of a video: its WebM copy, or why there is none. */
export type AlphaOutcome =
  { kind: 'made'; file: Blob; codec: string; elapsed: number } | { kind: 'kept'; reason: string; elapsed: number };

/** One frame to take: the time to seek the `<video>` to, and the frame's time in the copy (ms). */
export interface AlphaFrameTime {
  seek: number;
  timestamp: number;
}

/**
 * The frames of `track` its edit list plays, in presentation order: each one's time in the copy (in milliseconds,
 * after the edit list's leading delay) and the middle of its span on the `<video>`'s timeline (seconds).
 */
export function alphaSchedule(track: MovieTrack, movieTimescale: number): AlphaFrameTime[] {
  const span = playedSpan(track, movieTimescale);
  const times = track.samples
    .map((sample) => sample.cts)
    .filter((cts) => cts >= span.start && cts < span.end)
    .sort((a, b) => a - b);
  const out: AlphaFrameTime[] = [];
  times.forEach((cts, index) => {
    const end = times[index + 1] ?? span.end;
    const start = (cts - span.start) / track.timescale + span.delay;
    const middle = start + (end - cts) / track.timescale / 2;
    const timestamp = Math.round(start * 1000);
    // Two frames within a millisecond of each other: the later one shows.
    if (out.length > 0 && out.at(-1)!.timestamp >= timestamp) {
      out.pop();
    }
    out.push({ seek: middle, timestamp });
  });
  return out;
}

/** The VP9 levels (VP9 bitstream §A.1): level, picture size and luma sample rate limits. */
const VP9_LEVELS: readonly [number, number, number][] = [
  [10, 36_864, 829_440],
  [11, 73_728, 2_764_800],
  [20, 122_880, 4_608_000],
  [21, 245_760, 9_216_000],
  [30, 552_960, 20_736_000],
  [31, 983_040, 36_864_000],
  [40, 2_228_224, 83_558_400],
  [41, 2_228_224, 160_432_128],
  [50, 8_912_896, 311_951_360],
  [51, 8_912_896, 588_251_136],
  [52, 8_912_896, 1_176_502_272],
  [60, 35_651_584, 1_176_502_272],
  [61, 35_651_584, 2_353_004_544],
  [62, 35_651_584, 4_706_009_088],
];

/** The codec strings to try for `width` × `height` at `fps`: VP9 profile 0, 8 bits, at the least level that fits, then VP8. */
export function alphaCodecs(width: number, height: number, fps: number): string[] {
  const pixels = width * height;
  const level = VP9_LEVELS.find(([, size, rate]) => pixels <= size && pixels * fps <= rate)?.[0] ?? 62;
  return [`vp09.00.${level}.08`, 'vp8'];
}

/** The share of the colour's bitrate the alpha picture is given (mostly flat, it needs little). */
const ALPHA_SHARE = 0.15;

/**
 * The colour's bitrate: the original's per pixel (its alpha layer's bits included), at the planned size. A screen
 * recording keeps its text sharp at it; the alpha picture gets `ALPHA_SHARE` of it.
 */
export function alphaBitrates(plan: PlannedVideo): { colour: number; alpha: number } {
  const pixels = (plan.targetWidth * plan.targetHeight) / (plan.width * plan.height);
  const colour = Math.max(100_000, Math.round(sourceBitrate(plan) * pixels));
  return { colour, alpha: Math.max(50_000, Math.round(colour * ALPHA_SHARE)) };
}

// BT.709 (ITU-R BT.709-6 §3), limited range, in 16.16 fixed point.
const KR = 0.2126;
const KB = 0.0722;
const KG = 1 - KR - KB;
const FIX = 65536;
const Y_R = Math.round(((KR * 219) / 255) * FIX);
const Y_G = Math.round(((KG * 219) / 255) * FIX);
const Y_B = Math.round(((KB * 219) / 255) * FIX);
const CB_R = Math.round(((-KR / (2 * (1 - KB))) * 224 * FIX) / 255);
const CB_G = Math.round(((-KG / (2 * (1 - KB))) * 224 * FIX) / 255);
const CB_B = Math.round(((0.5 * 224) / 255) * FIX);
const CR_R = Math.round(((0.5 * 224) / 255) * FIX);
const CR_G = Math.round(((-KG / (2 * (1 - KR))) * 224 * FIX) / 255);
const CR_B = Math.round(((-KB / (2 * (1 - KR))) * 224 * FIX) / 255);
const HALF = FIX / 2;

/** The bytes of an I420 picture of `width` × `height`. */
export function i420Bytes(width: number, height: number): number {
  return width * height + 2 * Math.ceil(width / 2) * Math.ceil(height / 2);
}

/**
 * Splits straight (not premultiplied) RGBA pixels into an I420 colour picture (BT.709, limited range; each chroma
 * sample the mean of its 2 × 2 pixels) and an I420 alpha picture (its luma the alpha, its chroma 128). Returns the
 * pixels that are not opaque.
 */
export function splitRgba(
  rgba: Uint8Array | Uint8ClampedArray,
  width: number,
  height: number,
  colour: Uint8Array,
  alpha: Uint8Array,
): number {
  const cw = Math.ceil(width / 2);
  const ch = Math.ceil(height / 2);
  const uAt = width * height;
  const vAt = uAt + cw * ch;
  let clear = 0;
  for (let y = 0; y < height; y += 1) {
    let p = y * width * 4;
    let o = y * width;
    for (let x = 0; x < width; x += 1, p += 4, o += 1) {
      const a = rgba[p + 3]!;
      colour[o] = (16 * FIX + Y_R * rgba[p]! + Y_G * rgba[p + 1]! + Y_B * rgba[p + 2]! + HALF) >> 16;
      alpha[o] = a;
      if (a !== 255) {
        clear += 1;
      }
    }
  }
  for (let cy = 0; cy < ch; cy += 1) {
    const y0 = cy * 2;
    const y1 = Math.min(y0 + 1, height - 1);
    for (let cx = 0; cx < cw; cx += 1) {
      const x0 = cx * 2;
      const x1 = Math.min(x0 + 1, width - 1);
      const a = (y0 * width + x0) * 4;
      const b = (y0 * width + x1) * 4;
      const c = (y1 * width + x0) * 4;
      const d = (y1 * width + x1) * 4;
      const r = rgba[a]! + rgba[b]! + rgba[c]! + rgba[d]!;
      const g = rgba[a + 1]! + rgba[b + 1]! + rgba[c + 1]! + rgba[d + 1]!;
      const bl = rgba[a + 2]! + rgba[b + 2]! + rgba[c + 2]! + rgba[d + 2]!;
      const at = cy * cw + cx;
      colour[uAt + at] = (128 * FIX + ((CB_R * r + CB_G * g + CB_B * bl) >> 2) + HALF) >> 16;
      colour[vAt + at] = (128 * FIX + ((CR_R * r + CR_G * g + CR_B * bl) >> 2) + HALF) >> 16;
    }
  }
  alpha.fill(128, uAt);
  return clear;
}

// The worker's side: encoding the frames it is handed into the WebM.

export interface AlphaContextLike {
  imageSmoothingEnabled: boolean;
  imageSmoothingQuality: ImageSmoothingQuality;
  clearRect(x: number, y: number, w: number, h: number): void;
  drawImage(image: FrameLike, dx: number, dy: number, dw: number, dh: number): void;
  getImageData(x: number, y: number, w: number, h: number): { data: Uint8ClampedArray };
}

/** What the encoding needs of WebCodecs and canvases. */
export interface AlphaEncoderEnvironment {
  encoderSupported(config: VideoEncoderConfig): Promise<boolean>;
  videoEncoder(output: (chunk: EncodedLike) => void, error: (error: Error) => void): CodecLike<FrameLike>;
  /** An I420 frame of `data`, its colour stated (the alpha picture states none). */
  i420Frame(data: Uint8Array, init: { width: number; height: number; timestamp: number; colour: boolean }): FrameLike;
  context(width: number, height: number): AlphaContextLike;
}

/** How the worker encodes a copy: its size, codecs to try, bitrates and length. */
export interface AlphaSpec {
  width: number;
  height: number;
  codecs: string[];
  bitrates: { colour: number; alpha: number };
  framerate: number;
}

/** How many frames may wait in an encoder before the next is taken. */
const QUEUE = 8;

function breathe(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 1));
}

/** An encode in progress: frames in, the WebM out. */
export interface AlphaEncode {
  readonly codec: string;
  /**
   * Draws `image` and closes it before it returns (a caller may then let the frame's source move on), then splits
   * and encodes it; resolves once the encoders can take another.
   */
  add(image: FrameLike, timestamp: number, key: boolean): Promise<void>;
  /** The frames that held a pixel that is not opaque. */
  readonly transparentFrames: number;
  /** Flushes the encoders and writes the WebM, `duration` ms long. */
  finish(duration: number): Promise<Blob>;
  close(): void;
}

/** Starts an encode for `spec`, with the first codec both encoders take; a reason when none is. */
export async function startAlphaEncode(spec: AlphaSpec, env: AlphaEncoderEnvironment): Promise<AlphaEncode | string> {
  const { width, height } = spec;
  let codec: string | null = null;
  const config = (bitrate: number, candidate: string): VideoEncoderConfig => ({
    codec: candidate,
    width,
    height,
    bitrate,
    framerate: spec.framerate,
    bitrateMode: 'variable',
    latencyMode: 'quality',
    alpha: 'discard',
  });
  for (const candidate of spec.codecs) {
    if (
      (await env.encoderSupported(config(spec.bitrates.colour, candidate))) &&
      (await env.encoderSupported(config(spec.bitrates.alpha, candidate)))
    ) {
      codec = candidate;
      break;
    }
  }
  if (codec === null) {
    return `this browser encodes neither VP9 nor VP8 at ${width} × ${height}`;
  }
  let failure: Error | null = null;
  const fail = (error: Error) => {
    failure ??= error;
  };
  const colourChunks = new Map<number, { key: boolean; data: Uint8Array }>();
  const alphaChunks = new Map<number, Uint8Array>();
  const copy = (chunk: EncodedLike) => {
    const bytes = new Uint8Array(chunk.byteLength);
    chunk.copyTo(bytes);
    return bytes;
  };
  const colourEncoder = env.videoEncoder((chunk) => {
    colourChunks.set(chunk.timestamp, { key: chunk.type === 'key', data: copy(chunk) });
  }, fail);
  const alphaEncoder = env.videoEncoder((chunk) => {
    alphaChunks.set(chunk.timestamp, copy(chunk));
  }, fail);
  colourEncoder.configure(config(spec.bitrates.colour, codec));
  alphaEncoder.configure(config(spec.bitrates.alpha, codec));
  const context = env.context(width, height);
  context.imageSmoothingEnabled = true;
  context.imageSmoothingQuality = 'high';
  const size = i420Bytes(width, height);
  const submitted: { timestamp: number; key: boolean }[] = [];
  let transparentFrames = 0;
  const encoders = [colourEncoder, alphaEncoder];
  const close = () => {
    for (const encoder of encoders) {
      try {
        encoder.close();
      } catch {
        // Closed already, after an error.
      }
    }
  };
  return {
    codec,
    get transparentFrames() {
      return transparentFrames;
    },
    async add(image, timestamp, key) {
      try {
        context.clearRect(0, 0, width, height);
        context.drawImage(image, 0, 0, width, height);
      } finally {
        image.close();
      }
      const pixels = context.getImageData(0, 0, width, height).data;
      const colour = new Uint8Array(size);
      const alpha = new Uint8Array(size);
      if (splitRgba(pixels, width, height, colour, alpha) > 0) {
        transparentFrames += 1;
      }
      // Microseconds, as WebCodecs counts; a frame's time is a whole millisecond.
      const micros = timestamp * 1000;
      submitted.push({ timestamp: micros, key });
      for (const [encoder, data, isColour] of [
        [colourEncoder, colour, true],
        [alphaEncoder, alpha, false],
      ] as const) {
        const frame = env.i420Frame(data, { width, height, timestamp: micros, colour: isColour });
        encoder.encode!(frame, { keyFrame: key });
        frame.close();
      }
      while (encoders.some((encoder) => (encoder.encodeQueueSize ?? 0) > QUEUE)) {
        if (failure) {
          throw failure;
        }
        await breathe();
      }
      if (failure) {
        throw failure;
      }
    },
    async finish(duration) {
      try {
        await Promise.all(encoders.map((encoder) => encoder.flush()));
      } finally {
        close();
      }
      if (failure) {
        throw failure;
      }
      const frames: WebmFrame[] = [];
      for (const { timestamp, key } of submitted) {
        const colour = colourChunks.get(timestamp);
        const alpha = alphaChunks.get(timestamp);
        if (!colour || !alpha) {
          throw new Error(`the encoder dropped the frame at ${timestamp / 1000} ms`);
        }
        if (key && !colour.key) {
          throw new Error(`the encoder made no key frame at ${timestamp / 1000} ms`);
        }
        frames.push({ timestamp: timestamp / 1000, key: key && colour.key, data: colour.data, alpha });
      }
      return writeWebm({
        video: {
          codec: codec.startsWith('vp09') ? 'V_VP9' : 'V_VP8',
          width,
          height,
          alpha: true,
          colour: { matrix: 1, transfer: 1, primaries: 1, fullRange: false },
          frames,
        },
        sound: null,
        duration,
      });
    },
    close,
  };
}

// The page's side: the original played in a `<video>`, its frames taken one by one.

/** A frame of the `<video>`, which a worker can be handed. */
export type TakenFrame = FrameLike & Transferable;

/** The original in a `<video>`. */
export interface AlphaSource {
  /** The frame the video shows at `seconds`, stamped `timestamp` (µs). */
  frameAt(seconds: number, timestamp: number): Promise<TakenFrame>;
  close(): void;
}

/** Where frames are encoded (the worker, or the encode itself in tests). */
export interface AlphaSink {
  start(spec: AlphaSpec): Promise<string | null>;
  /** Hands over a frame; resolves once the sink has drawn it (the `<video>` may then seek on) and can take another. */
  frame(frame: TakenFrame, timestamp: number, key: boolean): Promise<void>;
  /** The WebM, or why there is none, with the frames that held a pixel that is not opaque. */
  finish(duration: number): Promise<{ file: Blob; codec: string; transparentFrames: number } | string>;
  close(): void;
}

/** What the page needs to make copies: the original's `<video>`, a worker, a clock and a probe of transparency. */
export interface AlphaEnvironment {
  /** Opens the original; a reason when this browser does not play it. */
  open(file: Blob, codec: string): Promise<AlphaSource | string>;
  sink(): AlphaSink;
  /** The pixels of `frame` that are not opaque (the frame is not closed). */
  transparentPixels(frame: TakenFrame): number;
  now(): number;
}

/** How many frames are sampled for transparency before the copy is made. */
const PROBE_FRAMES = 5;

class Cancelled extends Error {}

/** Makes `job`'s WebM copy (see the module comment). */
export async function makeAlphaCopy(
  job: AlphaJob,
  env: AlphaEnvironment,
  onProgress: (progress: AlphaProgress) => void = () => {},
  signal?: AbortSignal,
): Promise<AlphaOutcome> {
  const start = env.now();
  const kept = (reason: string): AlphaOutcome => ({ kind: 'kept', reason, elapsed: env.now() - start });
  const { plan } = job;
  // A sound every use plays muted is never heard, so the copy goes without it (`silent`); another is not carried yet.
  if (plan.audio !== null && !plan.silent) {
    return kept('its sound cannot be carried into a WebM copy');
  }
  let movie: Movie;
  try {
    movie = await readMovie(job.file);
  } catch (error) {
    return kept(error instanceof MovieError ? `its movie cannot be read (${error.message})` : String(error));
  }
  const track = movie.tracks.find((candidate) => candidate.handler === 'vide');
  if (!track) {
    return kept('its movie has no video track');
  }
  const schedule = alphaSchedule(track, movie.timescale);
  if (schedule.length === 0) {
    return kept('its movie plays no frame');
  }
  const source = await env.open(job.file, plan.codec);
  if (typeof source === 'string') {
    return kept(source);
  }
  const sink = env.sink();
  try {
    // A browser that plays the video without its alpha layer shows every frame opaque.
    let transparent = false;
    for (let index = 0; index < PROBE_FRAMES && !transparent; index += 1) {
      const at = schedule[Math.floor((index * (schedule.length - 1)) / Math.max(1, PROBE_FRAMES - 1))]!;
      const frame = await source.frameAt(at.seek, at.timestamp * 1000);
      try {
        transparent = env.transparentPixels(frame) > 0;
      } finally {
        frame.close();
      }
    }
    if (!transparent) {
      return kept('this browser plays it without its transparency (no transparent pixel in its sampled frames)');
    }
    const refused = await sink.start({
      width: plan.targetWidth,
      height: plan.targetHeight,
      codecs: alphaCodecs(plan.targetWidth, plan.targetHeight, frameRate(plan)),
      bitrates: alphaBitrates(plan),
      framerate: frameRate(plan),
    });
    if (refused !== null) {
      return kept(refused);
    }
    let lastKey = -Infinity;
    // The next frame is sought while the sink encodes the last one.
    let taking: Promise<void> = Promise.resolve();
    for (const [index, at] of schedule.entries()) {
      if (signal?.aborted) {
        throw new Cancelled();
      }
      const key = at.timestamp - lastKey >= KEY_FRAME_SECONDS * 1000;
      if (key) {
        lastKey = at.timestamp;
      }
      const frame = await source.frameAt(at.seek, at.timestamp * 1000);
      try {
        await taking;
      } catch (error) {
        frame.close();
        throw error;
      }
      taking = sink.frame(frame, at.timestamp, key);
      // Awaited before the next frame is handed over (a failure then ends the copy), not reported unhandled before.
      taking.catch(() => {});
      if ((index + 1) % 15 === 0) {
        onProgress({ frames: index + 1 });
      }
    }
    await taking;
    onProgress({ frames: schedule.length });
    const span = playedSpan(track, movie.timescale);
    const duration = Math.round((span.delay + (span.end - span.start) / track.timescale) * 1000);
    const made = await sink.finish(duration);
    if (typeof made === 'string') {
      return kept(made);
    }
    if (made.transparentFrames === 0) {
      return kept('this browser played every frame of it opaque');
    }
    return { kind: 'made', file: made.file, codec: made.codec, elapsed: env.now() - start };
  } catch (error) {
    if (error instanceof Cancelled) {
      return kept('the copy was cancelled');
    }
    return kept(`the copy failed (${error instanceof Error ? error.message : String(error)})`);
  } finally {
    source.close();
    sink.close();
  }
}

/** The worker's encoding environment: its WebCodecs and an `OffscreenCanvas`. */
export function browserAlphaEncoderEnvironment(scope: typeof globalThis = globalThis): AlphaEncoderEnvironment {
  return {
    encoderSupported: async (config) => {
      try {
        return (await scope.VideoEncoder.isConfigSupported(config)).supported === true;
      } catch {
        return false;
      }
    },
    videoEncoder: (output, error) =>
      new scope.VideoEncoder({
        output: output as EncodedVideoChunkOutputCallback,
        error,
      }) as unknown as CodecLike<FrameLike>,
    i420Frame: (data, { width, height, timestamp, colour }) =>
      new scope.VideoFrame(data, {
        format: 'I420',
        codedWidth: width,
        codedHeight: height,
        timestamp,
        ...(colour ? { colorSpace: { matrix: 'bt709', primaries: 'bt709', transfer: 'bt709', fullRange: false } } : {}),
      }),
    context: (width, height) => {
      const context = new OffscreenCanvas(width, height).getContext('2d', { willReadFrequently: true });
      if (!context) {
        throw new Error('the browser gave no 2D canvas');
      }
      return context as unknown as AlphaContextLike;
    },
  };
}

/** How long the `<video>` may take to load, and to seek. */
const LOAD_MS = 30_000;
const SEEK_MS = 15_000;

/** How many times a seek is asked before the copy fails. */
const SEEK_ATTEMPTS = 3;

function within<T>(promise: Promise<T>, ms: number, late: () => T): Promise<T> {
  return Promise.race([promise, new Promise<T>((resolve) => setTimeout(() => resolve(late()), ms))]);
}

/** The original in a hidden `<video>` of `doc`, played from an object URL. */
export async function openVideoSource(
  file: Blob,
  codec: string,
  doc: Document = document,
): Promise<AlphaSource | string> {
  const url = URL.createObjectURL(file);
  const video = doc.createElement('video');
  video.muted = true;
  video.playsInline = true;
  video.preload = 'auto';
  video.setAttribute('aria-hidden', 'true');
  video.style.cssText = 'position:fixed;left:0;top:0;width:2px;height:2px;opacity:0;pointer-events:none;z-index:-1';
  const close = () => {
    video.removeAttribute('src');
    video.load();
    video.remove();
    URL.revokeObjectURL(url);
  };
  doc.body.append(video);
  const loaded = await within(
    new Promise<string | null>((resolve) => {
      video.addEventListener('loadeddata', () => resolve(null), { once: true });
      video.addEventListener('error', () => resolve(`this browser plays no ${codec} video`), { once: true });
      video.src = url;
    }),
    LOAD_MS,
    () => `this browser did not load its ${codec} video in ${LOAD_MS / 1000} s`,
  );
  if (loaded !== null) {
    close();
    return loaded;
  }
  return {
    async frameAt(seconds, timestamp) {
      // A seek WebKit leaves pending (once in three downloads of a 1 GB deck, two copies under way) is asked again.
      for (let attempt = 1; ; attempt += 1) {
        const seeked = new Promise<boolean>((resolve) =>
          video.addEventListener('seeked', () => resolve(true), { once: true }),
        );
        video.currentTime = seconds;
        if (await within(seeked, SEEK_MS, () => false)) {
          break;
        }
        if (attempt === SEEK_ATTEMPTS) {
          throw new Error(`the video did not seek to ${seconds.toFixed(3)} s`);
        }
      }
      return new VideoFrame(video, { timestamp }) as unknown as TakenFrame;
    },
    close,
  };
}

/** The pixels of `frame` that are not opaque, drawn at its own size. */
export function transparentPixels(frame: TakenFrame): number {
  const { codedWidth: width, codedHeight: height } = frame;
  const context = new OffscreenCanvas(width, height).getContext('2d', { willReadFrequently: true });
  if (!context) {
    return 0;
  }
  context.drawImage(frame as unknown as CanvasImageSource, 0, 0);
  const pixels = context.getImageData(0, 0, width, height).data;
  let clear = 0;
  for (let at = 3; at < pixels.length; at += 4) {
    if (pixels[at] !== 255) {
      clear += 1;
    }
  }
  return clear;
}

/** A worker's messages (`alpha-worker.ts`). */
export type AlphaRequest =
  | { type: 'start'; spec: AlphaSpec }
  | { type: 'frame'; frame: TakenFrame; timestamp: number; key: boolean }
  | { type: 'finish'; duration: number };

export type AlphaReply =
  | { type: 'started'; refused: string | null }
  | { type: 'taken' }
  | { type: 'finished'; file: Blob; codec: string; transparentFrames: number }
  | { type: 'failed'; reason: string };

/** What the page needs of the worker, so tests can use a fake. */
export interface AlphaPort {
  postMessage(message: AlphaRequest, transfer?: Transferable[]): void;
  onmessage: ((event: MessageEvent<AlphaReply>) => void) | null;
  onerror: ((event: ErrorEvent) => void) | null;
  terminate(): void;
}

/** A sink over a worker that `start` makes: each request answered in order. */
export function workerAlphaSink(start: () => AlphaPort): AlphaSink {
  const port = start();
  const waiting: ((reply: AlphaReply) => void)[] = [];
  port.onmessage = ({ data }) => waiting.shift()?.(data);
  port.onerror = (event) => {
    event.preventDefault();
    for (const resolve of waiting.splice(0)) {
      resolve({ type: 'failed', reason: `the copy's worker failed (${event.message || 'no message'})` });
    }
  };
  const ask = (request: AlphaRequest, transfer: Transferable[] = []) =>
    new Promise<AlphaReply>((resolve) => {
      waiting.push(resolve);
      port.postMessage(request, transfer);
    });
  return {
    async start(spec) {
      const reply = await ask({ type: 'start', spec });
      return reply.type === 'started'
        ? reply.refused
        : reply.type === 'failed'
          ? reply.reason
          : 'the worker did not start';
    },
    async frame(frame, timestamp, key) {
      const reply = await ask({ type: 'frame', frame, timestamp, key }, [frame]);
      if (reply.type === 'failed') {
        throw new Error(reply.reason);
      }
    },
    async finish(duration) {
      const reply = await ask({ type: 'finish', duration });
      if (reply.type === 'finished') {
        return { file: reply.file, codec: reply.codec, transparentFrames: reply.transparentFrames };
      }
      return reply.type === 'failed' ? reply.reason : 'the worker gave no file';
    },
    close: () => port.terminate(),
  };
}

/** Starts one of the app's copy workers. */
export function startAlphaWorker(): AlphaPort {
  return new Worker(new URL('./alpha-worker.ts', import.meta.url), { type: 'module' }) as unknown as AlphaPort;
}

/** The page's environment: the original in a `<video>`, a worker per copy. */
export function browserAlphaEnvironment(start: () => AlphaPort = startAlphaWorker): AlphaEnvironment {
  return {
    open: (file, codec) => openVideoSource(file, codec),
    sink: () => workerAlphaSink(start),
    transparentPixels,
    now: () => performance.now(),
  };
}

/** Whether this page could make copies at all: a `<video>` it can take frames of, and a `VideoEncoder`. */
export function canMakeAlphaCopies(scope: typeof globalThis = globalThis): boolean {
  const names = ['VideoEncoder', 'VideoFrame', 'OffscreenCanvas', 'Worker'];
  return (
    typeof scope.document !== 'undefined' &&
    names.every((name) => typeof (scope as unknown as Record<string, unknown>)[name] === 'function')
  );
}

/** Makes a deck's copies. */
export interface AlphaCopier {
  copy(job: AlphaJob, onProgress?: (progress: AlphaProgress) => void): Promise<AlphaOutcome>;
  /** Stops the copies being made; pending ones are given up. */
  dispose(): void;
}

/**
 * Copies made at once at most. Two at once in WebKit, in the app with a 1 GB deck open, once left a seek pending and
 * once froze a `<video>` on one frame while its seeks went on completing; one at a time never did.
 */
export const MAX_ALPHA_COPIES = 1;

/** Makes copies with `env`, up to `limit` at a time, in the order asked. */
export function alphaCopier(env: AlphaEnvironment, limit = MAX_ALPHA_COPIES): AlphaCopier {
  const controller = new AbortController();
  const queue: (() => void)[] = [];
  let running = 0;
  const next = () => {
    while (running < limit && queue.length > 0) {
      running += 1;
      queue.shift()!();
    }
  };
  return {
    copy(job, onProgress) {
      if (controller.signal.aborted) {
        return Promise.resolve({ kind: 'kept', reason: 'the copies were stopped', elapsed: 0 });
      }
      return new Promise((resolve) => {
        queue.push(() => {
          makeAlphaCopy(job, env, onProgress, controller.signal)
            .then(resolve, (error: unknown) =>
              resolve({ kind: 'kept', reason: `the copy failed (${String(error)})`, elapsed: 0 }),
            )
            .finally(() => {
              running -= 1;
              next();
            });
        });
        next();
      });
    },
    dispose() {
      controller.abort();
    },
  };
}
