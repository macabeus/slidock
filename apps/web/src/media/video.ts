// Re-encodes a deck's video smaller for a download (a `videoPlan` entry), in the browser, off the main thread
// (`video-worker.ts`). The compiler planned the size (`targetWidth` × `targetHeight`: the size the video shows at, or
// smaller, each side even) and checks the file it is handed; this demuxes, decodes, scales, encodes and muxes:
//
// - Demuxing reads the movie's boxes, then its samples by `Blob` slices a few MiB at a time (`movie.ts`): the file
//   is never read whole.
// - Decoding with WebCodecs' `VideoDecoder` (H.264, and HEVC where the browser decodes it), honouring the edit list:
//   frames before its start or after its end are dropped, so the file plays the same span.
// - A frame at the planned size and upright is encoded as decoded. Another is drawn on a canvas at the planned size,
//   turned by the track matrix's quarter turn (the file then needs no matrix of its own).
// - Encoding with `VideoEncoder` to H.264 (High profile at the level the size and frame rate need, else the next
//   level the browser takes) at a variable bitrate of `BITS_PER_PIXEL` of the chosen quality, never more per pixel
//   than the original's, a key frame every `KEY_FRAME_SECONDS`; every frame keeps its time, so the frame rate
//   (variable ones too) and the length stay. An encoder that drops frames is tried again in software, then up a
//   ladder of bitrates every quality shares (`ladderBitrates`, `encoderAttempts`).
// - Sound: AAC is carried over as it is, its samples read from the original as the file is assembled; 16-bit and
//   wider PCM, and MP3, are re-encoded to AAC where the browser has an AAC encoder; any other sound keeps the original.
// - Muxing with `mp4-writer.ts`: the movie boxes first, the media interleaved by the second.
// - A sound the plan says is never heard (`silent`: every use plays the video muted) is left out. A video it would
//   keep otherwise (already under the bitrate, an encoder that drops frames, a codec this browser does not re-encode,
//   a reason the plan states against a re-encode only) is copied as it is without it: its samples sliced from the
//   original, its sample description, matrix, composition offsets and edit list written again; the plan's
//   `uncopyable` keeps it as it is.
//
// The original stays when the plan keeps it, when the browser decodes or encodes neither its video nor its sound,
// when its video already runs at or under the chosen bitrate at its own size, and when the new file is not smaller.
//
// Colour: the file states its colour in the H.264 stream alone, as the encoder wrote it (no `colr` box: encoders
// report their colour unreliably, and WebKit plays a file whose `colr` states its encoder's sRGB transfer shifted by
// about 9 levels). A frame drawn on a canvas goes through the browser's sRGB canvas (wide-gamut primaries lose their
// most saturated colours, as pictures do). A frame encoded as decoded keeps its colour; when the original states none,
// the stream's parameter sets are rewritten to state none either (`h264.ts`), so a browser plays it as it plays the
// original. The environment is injected so tests can fake the browser.
import type { PlannedVideo } from '@slidock/types';

import { unstatedAvcC, unstatedSample } from './h264';
import { type Movie, MovieError, type MovieTrack, type Sample, playedSpan, readMovie } from './movie';
import { type AudioSample, type AudioTrackOut, type Chunk, type VideoSample, writeMp4 } from './mp4-writer';

/** The quality choices of the download dialog. */
export type VideoQuality = 'high' | 'medium' | 'small';

export const VIDEO_QUALITIES: readonly VideoQuality[] = ['high', 'medium', 'small'];

/** The bits each choice spends per pixel of a frame (at up to 30 frames a second: see `targetBitrate`). */
export const BITS_PER_PIXEL: Readonly<Record<VideoQuality, number>> = { high: 0.1, medium: 0.06, small: 0.035 };

/**
 * The bits per pixel every quality climbs while the encoder drops frames: each starts at its own rung and goes up
 * through the rungs above it, so a lower quality tries every bitrate a higher one does.
 */
export const BITS_PER_PIXEL_LADDER: readonly number[] = [0.035, 0.06, 0.1, 0.2];

/** The bitrate of a sound re-encoded to AAC, by choice. */
export const AUDIO_BITRATE: Readonly<Record<VideoQuality, number>> = { high: 160_000, medium: 128_000, small: 96_000 };

/** A key frame at least this often, so a player seeks (a trim's start, a rewind) quickly. */
export const KEY_FRAME_SECONDS = 2;

/** How much of the original is read at a time. */
const READ_BYTES = 4 * 1024 * 1024;

/** How many frames may wait in the decoder or the encoder before reading on. */
const QUEUE = 8;

/** The media data is interleaved in chunks of this many seconds. */
const INTERLEAVE_SECONDS = 1;

/** A video to re-encode: the plan's entry, its file and the quality. */
export interface VideoJob {
  plan: PlannedVideo;
  file: Blob;
  quality: VideoQuality;
}

/** How far a re-encode is: frames of the video track decoded. */
export interface VideoProgress {
  frames: number;
}

/** What became of a video. */
export type VideoOutcome =
  | {
      kind: 'compressed';
      file: Blob;
      /** `copied` (AAC carried over), `encoded` (re-encoded to AAC), `none`, or `removed` (never heard). */
      audio: 'copied' | 'encoded' | 'none' | 'removed';
      /** `encoded`, or `copied` as it is (its sound removed). */
      video: 'encoded' | 'copied';
      /** The encoder's codec string, or the original's format for a copy. */
      codec: string;
      /** In milliseconds, in the worker. */
      elapsed: number;
    }
  | { kind: 'kept'; reason: string; elapsed: number };

// What a re-encode needs of WebCodecs and canvases.

export interface FrameLike {
  readonly timestamp: number;
  readonly codedWidth: number;
  readonly codedHeight: number;
  close(): void;
}

export interface EncodedLike {
  readonly type: 'key' | 'delta';
  readonly timestamp: number;
  readonly byteLength: number;
  copyTo(destination: Uint8Array): void;
}

export interface AudioDataLike {
  readonly numberOfFrames: number;
  close(): void;
}

export interface CodecLike<Input> {
  configure(config: object): void;
  readonly encodeQueueSize?: number;
  readonly decodeQueueSize?: number;
  flush(): Promise<void>;
  close(): void;
  encode?(input: Input, options?: { keyFrame?: boolean }): void;
  decode?(input: Input): void;
}

export interface Metadata {
  decoderConfig?: { description?: AllowSharedBufferSource };
}

export interface VideoContextLike {
  imageSmoothingEnabled: boolean;
  imageSmoothingQuality: ImageSmoothingQuality;
  setTransform(a: number, b: number, c: number, d: number, e: number, f: number): void;
  drawImage(image: FrameLike, dx: number, dy: number, dw: number, dh: number): void;
}

export interface VideoCanvasLike {
  width: number;
  height: number;
  getContext(kind: '2d'): VideoContextLike | null;
}

/** The browser a re-encode runs in: a worker's globals, or fakes. */
export interface VideoEnvironment {
  decoderSupported(config: VideoDecoderConfig): Promise<boolean>;
  encoderSupported(config: VideoEncoderConfig): Promise<boolean>;
  audioEncoderSupported(config: AudioEncoderConfig): Promise<boolean>;
  audioDecoderSupported(config: AudioDecoderConfig): Promise<boolean>;
  videoDecoder(output: (frame: FrameLike) => void, error: (error: Error) => void): CodecLike<unknown>;
  videoEncoder(
    output: (chunk: EncodedLike, metadata?: Metadata) => void,
    error: (error: Error) => void,
  ): CodecLike<FrameLike>;
  audioDecoder(output: (data: AudioDataLike) => void, error: (error: Error) => void): CodecLike<unknown>;
  audioEncoder(
    output: (chunk: EncodedLike, metadata?: Metadata) => void,
    error: (error: Error) => void,
  ): CodecLike<AudioDataLike>;
  videoChunk(init: { type: 'key' | 'delta'; timestamp: number; duration: number; data: Uint8Array }): unknown;
  audioChunk(init: { type: 'key'; timestamp: number; data: Uint8Array }): unknown;
  /** Interleaved PCM as audio data. */
  audioData(init: {
    format: 'u8' | 's16' | 's32' | 'f32';
    sampleRate: number;
    numberOfFrames: number;
    numberOfChannels: number;
    timestamp: number;
    data: ArrayBuffer;
  }): AudioDataLike;
  /** A frame of `canvas`'s pixels. */
  canvasFrame(canvas: VideoCanvasLike, timestamp: number): FrameLike;
  /** `frame` at another time. */
  retimed(frame: FrameLike, timestamp: number): FrameLike;
  canvas(width: number, height: number): VideoCanvasLike;
  now(): number;
}

/** Whether this browser has the WebCodecs a re-encode needs. */
export function hasVideoEncoder(scope: typeof globalThis = globalThis): boolean {
  const names = ['VideoDecoder', 'VideoEncoder', 'EncodedVideoChunk', 'VideoFrame', 'OffscreenCanvas'];
  return names.every((name) => typeof (scope as unknown as Record<string, unknown>)[name] === 'function');
}

type Ctor<T> = new (init: never) => T;

/** The environment of a worker (or a page) of this browser. */
export function browserVideoEnvironment(scope: typeof globalThis = globalThis): VideoEnvironment {
  const g = scope as unknown as Record<
    string,
    Ctor<unknown> & { isConfigSupported?: (c: object) => Promise<{ supported?: boolean }> }
  >;
  const supported = (name: string) => async (config: object) => {
    try {
      return (await g[name]?.isConfigSupported?.(config))?.supported === true;
    } catch {
      return false;
    }
  };
  const make = <T>(name: string, init: object): T => new (g[name] as unknown as new (init: object) => T)(init);
  const frame = (source: unknown, init: object) =>
    new (g.VideoFrame as unknown as new (source: unknown, init: object) => FrameLike)(source, init);
  return {
    decoderSupported: supported('VideoDecoder'),
    encoderSupported: supported('VideoEncoder'),
    audioEncoderSupported: supported('AudioEncoder'),
    audioDecoderSupported: supported('AudioDecoder'),
    videoDecoder: (output, error) => make('VideoDecoder', { output, error }),
    videoEncoder: (output, error) => make('VideoEncoder', { output, error }),
    audioDecoder: (output, error) => make('AudioDecoder', { output, error }),
    audioEncoder: (output, error) => make('AudioEncoder', { output, error }),
    videoChunk: (init) => make('EncodedVideoChunk', init),
    audioChunk: (init) => make('EncodedAudioChunk', init),
    audioData: (init) => make('AudioData', init),
    canvasFrame: (canvas, timestamp) => frame(canvas, { timestamp }),
    retimed: (source, timestamp) => frame(source, { timestamp }),
    canvas: (width, height) => new OffscreenCanvas(width, height) as unknown as VideoCanvasLike,
    now: () => performance.now(),
  };
}

/** The video's average frame rate. */
export function frameRate(plan: PlannedVideo): number {
  const seconds = plan.durationMs / 1000;
  return seconds > 0 && plan.frames > 0 ? plan.frames / seconds : 30;
}

/**
 * The bitrate a quality spends on a video at its planned size: `BITS_PER_PIXEL` per pixel per frame up to 30 frames a
 * second, and half that per frame above (a faster video's frames differ less from one another).
 */
export function targetBitrate(plan: PlannedVideo, quality: VideoQuality): number {
  return pixelBitrate(plan, BITS_PER_PIXEL[quality]);
}

/** The bitrate of `bitsPerPixel` per pixel of `plan`'s frames at its planned size (see `targetBitrate`). */
function pixelBitrate(plan: PlannedVideo, bitsPerPixel: number): number {
  const fps = frameRate(plan);
  const effective = fps <= 30 ? fps : 30 + (fps - 30) / 2;
  return Math.round(bitsPerPixel * plan.targetWidth * plan.targetHeight * effective);
}

/**
 * The bitrates a quality tries, in order, while the encoder drops frames: its rung of `BITS_PER_PIXEL_LADDER` and
 * each rung above, each capped at the original's bits per pixel (`encodeBitrate`), up to the first that reaches the
 * cap: past it a re-encode could only grow.
 */
export function ladderBitrates(plan: PlannedVideo, quality: VideoQuality): number[] {
  const pixels = (plan.targetWidth * plan.targetHeight) / (plan.width * plan.height);
  const cap = sourceBitrate(plan) * pixels;
  const bitrates: number[] = [];
  for (const rung of BITS_PER_PIXEL_LADDER.filter((bits) => bits >= BITS_PER_PIXEL[quality])) {
    const bitrate = pixelBitrate(plan, rung);
    bitrates.push(Math.max(1, Math.round(Math.min(bitrate, cap))));
    if (bitrate >= cap) {
      break;
    }
  }
  return bitrates;
}

/**
 * The bitrate a re-encode asks the encoder for: the quality's (`targetBitrate`), and never more bits per pixel than
 * the original spends, so a video scaled down a little from an efficient original is not given more than it had.
 */
export function encodeBitrate(plan: PlannedVideo, quality: VideoQuality): number {
  return ladderBitrates(plan, quality)[0]!;
}

/** The original's video bitrate. */
export function sourceBitrate(plan: PlannedVideo): number {
  const seconds = plan.durationMs / 1000;
  return seconds > 0 ? (plan.videoBytes * 8) / seconds : Infinity;
}

/** The H.264 levels (ITU-T H.264 Table A-1): `level_idc`, frame size and macroblock rate limits. */
const LEVELS: readonly [number, number, number][] = [
  [0x1f, 3600, 108_000],
  [0x20, 5120, 216_000],
  [0x28, 8192, 245_760],
  [0x2a, 8704, 522_240],
  [0x32, 22_080, 589_824],
  [0x33, 36_864, 983_040],
  [0x34, 36_864, 2_073_600],
  [0x3c, 139_264, 4_177_920],
  [0x3d, 139_264, 8_355_840],
  [0x3e, 139_264, 16_711_680],
];

/** The High-profile codec strings for `width` × `height` at `fps`: the least level that fits it, then the larger. */
export function h264Codecs(width: number, height: number, fps: number): string[] {
  const macroblocks = Math.ceil(width / 16) * Math.ceil(height / 16);
  const fits = LEVELS.findIndex(([, frame, rate]) => macroblocks <= frame && macroblocks * fps <= rate);
  return LEVELS.slice(fits === -1 ? LEVELS.length : fits).map(
    ([level]) => `avc1.6400${level.toString(16).padStart(2, '0')}`,
  );
}

/** How long the latency probe waits for its frames. */
const PROBE_MS = 2000;

/** Whether this browser's encoder hands back frames in `quality` latency mode (asked once per environment). */
const latencies = new WeakMap<VideoEnvironment, Promise<LatencyMode>>();

/** The frames the latency probe encodes: WebKit's `quality` encoder takes four, then stalls. */
const PROBE_FRAMES = 8;

/**
 * The latency mode this browser's encoder works in: `quality` (which may look ahead) where a probe of
 * `PROBE_FRAMES` small frames comes back within `PROBE_MS`, else `realtime` (WebKit's `quality` encoder takes four
 * frames and hands back nothing).
 */
export function encoderLatency(env: VideoEnvironment): Promise<LatencyMode> {
  let known = latencies.get(env);
  if (!known) {
    known = probeLatency(env).catch(() => 'realtime' as const);
    latencies.set(env, known);
  }
  return known;
}

async function probeLatency(env: VideoEnvironment): Promise<LatencyMode> {
  const config: VideoEncoderConfig = {
    codec: 'avc1.64001f',
    width: 64,
    height: 64,
    bitrate: 100_000,
    framerate: 30,
    latencyMode: 'quality',
    avc: { format: 'avc' },
  };
  if (!(await env.encoderSupported(config))) {
    return 'realtime';
  }
  let outputs = 0;
  const encoder = env.videoEncoder(
    () => (outputs += 1),
    () => {},
  );
  try {
    encoder.configure(config);
    const canvas = env.canvas(64, 64);
    for (let index = 0; index < PROBE_FRAMES; index += 1) {
      const frame = env.canvasFrame(canvas, index * 33_333);
      encoder.encode!(frame, { keyFrame: index === 0 });
      frame.close();
    }
    await Promise.race([encoder.flush().catch(() => {}), new Promise((resolve) => setTimeout(resolve, PROBE_MS))]);
    return outputs === PROBE_FRAMES ? 'quality' : 'realtime';
  } finally {
    try {
      encoder.close();
    } catch {
      // Closed already.
    }
  }
}

/** The frames an encode may lose: none but the 1 % (or 2) a rounding of the edit list's ends may. */
export function droppedAllowed(submitted: number): number {
  return Math.max(2, Math.floor(submitted / 100));
}

/**
 * The encodes to try, in order, while the encoder drops frames (a hardware encoder's rate control drops frames to
 * stay under its bitrate; WebKit's realtime encoder drops half of a video's frames at the Small quality): `config` at
 * the first of `bitrates` (the quality's `ladderBitrates`); the browser's software encoder at that bitrate where it has
 * one (Chromium's keeps every frame); then `config` at each bitrate up the ladder.
 */
export async function encoderAttempts(
  config: VideoEncoderConfig,
  bitrates: readonly number[],
  env: Pick<VideoEnvironment, 'encoderSupported'>,
): Promise<VideoEncoderConfig[]> {
  const [first = config.bitrate ?? 0, ...rest] = bitrates;
  const attempts: VideoEncoderConfig[] = [{ ...config, bitrate: first }];
  const software: VideoEncoderConfig = { ...config, bitrate: first, hardwareAcceleration: 'prefer-software' };
  if (config.hardwareAcceleration !== 'prefer-software' && (await env.encoderSupported(software))) {
    attempts.push(software);
  }
  attempts.push(...rest.map((bitrate) => ({ ...config, bitrate })));
  return attempts;
}

/** The encoder settings for `plan`: the first codec string and acceleration this browser takes. */
export async function chooseEncoder(
  plan: PlannedVideo,
  bitrate: number,
  env: Pick<VideoEnvironment, 'encoderSupported'>,
  latencyMode: LatencyMode = 'quality',
): Promise<VideoEncoderConfig | string> {
  const fps = frameRate(plan);
  const codecs = h264Codecs(plan.targetWidth, plan.targetHeight, fps);
  if (codecs.length === 0) {
    return `${plan.targetWidth} × ${plan.targetHeight} at ${Math.round(fps)} fps is past every H.264 level`;
  }
  for (const codec of codecs) {
    for (const hardwareAcceleration of ['prefer-hardware', 'no-preference'] as const) {
      const config: VideoEncoderConfig = {
        codec,
        width: plan.targetWidth,
        height: plan.targetHeight,
        bitrate,
        bitrateMode: 'variable',
        framerate: fps,
        latencyMode,
        hardwareAcceleration,
        avc: { format: 'avc' },
      };
      if (await env.encoderSupported(config)) {
        return config;
      }
    }
  }
  return `this browser encodes no H.264 at ${plan.targetWidth} × ${plan.targetHeight}`;
}

function hex(value: number): string {
  return value.toString(16).padStart(2, '0');
}

/** The WebCodecs codec string of an `avcC` or `hvcC` (ISO/IEC 14496-15 Annex E). */
export function decoderCodec(format: string, config: { kind: string; bytes: Uint8Array } | null): string | null {
  const bytes = config?.bytes;
  if ((format === 'avc1' || format === 'avc3') && config?.kind === 'avcC' && bytes && bytes.length >= 4) {
    return `${format}.${hex(bytes[1]!)}${hex(bytes[2]!)}${hex(bytes[3]!)}`;
  }
  if ((format === 'hvc1' || format === 'hev1') && config?.kind === 'hvcC' && bytes && bytes.length >= 13) {
    const space = ['', 'A', 'B', 'C'][bytes[1]! >> 6]!;
    const tier = bytes[1]! & 0x20 ? 'H' : 'L';
    const profile = bytes[1]! & 0x1f;
    let compatibility = ((bytes[2]! << 24) | (bytes[3]! << 16) | (bytes[4]! << 8) | bytes[5]!) >>> 0;
    // The flags reversed: bit 31 of the field is the least significant bit of the string.
    let reversed = 0;
    for (let bit = 0; bit < 32; bit += 1) {
      reversed = ((reversed << 1) | (compatibility & 1)) >>> 0;
      compatibility >>>= 1;
    }
    const constraints = Array.from(bytes.subarray(6, 12));
    while (constraints.length > 0 && constraints.at(-1) === 0) {
      constraints.pop();
    }
    return [
      `${format}.${space}${profile}`,
      reversed.toString(16),
      `${tier}${bytes[12]}`,
      ...constraints.map((byte) => byte.toString(16).toUpperCase()),
    ].join('.');
  }
  return null;
}

/** What becomes of a video's sound: none, carried over, re-encoded from PCM or MP3, or a reason to keep the original. */
export type AudioRoute =
  | { kind: 'none' }
  | { kind: 'copy' }
  | { kind: 'pcm'; format: 'u8' | 's16' | 's32' | 'f32'; bytes: number; bigEndian: boolean; widen24: boolean }
  | { kind: 'decode'; codec: string }
  | { kind: 'keep'; reason: string };

/** QuickTime's `lpcm` flags (version 2 sound descriptions). */
const LPCM_FLOAT = 1;
const LPCM_BIG_ENDIAN = 2;

/** The way `track`'s sound is written into the re-encode (see the module comment). */
export function audioRoute(track: MovieTrack | null): AudioRoute {
  if (!track) {
    return { kind: 'none' };
  }
  const audio = track.audio;
  if (!audio) {
    return { kind: 'keep', reason: `its sound (${track.format}) cannot be read` };
  }
  const objectType = audio.objectType;
  if (track.format === 'mp4a' && objectType !== null && [0x40, 0x66, 0x67, 0x68].includes(objectType)) {
    return audio.specific ? { kind: 'copy' } : { kind: 'keep', reason: 'its AAC sound has no decoder configuration' };
  }
  if (track.format === 'mp4a' && (objectType === 0x69 || objectType === 0x6b)) {
    return { kind: 'decode', codec: 'mp3' };
  }
  const pcm = (bits: number, float: boolean, bigEndian: boolean): AudioRoute => {
    if (float && bits === 32) {
      return { kind: 'pcm', format: 'f32', bytes: 4, bigEndian, widen24: false };
    }
    if (!float && (bits === 16 || bits === 24 || bits === 32)) {
      return {
        kind: 'pcm',
        format: bits === 16 ? 's16' : 's32',
        bytes: bits / 8,
        bigEndian,
        widen24: bits === 24,
      };
    }
    return { kind: 'keep', reason: `its sound is ${bits}-bit ${float ? 'float' : 'integer'} PCM` };
  };
  switch (track.format) {
    case 'sowt':
      return pcm(audio.sampleSize, false, false);
    case 'twos':
      return pcm(audio.sampleSize, false, true);
    case 'lpcm':
      return pcm(
        audio.sampleSize,
        ((audio.pcmFlags ?? 0) & LPCM_FLOAT) !== 0,
        ((audio.pcmFlags ?? 0) & LPCM_BIG_ENDIAN) !== 0,
      );
    case 'in24':
      return pcm(24, false, true);
    case 'in32':
      return pcm(32, false, true);
    case 'fl32':
      return pcm(32, true, true);
    default:
      return { kind: 'keep', reason: `its sound (${track.format}) cannot be carried over` };
  }
}

/** Why `plan` stays as it is before anything is decoded, or `null`. */
export function keptBeforeDecoding(plan: PlannedVideo, quality: VideoQuality): string | null {
  if (plan.keep !== null) {
    return plan.keep;
  }
  const scaled = plan.targetWidth !== plan.width || plan.targetHeight !== plan.height;
  const bitrate = targetBitrate(plan, quality);
  if (!scaled && plan.rotation === 0 && sourceBitrate(plan) <= bitrate) {
    const mbps = (bits: number) => `${(bits / 1e6).toFixed(2)} Mbit/s`;
    return `its video runs at ${mbps(sourceBitrate(plan))}, no more than the ${mbps(bitrate)} chosen`;
  }
  return null;
}

class Cancelled extends Error {}

/** A sleep long enough for a codec to take its queue down. */
function breathe(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 1));
}

/** The video track's frames as decoded, drawn and encoded: the encoded samples, their bytes and the encoder's config. */
interface EncodedVideo {
  /** The frames handed to the encoder. */
  submitted: number;
  samples: VideoSample[];
  /** Presentation times in the track's timescale, by sample. */
  times: number[];
  /** Each sample's bytes, in order. */
  parts: Uint8Array[];
  avcC: Uint8Array;
}

/**
 * Reads `samples` of `file` in runs of samples one after the other in the file, each at most `limit` bytes, handing
 * each run's bytes to `use` with its first and last sample's index.
 */
export async function readRuns(
  file: Blob,
  samples: readonly Sample[],
  limit: number,
  use: (first: number, last: number, bytes: Uint8Array<ArrayBuffer>) => Promise<void> | void,
  signal: AbortSignal | undefined,
): Promise<void> {
  let index = 0;
  while (index < samples.length) {
    if (signal?.aborted) {
      throw new Cancelled();
    }
    const first = samples[index]!;
    let last = index;
    while (
      last + 1 < samples.length &&
      samples[last + 1]!.offset === samples[last]!.offset + samples[last]!.size &&
      samples[last + 1]!.offset + samples[last + 1]!.size - first.offset <= limit
    ) {
      last += 1;
    }
    const end = samples[last]!.offset + samples[last]!.size;
    await use(index, last, new Uint8Array(await file.slice(first.offset, end).arrayBuffer()));
    index = last + 1;
  }
}

/** Reads `samples` of `file` a few MiB at a time, handing each sample's bytes to `use` in order. */
function readSamples(
  file: Blob,
  samples: readonly Sample[],
  use: (sample: Sample, bytes: Uint8Array) => Promise<void> | void,
  signal: AbortSignal | undefined,
): Promise<void> {
  return readRuns(
    file,
    samples,
    READ_BYTES,
    async (first, last, bytes) => {
      const base = samples[first]!.offset;
      for (let at = first; at <= last; at += 1) {
        const sample = samples[at]!;
        await use(sample, bytes.subarray(sample.offset - base, sample.offset - base + sample.size));
      }
    },
    signal,
  );
}

/**
 * The AudioSpecificConfig of an AAC encoder's decoder description: the description itself, or, where a browser hands
 * over a whole ES descriptor (WebKit), its decoder specific information.
 */
export function audioSpecificConfig(description: Uint8Array): Uint8Array {
  if (description[0] !== 0x03) {
    return description;
  }
  const read = (at: number): { tag: number; start: number; end: number } | null => {
    let size = 0;
    let cursor = at + 1;
    for (let index = 0; index < 4 && cursor < description.length; index += 1) {
      const byte = description[cursor]!;
      cursor += 1;
      size = (size << 7) | (byte & 0x7f);
      if ((byte & 0x80) === 0) {
        break;
      }
    }
    return cursor + size <= description.length ? { tag: description[at]!, start: cursor, end: cursor + size } : null;
  };
  const es = read(0);
  if (!es) {
    return description;
  }
  const flags = description[es.start + 2] ?? 0;
  let at = es.start + 3 + (flags & 0x80 ? 2 : 0);
  if (flags & 0x40) {
    at += 1 + (description[at] ?? 0);
  }
  if (flags & 0x20) {
    at += 2;
  }
  const config = read(at);
  const specific = config && config.tag === 4 ? read(config.start + 13) : null;
  return specific && specific.tag === 5 ? description.slice(specific.start, specific.end) : description;
}

/** AAC frames: 1024 samples each. */
const AAC_FRAME = 1024;

/**
 * The priming samples an AAC encoder put before the sound, from how many more samples it wrote than it was given: the
 * 2112 of Apple's encoder (and of most others), else whole frames.
 */
export function aacPriming(encodedFrames: number, inputSamples: number): number {
  const extra = encodedFrames * AAC_FRAME - inputSamples;
  return extra >= 2112 ? 2112 : extra >= AAC_FRAME ? AAC_FRAME : 0;
}

/** Whether `plan`'s frames go to the encoder as decoded: at their size, upright, in BT.709's gamut. */
export function direct(plan: PlannedVideo): boolean {
  return (
    plan.rotation === 0 &&
    plan.targetWidth === plan.width &&
    plan.targetHeight === plan.height &&
    !plan.colour.wideGamut
  );
}

/** Whether the original states no colour (an untagged stream, no `colr`). */
export function untagged(plan: PlannedVideo): boolean {
  const { primaries, transfer, matrix } = plan.colour;
  return primaries === null && transfer === null && matrix === null;
}

/** The canvas draw that scales a coded frame to the planned size, upright. */
function drawer(plan: PlannedVideo, env: VideoEnvironment) {
  const canvas = env.canvas(plan.targetWidth, plan.targetHeight);
  const context = canvas.getContext('2d');
  if (!context) {
    throw new Error('the browser gave no 2D canvas');
  }
  context.imageSmoothingEnabled = true;
  context.imageSmoothingQuality = 'high';
  const [w, h] = [plan.targetWidth, plan.targetHeight];
  // The quarter turn about the canvas's corner, then the frame drawn at the turned size.
  const turns: Record<number, [number, number, number, number, number, number]> = {
    0: [1, 0, 0, 1, 0, 0],
    90: [0, 1, -1, 0, w, 0],
    180: [-1, 0, 0, -1, w, h],
    270: [0, -1, 1, 0, 0, h],
  };
  const quarter = plan.rotation % 180 === 90;
  return {
    canvas,
    draw(frame: FrameLike) {
      context.setTransform(...turns[plan.rotation]!);
      context.drawImage(frame, 0, 0, quarter ? h : w, quarter ? w : h);
    },
  };
}

async function encodeVideo(
  job: VideoJob,
  movie: Movie,
  track: MovieTrack,
  config: VideoEncoderConfig,
  decoderConfig: VideoDecoderConfig,
  env: VideoEnvironment,
  onProgress: (progress: VideoProgress) => void,
  signal: AbortSignal | undefined,
): Promise<EncodedVideo> {
  const { plan } = job;
  const span = playedSpan(track, movie.timescale);
  const micros = (tick: number) => Math.round((tick * 1e6) / track.timescale);
  const draw = direct(plan) ? null : drawer(plan, env);
  let failure: Error | null = null;
  const fail = (error: Error) => {
    failure ??= error;
  };
  const out: EncodedVideo = { submitted: 0, samples: [], times: [], parts: [], avcC: new Uint8Array() };
  const encoder = env.videoEncoder((chunk, metadata) => {
    const bytes = new Uint8Array(chunk.byteLength);
    chunk.copyTo(bytes);
    const time = Math.round((chunk.timestamp * track.timescale) / 1e6);
    if (out.times.length > 0 && time <= out.times.at(-1)!) {
      fail(new Error('the encoder reordered frames'));
    }
    out.times.push(time);
    out.samples.push({ size: bytes.length, duration: 0, sync: chunk.type === 'key' });
    out.parts.push(bytes);
    const description = metadata?.decoderConfig?.description;
    if (description && out.avcC.length === 0) {
      out.avcC = ArrayBuffer.isView(description)
        ? new Uint8Array(description.buffer, description.byteOffset, description.byteLength).slice()
        : new Uint8Array(description as ArrayBuffer).slice();
    }
  }, fail);
  encoder.configure(config);
  let lastKey = -Infinity;
  let decoded = 0;
  const decoder = env.videoDecoder((frame) => {
    decoded += 1;
    try {
      const cts = Math.round((frame.timestamp * track.timescale) / 1e6);
      if (cts < span.start || cts >= span.end || failure) {
        return;
      }
      const time = cts - span.start;
      const timestamp = micros(time);
      const keyFrame = timestamp - lastKey >= KEY_FRAME_SECONDS * 1e6;
      if (keyFrame) {
        lastKey = timestamp;
      }
      out.submitted += 1;
      if (draw) {
        draw.draw(frame);
        const drawn = env.canvasFrame(draw.canvas, timestamp);
        encoder.encode!(drawn, { keyFrame });
        drawn.close();
      } else if (frame.timestamp === timestamp) {
        encoder.encode!(frame, { keyFrame });
      } else {
        const retimed = env.retimed(frame, timestamp);
        encoder.encode!(retimed, { keyFrame });
        retimed.close();
      }
    } catch (error) {
      fail(error instanceof Error ? error : new Error(String(error)));
    } finally {
      frame.close();
    }
  }, fail);
  decoder.configure(decoderConfig);
  try {
    let lastReport = 0;
    await readSamples(
      job.file,
      track.samples,
      async (sample, bytes) => {
        while ((decoder.decodeQueueSize ?? 0) > QUEUE || (encoder.encodeQueueSize ?? 0) > QUEUE) {
          if (failure) {
            throw failure;
          }
          if (signal?.aborted) {
            throw new Cancelled();
          }
          await breathe();
        }
        if (failure) {
          throw failure;
        }
        decoder.decode!(
          env.videoChunk({
            type: sample.sync ? 'key' : 'delta',
            timestamp: micros(sample.cts),
            duration: micros(sample.duration),
            data: bytes,
          }),
        );
        if (decoded - lastReport >= 15) {
          lastReport = decoded;
          onProgress({ frames: decoded });
        }
      },
      signal,
    );
    await decoder.flush();
    await encoder.flush();
    if (failure) {
      throw failure;
    }
    onProgress({ frames: decoded });
  } finally {
    for (const codec of [decoder, encoder]) {
      try {
        codec.close();
      } catch {
        // Closed already, after an error.
      }
    }
  }
  if (out.samples.length === 0) {
    throw new Error('no frame was encoded');
  }
  if (out.avcC.length === 0) {
    throw new Error('the encoder gave no decoder configuration');
  }
  // Each frame lasts until the next; the last until the span's end.
  const end = Math.round(span.end - span.start);
  out.samples.forEach((sample, index) => {
    const next = out.times[index + 1] ?? Math.max(end, out.times[index]! + 1);
    sample.duration = Math.max(1, next - out.times[index]!);
  });
  // The first frame shows from the start (an empty edit's delay is the file's edit list's).
  if (out.times[0]! > 0) {
    out.samples[0]!.duration += out.times[0]!;
    out.times[0] = 0;
  }
  return out;
}

/** The original's AAC samples, carried over: their sizes and durations, and the ranges of the file they are. */
function copiedAudio(track: MovieTrack, movie: Movie): { out: AudioTrackOut; times: number[] } {
  const audio = track.audio!;
  const samples: AudioSample[] = track.samples.map((sample) => ({ size: sample.size, duration: sample.duration }));
  const totalBytes = samples.reduce((total, sample) => total + sample.size, 0);
  const seconds = track.duration / track.timescale || 1;
  const edits = track.edits.map((edit) => ({
    duration: Math.round((edit.duration * 1000) / movie.timescale),
    mediaTime: edit.mediaTime,
  }));
  const span = playedSpan(track, movie.timescale);
  return {
    out: {
      timescale: track.timescale,
      sampleRate: audio.sampleRate,
      channels: audio.channels,
      specific: audio.specific!,
      bitrate: Math.round((totalBytes * 8) / seconds),
      samples,
      edits,
    },
    times: track.samples.map((sample) => (sample.dts - span.start) / track.timescale + span.delay),
  };
}

/** Converts big-endian samples of `bytes` each `size` wide to little-endian in place; 24-bit ones widen to 32. */
function littleEndian(
  bytes: Uint8Array<ArrayBuffer>,
  size: number,
  bigEndian: boolean,
  widen24: boolean,
): Uint8Array<ArrayBuffer> {
  if (widen24) {
    const out = new Uint8Array((bytes.length / 3) * 4);
    for (let at = 0, to = 0; at + 2 < bytes.length; at += 3, to += 4) {
      const [a, b, c] = bigEndian
        ? [bytes[at + 2]!, bytes[at + 1]!, bytes[at]!]
        : [bytes[at]!, bytes[at + 1]!, bytes[at + 2]!];
      out[to] = 0;
      out[to + 1] = a;
      out[to + 2] = b;
      out[to + 3] = c;
    }
    return out;
  }
  if (bigEndian) {
    for (let at = 0; at + size <= bytes.length; at += size) {
      bytes.subarray(at, at + size).reverse();
    }
  }
  return bytes;
}

/** The original's sound re-encoded to AAC: from its PCM as it is, or through the browser's decoder. */
async function encodedAudio(
  job: VideoJob,
  track: MovieTrack,
  movie: Movie,
  route: Extract<AudioRoute, { kind: 'pcm' | 'decode' }>,
  env: VideoEnvironment,
  signal: AbortSignal | undefined,
): Promise<{ out: AudioTrackOut; parts: Uint8Array[]; times: number[] }> {
  const audio = track.audio!;
  const bitrate = AUDIO_BITRATE[job.quality];
  const config: AudioEncoderConfig = {
    codec: 'mp4a.40.2',
    sampleRate: audio.sampleRate,
    numberOfChannels: audio.channels,
    bitrate,
  };
  if (!(await env.audioEncoderSupported(config))) {
    throw new Error(`this browser encodes no AAC at ${audio.sampleRate} Hz, ${audio.channels} channels`);
  }
  let failure: Error | null = null;
  const fail = (error: Error) => {
    failure ??= error;
  };
  const parts: Uint8Array[] = [];
  const times: number[] = [];
  let specific: Uint8Array | null = null;
  const encoder = env.audioEncoder((chunk, metadata) => {
    const bytes = new Uint8Array(chunk.byteLength);
    chunk.copyTo(bytes);
    parts.push(bytes);
    times.push(chunk.timestamp / 1e6);
    const description = metadata?.decoderConfig?.description;
    if (description && !specific) {
      specific = audioSpecificConfig(
        ArrayBuffer.isView(description)
          ? new Uint8Array(description.buffer, description.byteOffset, description.byteLength).slice()
          : new Uint8Array(description as ArrayBuffer).slice(),
      );
    }
  }, fail);
  encoder.configure(config);
  /** The sound's samples handed to the encoder. */
  let frames = 0;
  const encode = (data: AudioDataLike) => {
    try {
      frames += data.numberOfFrames;
      encoder.encode!(data);
    } finally {
      data.close();
    }
  };
  let decoder: CodecLike<unknown> | null = null;
  const wait = async () => {
    while ((encoder.encodeQueueSize ?? 0) > QUEUE * 4 || (decoder?.decodeQueueSize ?? 0) > QUEUE * 4) {
      await breathe();
    }
    if (failure) {
      throw failure;
    }
  };
  try {
    if (route.kind === 'decode') {
      const decoderConfig: AudioDecoderConfig = {
        codec: route.codec,
        sampleRate: audio.sampleRate,
        numberOfChannels: audio.channels,
      };
      if (!(await env.audioDecoderSupported(decoderConfig))) {
        throw new Error(`this browser decodes no ${route.codec} sound`);
      }
      const active = env.audioDecoder(encode, fail);
      decoder = active;
      active.configure(decoderConfig);
      await readSamples(
        job.file,
        track.samples,
        async (sample, bytes) => {
          await wait();
          active.decode!(
            env.audioChunk({ type: 'key', timestamp: Math.round((sample.dts / track.timescale) * 1e6), data: bytes }),
          );
        },
        signal,
      );
      await active.flush();
    } else {
      // PCM: a second of sound at a time.
      const width = route.widen24 ? 4 : route.bytes;
      const frameBytes = route.bytes * audio.channels;
      await readRuns(
        job.file,
        track.samples,
        Math.max(frameBytes, audio.sampleRate * frameBytes),
        async (_first, _last, bytes) => {
          await wait();
          const pcm = littleEndian(bytes, route.bytes, route.bigEndian, route.widen24);
          const numberOfFrames = Math.floor(pcm.length / (width * audio.channels));
          if (numberOfFrames > 0) {
            encode(
              env.audioData({
                format: route.format,
                sampleRate: audio.sampleRate,
                numberOfFrames,
                numberOfChannels: audio.channels,
                timestamp: Math.round((frames / audio.sampleRate) * 1e6),
                data: pcm.slice(0, numberOfFrames * width * audio.channels).buffer,
              }),
            );
          }
        },
        signal,
      );
    }
    await encoder.flush();
    if (failure) {
      throw failure;
    }
  } finally {
    for (const codec of [decoder, encoder]) {
      try {
        codec?.close();
      } catch {
        // Closed already.
      }
    }
  }
  if (!specific || parts.length === 0) {
    throw new Error('the AAC encoder gave no sound');
  }
  const span = playedSpan(track, movie.timescale);
  const edits = encodedEdits(track, movie.timescale, audio.sampleRate, frames, aacPriming(parts.length, frames));
  return {
    out: {
      timescale: audio.sampleRate,
      sampleRate: audio.sampleRate,
      channels: audio.channels,
      specific,
      bitrate,
      samples: parts.map((part) => ({ size: part.length, duration: AAC_FRAME })),
      edits,
    },
    parts,
    times: times.map((time) => time - span.start / track.timescale + span.delay),
  };
}

/**
 * The edit list of a sound re-encoded from its whole media (`frames` at `sampleRate`, after the encoder's `priming`
 * samples): the original's, each media time moved to the new timescale past the priming, so the file plays the span
 * the original plays; without one, the whole sound.
 */
export function encodedEdits(
  track: Pick<MovieTrack, 'edits' | 'timescale'>,
  movieTimescale: number,
  sampleRate: number,
  frames: number,
  priming: number,
): { duration: number; mediaTime: number }[] {
  if (track.edits.length === 0) {
    return [{ duration: Math.round((frames / sampleRate) * 1000), mediaTime: priming }];
  }
  return track.edits.map((edit) => ({
    duration: Math.round((edit.duration * 1000) / movieTimescale),
    mediaTime: edit.mediaTime < 0 ? -1 : Math.round((edit.mediaTime * sampleRate) / track.timescale) + priming,
  }));
}

/** A sound's samples for the media data: their presentation times in seconds and the bytes of a run of them. */
interface AudioParts {
  times: number[];
  bytes(start: number, end: number): Blob;
}

/** Chunks of `INTERLEAVE_SECONDS`: each second's video samples, then its sound's. */
export function interleave(
  video: { times: number[]; timescale: number; bytes: (start: number, end: number) => Blob },
  audio: AudioParts | null,
): Chunk[] {
  const chunks: Chunk[] = [];
  const videoSeconds = video.times.map((time) => time / video.timescale);
  const last = Math.max(videoSeconds.at(-1) ?? 0, audio?.times.at(-1) ?? 0);
  const windows = Math.ceil(last / INTERLEAVE_SECONDS) + 1;
  let v = 0;
  let a = 0;
  for (let window = 1; window <= windows; window += 1) {
    const end = window === windows ? Infinity : window * INTERLEAVE_SECONDS;
    const first = v;
    while (v < videoSeconds.length && videoSeconds[v]! < end) {
      v += 1;
    }
    if (v > first) {
      chunks.push({ track: 'video', samples: v - first, bytes: video.bytes(first, v) });
    }
    if (audio) {
      const start = a;
      while (a < audio.times.length && audio.times[a]! < end) {
        a += 1;
      }
      if (a > start) {
        chunks.push({ track: 'audio', samples: a - start, bytes: audio.bytes(start, a) });
      }
    }
  }
  return chunks;
}

/** The bytes of a run of the original's `samples`, sliced from `file` where one follows another. */
function copiedBytes(file: Blob, samples: readonly Sample[]): (start: number, end: number) => Blob {
  return (start, end) => {
    const parts: Blob[] = [];
    let run = start;
    for (let index = start + 1; index <= end; index += 1) {
      const previous = samples[index - 1]!;
      if (index === end || samples[index]!.offset !== previous.offset + previous.size) {
        parts.push(file.slice(samples[run]!.offset, previous.offset + previous.size));
        run = index;
      }
    }
    return new Blob(parts);
  };
}

/** Re-encodes `job`'s video at its planned size and quality, or says why the original stays. */
export async function compressVideo(
  job: VideoJob,
  env: VideoEnvironment,
  onProgress: (progress: VideoProgress) => void = () => {},
  signal?: AbortSignal,
): Promise<VideoOutcome> {
  const start = env.now();
  const outcome = await reencoded(job, env, onProgress, signal, start);
  const { plan } = job;
  if (outcome.kind === 'compressed' || !plan.silent || signal?.aborted) {
    return outcome;
  }
  // Kept: a sound never heard is still left out, the video copied as it is.
  const copy = plan.uncopyable === null ? await copiedWithoutSound(job) : plan.uncopyable;
  if (typeof copy === 'string') {
    return {
      ...outcome,
      reason: `${outcome.reason}; its sound, never heard, stays: ${copy}`,
      elapsed: env.now() - start,
    };
  }
  onProgress({ frames: plan.frames });
  return {
    kind: 'compressed',
    file: copy,
    audio: 'removed',
    video: 'copied',
    codec: plan.codec,
    elapsed: env.now() - start,
  };
}

/**
 * The original's video track copied as it is (its samples by `Blob` slices, its sample description, matrix,
 * composition offsets and edit list), without its sound: or why it cannot be.
 */
export async function copiedWithoutSound(job: VideoJob): Promise<Blob | string> {
  let movie: Movie;
  try {
    movie = await readMovie(job.file);
  } catch (error) {
    return error instanceof MovieError ? `its movie cannot be read (${error.message})` : String(error);
  }
  const tracks = movie.tracks.filter((candidate) => candidate.handler === 'vide');
  const track = tracks[0];
  if (tracks.length !== 1 || !track?.visual || track.descriptions !== 1 || track.samples.length === 0) {
    return 'its video track is not one of one description';
  }
  if (track.edits.some((edit) => edit.mediaTime >= 0 && edit.rate !== 1)) {
    return 'its edit list changes the rate';
  }
  const toMovie = (duration: number) => Math.round((duration * 1000) / movie.timescale);
  const samples = track.samples;
  const file = writeMp4(
    {
      timescale: track.timescale,
      width: track.visual.width,
      height: track.visual.height,
      avcC: new Uint8Array(),
      entry: track.visual.entry,
      matrix: track.matrix,
      presented: [track.width, track.height],
      samples: samples.map((sample) => ({
        size: sample.size,
        duration: sample.duration,
        sync: sample.sync,
        offset: sample.cts - sample.dts,
      })),
      edits: track.edits.map((edit) => ({ duration: toMovie(edit.duration), mediaTime: edit.mediaTime })),
      movieDuration: toMovie(movie.duration),
    },
    null,
    interleave(
      { times: samples.map((sample) => sample.dts), timescale: track.timescale, bytes: copiedBytes(job.file, samples) },
      null,
    ),
  );
  if (file.size >= job.plan.bytes) {
    return `the copy (${file.size} bytes) is not smaller than the original (${job.plan.bytes})`;
  }
  return file;
}

/** The re-encode, or why the original stays. */
async function reencoded(
  job: VideoJob,
  env: VideoEnvironment,
  onProgress: (progress: VideoProgress) => void,
  signal: AbortSignal | undefined,
  start: number,
): Promise<VideoOutcome> {
  const kept = (reason: string): VideoOutcome => ({ kind: 'kept', reason, elapsed: env.now() - start });
  const { plan } = job;
  const early = keptBeforeDecoding(plan, job.quality);
  if (early !== null) {
    return kept(early);
  }
  let movie: Movie;
  try {
    movie = await readMovie(job.file);
  } catch (error) {
    return kept(error instanceof MovieError ? `its movie cannot be read (${error.message})` : String(error));
  }
  const track = movie.tracks.find((candidate) => candidate.handler === 'vide');
  const sound = movie.tracks.find((candidate) => candidate.handler === 'soun') ?? null;
  if (!track?.visual) {
    return kept('its movie has no video track');
  }
  const codec = decoderCodec(track.format, track.visual.config);
  if (codec === null) {
    return kept(`this app decodes no ${track.format} video`);
  }
  const decoderConfig: VideoDecoderConfig = {
    codec,
    codedWidth: track.visual.width,
    codedHeight: track.visual.height,
    ...(track.visual.config ? { description: track.visual.config.bytes } : {}),
  };
  if (!(await env.decoderSupported(decoderConfig))) {
    return kept(`this browser decodes no ${codec} video`);
  }
  const bitrate = encodeBitrate(plan, job.quality);
  const encoder = await chooseEncoder(plan, bitrate, env, await encoderLatency(env));
  if (typeof encoder === 'string') {
    return kept(encoder);
  }
  // A sound never heard is left out.
  const route: AudioRoute = plan.silent ? { kind: 'none' } : audioRoute(sound);
  if (route.kind === 'keep') {
    return kept(route.reason);
  }
  try {
    let video: EncodedVideo | null = null;
    let dropped = '';
    for (const config of await encoderAttempts(encoder, ladderBitrates(plan, job.quality), env)) {
      const attempt = await encodeVideo(job, movie, track, config, decoderConfig, env, onProgress, signal);
      if (attempt.submitted - attempt.samples.length <= droppedAllowed(attempt.submitted)) {
        video = attempt;
        break;
      }
      dropped = `the browser's encoder dropped ${attempt.submitted - attempt.samples.length} of ${attempt.submitted} frames`;
    }
    if (video === null) {
      return kept(dropped);
    }
    if (direct(plan) && untagged(plan)) {
      video.avcC = unstatedAvcC(video.avcC);
      video.parts = video.parts.map((part, index) => (video.samples[index]!.sync ? unstatedSample(part) : part));
      video.parts.forEach((part, index) => (video.samples[index]!.size = part.length));
    }
    let audioOut: AudioTrackOut | null = null;
    let audioParts: AudioParts | null = null;
    if (route.kind === 'copy' && sound) {
      const copied = copiedAudio(sound, movie);
      audioOut = copied.out;
      audioParts = { times: copied.times, bytes: copiedBytes(job.file, sound.samples) };
    } else if ((route.kind === 'pcm' || route.kind === 'decode') && sound) {
      const encoded = await encodedAudio(job, sound, movie, route, env, signal);
      audioOut = encoded.out;
      audioParts = {
        times: encoded.times,
        bytes: (start, end) => new Blob(encoded.parts.slice(start, end) as BlobPart[]),
      };
    }
    // The original's leading empty edit delays the video as it delays the original's.
    const delay = playedSpan(track, movie.timescale).delay;
    const length = video.samples.reduce((sum, sample) => sum + sample.duration, 0) / track.timescale;
    const encodedParts = video.parts;
    const file = writeMp4(
      {
        timescale: track.timescale,
        width: plan.targetWidth,
        height: plan.targetHeight,
        avcC: video.avcC,
        samples: video.samples,
        edits:
          delay > 0
            ? [
                { duration: Math.round(delay * 1000), mediaTime: -1 },
                { duration: Math.round(length * 1000), mediaTime: 0 },
              ]
            : [],
        // As long as the original, its sound left out.
        ...(plan.silent ? { movieDuration: Math.round(plan.durationMs) } : {}),
      },
      audioOut,
      interleave(
        {
          times: video.times.map((time) => time + delay * track.timescale),
          timescale: track.timescale,
          bytes: (start, end) => new Blob(encodedParts.slice(start, end) as BlobPart[]),
        },
        audioParts,
      ),
    );
    if (file.size >= plan.bytes) {
      return kept(`the new file (${file.size} bytes) is not smaller than the original (${plan.bytes})`);
    }
    return {
      kind: 'compressed',
      file,
      audio: plan.silent ? 'removed' : route.kind === 'copy' ? 'copied' : route.kind === 'none' ? 'none' : 'encoded',
      video: 'encoded',
      codec: encoder.codec,
      elapsed: env.now() - start,
    };
  } catch (error) {
    if (error instanceof Cancelled) {
      return kept('the re-encode was cancelled');
    }
    return kept(`the re-encode failed (${error instanceof Error ? error.message : String(error)})`);
  }
}
