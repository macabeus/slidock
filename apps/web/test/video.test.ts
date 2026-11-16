import type { PlannedVideo } from '@slidock/types';
import { describe, expect, it } from 'vitest';

import { type Timers, type VideoPort, workerVideoCompressor } from '../src/media/compressor';
import { type MovieTrack, readMovie } from '../src/media/movie';
import { writeMp4 } from '../src/media/mp4-writer';
import {
  type AudioDataLike,
  type CodecLike,
  type EncodedLike,
  type FrameLike,
  type Metadata,
  type VideoEnvironment,
  type VideoOutcome,
  aacPriming,
  audioRoute,
  audioSpecificConfig,
  chooseEncoder,
  compressVideo,
  decoderCodec,
  droppedAllowed,
  encodeBitrate,
  encodedEdits,
  encoderLatency,
  h264Codecs,
  keptBeforeDecoding,
  ladderBitrates,
  readRuns,
  targetBitrate,
} from '../src/media/video';

const AVCC = Uint8Array.from([1, 0x64, 0, 0x1f, 0xff, 0xe1, 0, 4, 0x67, 0x64, 0, 0x1f, 1, 0, 2, 0x68, 0xee]);
const ASC = Uint8Array.from([0x11, 0x90]);

function plan(extra: Partial<PlannedVideo> = {}): PlannedVideo {
  return {
    path: 'media/a.mp4',
    mime: 'video/mp4',
    bytes: 1_000_000,
    codec: 'avc1',
    width: 1280,
    height: 720,
    rotation: 0,
    durationMs: 4000,
    frames: 120,
    videoBytes: 950_000,
    audio: null,
    colour: { primaries: 1, transfer: 1, matrix: 1, fullRange: false, wideGamut: false },
    targetWidth: 640,
    targetHeight: 360,
    keep: null,
    silent: false,
    uncopyable: null,
    ...extra,
  };
}

/** A source movie of `frames` 1280 × 720 frames at 30 fps (a key frame every 30), with `sound` AAC frames. */
async function source(frames = 120, sound = 0, frameBytes = 8000, delay = 0): Promise<Blob> {
  const delayed = (edits: { duration: number; mediaTime: number }[]) =>
    delay > 0 ? [{ duration: delay, mediaTime: -1 }, ...edits] : edits;
  const video = Array.from({ length: frames }, (_, index) => ({
    size: frameBytes,
    duration: 3000,
    sync: index % 30 === 0,
  }));
  const audio = Array.from({ length: sound }, () => ({ size: 50, duration: 1024 }));
  const file = writeMp4(
    {
      timescale: 90_000,
      width: 1280,
      height: 720,
      avcC: AVCC,
      samples: video,
      edits: delayed(delay > 0 ? [{ duration: (frames * 1000) / 30, mediaTime: 0 }] : []),
    },
    sound > 0
      ? {
          timescale: 48_000,
          sampleRate: 48_000,
          channels: 2,
          specific: ASC,
          bitrate: 128_000,
          samples: audio,
          edits: delayed([{ duration: Math.round((sound * 1024 * 1000) / 48_000) - 21, mediaTime: 1024 }]),
        }
      : null,
    [
      { track: 'video', samples: frames, bytes: new Blob([new Uint8Array(frames * frameBytes).fill(7)]) },
      ...(sound > 0
        ? [{ track: 'audio' as const, samples: sound, bytes: new Blob([new Uint8Array(sound * 50).fill(9)]) }]
        : []),
    ],
  );
  return file;
}

class FakeFrame implements FrameLike {
  closed = false;
  constructor(
    readonly timestamp: number,
    readonly codedWidth = 1280,
    readonly codedHeight = 720,
  ) {}
  close() {
    this.closed = true;
  }
}

interface Fakes {
  /** The browser decodes this codec. */
  decodes?: boolean;
  /** The browser encodes H.264 at these codec strings (all by default). */
  encodes?: (codec: string) => boolean;
  /** The encoder's `quality` mode hands back frames. */
  quality?: boolean;
  /** Bytes per encoded frame. */
  frameBytes?: number;
  /** The encoder says its colour. */
  colour?: VideoColorSpaceInit;
  aac?: boolean;
  /** The encoder drops every other frame under this config. */
  drops?: (config: VideoEncoderConfig) => boolean;
}

function fakeEnvironment(options: Fakes = {}) {
  const log = {
    decoded: 0,
    encoded: [] as { timestamp: number; keyFrame: boolean }[],
    configs: [] as object[],
    drawn: 0,
    closed: 0,
    audioData: 0,
  };
  const encoder = <Input>(
    output: (chunk: EncodedLike, metadata?: Metadata) => void,
    onFrame: (input: Input, keyFrame: boolean) => { timestamp: number; bytes: number; description?: Uint8Array },
  ): CodecLike<Input> => {
    let described = false;
    let quality = false;
    let dropping = false;
    let count = 0;
    const pending: (() => void)[] = [];
    return {
      encodeQueueSize: 0,
      configure: (config) => {
        log.configs.push(config);
        quality = (config as { latencyMode?: string }).latencyMode === 'quality';
        dropping =
          (config as { width?: number }).width !== 64 && (options.drops?.(config as VideoEncoderConfig) ?? false);
      },
      encode(input, opts) {
        const { timestamp, bytes, description } = onFrame(input, opts?.keyFrame === true);
        count += 1;
        if (dropping && count % 2 === 0) {
          return;
        }
        const emit = () => {
          const metadata: Metadata | undefined =
            !described && description
              ? { decoderConfig: { description, ...(options.colour ? { colorSpace: options.colour } : {}) } }
              : undefined;
          described = true;
          output(
            {
              type: opts?.keyFrame ? 'key' : 'delta',
              timestamp,
              byteLength: bytes,
              copyTo: (destination) => destination.fill(1),
            },
            metadata,
          );
        };
        if (quality && options.quality === false) {
          // WebKit: frames taken, nothing handed back.
          pending.push(emit);
          return;
        }
        emit();
      },
      flush: () => (quality && options.quality === false ? new Promise(() => {}) : Promise.resolve()),
      close: () => {
        log.closed += 1;
      },
    };
  };
  const env: VideoEnvironment = {
    decoderSupported: async () => options.decodes !== false,
    encoderSupported: async (config) => options.encodes?.(config.codec) ?? true,
    audioEncoderSupported: async () => options.aac !== false,
    audioDecoderSupported: async () => true,
    videoDecoder: (output) => ({
      decodeQueueSize: 0,
      configure: (config) => log.configs.push(config),
      decode: (chunk) => {
        log.decoded += 1;
        output(new FakeFrame((chunk as { timestamp: number }).timestamp));
      },
      flush: () => Promise.resolve(),
      close: () => {
        log.closed += 1;
      },
    }),
    videoEncoder: (output) =>
      encoder<FrameLike>(output, (frame, keyFrame) => {
        if (frame.codedWidth !== 64) {
          log.encoded.push({ timestamp: frame.timestamp, keyFrame });
        }
        return { timestamp: frame.timestamp, bytes: options.frameBytes ?? 1000, description: AVCC };
      }),
    audioDecoder: (output) => ({
      configure: () => {},
      decode: () => output({ numberOfFrames: 1152, close: () => {} }),
      flush: () => Promise.resolve(),
      close: () => {},
    }),
    audioEncoder: (output) => {
      let frames = 0;
      let at = 0;
      return encoder<AudioDataLike>(output, (data) => {
        log.audioData += 1;
        frames += data.numberOfFrames;
        const timestamp = (at * 1024 * 1e6) / 44_100;
        at += 1;
        void frames;
        return { timestamp, bytes: 200, description: Uint8Array.from([0x12, 0x10]) };
      });
    },
    videoChunk: (init) => init,
    audioChunk: (init) => init,
    audioData: (init) => ({ numberOfFrames: init.numberOfFrames, close: () => {} }),
    canvasFrame: (canvas, timestamp) => {
      // The latency probe's 64 × 64 frames are not the video's.
      log.drawn += canvas.width === 64 ? 0 : 1;
      return new FakeFrame(timestamp, canvas.width, canvas.height);
    },
    retimed: (frame, timestamp) => new FakeFrame(timestamp, frame.codedWidth, frame.codedHeight),
    canvas: (width, height) => ({
      width,
      height,
      getContext: () => ({
        imageSmoothingEnabled: false,
        imageSmoothingQuality: 'low',
        setTransform: () => {},
        drawImage: () => {},
      }),
    }),
    now: () => 0,
  };
  return { env, log };
}

describe('the encoder settings', () => {
  it('names the least H.264 level that fits the size and frame rate, then the larger ones', () => {
    expect(h264Codecs(1280, 720, 30)[0]).toBe('avc1.64001f');
    expect(h264Codecs(1920, 1080, 30)[0]).toBe('avc1.640028');
    expect(h264Codecs(1920, 1080, 60)[0]).toBe('avc1.64002a');
    expect(h264Codecs(3840, 2160, 24)[0]).toBe('avc1.640033');
    expect(h264Codecs(3390, 2106, 57)[0]).toBe('avc1.640034');
    expect(h264Codecs(3390, 2106, 120)[0]).toBe('avc1.64003c');
    expect(h264Codecs(1280, 720, 30).at(-1)).toBe('avc1.64003e');
    expect(h264Codecs(16_384, 16_384, 30)).toEqual([]);
  });

  it('spends the quality’s bits per pixel, half as many per frame above 30 frames a second', () => {
    expect(targetBitrate(plan(), 'high')).toBe(Math.round(0.1 * 640 * 360 * 30));
    expect(targetBitrate(plan({ frames: 240 }), 'medium')).toBe(Math.round(0.06 * 640 * 360 * 45));
  });

  it('asks for no more bits per pixel than the original spends', () => {
    // 950,000 bytes over 4 s: 1.9 Mbit/s at 1280 × 720, so 475 kbit/s at a quarter of the pixels.
    expect(encodeBitrate(plan(), 'high')).toBe(475_000);
    expect(encodeBitrate(plan({ videoBytes: 50_000_000 }), 'high')).toBe(targetBitrate(plan(), 'high'));
  });

  it('takes the first codec and acceleration the browser encodes, or says none fits', async () => {
    const tried: string[] = [];
    const config = await chooseEncoder(plan(), 1_000_000, {
      encoderSupported: async (c) => {
        tried.push(`${c.codec} ${c.hardwareAcceleration}`);
        return c.codec === 'avc1.640020' && c.hardwareAcceleration === 'no-preference';
      },
    });
    expect(config).toMatchObject({ codec: 'avc1.640020', width: 640, height: 360, bitrateMode: 'variable' });
    expect(tried.slice(0, 4)).toEqual([
      'avc1.64001f prefer-hardware',
      'avc1.64001f no-preference',
      'avc1.640020 prefer-hardware',
      'avc1.640020 no-preference',
    ]);
    expect(await chooseEncoder(plan(), 1, { encoderSupported: async () => false })).toBe(
      'this browser encodes no H.264 at 640 × 360',
    );
  });

  it('works in quality latency where the encoder hands frames back, else realtime (asked once)', async () => {
    const webkit = fakeEnvironment({ quality: false });
    expect(await encoderLatency(webkit.env)).toBe('realtime');
    const chrome = fakeEnvironment();
    expect(await encoderLatency(chrome.env)).toBe('quality');
    const probes = chrome.log.configs.length;
    expect(await encoderLatency(chrome.env)).toBe('quality');
    expect(chrome.log.configs.length).toBe(probes);
  }, 10_000);

  it('names the decoder of an avcC and an hvcC', () => {
    expect(decoderCodec('avc1', { kind: 'avcC', bytes: AVCC })).toBe('avc1.64001f');
    // Apple's HEVC: Main, compatible with Main and Main 10, level 6.0 (`L180`), progressive source.
    const hvcc = Uint8Array.from([1, 0x01, 0x60, 0, 0, 0, 0xb0, 0, 0, 0, 0, 0, 180, 0xf0]);
    expect(decoderCodec('hvc1', { kind: 'hvcC', bytes: hvcc })).toBe('hvc1.1.6.L180.B0');
    expect(decoderCodec('mp4v', null)).toBeNull();
  });
});

describe('what stays as it is', () => {
  it('keeps what the plan keeps, and a video already at or under the bitrate at its own size', () => {
    expect(keptBeforeDecoding(plan({ keep: 'it has an alpha channel' }), 'high')).toBe('it has an alpha channel');
    const small = plan({ targetWidth: 1280, targetHeight: 720, videoBytes: 100_000 });
    expect(keptBeforeDecoding(small, 'high')).toMatch(/runs at 0.20 Mbit\/s, no more than the 2.76 Mbit\/s chosen/);
    // Scaled down or turned, it is re-encoded whatever its bitrate.
    expect(keptBeforeDecoding({ ...small, targetWidth: 640, targetHeight: 360 }, 'high')).toBeNull();
    expect(keptBeforeDecoding({ ...small, rotation: 90 }, 'high')).toBeNull();
    expect(keptBeforeDecoding(plan(), 'small')).toBeNull();
  });

  it('carries AAC over, re-encodes PCM and MP3, and keeps any other sound', () => {
    const track = (format: string, audio: Partial<NonNullable<MovieTrack['audio']>> | null): MovieTrack => ({
      id: 2,
      handler: 'soun',
      timescale: 48_000,
      duration: 48_000,
      edits: [],
      format,
      descriptions: 1,
      visual: null,
      audio:
        audio === null
          ? null
          : {
              channels: 2,
              sampleSize: 16,
              sampleRate: 48_000,
              objectType: null,
              specific: null,
              pcmFlags: null,
              frameBytes: null,
              ...audio,
            },
      samples: [],
      matrix: [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x40000000],
      width: 0,
      height: 0,
    });
    expect(audioRoute(null)).toEqual({ kind: 'none' });
    expect(audioRoute(track('mp4a', { objectType: 0x40, specific: ASC }))).toEqual({ kind: 'copy' });
    expect(audioRoute(track('mp4a', { objectType: 0x40 }))).toMatchObject({ kind: 'keep' });
    expect(audioRoute(track('mp4a', { objectType: 0x6b }))).toEqual({ kind: 'decode', codec: 'mp3' });
    expect(audioRoute(track('sowt', {}))).toEqual({
      kind: 'pcm',
      format: 's16',
      bytes: 2,
      bigEndian: false,
      widen24: false,
    });
    expect(audioRoute(track('twos', {}))).toMatchObject({ format: 's16', bigEndian: true });
    expect(audioRoute(track('in24', {}))).toMatchObject({ format: 's32', bytes: 3, widen24: true });
    expect(audioRoute(track('lpcm', { sampleSize: 32, pcmFlags: 1 }))).toMatchObject({
      format: 'f32',
      bigEndian: false,
    });
    expect(audioRoute(track('alac', {}))).toEqual({ kind: 'keep', reason: 'its sound (alac) cannot be carried over' });
    expect(audioRoute(track('sowt', { sampleSize: 8 }))).toMatchObject({ kind: 'keep' });
    expect(audioRoute(track('mp4a', null))).toMatchObject({ kind: 'keep' });
  });

  it('carries the original sound’s edit list over to its AAC re-encode, past the priming', () => {
    // QuickTime at 600 a second: a 1 s delay, then 2 s from 0.5 s into the sound (44.1 kHz).
    const edits = [
      { duration: 600, mediaTime: -1, rate: 1 },
      { duration: 1200, mediaTime: 22_050, rate: 1 },
    ];
    expect(encodedEdits({ edits, timescale: 44_100 }, 600, 44_100, 176_400, 2112)).toEqual([
      { duration: 1000, mediaTime: -1 },
      { duration: 2000, mediaTime: 24_162 },
    ]);
    // Without one: the whole sound past the priming.
    expect(encodedEdits({ edits: [], timescale: 44_100 }, 600, 44_100, 70_560, 2112)).toEqual([
      { duration: 1600, mediaTime: 2112 },
    ]);
  });

  it('reads the AudioSpecificConfig out of the ES descriptor WebKit hands over, and the priming', () => {
    expect(audioSpecificConfig(Uint8Array.from([0x12, 0x10]))).toEqual(Uint8Array.from([0x12, 0x10]));
    const webkit = Uint8Array.from([
      0x03, 0x80, 0x80, 0x80, 0x22, 0, 0, 0, 0x04, 0x80, 0x80, 0x80, 0x14, 0x40, 0x14, 0, 0x18, 0, 0, 0, 0, 0, 0, 0, 0,
      0, 0x05, 0x80, 0x80, 0x80, 0x02, 0x12, 0x10, 0x06, 0x80, 0x80, 0x80, 0x01, 0x02,
    ]);
    expect(audioSpecificConfig(webkit)).toEqual(Uint8Array.from([0x12, 0x10]));
    // 1.6 s at 44.1 kHz: Apple's 2112 priming samples and padding to whole frames.
    expect(aacPriming(71, 70_560)).toBe(2112);
    expect(aacPriming(70, 70_560)).toBe(1024);
    expect(aacPriming(69, 70_560)).toBe(0);
  });
});

describe('compressVideo', () => {
  it('re-encodes every frame at the planned size, keeps the times, and carries the AAC sound over', async () => {
    const file = await source(120, 188);
    const { env, log } = fakeEnvironment({
      colour: { primaries: 'bt709', transfer: 'bt709', matrix: 'bt709', fullRange: false },
    });
    const progress: number[] = [];
    const outcome = await compressVideo(
      {
        plan: plan({
          bytes: file.size,
          audio: {
            codec: 'mp4a',
            objectType: 64,
            aac: true,
            channels: 2,
            sampleRate: 48_000,
            sampleSize: 16,
            bytes: 32_000,
          },
        }),
        file,
        quality: 'high',
      },
      env,
      ({ frames }) => progress.push(frames),
    );
    expect(outcome.kind).toBe('compressed');
    const { file: out, audio } = outcome as Extract<VideoOutcome, { kind: 'compressed' }>;
    expect(audio).toBe('copied');
    // Drawn on a canvas at the planned size: scaled down.
    expect(log.drawn).toBe(120);
    expect(log.encoded).toHaveLength(120);
    // A key frame every 2 s.
    expect(log.encoded.filter(({ keyFrame }) => keyFrame).map(({ timestamp }) => timestamp)).toEqual([0, 2_000_000]);
    expect(progress.at(-1)).toBe(120);
    expect(log.closed).toBeGreaterThanOrEqual(2);

    const movie = await readMovie(out);
    const [video, sound] = movie.tracks;
    // The stream states its colour; the file has no colr box (WebKit plays one of its encoder's shifted).
    expect(new TextDecoder('latin1').decode(new Uint8Array(await out.arrayBuffer())).includes('colr')).toBe(false);
    expect([video!.format, video!.visual?.width, video!.visual?.height]).toEqual(['avc1', 640, 360]);
    expect(video!.timescale).toBe(90_000);
    expect(video!.samples.map((sample) => sample.cts)).toEqual(Array.from({ length: 120 }, (_, index) => index * 3000));
    expect(video!.samples.filter((sample) => sample.sync)).toHaveLength(2);
    expect(movie.duration / movie.timescale).toBeCloseTo(4, 2);
    // The sound as it was: its samples' bytes, its edit list.
    expect([sound!.format, sound!.audio?.objectType, sound!.audio?.specific]).toEqual(['mp4a', 0x40, ASC]);
    expect(sound!.samples).toHaveLength(188);
    expect(sound!.edits).toEqual([{ duration: 3990, mediaTime: 1024, rate: 1 }]);
    const first = sound!.samples[0]!;
    expect(new Uint8Array(await out.slice(first.offset, first.offset + first.size).arrayBuffer())).toEqual(
      new Uint8Array(50).fill(9),
    );
    // Interleaved by the second: video, sound, video, sound…
    const offsets = [
      ...video!.samples.map((s) => ['v', s.offset] as const),
      ...sound!.samples.map((s) => ['a', s.offset] as const),
    ]
      .sort((a, b) => a[1] - b[1])
      .map(([kind]) => kind)
      .join('')
      .replace(/(.)\1+/g, '$1');
    expect(offsets).toBe('vavavava');
  });

  it('encodes frames at their size and upright as decoded, retimed to the edit list', async () => {
    const file = await source(60);
    const { env, log } = fakeEnvironment();
    const outcome = await compressVideo(
      {
        plan: plan({ bytes: file.size, frames: 60, durationMs: 2000, targetWidth: 1280, targetHeight: 720 }),
        file,
        quality: 'high',
      },
      env,
    );
    expect(outcome.kind).toBe('compressed');
    expect(log.drawn).toBe(0);
    expect(log.encoded).toHaveLength(60);
    // Encoded as decoded, the original stating its colour: the file has no colr box, the encoder's stream as it is.
    const bytes = new Uint8Array(await (outcome as Extract<VideoOutcome, { kind: 'compressed' }>).file.arrayBuffer());
    expect(new TextDecoder('latin1').decode(bytes).includes('colr')).toBe(false);
  });

  it('keeps the original where the browser decodes or encodes neither, or the file is not smaller', async () => {
    const file = await source(30, 0, 100);
    const job = { plan: plan({ bytes: file.size, frames: 30, durationMs: 1000 }), file, quality: 'high' as const };
    expect(await compressVideo(job, fakeEnvironment({ decodes: false }).env)).toMatchObject({
      kind: 'kept',
      reason: 'this browser decodes no avc1.64001f video',
    });
    expect(await compressVideo(job, fakeEnvironment({ encodes: () => false }).env)).toMatchObject({
      kind: 'kept',
      reason: 'this browser encodes no H.264 at 640 × 360',
    });
    expect(await compressVideo(job, fakeEnvironment({ frameBytes: 5000 }).env)).toMatchObject({
      kind: 'kept',
      reason: expect.stringMatching(/is not smaller than the original/),
    });
    expect(await compressVideo({ ...job, file: new Blob([new Uint8Array(64)]) }, fakeEnvironment().env)).toMatchObject({
      kind: 'kept',
      reason: expect.stringMatching(/^its movie cannot be read/),
    });
  });

  it('tries the software encoder, then up the bitrate ladder, while the encoder drops frames, else keeps the original', async () => {
    const file = await source(60);
    const job = { plan: plan({ bytes: file.size, frames: 60, durationMs: 2000 }), file, quality: 'high' as const };
    const hardware = fakeEnvironment({ drops: (config) => config.hardwareAcceleration !== 'prefer-software' });
    expect(await compressVideo(job, hardware.env)).toMatchObject({ kind: 'compressed' });
    const tried = (log: { configs: object[] }) =>
      log.configs
        .filter((config) => 'bitrate' in config && (config as VideoEncoderConfig).width === 640)
        .map((config) => (config as VideoEncoderConfig).hardwareAcceleration);
    expect(tried(hardware.log)).toEqual(['prefer-hardware', 'prefer-software']);
    const always = fakeEnvironment({ drops: () => true });
    expect(await compressVideo(job, always.env)).toEqual({
      kind: 'kept',
      reason: "the browser's encoder dropped 30 of 60 frames",
      elapsed: 0,
    });
    const bitrates = always.log.configs
      .filter((config) => (config as VideoEncoderConfig).width === 640)
      .map((config) => (config as VideoEncoderConfig).bitrate);
    // 0.1 bits per pixel, then the original's bits per pixel (950 kbit/s at a quarter of its pixels), short of 0.2.
    expect(ladderBitrates(job.plan, 'high')).toEqual([691_200, 950_000]);
    expect(bitrates).toEqual([691_200, 691_200, 950_000]);
    expect(droppedAllowed(60)).toBe(2);
    expect(droppedAllowed(4000)).toBe(40);
  });

  it('climbs one bitrate ladder at every quality: a lower quality tries every bitrate a higher one does', async () => {
    // No cap; a cap between two rungs; a cap under every rung.
    for (const videoBytes of [500_000_000, 400_000, 10_000]) {
      const video = plan({ videoBytes });
      const [high, medium, small] = (['high', 'medium', 'small'] as const).map((quality) =>
        ladderBitrates(video, quality),
      );
      for (const ladder of [high!, medium!, small!]) {
        expect([...ladder].sort((a, b) => a - b)).toEqual(ladder);
      }
      expect(medium!.slice(-high!.length)).toEqual(high);
      expect(small!.slice(-medium!.length)).toEqual(medium);
    }
    expect(ladderBitrates(plan({ videoBytes: 500_000_000 }), 'small')).toEqual([241_920, 414_720, 691_200, 1_382_400]);
    // Where the encoder drops frames under some bitrate, Small re-encodes every video Medium does.
    const file = await source(60);
    for (const floor of [300_000, 500_000, 800_000, 1_500_000]) {
      const outcome = async (quality: 'medium' | 'small') =>
        (
          await compressVideo(
            { plan: plan({ bytes: file.size, frames: 60, durationMs: 2000, videoBytes: 50_000_000 }), file, quality },
            fakeEnvironment({ drops: (config) => (config.bitrate ?? 0) < floor }).env,
          )
        ).kind;
      const [medium, small] = [await outcome('medium'), await outcome('small')];
      expect(medium === 'kept' || small === 'compressed', `floor ${floor}: ${medium}, ${small}`).toBe(true);
    }
  });

  it('keeps the original’s leading empty edit on the video as on its sound', async () => {
    // Both tracks start 1 s late.
    const file = await source(120, 188, 8000, 1000);
    const audio = {
      codec: 'mp4a',
      objectType: 64,
      aac: true,
      channels: 2,
      sampleRate: 48_000,
      sampleSize: 16,
      bytes: 32_000,
    };
    const outcome = await compressVideo(
      { plan: plan({ bytes: file.size, durationMs: 5000, audio }), file, quality: 'high' },
      fakeEnvironment().env,
    );
    expect(outcome.kind).toBe('compressed');
    const [video, sound] = (await readMovie((outcome as Extract<VideoOutcome, { kind: 'compressed' }>).file)).tracks;
    expect(video!.edits).toEqual([
      { duration: 1000, mediaTime: -1, rate: 1 },
      { duration: 4000, mediaTime: 0, rate: 1 },
    ]);
    expect(video!.samples[0]!.cts).toBe(0);
    expect(sound!.edits[0]).toEqual({ duration: 1000, mediaTime: -1, rate: 1 });
  });

  it('stops when cancelled, closing the codecs', async () => {
    const file = await source(120);
    const { env, log } = fakeEnvironment();
    const controller = new AbortController();
    controller.abort();
    const outcome = await compressVideo(
      { plan: plan({ bytes: file.size }), file, quality: 'high' },
      env,
      () => {},
      controller.signal,
    );
    expect(outcome).toMatchObject({ kind: 'kept', reason: 'the re-encode was cancelled' });
    expect(log.encoded).toHaveLength(0);
    expect(log.closed).toBeGreaterThanOrEqual(2);
  });

  it('reads PCM a second at a time: runs of samples one after the other, not one sample each', async () => {
    const samples = Array.from({ length: 96_000 }, (_, index) => ({
      offset: index * 4,
      size: 4,
      dts: index,
      cts: index,
      duration: 1,
      sync: true,
    }));
    const runs: [number, number, number][] = [];
    await readRuns(
      new Blob([new Uint8Array(96_000 * 4)]),
      samples,
      48_000 * 4,
      (first, last, bytes) => {
        runs.push([first, last, bytes.length]);
      },
      undefined,
    );
    expect(runs).toEqual([
      [0, 47_999, 192_000],
      [48_000, 95_999, 192_000],
    ]);
  });
});

describe('a sound never heard', () => {
  const AAC = {
    codec: 'mp4a',
    objectType: 64,
    aac: true,
    channels: 2,
    sampleRate: 48_000,
    sampleSize: 16,
    bytes: 9400,
  };

  /** A mirrored 1280 × 720 movie of B-frames (composition offsets), its video edited to start a frame in, with AAC. */
  async function bframes(): Promise<Blob> {
    const offsets = [3000, 9000, 0, 0];
    const samples = Array.from({ length: 60 }, (_, index) => ({
      size: 100 + index,
      duration: 3000,
      sync: index % 30 === 0,
      offset: offsets[index % 4]!,
    }));
    const sound = Array.from({ length: 94 }, () => ({ size: 100, duration: 1024 }));
    return writeMp4(
      {
        timescale: 90_000,
        width: 1280,
        height: 720,
        avcC: AVCC,
        matrix: [-0x10000, 0, 0, 0, 0x10000, 0, 1280 * 0x10000, 0, 0x40000000],
        samples,
        edits: [{ duration: 1967, mediaTime: 3000 }],
      },
      {
        timescale: 48_000,
        sampleRate: 48_000,
        channels: 2,
        specific: ASC,
        bitrate: 128_000,
        samples: sound,
        edits: [],
      },
      [
        {
          track: 'video',
          samples: 60,
          bytes: new Blob(samples.map((sample, index) => new Uint8Array(sample.size).fill(index))),
        },
        { track: 'audio', samples: 94, bytes: new Blob([new Uint8Array(9400).fill(9)]) },
      ],
    );
  }

  it('is left out of a re-encode, the file as long as the original', async () => {
    const file = await source(120, 188);
    const job = {
      plan: plan({ bytes: file.size, durationMs: 4500, audio: AAC, silent: true }),
      file,
      quality: 'high' as const,
    };
    const outcome = await compressVideo(job, fakeEnvironment().env);
    expect(outcome).toMatchObject({ kind: 'compressed', audio: 'removed', video: 'encoded' });
    const movie = await readMovie((outcome as Extract<VideoOutcome, { kind: 'compressed' }>).file);
    expect(movie.tracks.map((track) => track.handler)).toEqual(['vide']);
    expect(movie.duration / movie.timescale).toBeCloseTo(4.5, 2);
    // Heard: carried over.
    const heard = await compressVideo({ ...job, plan: { ...job.plan, silent: false } }, fakeEnvironment().env);
    expect(heard).toMatchObject({ kind: 'compressed', audio: 'copied' });
    expect((await readMovie((heard as Extract<VideoOutcome, { kind: 'compressed' }>).file)).tracks).toHaveLength(2);
  });

  it('is left out of a video kept otherwise, its video copied as it is', async () => {
    const file = await bframes();
    const original = await readMovie(file);
    const source = original.tracks.find((track) => track.handler === 'vide')!;
    // At its own size and under the bitrate: kept, unless its sound is never heard.
    const own = plan({
      bytes: file.size,
      frames: 60,
      durationMs: 2000,
      videoBytes: 9000,
      targetWidth: 1280,
      targetHeight: 720,
      audio: AAC,
    });
    const { env, log } = fakeEnvironment();
    expect(await compressVideo({ plan: own, file, quality: 'high' }, env)).toMatchObject({
      kind: 'kept',
      reason: expect.stringMatching(/no more than the/),
    });
    const progress: number[] = [];
    const outcome = await compressVideo({ plan: { ...own, silent: true }, file, quality: 'high' }, env, ({ frames }) =>
      progress.push(frames),
    );
    expect(outcome).toMatchObject({ kind: 'compressed', audio: 'removed', video: 'copied', codec: 'avc1' });
    expect(log.encoded).toHaveLength(0);
    expect(progress).toEqual([60]);
    const out = (outcome as Extract<VideoOutcome, { kind: 'compressed' }>).file;
    expect(out.size).toBeLessThan(file.size - 9000);
    const movie = await readMovie(out);
    expect(movie.tracks).toHaveLength(1);
    const [video] = movie.tracks;
    // Its description, matrix, composition offsets, key frames, edit list and length, as the original's.
    expect(video!.visual!.entry).toEqual(source.visual!.entry);
    expect(video!.matrix).toEqual(source.matrix);
    expect([video!.width, video!.height]).toEqual([1280, 720]);
    expect(video!.samples.map(({ cts, dts, sync, size }) => [cts, dts, sync, size])).toEqual(
      source.samples.map(({ cts, dts, sync, size }) => [cts, dts, sync, size]),
    );
    expect(video!.edits).toEqual(source.edits);
    // As written: the mirror, and B-frames shown after the frames decoded after them.
    expect(video!.matrix).toEqual([-0x10000, 0, 0, 0, 0x10000, 0, 1280 * 0x10000, 0, 0x40000000]);
    expect(video!.samples.slice(0, 4).map(({ cts, dts }) => cts - dts)).toEqual([3000, 9000, 0, 0]);
    expect(movie.duration / movie.timescale).toBeCloseTo(original.duration / original.timescale, 2);
    // Its samples' bytes.
    for (const index of [0, 1, 59]) {
      const sample = video!.samples[index]!;
      expect(new Uint8Array(await out.slice(sample.offset, sample.offset + sample.size).arrayBuffer())).toEqual(
        new Uint8Array(100 + index).fill(index),
      );
    }
    // Kept by the plan for what a copy carries as it is: copied too.
    const alpha = { ...own, silent: true, keep: 'it has an alpha channel, which H.264 cannot carry' };
    expect(await compressVideo({ plan: alpha, file, quality: 'high' }, env)).toMatchObject({ video: 'copied' });
  });

  it('stays where the plan says a copy does not carry the video, or a cancel stopped it', async () => {
    const file = await bframes();
    const edits = plan({
      bytes: file.size,
      frames: 60,
      audio: AAC,
      silent: true,
      keep: 'its edit list plays 2 pieces',
      uncopyable: 'its edit list plays 2 pieces',
    });
    expect(await compressVideo({ plan: edits, file, quality: 'high' }, fakeEnvironment().env)).toMatchObject({
      kind: 'kept',
      reason: 'its edit list plays 2 pieces; its sound, never heard, stays: its edit list plays 2 pieces',
    });
    const controller = new AbortController();
    controller.abort();
    const own = { ...edits, keep: null, uncopyable: null, videoBytes: 9000, targetWidth: 1280, targetHeight: 720 };
    expect(
      await compressVideo({ plan: own, file, quality: 'high' }, fakeEnvironment().env, () => {}, controller.signal),
    ).toMatchObject({ kind: 'kept' });
  });
});

describe('workerVideoCompressor', () => {
  class FakePort implements VideoPort {
    sent: { id: number }[] = [];
    terminated = false;
    onmessage: VideoPort['onmessage'] = null;
    onerror: VideoPort['onerror'] = null;
    postMessage(message: { id: number }) {
      this.sent.push(message);
    }
    terminate() {
      this.terminated = true;
    }
    reply(data: object) {
      this.onmessage?.({ data } as MessageEvent);
    }
  }

  function fakeTimers() {
    const pending = new Map<number, () => void>();
    let next = 1;
    const timers: Timers = {
      setTimeout: (run) => {
        pending.set(next, run);
        return next++;
      },
      clearTimeout: (timer) => void pending.delete(timer as number),
    };
    return { timers, fire: () => [...pending.values()].forEach((run) => run()), pending };
  }

  it('passes progress on, waits while it comes, and gives a stalled video up', async () => {
    const ports: FakePort[] = [];
    const { timers, fire, pending } = fakeTimers();
    const compressor = workerVideoCompressor(
      () => {
        const port = new FakePort();
        ports.push(port);
        return port;
      },
      1,
      60_000,
      timers,
    );
    const progress: number[] = [];
    const job = { plan: plan(), file: new Blob(), quality: 'high' as const };
    const first = compressor.compress(job, ({ frames }) => progress.push(frames));
    const second = compressor.compress(job);
    expect(ports).toHaveLength(1);
    const started = [...pending.keys()];
    ports[0]!.reply({ id: 1, progress: { frames: 30 } });
    ports[0]!.reply({ id: 1, progress: { frames: 60 } });
    expect(progress).toEqual([30, 60]);
    // Each progress restarts the clock: the first timer is gone, one new one pending.
    expect(pending.size).toBe(1);
    expect([...pending.keys()]).not.toEqual(started);
    ports[0]!.reply({ id: 1, outcome: { kind: 'kept', reason: 'not smaller', elapsed: 1 } });
    expect(await first).toMatchObject({ reason: 'not smaller' });
    fire();
    expect(await second).toEqual({ kind: 'kept', reason: 're-encoding made no progress for 60 s', elapsed: 0 });
    expect(ports[0]!.terminated).toBe(true);
  });

  it('dispose (a cancel) stops the workers and keeps every pending video', async () => {
    const ports: FakePort[] = [];
    const compressor = workerVideoCompressor(() => {
      const port = new FakePort();
      ports.push(port);
      return port;
    }, 1);
    const job = { plan: plan(), file: new Blob(), quality: 'high' as const };
    const running = compressor.compress(job);
    const queued = compressor.compress(job);
    compressor.dispose();
    const stopped = { kind: 'kept', reason: 'the video workers were stopped', elapsed: 0 };
    expect(await running).toEqual(stopped);
    expect(await queued).toEqual(stopped);
    expect(ports[0]!.terminated).toBe(true);
    expect(await compressor.compress(job)).toEqual(stopped);
  });
});
