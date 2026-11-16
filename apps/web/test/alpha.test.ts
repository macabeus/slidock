import type { PlannedVideo } from '@slidock/types';
import { describe, expect, it } from 'vitest';

import {
  type AlphaContextLike,
  type AlphaEncoderEnvironment,
  type AlphaEnvironment,
  type AlphaSink,
  type AlphaSource,
  type AlphaSpec,
  type TakenFrame,
  alphaBitrates,
  alphaCodecs,
  alphaCopier,
  alphaSchedule,
  i420Bytes,
  makeAlphaCopy,
  splitRgba,
  startAlphaEncode,
} from '../src/media/alpha';
import type { MovieTrack } from '../src/media/movie';
import { writeMp4 } from '../src/media/mp4-writer';
import type { CodecLike, EncodedLike, FrameLike } from '../src/media/video';
import { vint, writeWebm } from '../src/media/webm-writer';

function plan(extra: Partial<PlannedVideo> = {}): PlannedVideo {
  return {
    path: 'media/a.mov',
    mime: 'video/quicktime',
    bytes: 1_000_000,
    codec: 'hvc1',
    width: 640,
    height: 480,
    rotation: 0,
    durationMs: 1000,
    frames: 30,
    videoBytes: 950_000,
    audio: null,
    colour: { primaries: 1, transfer: 1, matrix: 1, fullRange: false, wideGamut: false },
    targetWidth: 320,
    targetHeight: 240,
    keep: 'it has an alpha channel, which H.264 cannot carry',
    silent: false,
    uncopyable: null,
    ...extra,
  };
}

function track(cts: number[], extra: Partial<MovieTrack> = {}): MovieTrack {
  return {
    id: 1,
    handler: 'vide',
    timescale: 600,
    duration: 600,
    edits: [],
    format: 'hvc1',
    descriptions: 1,
    visual: { width: 640, height: 480, depth: 24, config: null, entry: new Uint8Array() },
    audio: null,
    matrix: [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x40000000],
    width: 640,
    height: 480,
    samples: cts.map((time, index) => ({
      offset: index * 10,
      size: 10,
      dts: index * 20,
      cts: time,
      duration: 20,
      sync: index === 0,
    })),
    ...extra,
  };
}

describe('alphaSchedule', () => {
  it('takes each frame at the middle of its time, in presentation order, its time in ms from the start', () => {
    // Out of decode order (B-frames), at 30 fps in a 600 timescale.
    const schedule = alphaSchedule(track([0, 40, 20, 60], { duration: 80 }), 600);
    expect(schedule.map((frame) => frame.timestamp)).toEqual([0, 33, 67, 100]);
    expect(schedule[0]!.seek).toBeCloseTo(1 / 60, 6);
    expect(schedule[3]!.seek).toBeCloseTo(0.1 + 1 / 60, 6);
  });

  it('plays the edit list: a leading empty edit delays every frame, frames outside the edit are dropped', () => {
    const edited = track([0, 20, 40, 60, 80], {
      edits: [
        { duration: 500, mediaTime: -1, rate: 1 },
        { duration: 50, mediaTime: 20, rate: 1 },
      ],
    });
    // Movie timescale 1000: a 0.5 s delay, then 50 ms of media from 20 (1/30 s): two frames.
    const schedule = alphaSchedule(edited, 1000);
    expect(schedule.map((frame) => frame.timestamp)).toEqual([500, 533]);
    expect(schedule[0]!.seek).toBeCloseTo(0.5 + 1 / 60, 6);
  });

  it('keeps the later of two frames within a millisecond', () => {
    const close = track([0, 0.2, 20], { duration: 40 });
    expect(alphaSchedule(close, 600).map((frame) => frame.timestamp)).toEqual([0, 33]);
  });
});

describe('codecs and bitrates', () => {
  it('asks VP9 at the least level that fits, then VP8', () => {
    expect(alphaCodecs(192, 144, 30)).toEqual(['vp09.00.10.08', 'vp8']);
    expect(alphaCodecs(320, 240, 30)).toEqual(['vp09.00.20.08', 'vp8']);
    expect(alphaCodecs(1920, 1080, 30)).toEqual(['vp09.00.40.08', 'vp8']);
    expect(alphaCodecs(1920, 1080, 60)).toEqual(['vp09.00.41.08', 'vp8']);
    expect(alphaCodecs(2598, 1934, 57)).toEqual(['vp09.00.50.08', 'vp8']);
  });

  it("gives the colour the original's bits per pixel at the planned size, the alpha a share", () => {
    // 950 000 bytes over 1 s is 7.6 Mbit/s, a quarter of the pixels.
    expect(alphaBitrates(plan())).toEqual({ colour: 1_900_000, alpha: 285_000 });
  });
});

describe('splitRgba', () => {
  it('writes BT.709 limited-range colour and the alpha as luma, chroma neutral', () => {
    const [w, h] = [2, 2];
    // Opaque white, opaque black, half-transparent red, clear.
    const rgba = Uint8Array.from([255, 255, 255, 255, 0, 0, 0, 255, 255, 0, 0, 128, 0, 0, 0, 0]);
    const colour = new Uint8Array(i420Bytes(w, h));
    const alpha = new Uint8Array(i420Bytes(w, h));
    expect(splitRgba(rgba, w, h, colour, alpha)).toBe(2);
    expect(Array.from(colour.subarray(0, 4))).toEqual([235, 16, 63, 16]);
    // The chroma of the 2 × 2 block: of its mean colour (straight: the clear pixel is black).
    const [r, g, b] = [(255 + 255) / 4, 255 / 4, 255 / 4];
    const cb = 128 + ((-0.2126 * r - 0.7152 * g) / 1.8556 + 0.5 * b) * (224 / 255);
    const cr = 128 + (0.5 * r + (-0.7152 * g - 0.0722 * b) / 1.5748) * (224 / 255);
    expect(Math.abs(colour[4]! - cb)).toBeLessThanOrEqual(0.5);
    expect(Math.abs(colour[5]! - cr)).toBeLessThanOrEqual(0.5);
    expect(Array.from(alpha)).toEqual([255, 255, 128, 0, 128, 128]);
  });

  it('counts no pixel of an opaque picture', () => {
    const rgba = new Uint8Array(4 * 4 * 4).fill(255);
    expect(splitRgba(rgba, 4, 4, new Uint8Array(i420Bytes(4, 4)), new Uint8Array(i420Bytes(4, 4)))).toBe(0);
  });
});

/** The top-level elements of a WebM and the segment's children, as `[id, size]`. */
function webmElements(bytes: Uint8Array): { ids: number[]; segment: number[]; text: string } {
  const read = (at: number, marker: boolean): [number, number] => {
    const first = bytes[at]!;
    let length = 1;
    while (length <= 8 && !(first & (0x80 >> (length - 1)))) {
      length += 1;
    }
    let value = marker ? first : first & (0xff >> length);
    for (let i = 1; i < length; i += 1) {
      value = value * 256 + bytes[at + i]!;
    }
    return [value, length];
  };
  const walk = (start: number, end: number) => {
    const ids: { id: number; body: number; end: number }[] = [];
    let at = start;
    while (at < end) {
      const [id, idLength] = read(at, true);
      const [size, sizeLength] = read(at + idLength, false);
      const body = at + idLength + sizeLength;
      ids.push({ id, body, end: body + size });
      at = body + size;
    }
    return ids;
  };
  const top = walk(0, bytes.length);
  const segment = top.find((element) => element.id === 0x18538067)!;
  return {
    ids: top.map((element) => element.id),
    segment: walk(segment.body, segment.end).map((element) => element.id),
    text: new TextDecoder('latin1').decode(bytes),
  };
}

describe('writeWebm', () => {
  it('writes the EBML header, then a segment of seek head, info, tracks, cues and a cluster per key frame', async () => {
    const frame = (timestamp: number, key: boolean) => ({
      timestamp,
      key,
      data: new Uint8Array(5).fill(1),
      alpha: new Uint8Array(3).fill(2),
    });
    const blob = writeWebm({
      video: {
        codec: 'V_VP9',
        width: 320,
        height: 240,
        alpha: true,
        colour: { matrix: 1, transfer: 1, primaries: 1, fullRange: false },
        frames: [frame(0, true), frame(40, false), frame(2000, true), frame(2040, false)],
      },
      sound: null,
      duration: 2080,
    });
    expect(blob.type).toBe('video/webm');
    const { ids, segment, text } = webmElements(new Uint8Array(await blob.arrayBuffer()));
    expect(ids).toEqual([0x1a45dfa3, 0x18538067]);
    expect(segment).toEqual([0x114d9b74, 0x1549a966, 0x1654ae6b, 0x1c53bb6b, 0x1f43b675, 0x1f43b675]);
    expect(text).toContain('webm');
    expect(text).toContain('V_VP9');
  });

  it('refuses a video that does not start with a key frame', () => {
    expect(() =>
      writeWebm({
        video: {
          codec: 'V_VP9',
          width: 2,
          height: 2,
          alpha: false,
          colour: null,
          frames: [{ timestamp: 0, key: false, data: new Uint8Array(1), alpha: null }],
        },
        sound: null,
        duration: 40,
      }),
    ).toThrow(/key frame/);
  });

  it('writes sizes as EBML variable-length integers', () => {
    expect(vint(0)).toEqual([0x80]);
    expect(vint(126)).toEqual([0xfe]);
    // 127 is all ones in one byte: "unknown size", so it takes two.
    expect(vint(127)).toEqual([0x40, 0x7f]);
    expect(vint(5, 8)).toEqual([1, 0, 0, 0, 0, 0, 0, 5]);
  });
});

class FakeFrame implements FrameLike {
  closed = false;
  constructor(
    readonly timestamp: number,
    readonly codedWidth = 320,
    readonly codedHeight = 240,
    /** Its pixels' alpha. */
    readonly alpha = 128,
  ) {}
  close() {
    this.closed = true;
  }
}

/** An encoder environment that encodes every frame, a key frame when asked, unless `drop` says otherwise. */
function fakeEncoders(options: { supports?: (codec: string) => boolean; drop?: number; noKey?: boolean } = {}) {
  const log = {
    configs: [] as VideoEncoderConfig[],
    encoded: [] as { timestamp: number; key: boolean; colour: boolean }[],
  };
  let alpha = 255;
  const env: AlphaEncoderEnvironment = {
    encoderSupported: (config) => Promise.resolve(options.supports?.(config.codec) ?? true),
    videoEncoder: (output) => {
      let queue = 0;
      const encoder: CodecLike<FrameLike> = {
        configure: (config) => log.configs.push(config as VideoEncoderConfig),
        get encodeQueueSize() {
          return queue;
        },
        flush: () => Promise.resolve(),
        close: () => {},
        encode(frame, opts) {
          const colour = (frame as FakeFrame & { colour: boolean }).colour;
          log.encoded.push({ timestamp: frame.timestamp, key: opts?.keyFrame === true, colour });
          if (options.drop === frame.timestamp / 1000 && colour) {
            return;
          }
          queue += 0;
          const chunk: EncodedLike = {
            type: opts?.keyFrame && !options.noKey ? 'key' : 'delta',
            timestamp: frame.timestamp,
            byteLength: 4,
            copyTo: (destination) => destination.set([1, 2, 3, 4]),
          };
          output(chunk);
        },
      };
      return encoder;
    },
    i420Frame: (_data, init) =>
      Object.assign(new FakeFrame(init.timestamp, init.width, init.height), { colour: init.colour }),
    context: (width, height): AlphaContextLike => ({
      imageSmoothingEnabled: false,
      imageSmoothingQuality: 'low',
      clearRect: () => {},
      drawImage: (image) => {
        alpha = (image as FakeFrame).alpha;
      },
      getImageData: () => {
        const data = new Uint8ClampedArray(width * height * 4).fill(200);
        for (let at = 3; at < data.length; at += 4) {
          data[at] = alpha;
        }
        return { data };
      },
    }),
  };
  return { env, log };
}

const SPEC: AlphaSpec = {
  width: 320,
  height: 240,
  codecs: ['vp09.00.10.08', 'vp8'],
  bitrates: { colour: 1_000_000, alpha: 150_000 },
  framerate: 30,
};

describe('startAlphaEncode', () => {
  it('encodes each frame twice (colour, then alpha), the same key frames in both, into a WebM with alpha', async () => {
    const { env, log } = fakeEncoders();
    const encode = await startAlphaEncode(SPEC, env);
    if (typeof encode === 'string') {
      throw new Error(encode);
    }
    expect(encode.codec).toBe('vp09.00.10.08');
    const frames = [new FakeFrame(0), new FakeFrame(0, 320, 240, 255), new FakeFrame(0)];
    for (const [index, frame] of frames.entries()) {
      await encode.add(frame, index * 40, index === 0);
    }
    expect(frames.every((frame) => frame.closed)).toBe(true);
    expect(encode.transparentFrames).toBe(2);
    expect(log.configs.map((config) => [config.codec, config.bitrate, config.alpha])).toEqual([
      ['vp09.00.10.08', 1_000_000, 'discard'],
      ['vp09.00.10.08', 150_000, 'discard'],
    ]);
    expect(log.encoded).toEqual([
      { timestamp: 0, key: true, colour: true },
      { timestamp: 0, key: true, colour: false },
      { timestamp: 40_000, key: false, colour: true },
      { timestamp: 40_000, key: false, colour: false },
      { timestamp: 80_000, key: false, colour: true },
      { timestamp: 80_000, key: false, colour: false },
    ]);
    const file = await encode.finish(120);
    expect(file.type).toBe('video/webm');
    const { text } = webmElements(new Uint8Array(await file.arrayBuffer()));
    expect(text).toContain('V_VP9');
  });

  it('falls back to VP8, and says why when the browser encodes neither', async () => {
    const vp8 = await startAlphaEncode(SPEC, fakeEncoders({ supports: (codec) => codec === 'vp8' }).env);
    expect(typeof vp8 === 'string' ? vp8 : vp8.codec).toBe('vp8');
    expect(await startAlphaEncode(SPEC, fakeEncoders({ supports: () => false }).env)).toBe(
      'this browser encodes neither VP9 nor VP8 at 320 × 240',
    );
  });

  it('fails on a dropped frame or a key frame the encoder did not make', async () => {
    for (const [options, message] of [
      [{ drop: 40 }, /dropped the frame at 40 ms/],
      [{ noKey: true }, /no key frame at 0 ms/],
    ] as const) {
      const encode = await startAlphaEncode(SPEC, fakeEncoders(options).env);
      if (typeof encode === 'string') {
        throw new Error(encode);
      }
      await encode.add(new FakeFrame(0), 0, true);
      await encode.add(new FakeFrame(0), 40, false);
      await expect(encode.finish(80)).rejects.toThrow(message);
    }
  });
});

/** A movie of `frames` HEVC frames at 30 fps (its samples' bytes are not read). */
function movie(frames = 30): Blob {
  return writeMp4(
    {
      timescale: 600,
      width: 640,
      height: 480,
      avcC: Uint8Array.from([1, 0x64, 0, 0x1f, 0xff, 0xe1, 0, 0]),
      samples: Array.from({ length: frames }, (_, index) => ({ size: 10, duration: 20, sync: index === 0 })),
      edits: [],
    },
    null,
    [{ track: 'video', samples: frames, bytes: new Blob([new Uint8Array(frames * 10)]) }],
  );
}

/** A page environment over fakes: the `<video>` shows frames of `alpha`, the sink encodes in process. */
function fakePage(options: { opens?: string; alpha?: number; encoders?: Parameters<typeof fakeEncoders>[0] } = {}) {
  const log = { seeks: [] as number[], sinkFrames: 0, closed: { source: false, sink: false } };
  const { env: encoders } = fakeEncoders(options.encoders);
  const source: AlphaSource = {
    frameAt: (seconds, timestamp) => {
      log.seeks.push(seconds);
      return Promise.resolve(new FakeFrame(timestamp, 640, 480, options.alpha ?? 128) as unknown as TakenFrame);
    },
    close: () => (log.closed.source = true),
  };
  const sink = (): AlphaSink => {
    let encode: Awaited<ReturnType<typeof startAlphaEncode>> | null = null;
    return {
      async start(spec) {
        encode = await startAlphaEncode(spec, encoders);
        return typeof encode === 'string' ? encode : null;
      },
      async frame(frame, timestamp, key) {
        log.sinkFrames += 1;
        if (encode && typeof encode !== 'string') {
          await encode.add(frame, timestamp, key);
        }
      },
      async finish(duration) {
        if (!encode || typeof encode === 'string') {
          return 'not started';
        }
        return {
          file: await encode.finish(duration),
          codec: encode.codec,
          transparentFrames: encode.transparentFrames,
        };
      },
      close: () => (log.closed.sink = true),
    };
  };
  const env: AlphaEnvironment = {
    open: () => Promise.resolve(options.opens ?? source),
    sink,
    transparentPixels: (frame) => ((frame as unknown as FakeFrame).alpha < 255 ? 1 : 0),
    now: () => 0,
  };
  return { env, log };
}

/** An AAC sound track's plan. */
const SOUND = {
  codec: 'mp4a',
  objectType: 0x40,
  aac: true,
  channels: 2,
  sampleRate: 48_000,
  sampleSize: 16,
  bytes: 32_000,
};

describe('makeAlphaCopy', () => {
  it('samples frames for transparency, then takes every frame at its time into the copy', async () => {
    const { env, log } = fakePage();
    const progress: number[] = [];
    const outcome = await makeAlphaCopy({ plan: plan(), file: movie() }, env, ({ frames }) => progress.push(frames));
    expect(outcome.kind).toBe('made');
    expect(outcome.kind === 'made' && outcome.codec).toBe('vp09.00.20.08');
    // One sampled frame showed transparency; then the 30 frames.
    expect(log.seeks).toHaveLength(31);
    expect(log.sinkFrames).toBe(30);
    expect(progress).toEqual([15, 30, 30]);
    expect(log.closed).toEqual({ source: true, sink: true });
  });

  it('keeps the original alone where the browser plays it opaque, cannot play it, or encodes no VP9 or VP8', async () => {
    const opaque = await makeAlphaCopy({ plan: plan(), file: movie() }, fakePage({ alpha: 255 }).env);
    expect(opaque).toMatchObject({ kind: 'kept', reason: expect.stringMatching(/without its transparency/) });
    const unplayable = await makeAlphaCopy(
      { plan: plan(), file: movie() },
      fakePage({ opens: 'this browser plays no hvc1 video' }).env,
    );
    expect(unplayable).toMatchObject({ kind: 'kept', reason: 'this browser plays no hvc1 video' });
    const noEncoder = await makeAlphaCopy(
      { plan: plan(), file: movie() },
      fakePage({ encoders: { supports: () => false } }).env,
    );
    expect(noEncoder).toMatchObject({ kind: 'kept', reason: expect.stringMatching(/neither VP9 nor VP8/) });
    const sound = await makeAlphaCopy(
      {
        plan: plan({
          audio: SOUND,
        }),
        file: movie(),
      },
      fakePage().env,
    );
    expect(sound).toMatchObject({ kind: 'kept', reason: expect.stringMatching(/sound/) });
  });

  it('seeks the next frame while the sink encodes the last, and stops at a sink that fails', async () => {
    const events: string[] = [];
    const frames: FakeFrame[] = [];
    const { env } = fakePage();
    const piped = (failAt: number | null): AlphaEnvironment => ({
      ...env,
      open: () =>
        Promise.resolve({
          frameAt: (_seconds, timestamp) => {
            events.push(`seek ${timestamp / 1000}`);
            const frame = new FakeFrame(timestamp, 640, 480, 128);
            frames.push(frame);
            return Promise.resolve(frame as unknown as TakenFrame);
          },
          close: () => {},
        }),
      sink: () => ({
        start: () => Promise.resolve(null),
        frame: async (frame, timestamp) => {
          events.push(`take ${timestamp}`);
          await new Promise((resolve) => setTimeout(resolve, 1));
          frame.close();
          if (timestamp === failAt) {
            throw new Error('the encoder failed');
          }
          events.push(`taken ${timestamp}`);
        },
        finish: () => Promise.resolve({ file: new Blob(['webm']), codec: 'vp8', transparentFrames: 3 }),
        close: () => {},
      }),
    });
    const made = await makeAlphaCopy({ plan: plan(), file: movie(3) }, piped(null));
    expect(made.kind).toBe('made');
    // After the probe's sampled frame: the second frame is sought before the first is taken.
    expect(events.slice(1, 6)).toEqual(['seek 0', 'take 0', 'seek 33', 'taken 0', 'take 33']);
    events.length = 0;
    frames.length = 0;
    const failed = await makeAlphaCopy({ plan: plan(), file: movie(3) }, piped(33));
    expect(failed).toMatchObject({ kind: 'kept', reason: 'the copy failed (the encoder failed)' });
    expect(frames.every((frame) => frame.closed)).toBe(true);
  });

  it('copies a video with sound that every use plays muted, without its sound', async () => {
    const silent = await makeAlphaCopy({ plan: plan({ audio: SOUND, silent: true }), file: movie() }, fakePage().env);
    expect(silent.kind).toBe('made');
    // Negative control: a sound some use may play is not carried, so there is no copy.
    const heard = await makeAlphaCopy({ plan: plan({ audio: SOUND, silent: false }), file: movie() }, fakePage().env);
    expect(heard).toMatchObject({ kind: 'kept', reason: expect.stringMatching(/sound/) });
  });

  it('stops at a cancel', async () => {
    const controller = new AbortController();
    controller.abort();
    const outcome = await makeAlphaCopy({ plan: plan(), file: movie() }, fakePage().env, () => {}, controller.signal);
    expect(outcome).toMatchObject({ kind: 'kept', reason: 'the copy was cancelled' });
  });
});

describe('alphaCopier', () => {
  it('makes at most `limit` copies at once and gives up the rest once disposed', async () => {
    let running = 0;
    let most = 0;
    const { env } = fakePage();
    const slow: AlphaEnvironment = {
      ...env,
      open: async (file, codec) => {
        running += 1;
        most = Math.max(most, running);
        await new Promise((resolve) => setTimeout(resolve, 5));
        running -= 1;
        return env.open(file, codec);
      },
    };
    const copier = alphaCopier(slow, 2);
    const outcomes = await Promise.all([1, 2, 3].map(() => copier.copy({ plan: plan(), file: movie() })));
    expect(outcomes.map((outcome) => outcome.kind)).toEqual(['made', 'made', 'made']);
    expect(most).toBe(2);
    copier.dispose();
    expect(await copier.copy({ plan: plan(), file: movie() })).toMatchObject({ kind: 'kept' });
  });
});
