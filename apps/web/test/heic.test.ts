import type { HeicPlan, HevcDecoderConfig, HevcSignal } from '@slidock/types';
import { describe, expect, it } from 'vitest';

import {
  type FrameLike,
  Undecodable,
  carriedProfile,
  decodeHeic,
  frameColour,
  withIccProfile,
} from '../src/media/heic';
import { FakeCanvas, fakeDecodeBrowser, gridFile, gridPlan, jpegHeader } from '../src/testing/fake-photos';

const CODEC = 'hvc1.3.E.L90.B0';
const P3_ICC = { icc: [1, 2, 3], iccPrimaries: 12, iccTransfer: 13 };

describe('decodeHeic', () => {
  it('decodes each tile as one key chunk and draws it at its place, then encodes a JPEG of the plan size', async () => {
    const plan = gridPlan(3, 2);
    const browser = fakeDecodeBrowser({ codecs: [CODEC] });
    const photo = await decodeHeic(plan, gridFile(plan), browser.env);
    expect(photo.path).toBe('webcodecs');
    expect([photo.width, photo.height, photo.mime]).toEqual([11, 7, 'image/jpeg']);
    expect(new Uint8Array(photo.bytes)).toEqual(jpegHeader(11, 7));
    // One chunk per tile, its bytes the tile's ranges of the file.
    expect(browser.chunks).toEqual(plan.tiles.map((_, index) => ({ timestamp: index, bytes: [index] })));
    expect(browser.draws.map(({ x, y }) => [x, y])).toEqual(plan.tiles.map(({ x, y }) => [x, y]));
    expect(browser.frames.every((frame) => frame.closed)).toBe(true);
    expect(browser.open()).toBe(0);
    const [config] = browser.configs;
    expect(config?.codec).toBe(CODEC);
    expect(Array.from(config?.description as Uint8Array)).toEqual([1, 2, 3]);
    expect(browser.canvases.every((canvas) => canvas.colorSpace === 'srgb')).toBe(true);
  });

  it('applies the transforms in their order: crop, quarter turns anticlockwise, mirror', async () => {
    const plan = gridPlan(2, 2, {
      width: 3,
      height: 5,
      transforms: [
        { kind: 'crop', x: 1, y: 2, width: 5, height: 3 },
        { kind: 'rotate', turns: 1 },
        { kind: 'mirror', axis: 'horizontal' },
      ],
    });
    const browser = fakeDecodeBrowser({ codecs: [CODEC] });
    const photo = await decodeHeic(plan, gridFile(plan), browser.env);
    expect([photo.width, photo.height]).toEqual([3, 5]);
    const steps = browser.draws.filter((draw) => draw.image instanceof FakeCanvas);
    const sized = (draw: (typeof steps)[number]) => [...draw.canvas.size];
    expect(steps.map(sized)).toEqual([
      [5, 3],
      [3, 5],
      [3, 5],
    ]);
    // The crop draws the coded image shifted; a quarter anticlockwise turn of a 5 × 3 image sends (x, y) to
    // (y, 5 − x); the mirror about the horizontal axis sends (x, y) to (x, 5 − y).
    expect(steps.map(({ x, y, matrix }) => [x, y, ...matrix])).toEqual([
      [-1, -2, 1, 0, 0, 1, 0, 0],
      [0, 0, 0, -1, 1, 0, 0, 5],
      [0, 0, 1, 0, 0, -1, 0, 5],
    ]);
  });

  it('turns by two and three quarters, and mirrors about the vertical axis', async () => {
    for (const [turns, matrix, size] of [
      [2, [-1, 0, 0, -1, 7, 7], [7, 7]],
      [3, [0, 1, -1, 0, 7, 0], [7, 7]],
    ] as const) {
      const plan = gridPlan(2, 2, { transforms: [{ kind: 'rotate', turns }] });
      const browser = fakeDecodeBrowser({ codecs: [CODEC] });
      await decodeHeic(plan, gridFile(plan), browser.env);
      const turned = browser.draws.at(-1)!;
      expect(turned.matrix).toEqual(matrix);
      expect(turned.canvas.size).toEqual(size);
    }
    const plan = gridPlan(2, 1, { transforms: [{ kind: 'mirror', axis: 'vertical' }] });
    const browser = fakeDecodeBrowser({ codecs: [CODEC] });
    await decodeHeic(plan, gridFile(plan), browser.env);
    expect(browser.draws.at(-1)?.matrix).toEqual([-1, 0, 0, 1, 7, 0]);
  });

  it("takes the browser's own image decoder first, which applies the transforms itself", async () => {
    const plan = gridPlan(2, 2, { width: 7, height: 7, transforms: [{ kind: 'rotate', turns: 1 }] });
    const browser = fakeDecodeBrowser({ codecs: [CODEC], nativeSize: { width: 7, height: 7 } });
    const photo = await decodeHeic(plan, gridFile(plan), browser.env);
    expect(photo.path).toBe('native');
    expect(browser.chunks).toEqual([]);
    expect(browser.draws).toHaveLength(1);
  });

  it("never transforms a native decode again: one of another size than the plan's output is not taken", async () => {
    // A quarter-turned 7 × 3 image: a bitmap of its coded size would be ambiguous for a half turn, a mirror or a
    // square image, so it is refused, and WebCodecs decodes it.
    const plan = gridPlan(2, 1, { width: 3, height: 7, transforms: [{ kind: 'rotate', turns: 3 }] });
    const browser = fakeDecodeBrowser({ codecs: [CODEC], nativeSize: { width: 7, height: 3 } });
    const photo = await decodeHeic(plan, gridFile(plan), browser.env);
    expect([photo.path, photo.width, photo.height]).toEqual(['webcodecs', 3, 7]);
    // A half-turned or mirrored image of its coded size is the native decoder's output as it is.
    for (const step of [
      { kind: 'rotate', turns: 2 },
      { kind: 'mirror', axis: 'horizontal' },
    ] as const) {
      const same = gridPlan(2, 2, { transforms: [step] });
      const native = fakeDecodeBrowser({ nativeSize: { width: 7, height: 7 } });
      expect((await decodeHeic(same, gridFile(same), native.env)).path).toBe('native');
      expect(native.draws.map(({ matrix }) => matrix)).toEqual([[1, 0, 0, 1, 0, 0]]);
    }
    const refused = fakeDecodeBrowser({ nativeSize: { width: 7, height: 3 } });
    const failure = await decodeHeic(plan, gridFile(plan), refused.env).catch((error: unknown) => error);
    expect((failure as Undecodable).reasons[0]).toBe('this browser decoded it at 7 × 3, not 3 × 7');
  });

  it('releases every canvas once the next step holds it, the last once it is encoded', async () => {
    const plan = gridPlan(2, 2, {
      width: 3,
      height: 5,
      transforms: [
        { kind: 'crop', x: 1, y: 2, width: 5, height: 3 },
        { kind: 'rotate', turns: 1 },
        { kind: 'mirror', axis: 'horizontal' },
      ],
    });
    const browser = fakeDecodeBrowser({ codecs: [CODEC] });
    await decodeHeic(plan, gridFile(plan), browser.env);
    expect(browser.canvases.map((canvas) => canvas.size)).toEqual([
      [7, 7],
      [5, 3],
      [3, 5],
      [3, 5],
    ]);
    expect(browser.canvases.every((canvas) => canvas.width === 0 && canvas.height === 0)).toBe(true);
  });

  it('falls back to WebCodecs when the browser cannot decode HEIC images', async () => {
    const plan = gridPlan(2, 2);
    const browser = fakeDecodeBrowser({ codecs: [CODEC] });
    expect((await decodeHeic(plan, gridFile(plan), browser.env)).path).toBe('webcodecs');
  });

  it('says why when neither path decodes it', async () => {
    const plan = gridPlan(2, 2);
    const browser = fakeDecodeBrowser({ codecs: ['vp09.00.10.08'] });
    const failure = await decodeHeic(plan, gridFile(plan), browser.env).catch((error: unknown) => error);
    expect(failure).toBeInstanceOf(Undecodable);
    expect((failure as Undecodable).reasons).toEqual([
      'this browser does not decode HEIC images (The source image could not be decoded.)',
      `this browser's video decoder does not take ${CODEC}`,
    ]);
    // No WebCodecs at all.
    const none = await decodeHeic(plan, gridFile(plan), fakeDecodeBrowser().env).catch((error: unknown) => error);
    expect((none as Undecodable).reasons.at(-1)).toBe('this browser has no WebCodecs video decoder');
  });

  it('gives up on a decoder error mid-grid, closing the decoder', async () => {
    const plan = gridPlan(3, 2);
    const browser = fakeDecodeBrowser({ codecs: [CODEC], failAt: 3 });
    const failure = await decodeHeic(plan, gridFile(plan), browser.env).catch((error: unknown) => error);
    expect((failure as Undecodable).reasons.at(-1)).toBe(`the video decoder failed on ${CODEC}: tile 3 is corrupt`);
    expect(browser.open()).toBe(0);
    expect(browser.frames.every((frame) => frame.closed)).toBe(true);
    // It stops feeding chunks after the error.
    expect(browser.chunks.length).toBeLessThan(plan.tiles.length);
  });

  it('refuses a decoder that gives fewer frames than tiles', async () => {
    const plan = gridPlan(2, 1);
    const browser = fakeDecodeBrowser({ codecs: [CODEC] });
    const decoder = browser.env.VideoDecoder!;
    browser.env.VideoDecoder = class extends decoder {
      constructor(init: { output: (frame: FrameLike) => void; error: (error: Error) => void }) {
        super({ output: (frame) => (frame.timestamp === 0 ? init.output(frame) : frame.close()), error: init.error });
      }
    };
    const failure = await decodeHeic(plan, gridFile(plan), browser.env).catch((error: unknown) => error);
    expect((failure as Undecodable).reasons.at(-1)).toBe('the video decoder gave 1 of 2 tiles');
  });
});

describe('frameColour', () => {
  const config = (plan: HeicPlan, signal: Partial<HevcSignal> = {}): HevcDecoderConfig => ({
    ...plan.configs[0]!,
    signal: { ...plan.configs[0]!.signal, ...signal },
  });

  it("labels the frames with the photo's nclx, else the space its ICC profile matches, else sRGB", () => {
    const plain = gridPlan(1, 1);
    expect(frameColour(plain, config(plain))).toEqual({
      primaries: 'bt709',
      transfer: 'iec61966-2-1',
      matrix: 'smpte170m',
      fullRange: true,
    });
    // An iPhone's photo: Display P3 by its profile; its VUI states BT.601 full range and no primaries.
    const p3 = gridPlan(1, 1, { colour: { ...gridPlan(1, 1).colour, ...P3_ICC } });
    expect(frameColour(p3, config(p3, { fullRange: true, matrix: 6 }))).toEqual({
      primaries: 'smpte432',
      transfer: 'iec61966-2-1',
      matrix: 'smpte170m',
      fullRange: true,
    });
    expect(carriedProfile(p3)).toBeNull();
    // nclx wins; unspecified (2) code points read as absent.
    const nclx = gridPlan(1, 1, {
      colour: { ...P3_ICC, primaries: 9, transfer: 2, matrix: 1, fullRange: false },
    });
    expect(frameColour(nclx, config(nclx, { fullRange: true, matrix: 6, transfer: 1 }))).toEqual({
      primaries: 'bt2020',
      transfer: 'iec61966-2-1',
      matrix: 'bt709',
      fullRange: false,
    });
  });

  it('takes what the coded pictures state where the file names nothing, a default only where they are silent', () => {
    // No nclx, no profile: a limited-range BT.709 stream keeps its own levels and matrix.
    const plain = gridPlan(1, 1);
    expect(frameColour(plain, config(plain, { fullRange: false, primaries: 1, transfer: 1, matrix: 1 }))).toEqual({
      primaries: 'bt709',
      transfer: 'bt709',
      matrix: 'bt709',
      fullRange: false,
    });
    // A profile names the primaries and curve over the VUI's (IMG_6913: VUI P3 and BT.709's curve, profile sRGB's).
    const p3 = gridPlan(1, 1, { colour: { ...gridPlan(1, 1).colour, ...P3_ICC } });
    expect(frameColour(p3, config(p3, { fullRange: true, primaries: 12, transfer: 1, matrix: 6 }))).toEqual({
      primaries: 'smpte432',
      transfer: 'iec61966-2-1',
      matrix: 'smpte170m',
      fullRange: true,
    });
    // Each configuration is labelled with its own pictures' colour.
    const two = gridPlan(2, 1, {
      configs: [config(plain, { fullRange: false }), config(plain, { fullRange: true })],
      tiles: gridPlan(2, 1).tiles.map((tile, index) => ({ ...tile, config: index })),
    });
    const browser = fakeDecodeBrowser({ codecs: [CODEC] });
    return decodeHeic(two, gridFile(two), browser.env).then(() => {
      expect(browser.configs.map((configured) => configured.colorSpace?.fullRange)).toEqual([false, true]);
    });
  });

  it('carries a profile of no known space with the JPEG, the frames drawn as sRGB', () => {
    const odd = gridPlan(1, 1, {
      colour: {
        primaries: null,
        transfer: null,
        matrix: null,
        fullRange: null,
        icc: [9, 9],
        iccPrimaries: null,
        iccTransfer: 13,
      },
    });
    expect(frameColour(odd, config(odd)).primaries).toBe('bt709');
    expect(Array.from(carriedProfile(odd) ?? [])).toEqual([9, 9]);
  });
});

describe('withIccProfile', () => {
  it('puts the profile after the JFIF segment in place of any profile the JPEG had', () => {
    const jpeg = jpegHeader(2, 1);
    const tagged = withIccProfile(jpeg, new Uint8Array([7, 8, 9]));
    // SOI, APP0 (18 bytes), then APP2: length 19, `ICC_PROFILE\0`, chunk 1 of 1, the profile.
    expect(Array.from(tagged.subarray(0, 2))).toEqual([0xff, 0xd8]);
    expect(Array.from(tagged.subarray(20, 24))).toEqual([0xff, 0xe2, 0, 19]);
    expect(new TextDecoder().decode(tagged.subarray(24, 36))).toBe('ICC_PROFILE\0');
    expect(Array.from(tagged.subarray(36, 41))).toEqual([1, 1, 7, 8, 9]);
    expect(Array.from(tagged.subarray(41))).toEqual(Array.from(jpeg.subarray(20)));
    // Tagged again, the old profile goes.
    const again = withIccProfile(tagged, new Uint8Array([5]));
    expect(again.length).toBe(jpeg.length + 2 + 2 + 12 + 2 + 1);
    expect(again[38]).toBe(5);
    // A large profile spans segments of at most 65 519 bytes.
    const large = withIccProfile(jpeg, new Uint8Array(70_000));
    expect(Array.from(large.subarray(36, 38))).toEqual([1, 2]);
    expect(() => withIccProfile(new Uint8Array([0, 0]), new Uint8Array([1]))).toThrow('not a JPEG');
  });
});
