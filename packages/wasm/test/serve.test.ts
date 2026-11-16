import type { BundleReport, Deck, FontReport, FromWorker, HeicPlan, ToWorker, WorkerError } from '@slidock/types';
import { describe, expect, it } from 'vitest';

import type { Printed, WasmCompiler } from '../src/compiler';
import { serve } from '../src/serve';
import type { CompilerModule, WorkerScope } from '../src/serve';

/** A worker scope whose tasks run when the test says. */
class FakeScope implements WorkerScope {
  readonly sent: { message: FromWorker; transfer: Transferable[] }[] = [];
  readonly tasks: (() => void)[] = [];
  #listener: ((event: MessageEvent<ToWorker>) => void) | undefined;

  postMessage(message: FromWorker, transfer: Transferable[]): void {
    this.sent.push({ message, transfer });
  }

  addEventListener(_type: 'message', listener: (event: MessageEvent<ToWorker>) => void): void {
    this.#listener = listener;
  }

  send(message: ToWorker): void {
    this.#listener?.({ data: message } as MessageEvent<ToWorker>);
  }

  /** Runs every pending task, and the tasks they schedule. */
  drain(): void {
    for (let task = this.tasks.shift(); task !== undefined; task = this.tasks.shift()) {
      task();
    }
  }

  messages(): FromWorker[] {
    return this.sent.map(({ message }) => message);
  }

  types(): string[] {
    return this.messages().map((message) => ('phase' in message ? `${message.type}:${message.phase}` : message.type));
  }
}

const deck = { format: 'slidock-deck@1', id: 'd', title: 'Deck' } as Deck;
const plan: HeicPlan = {
  width: 2,
  height: 1,
  codedWidth: 2,
  codedHeight: 1,
  tileWidth: 2,
  tileHeight: 1,
  columns: 1,
  rows: 1,
  configs: [
    {
      codec: 'hvc1.3.E.L90.B0',
      description: [1],
      chromaFormat: 1,
      bitDepth: 8,
      signal: { fullRange: true, primaries: null, transfer: null, matrix: 6 },
    },
  ],
  tiles: [{ config: 0, x: 0, y: 0, extents: [{ offset: 0, length: 2 }] }],
  transforms: [],
  colour: {
    primaries: null,
    transfer: null,
    matrix: null,
    fullRange: null,
    icc: [1],
    iccPrimaries: 12,
    iccTransfer: 13,
  },
};
const report: BundleReport = { files: [], bytes: 3, singleFile: false, fontsLeftOut: [] };
const face: FontReport = {
  family: 'Lato',
  postscriptName: 'Lato-Regular',
  weight: 400,
  style: 'normal',
  source: 'embedded',
  shipped: true,
  bytes: 3,
  subset: true,
  demoted: false,
  restricted: false,
  unstated: false,
  repaired: null,
  refused: null,
  notice: false,
  serves: [{ family: 'Lato', weight: 400, style: 'normal' }],
  licence: 'OFL-1.1',
};

function stageError(kind: WorkerError['kind'], message: string): string {
  return JSON.stringify({ kind, message, diagnostics: [] } satisfies WorkerError);
}

/** A compiler that records its calls and answers from `overrides`. */
function fakeCompiler(calls: string[], overrides: Partial<WasmCompiler> = {}): WasmCompiler {
  const printed: Printed = {
    deck,
    slides: [{ id: 's1', html: '<main data-slide="s1"></main>' }],
    media: [
      { path: 'media/a.png', mime: 'image/png', range: { offset: 2, length: 3 } },
      { path: 'media/b.png', mime: 'image/png', range: null },
    ],
    diagnostics: [],
    fonts: [
      { family: 'Lato', weight: '400', style: 'normal', unicodeRange: null, format: 'woff2' },
      { family: 'Lato', weight: '700', style: 'normal', unicodeRange: 'U+20-7E', format: 'woff2' },
    ],
    faces: [face],
    fontIssues: [{ code: 'MID5410', message: 'm' }],
    substitutes: [{ family: 'Liberation Sans', postscriptName: 'LiberationSans', weight: 400, style: 'normal' }],
    page: { title: 'Deck', description: null },
  };
  return {
    open(name, source, fonts) {
      calls.push(`open ${name} ${source.size} ${Array.from(source.read(1, 2)).join(',')} ${fonts}`);
      return {
        diagnostics: [
          { code: 'IR0001', severity: 'warning', message: 'm', feature: null, container: null, node: null },
        ],
        needs: [],
        media: [
          { id: 'a'.repeat(64), mime: 'image/heic', range: { offset: 1, length: 2 }, plan, refusal: null },
          {
            id: 'b'.repeat(64),
            mime: 'image/heic',
            range: null,
            plan: null,
            refusal: 'its primary image is a `iovl` item',
          },
        ],
      };
    },
    fontNeeds: () => [],
    mediaNeeds: () => [],
    mediaNeedBytes(need) {
      calls.push(`mediaNeedBytes ${need.slice(0, 1)}`);
      return new Uint8Array([5, 6, 7]);
    },
    addMedia(need, bytes) {
      calls.push(`addMedia ${need.slice(0, 1)} ${bytes.length}`);
    },
    addFont(bytes, source, unicodeRange) {
      calls.push(
        `addFont ${bytes.length} ${source}${unicodeRange === undefined || unicodeRange === null ? '' : ` ${unicodeRange}`}`,
      );
      return [];
    },
    selectLocal(fonts) {
      calls.push(`selectLocal ${fonts}`);
      return [{ index: 0, postscriptName: 'Lato-Regular', family: 'Lato', exactPostscript: true }];
    },
    googleFaces(css) {
      calls.push(`googleFaces ${css}`);
      return [];
    },
    fontBytes(index) {
      calls.push(`fontBytes ${index}`);
      return new Uint8Array([index, 7]);
    },
    finish: () => void calls.push('finish'),
    print: () => printed,
    mediaBytes(path) {
      calls.push(`mediaBytes ${path}`);
      return new Uint8Array([9, 9]);
    },
    documents: () => ({ text: {}, media: [] }),
    imagePlan: () => {
      calls.push('imagePlan');
      return [];
    },
    addImage(path, bytes) {
      calls.push(`addImage ${path} ${bytes.length}`);
    },
    videoPlan: () => {
      calls.push('videoPlan');
      return [];
    },
    addVideo(path, source) {
      calls.push(`addVideo ${path} ${source.size}`);
    },
    alternatePlan: () => {
      calls.push('alternatePlan');
      return [];
    },
    addAlternate(path, source) {
      calls.push(`addAlternate ${path} ${source.size}`);
    },
    bundle(settings, sink) {
      calls.push(`bundle ${settings}`);
      sink(new Uint8Array([1, 2]));
      sink(new Uint8Array([3]));
      return report;
    },
    ...overrides,
  };
}

/** The bytes of each blob `file` makes, for `reader`. */
const contents = new WeakMap<Blob, Uint8Array<ArrayBuffer>>();

/** Reads blobs as the worker's `FileReaderSync` would, from the bytes they were made of. */
const reader = {
  readAsArrayBuffer(blob: Blob): ArrayBuffer {
    const bytes = contents.get(blob);
    if (bytes === undefined) {
      throw new Error('no bytes');
    }
    return bytes.slice().buffer;
  },
};

/** A `File` whose slices remember their bytes, for `reader`. */
function file(name: string, content: number[]): File {
  const make = (bytes: Uint8Array<ArrayBuffer>, type = ''): Blob => {
    const blob = new Blob([bytes], { type });
    contents.set(blob, bytes);
    blob.slice = (start = 0, end = bytes.length, contentType = '') => make(bytes.slice(start, end), contentType);
    return blob;
  };
  return Object.assign(make(new Uint8Array(content)), { name, lastModified: 0, webkitRelativePath: '' }) as File;
}

async function started(compiler: WasmCompiler, memoryBytes = () => 0): Promise<FakeScope> {
  const scope = new FakeScope();
  const module: CompilerModule = { version: () => '0.1.0', memoryBytes, create: () => compiler };
  serve(scope, { module: Promise.resolve(module), reader, later: (task) => scope.tasks.push(task) });
  await Promise.resolve();
  await Promise.resolve();
  return scope;
}

describe('serve', () => {
  it('says it is ready, then answers open with progress, diagnostics, needs, media needs and done', async () => {
    const calls: string[] = [];
    const scope = await started(fakeCompiler(calls));
    scope.send({ type: 'open', id: 1, file: file('deck.key', [0, 1, 2, 3]) });
    scope.drain();
    expect(scope.types()).toEqual(['ready', 'progress:open', 'diagnostics', 'needs', 'mediaNeeds', 'done']);
    expect(calls).toEqual(['open deck.key 4 1,2 true', 'mediaNeedBytes b']);
    expect(scope.messages().every((m) => m.type === 'ready' || m.id === 1)).toBe(true);
    // Without fonts, the parity path.
    scope.send({ type: 'open', id: 2, file: file('deck.key', [0, 1, 2, 3]), fonts: false });
    scope.drain();
    expect(calls.at(-2)).toBe('open deck.key 4 1,2 false');
  });

  it('answers finish with the deck, serving copied media as slices of the input and the rest as bytes', async () => {
    const calls: string[] = [];
    const scope = await started(fakeCompiler(calls));
    scope.send({ type: 'open', id: 1, file: file('deck.key', [0, 1, 2, 3, 4, 5]) });
    scope.send({ type: 'finish', id: 2 });
    scope.drain();
    expect(scope.types().slice(6)).toEqual(['progress:finish', 'progress:print', 'deck', 'done']);
    const message = scope.messages().find((m) => m.type === 'deck');
    if (message?.type !== 'deck') {
      throw new Error('no deck');
    }
    expect(message.deck).toEqual(deck);
    expect(message.slides).toHaveLength(1);
    const [copied, written] = message.media;
    expect(copied?.blob.type).toBe('image/png');
    expect(Array.from(new Uint8Array(reader.readAsArrayBuffer(copied!.blob)))).toEqual([2, 3, 4]);
    expect(Array.from(new Uint8Array(await written!.blob.arrayBuffer()))).toEqual([9, 9]);
    expect(calls).toContain('mediaBytes media/b.png');
    expect(calls).not.toContain('mediaBytes media/a.png');
  });

  it('sends the shipped fonts with the deck, each file transferred, with the faces, the font issues, the substitutes and the page defaults', async () => {
    const scope = await started(fakeCompiler([]));
    scope.send({ type: 'finish', id: 1 });
    scope.drain();
    const sent = scope.sent.find(({ message }) => message.type === 'deck');
    if (sent?.message.type !== 'deck') {
      throw new Error('no deck');
    }
    const { fonts = [], faces, fontIssues, substitutes, page } = sent.message;
    expect(fonts.map(({ family, weight, unicodeRange }) => [family, weight, unicodeRange])).toEqual([
      ['Lato', '400', null],
      ['Lato', '700', 'U+20-7E'],
    ]);
    expect(fonts.map(({ bytes }) => Array.from(new Uint8Array(bytes as ArrayBuffer)))).toEqual([
      [0, 7],
      [1, 7],
    ]);
    expect(sent.transfer).toEqual(fonts.map(({ bytes }) => bytes));
    expect(faces).toEqual([face]);
    expect(fontIssues).toEqual([{ code: 'MID5410', message: 'm' }]);
    expect(substitutes).toEqual([
      { family: 'Liberation Sans', postscriptName: 'LiberationSans', weight: 400, style: 'normal' },
    ]);
    expect(page).toEqual({ title: 'Deck', description: null });
  });

  it('sends each media need with its source file, and passes the decoded files on', async () => {
    const calls: string[] = [];
    const scope = await started(
      fakeCompiler(calls, {
        addMedia(need, bytes) {
          calls.push(`addMedia ${need.slice(0, 1)} ${bytes.length}`);
          if (bytes.length === 0) {
            throw stageError('request', 'The decoded medium is not taken');
          }
        },
      }),
    );
    scope.send({ type: 'open', id: 1, file: file('deck.key', [0, 1, 2, 3]) });
    scope.drain();
    const message = scope.messages().find((m) => m.type === 'mediaNeeds');
    if (message?.type !== 'mediaNeeds') {
      throw new Error('no media needs');
    }
    const [sliced, written] = message.needs;
    // A stored entry is a slice of the input; another source is the bytes the compiler read.
    expect(sliced?.blob.type).toBe('image/heic');
    expect(Array.from(new Uint8Array(reader.readAsArrayBuffer(sliced!.blob)))).toEqual([1, 2]);
    expect(sliced?.plan).toEqual(plan);
    expect(Array.from(new Uint8Array(await written!.blob.arrayBuffer()))).toEqual([5, 6, 7]);
    expect(written?.plan).toBeNull();
    expect(written?.refusal).toBe('its primary image is a `iovl` item');

    scope.send({ type: 'addMedia', id: 2, need: 'a'.repeat(64), bytes: new ArrayBuffer(3) });
    scope.send({ type: 'addMedia', id: 3, need: 'a'.repeat(64), bytes: new ArrayBuffer(0) });
    scope.drain();
    expect(scope.types().slice(-3)).toEqual(['done', 'done', 'error']);
    expect(scope.messages().at(-1)).toEqual({
      type: 'error',
      id: 3,
      error: { kind: 'request', message: 'The decoded medium is not taken', diagnostics: [] },
    });
    expect(calls.slice(-2)).toEqual(['addMedia a 3', 'addMedia a 0']);
  });

  it('answers imagePlan with the planned pictures, and passes a re-encoded one on', async () => {
    const calls: string[] = [];
    const image = {
      path: 'media/a.png',
      mime: 'image/png',
      bytes: 9,
      width: 40,
      height: 30,
      orientation: 1,
      targetWidth: 20,
      targetHeight: 15,
    };
    const scope = await started(
      fakeCompiler(calls, {
        imagePlan: () => {
          calls.push('imagePlan');
          return [image];
        },
        addImage(path, bytes) {
          calls.push(`addImage ${path} ${bytes.length}`);
          if (bytes.length === 0) {
            throw stageError('request', 'media/a.png: the file is of no known type');
          }
        },
      }),
    );
    scope.send({ type: 'imagePlan', id: 1 });
    scope.send({ type: 'addImage', id: 2, path: 'media/a.png', bytes: new ArrayBuffer(4) });
    scope.send({ type: 'addImage', id: 3, path: 'media/a.png', bytes: new ArrayBuffer(0) });
    scope.drain();
    expect(scope.types().slice(1)).toEqual(['imagePlan', 'done', 'done', 'error']);
    expect(scope.messages()[1]).toEqual({ type: 'imagePlan', id: 1, images: [image] });
    expect(scope.messages().at(-1)).toEqual({
      type: 'error',
      id: 3,
      error: { kind: 'request', message: 'media/a.png: the file is of no known type', diagnostics: [] },
    });
    expect(calls).toEqual(['imagePlan', 'addImage media/a.png 4', 'addImage media/a.png 0']);
  });

  it('answers videoPlan with the planned videos, and reads a re-encoded one from its blob', async () => {
    const calls: string[] = [];
    const video = {
      path: 'media/a.mov',
      mime: 'video/quicktime',
      bytes: 90,
      codec: 'avc1',
      width: 64,
      height: 32,
      rotation: 0,
      durationMs: 1000,
      frames: 30,
      videoBytes: 80,
      audio: null,
      colour: { primaries: 1, transfer: 1, matrix: 1, fullRange: null, wideGamut: false },
      targetWidth: 32,
      targetHeight: 16,
      keep: null,
    };
    const scope = await started(
      fakeCompiler(calls, {
        videoPlan: () => {
          calls.push('videoPlan');
          return [video];
        },
        addVideo(path, source) {
          // The compiler reads the movie's boxes as it checks the file.
          const head = Array.from(source.read(0, 4));
          calls.push(`addVideo ${path} ${source.size} ${head.join(',')}`);
          if (source.size < 6) {
            throw stageError('request', 'media/a.mov: the file is of no known type');
          }
        },
      }),
    );
    scope.send({ type: 'videoPlan', id: 1 });
    scope.send({ type: 'addVideo', id: 2, path: 'media/a.mov', file: file('a.mp4', [0, 0, 0, 24, 102, 116]) });
    scope.send({ type: 'addVideo', id: 3, path: 'media/a.mov', file: file('b.mp4', [1, 2, 3, 4]) });
    scope.drain();
    expect(scope.types().slice(1)).toEqual(['videoPlan', 'done', 'done', 'error']);
    expect(scope.messages()[1]).toEqual({ type: 'videoPlan', id: 1, videos: [video] });
    expect(scope.messages().at(-1)).toEqual({
      type: 'error',
      id: 3,
      error: { kind: 'request', message: 'media/a.mov: the file is of no known type', diagnostics: [] },
    });
    expect(calls).toEqual(['videoPlan', 'addVideo media/a.mov 6 0,0,0,24', 'addVideo media/a.mov 4 1,2,3,4']);
  });

  it('answers alternatePlan with the videos that get a copy, and reads a copy from its blob', async () => {
    const calls: string[] = [];
    const video = {
      path: 'media/a.mov',
      mime: 'video/quicktime',
      bytes: 90,
      codec: 'hvc1',
      width: 64,
      height: 32,
      rotation: 0,
      durationMs: 1000,
      frames: 30,
      videoBytes: 80,
      audio: null,
      colour: { primaries: 1, transfer: 1, matrix: 1, fullRange: null, wideGamut: false },
      targetWidth: 64,
      targetHeight: 32,
      keep: 'it has an alpha channel, which H.264 cannot carry',
      silent: false,
      uncopyable: null,
    };
    const scope = await started(
      fakeCompiler(calls, {
        alternatePlan: () => {
          calls.push('alternatePlan');
          return [video];
        },
        addAlternate(path, source) {
          // The compiler reads the WebM's element headers as it checks the file.
          const head = Array.from(source.read(0, 4));
          calls.push(`addAlternate ${path} ${source.size} ${head.join(',')}`);
          if (head[0] !== 0x1a) {
            throw stageError('request', 'media/a.mov: the file is of no known type, not WebM');
          }
        },
      }),
    );
    scope.send({ type: 'alternatePlan', id: 1 });
    scope.send({ type: 'addAlternate', id: 2, path: 'media/a.mov', file: file('a.webm', [0x1a, 0x45, 0xdf, 0xa3]) });
    scope.send({ type: 'addAlternate', id: 3, path: 'media/a.mov', file: file('b.mp4', [1, 2, 3, 4]) });
    scope.drain();
    expect(scope.types().slice(1)).toEqual(['alternatePlan', 'done', 'done', 'error']);
    expect(scope.messages()[1]).toEqual({ type: 'alternatePlan', id: 1, videos: [video] });
    expect(scope.messages().at(-1)).toMatchObject({ type: 'error', id: 3, error: { kind: 'request' } });
    expect(calls).toEqual([
      'alternatePlan',
      'addAlternate media/a.mov 4 26,69,223,163',
      'addAlternate media/a.mov 4 1,2,3,4',
    ]);
  });

  it('answers selectLocal with the picks, googleFaces with the faces, and passes a chunk on', async () => {
    const calls: string[] = [];
    const scope = await started(fakeCompiler(calls));
    const meta = { postscriptName: 'Lato-Regular', fullName: 'Lato', family: 'Lato', style: 'Regular' };
    scope.send({ type: 'selectLocal', id: 1, fonts: [meta] });
    scope.send({ type: 'googleFaces', id: 2, css: '@font-face{}' });
    scope.send({ type: 'addFont', id: 3, bytes: new ArrayBuffer(2), source: 'google', unicodeRange: 'U+0-FF' });
    scope.drain();
    expect(scope.types().slice(1)).toEqual(['localPicks', 'done', 'googleFaces', 'done', 'needs', 'done']);
    expect(scope.messages()[1]).toEqual({
      type: 'localPicks',
      id: 1,
      picks: [{ index: 0, postscriptName: 'Lato-Regular', family: 'Lato', exactPostscript: true }],
    });
    expect(calls).toEqual([
      `selectLocal ${JSON.stringify([meta])}`,
      'googleFaces @font-face{}',
      'addFont 2 google U+0-FF',
    ]);
  });

  it('serves each medium of an open input once, however often it finishes', async () => {
    const calls: string[] = [];
    const scope = await started(fakeCompiler(calls));
    scope.send({ type: 'open', id: 1, file: file('deck.key', [0, 1, 2, 3, 4, 5]) });
    scope.send({ type: 'finish', id: 2 });
    scope.send({ type: 'addFont', id: 3, bytes: new ArrayBuffer(4), source: 'user' });
    scope.send({ type: 'finish', id: 4 });
    scope.drain();
    const decks = scope.messages().filter((m) => m.type === 'deck');
    expect(decks).toHaveLength(2);
    const [first, second] = decks.map((m) => (m.type === 'deck' ? m.media.map(({ blob }) => blob) : []));
    expect(second?.every((blob, index) => blob === first?.[index])).toBe(true);
    expect(calls.filter((call) => call.startsWith('mediaBytes'))).toEqual(['mediaBytes media/b.png']);

    // Another input's media are its own.
    scope.send({ type: 'open', id: 5, file: file('other.key', [0, 1, 2, 3, 4, 5]) });
    scope.send({ type: 'finish', id: 6 });
    scope.drain();
    expect(calls.filter((call) => call.startsWith('mediaBytes'))).toHaveLength(2);
  });

  it('streams a bundle as transferred chunks and ends with its report', async () => {
    const calls: string[] = [];
    const scope = await started(fakeCompiler(calls));
    scope.send({
      type: 'bundle',
      id: 3,
      settings: { singleFile: true, singleFileThreshold: null, leaveOut: [], include: [] },
    });
    scope.drain();
    const sent = scope.sent.slice(1);
    expect(sent.map(({ message }) => message.type)).toEqual(['progress', 'zipChunk', 'zipChunk', 'done']);
    for (const { message, transfer } of sent) {
      if (message.type === 'zipChunk') {
        expect(transfer).toEqual([message.bytes]);
      }
    }
    expect(sent.at(-1)?.message).toEqual({ type: 'done', id: 3, report });
    expect(calls).toEqual(['bundle {"singleFile":true,"singleFileThreshold":null,"leaveOut":[],"include":[]}']);
  });

  it("answers memory with the module's linear memory as it is when the request runs", async () => {
    let bytes = 65_536;
    const scope = await started(fakeCompiler([]), () => bytes);
    scope.send({ type: 'memory', id: 4 });
    bytes = 3 * 65_536;
    scope.drain();
    expect(scope.messages().slice(1)).toEqual([
      { type: 'memory', id: 4, bytes: 3 * 65_536 },
      { type: 'done', id: 4, report: null },
    ]);
  });

  it('runs requests one at a time in order, each in its own task, and cancels one still queued', async () => {
    const calls: string[] = [];
    const scope = await started(fakeCompiler(calls));
    scope.send({ type: 'finish', id: 1 });
    scope.send({ type: 'finish', id: 2 });
    scope.send({ type: 'addFont', id: 3, bytes: new ArrayBuffer(4), source: 'google' });
    expect(scope.tasks).toHaveLength(1);
    scope.send({ type: 'cancel', id: 2 });
    scope.send({ type: 'cancel', id: 9 });
    scope.drain();
    const ends = scope.messages().filter((m) => m.type === 'done' || m.type === 'error');
    expect(ends).toEqual([
      { type: 'error', id: 2, error: { kind: 'cancelled', message: 'cancelled before it started', diagnostics: [] } },
      { type: 'done', id: 1, report: null },
      { type: 'done', id: 3, report: null },
    ]);
    expect(calls.filter((call) => !/^(mediaBytes|fontBytes)/.test(call))).toEqual(['finish', 'addFont 4 google']);
  });

  it('queues requests until the module loads', async () => {
    const scope = new FakeScope();
    let load: (module: CompilerModule) => void = () => {};
    const module = new Promise<CompilerModule>((resolve) => (load = resolve));
    serve(scope, { module, reader, later: (task) => scope.tasks.push(task) });
    scope.send({ type: 'fontsNeeded', id: 1 });
    expect(scope.tasks).toHaveLength(0);
    load({ version: () => '1', memoryBytes: () => 0, create: () => fakeCompiler([]) });
    await module;
    await Promise.resolve();
    scope.drain();
    expect(scope.types()).toEqual(['ready', 'needs', 'done']);
  });

  it('reports a failed stage as its typed error and keeps serving', async () => {
    const scope = await started(
      fakeCompiler([], {
        finish: () => {
          throw stageError('request', 'no presentation is open');
        },
      }),
    );
    scope.send({ type: 'finish', id: 1 });
    scope.send({ type: 'fontsNeeded', id: 2 });
    scope.drain();
    expect(scope.messages().slice(1)).toEqual([
      { type: 'progress', id: 1, phase: 'finish' },
      { type: 'error', id: 1, error: { kind: 'request', message: 'no presentation is open', diagnostics: [] } },
      { type: 'needs', id: 2, needs: [] },
      { type: 'done', id: 2, report: null },
    ]);
  });

  it('reports a crash as internal and refuses every later request', async () => {
    const scope = await started(
      fakeCompiler([], {
        finish: () => {
          throw new WebAssembly.RuntimeError('unreachable');
        },
      }),
    );
    scope.send({ type: 'finish', id: 1 });
    scope.send({ type: 'fontsNeeded', id: 2 });
    scope.drain();
    const errors = scope.messages().filter((m) => m.type === 'error');
    expect(errors.map((m) => [m.id, m.error.kind])).toEqual([
      [1, 'internal'],
      [2, 'internal'],
    ]);
    expect(errors[0]?.error.message).toContain('unreachable');
    expect(errors[1]?.error.message).toContain('start a new worker');
  });

  it('reports a module that does not load with no request id, and fails the queued requests', async () => {
    const scope = new FakeScope();
    const module = Promise.reject(new Error('no wasm'));
    serve(scope, { module, reader, later: (task) => scope.tasks.push(task) });
    scope.send({ type: 'fontsNeeded', id: 1 });
    await module.catch(() => {});
    await Promise.resolve();
    scope.drain();
    const messages = scope.messages();
    expect(messages.map((m) => (m.type === 'error' ? [m.id, m.error.kind] : m.type))).toEqual([
      [null, 'internal'],
      [1, 'internal'],
    ]);
  });
});
