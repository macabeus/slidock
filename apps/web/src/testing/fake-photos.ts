import type { HeicPlan, MediaNeed } from '@slidock/types';

import type {
  CanvasLike,
  ContextLike,
  DecodeEnvironment,
  DecoderClass,
  DecoderLike,
  Drawable,
  FrameLike,
  ImageLike,
} from '../media/heic';
import { type DecodedFile, type PhotoDecoder, PhotoUndecodable } from '../media/photos';

/** The header of a baseline JPEG of `width` × `height` (SOI, APP0 JFIF, SOF0, EOI): what the compiler reads of one. */
export function jpegHeader(width: number, height: number): Uint8Array<ArrayBuffer> {
  return new Uint8Array([
    0xff,
    0xd8,
    0xff,
    0xe0,
    0,
    16,
    0x4a,
    0x46,
    0x49,
    0x46,
    0,
    1,
    1,
    0,
    0,
    1,
    0,
    1,
    0,
    0,
    0xff,
    0xc0,
    0,
    17,
    8,
    height >> 8,
    height & 0xff,
    width >> 8,
    width & 0xff,
    3,
    1,
    0x22,
    0,
    2,
    0x11,
    1,
    3,
    0x11,
    1,
    0xff,
    0xd9,
  ]);
}

/** What a fake canvas was asked to draw. */
export interface Draw {
  canvas: FakeCanvas;
  image: Drawable;
  x: number;
  y: number;
  /** The transform it was drawn through (`setTransform`'s six values). */
  matrix: readonly number[];
}

/** A canvas that records its draws and encodes a JPEG header of its size. */
export class FakeCanvas implements CanvasLike {
  width: number;
  height: number;
  /** Its size before it was released (set to 0 × 0). */
  readonly size: readonly [number, number];
  readonly draws: Draw[];
  colorSpace: PredefinedColorSpace | undefined;

  constructor(width: number, height: number, draws: Draw[]) {
    this.width = width;
    this.height = height;
    this.size = [width, height];
    this.draws = draws;
  }

  getContext(_kind: '2d', options?: { colorSpace?: PredefinedColorSpace }): ContextLike {
    this.colorSpace = options?.colorSpace;
    let matrix: readonly number[] = [1, 0, 0, 1, 0, 0];
    return {
      setTransform: (...values) => (matrix = values),
      drawImage: (image, x, y) => this.draws.push({ canvas: this, image, x, y, matrix }),
    };
  }

  convertToBlob(options: { type: string; quality?: number }): Promise<Blob> {
    return Promise.resolve(new Blob([jpegHeader(this.width, this.height)], { type: options.type }));
  }
}

/** A frame of a fake decoder. */
export class FakeFrame implements FrameLike {
  readonly timestamp: number;
  readonly displayWidth: number;
  readonly displayHeight: number;
  closed = false;

  constructor(timestamp: number, width: number, height: number) {
    this.timestamp = timestamp;
    this.displayWidth = width;
    this.displayHeight = height;
  }

  close() {
    this.closed = true;
  }
}

export interface FakeDecodeOptions {
  /** The codec strings the video decoder takes; none: there is no `VideoDecoder`. */
  codecs?: readonly string[];
  /** Fails decoding the chunk of this tile index (of all decoded so far) with an error. */
  failAt?: number;
  /** The size `createImageBitmap` decodes the file at; none: it cannot decode HEIC. */
  nativeSize?: { width: number; height: number };
}

/** A fake browser: what it was asked to configure, decode and draw. */
export interface FakeDecodeBrowser {
  env: DecodeEnvironment;
  configs: VideoDecoderConfig[];
  chunks: { timestamp: number; bytes: number[] }[];
  frames: FakeFrame[];
  draws: Draw[];
  canvases: FakeCanvas[];
  /** Decoders not closed. */
  open(): number;
}

export function fakeDecodeBrowser(options: FakeDecodeOptions = {}): FakeDecodeBrowser {
  const browser: FakeDecodeBrowser = {
    env: undefined as unknown as DecodeEnvironment,
    configs: [],
    chunks: [],
    frames: [],
    draws: [],
    canvases: [],
    open: () => decoders.filter((decoder) => !decoder.closed).length,
  };
  const decoders: { closed: boolean }[] = [];
  let decoded = 0;
  const Decoder = class implements DecoderLike {
    readonly #init: { output: (frame: FrameLike) => void; error: (error: Error) => void };
    #config: VideoDecoderConfig | null = null;
    #state = { closed: false };

    constructor(init: { output: (frame: FrameLike) => void; error: (error: Error) => void }) {
      this.#init = init;
      decoders.push(this.#state);
    }

    static isConfigSupported(config: VideoDecoderConfig) {
      return Promise.resolve({ supported: options.codecs?.includes(config.codec) ?? false });
    }

    configure(config: VideoDecoderConfig) {
      this.#config = config;
      browser.configs.push(config);
    }

    decode(chunk: unknown) {
      const { timestamp, data } = chunk as { timestamp: number; data: Uint8Array };
      browser.chunks.push({ timestamp, bytes: Array.from(data) });
      const index = decoded++;
      queueMicrotask(() => {
        if (index === options.failAt) {
          this.#init.error(new Error(`tile ${index} is corrupt`));
          return;
        }
        if (this.#config) {
          const frame = new FakeFrame(timestamp, 4, 4);
          browser.frames.push(frame);
          this.#init.output(frame);
        }
      });
    }

    flush() {
      return new Promise<void>((resolve) => setTimeout(resolve, 0));
    }

    close() {
      if (this.#state.closed) {
        throw new Error('closed already');
      }
      this.#state.closed = true;
    }
  } satisfies DecoderClass;
  browser.env = {
    ...(options.codecs ? { VideoDecoder: Decoder, makeChunk: (init) => init } : {}),
    createImageBitmap: (blob: Blob): Promise<ImageLike> => {
      const size = options.nativeSize;
      if (!size || blob.type !== 'image/heic') {
        return Promise.reject(new Error('The source image could not be decoded.'));
      }
      return Promise.resolve({ ...size, close: () => {} });
    },
    canvas: (width, height) => {
      const canvas = new FakeCanvas(width, height, browser.draws);
      browser.canvases.push(canvas);
      return canvas;
    },
    colorSpace: 'srgb',
    now: () => 0,
  };
  return browser;
}

/** A plan of `columns` × `rows` tiles of 4 × 4 pixels, its file holding one byte per tile (the tile's index). */
export function gridPlan(columns: number, rows: number, extra: Partial<HeicPlan> = {}): HeicPlan {
  const tiles = Array.from({ length: columns * rows }, (_, index) => ({
    config: 0,
    x: (index % columns) * 4,
    y: Math.floor(index / columns) * 4,
    extents: [{ offset: index, length: 1 }],
  }));
  const width = columns * 4 - 1;
  const height = rows * 4 - 1;
  return {
    width,
    height,
    codedWidth: width,
    codedHeight: height,
    tileWidth: 4,
    tileHeight: 4,
    columns,
    rows,
    configs: [
      {
        codec: 'hvc1.3.E.L90.B0',
        description: [1, 2, 3],
        chromaFormat: 1,
        bitDepth: 8,
        signal: { fullRange: null, primaries: null, transfer: null, matrix: null },
      },
    ],
    tiles,
    transforms: [],
    colour: {
      primaries: null,
      transfer: null,
      matrix: null,
      fullRange: null,
      icc: null,
      iccPrimaries: null,
      iccTransfer: null,
    },
    ...extra,
  };
}

/** The file of a `gridPlan`: one byte per tile. */
export function gridFile(plan: HeicPlan): Blob {
  return new Blob([new Uint8Array(plan.tiles.map((_, index) => index))], { type: 'image/heic' });
}

/** A media need of `plan` (or a refused one, with `plan` `null`). */
export function mediaNeed(id: string, plan: HeicPlan | null, refusal: string | null = null): MediaNeed {
  return { id, mime: 'image/heic', blob: plan ? gridFile(plan) : new Blob([]), plan, refusal };
}

/** A photo decoder answering from `answer`: a file, or the reasons it cannot decode the need. */
export function fakePhotoDecoder(answer: (need: MediaNeed) => DecodedFile | string[]): PhotoDecoder & {
  decoded: string[];
  disposed: number;
} {
  const decoder = {
    decoded: [] as string[],
    disposed: 0,
    decode(need: MediaNeed) {
      decoder.decoded.push(need.id);
      const result = answer(need);
      return Array.isArray(result) ? Promise.reject(new PhotoUndecodable(result)) : Promise.resolve(result);
    },
    dispose() {
      decoder.disposed += 1;
    },
  };
  return decoder;
}

/** A decoded JPEG of `plan`'s size. */
export function decodedFile(plan: HeicPlan | null): DecodedFile {
  return {
    bytes: jpegHeader(plan?.width ?? 1, plan?.height ?? 1).buffer,
    mime: 'image/jpeg',
    path: 'webcodecs',
    elapsed: 5,
  };
}
