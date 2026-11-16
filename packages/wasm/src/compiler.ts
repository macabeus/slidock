// The wasm `Compiler` as the worker uses it: its results typed by the module's declarations (generated from the
// Rust types), and a failed stage as a `StageError` carrying its `WorkerError`. Anything else the module throws is a
// crash.
import type {
  AddedFontSource,
  BundleReport,
  BundleSettings,
  FontNeed,
  GoogleFace,
  LocalFontMeta,
  LocalPick,
  PlannedImage,
  PlannedVideo,
  WorkerError,
} from '@slidock/types';

import type {
  DocumentSet,
  Opened,
  OpenedMediaNeed,
  Printed,
  Compiler as WasmModuleCompiler,
} from '../pkg/slidock_wasm.js';

export type {
  DocumentSet,
  MediaRange,
  Opened,
  OpenedMediaNeed,
  Printed,
  PrintedFont,
  PrintedMedia,
} from '../pkg/slidock_wasm.js';

/** The input, read by byte range (`RangeSource` in the wasm module's declarations). */
export interface RangeSource {
  /** In bytes. */
  readonly size: number;
  /** Exactly `length` bytes from `offset`. */
  read(offset: number, length: number): Uint8Array;
}

/** The surface of the wasm module's `Compiler` class the worker calls. */
export type WasmCompiler = Pick<
  WasmModuleCompiler,
  | 'open'
  | 'fontNeeds'
  | 'addFont'
  | 'selectLocal'
  | 'googleFaces'
  | 'mediaNeeds'
  | 'mediaNeedBytes'
  | 'addMedia'
  | 'finish'
  | 'print'
  | 'fontBytes'
  | 'mediaBytes'
  | 'documents'
  | 'imagePlan'
  | 'addImage'
  | 'videoPlan'
  | 'addVideo'
  | 'alternatePlan'
  | 'addAlternate'
  | 'bundle'
>;

/** A stage that failed, with the error the worker reports. */
export class StageError extends Error {
  readonly error: WorkerError;

  constructor(error: WorkerError) {
    super(error.message);
    this.name = 'StageError';
    this.error = error;
  }
}

function isWorkerError(value: unknown): value is WorkerError {
  return (
    typeof value === 'object' &&
    value !== null &&
    'kind' in value &&
    typeof value.kind === 'string' &&
    'message' in value &&
    typeof value.message === 'string' &&
    'diagnostics' in value &&
    Array.isArray(value.diagnostics)
  );
}

/** Runs one call into the module: a failed stage throws the JSON of its `WorkerError`, which becomes a `StageError`. */
function call<T>(run: () => T): T {
  try {
    return run();
  } catch (thrown) {
    if (typeof thrown === 'string') {
      let parsed: unknown;
      try {
        parsed = JSON.parse(thrown);
      } catch {
        parsed = undefined;
      }
      if (isWorkerError(parsed)) {
        throw new StageError(parsed);
      }
    }
    throw thrown;
  }
}

/** The compiler, one stage per call. */
export class Compiler {
  readonly #wasm: WasmCompiler;

  constructor(wasm: WasmCompiler) {
    this.#wasm = wasm;
  }

  /**
   * Opens `source` as the presentation `name`. With `fonts` (the default), its embedded fonts are added and `finish`
   * ships the faces the deck draws; without, it binds against the built-in fonts only (the parity path).
   */
  open(name: string, source: RangeSource, fonts = true): Opened {
    return call(() => this.#wasm.open(name, source, fonts));
  }

  fontNeeds(): FontNeed[] {
    return call(() => this.#wasm.fontNeeds());
  }

  /** Adds a font file; `unicodeRange` marks one per-script chunk of a face (a `googleFaces` face's range). */
  addFont(bytes: Uint8Array, source: AddedFontSource, unicodeRange: string | null = null): FontNeed[] {
    return call(() => this.#wasm.addFont(bytes, source, unicodeRange ?? undefined));
  }

  /** The installed faces worth reading for the faces still open, by index into `fonts`. */
  selectLocal(fonts: LocalFontMeta[]): LocalPick[] {
    return call(() => this.#wasm.selectLocal(JSON.stringify(fonts)));
  }

  /** The faces of a Google Fonts `css2` response worth fetching. */
  googleFaces(css: string): GoogleFace[] {
    return call(() => this.#wasm.googleFaces(css));
  }

  /** The media the app may decode (HEIC photos), each with its decode plan or why it has none. */
  mediaNeeds(): OpenedMediaNeed[] {
    return call(() => this.#wasm.mediaNeeds());
  }

  /** The source file of a media need with no `range`, in an `ArrayBuffer` of its own. */
  mediaNeedBytes(need: string): Uint8Array<ArrayBuffer> {
    return call(() => this.#wasm.mediaNeedBytes(need)) as Uint8Array<ArrayBuffer>;
  }

  /** Adds the app's decode of a media need (a PNG or JPEG of its plan's output size) for the next `finish`. */
  addMedia(need: string, bytes: Uint8Array): void {
    call(() => this.#wasm.addMedia(need, bytes));
  }

  finish(): void {
    call(() => this.#wasm.finish());
  }

  print(): Printed {
    return call(() => this.#wasm.print());
  }

  /** The file of the printed deck's `index`th shipped font rule (`Printed.fonts`), in an `ArrayBuffer` of its own. */
  fontBytes(index: number): Uint8Array<ArrayBuffer> {
    return call(() => this.#wasm.fontBytes(index)) as Uint8Array<ArrayBuffer>;
  }

  /** A fresh copy of the bytes, in an `ArrayBuffer` of their own. */
  mediaBytes(path: string): Uint8Array<ArrayBuffer> {
    return call(() => this.#wasm.mediaBytes(path)) as Uint8Array<ArrayBuffer>;
  }

  documents(): DocumentSet {
    return call(() => this.#wasm.documents());
  }

  /** The pictures of the printed deck the app may re-encode smaller, forgetting those it handed over before. */
  imagePlan(): PlannedImage[] {
    return call(() => this.#wasm.imagePlan());
  }

  /** Hands over the app's re-encode of a planned picture for the bundles with `compressImages`. */
  addImage(path: string, bytes: Uint8Array): void {
    call(() => this.#wasm.addImage(path, bytes));
  }

  /** The videos of the printed deck, each with its planned size or why it stays as it is. */
  videoPlan(): PlannedVideo[] {
    return call(() => this.#wasm.videoPlan());
  }

  /**
   * Hands over the app's re-encode of a planned video, read from `source` as it is checked now and bundled later, for
   * the bundles with `compressVideos`.
   */
  addVideo(path: string, source: RangeSource): void {
    call(() => this.#wasm.addVideo(path, source));
  }

  /** The videos of the printed deck that get a copy other browsers play, each with the size to make it at. */
  alternatePlan(): PlannedVideo[] {
    return call(() => this.#wasm.alternatePlan());
  }

  /**
   * Hands over the app's copy of a video, read from `source` as it is checked now and bundled later, for the bundles
   * with `alternateVideos`.
   */
  addAlternate(path: string, source: RangeSource): void {
    call(() => this.#wasm.addAlternate(path, source));
  }

  /** Bundles the last printed deck, handing `sink` each chunk (at most 1 MiB, its own `ArrayBuffer`) in order. */
  bundle(settings: BundleSettings, sink: (chunk: Uint8Array<ArrayBuffer>) => void): BundleReport {
    return call(() => this.#wasm.bundle(JSON.stringify(settings), (chunk) => sink(chunk as Uint8Array<ArrayBuffer>)));
  }
}

/** Something that reads a `Blob` synchronously: `FileReaderSync` in a worker. */
export interface SyncBlobReader {
  readAsArrayBuffer(blob: Blob): ArrayBuffer;
}

/** A `File` (or any `Blob`) as a range source: each read is one `slice`, read whole, so the file is never copied. */
export function blobSource(blob: Blob, reader: SyncBlobReader): RangeSource {
  return {
    size: blob.size,
    read(offset, length) {
      const bytes = reader.readAsArrayBuffer(blob.slice(offset, offset + length));
      if (bytes.byteLength !== length) {
        throw new RangeError(`read ${bytes.byteLength} bytes of ${length} at ${offset}`);
      }
      return new Uint8Array(bytes);
    },
  };
}
