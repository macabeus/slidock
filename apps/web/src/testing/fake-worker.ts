import type {
  AddedFontSource,
  BundleReport,
  DeckDiagnostic,
  FontNeed,
  FromWorker,
  LocalFontMeta,
  LocalPick,
  MediaNeed,
  PlannedImage,
  PlannedVideo,
  ToWorker,
  WorkerError,
} from '@slidock/types';

import type { CompiledDeck } from '../compile/client';
import type { WorkerPort } from '../compile/port';

type RequestType =
  | 'open'
  | 'finish'
  | 'bundle'
  | 'addFont'
  | 'addMedia'
  | 'imagePlan'
  | 'addImage'
  | 'videoPlan'
  | 'addVideo'
  | 'alternatePlan'
  | 'addAlternate';

/** A font file the fake worker took since the last `open`. */
export interface FakeFont {
  source: AddedFontSource;
  bytes: ArrayBuffer;
  unicodeRange: string | null;
}

export interface FakeWorkerScript {
  version?: string;
  /** Fail to load: an `error` with `id: null` instead of `ready`. */
  loadError?: string;
  /** Hold `ready` until `becomeReady()`. */
  holdReady?: boolean;
  /** The deck each `finish` prints, from the fonts added since the last `open`. */
  deck?: CompiledDeck | ((fonts: readonly FakeFont[]) => CompiledDeck);
  diagnostics?: DeckDiagnostic[];
  /** What is still missing, from the fonts added since the last `open`. */
  needs?: FontNeed[] | ((fonts: readonly FakeFont[]) => FontNeed[]);
  /** The installed faces a `selectLocal` picks; none by default. */
  pickLocal?(fonts: LocalFontMeta[]): LocalPick[];
  /** `false`: answer `selectLocal` with `done` only, as a worker that cannot match installed fonts. */
  matchesLocal?: boolean;
  /** Refuses a font file with this message (an `error` of kind `request`); `null` takes it. */
  refuseFont?(font: FakeFont): string | null;
  /** The media each `open` lists for the app to decode; none by default. */
  mediaNeeds?: MediaNeed[];
  /** Refuses a decoded medium with this message; `null` takes it. */
  refuseMedia?(need: string, bytes: ArrayBuffer): string | null;
  /** The pictures each `imagePlan` lists for the app to re-encode; none by default. */
  images?: PlannedImage[];
  /** Refuses a re-encoded picture with this message; `null` takes it. */
  refuseImage?(path: string, bytes: ArrayBuffer): string | null;
  /** The videos each `videoPlan` lists; none by default. */
  videos?: PlannedVideo[];
  /** Refuses a re-encoded video with this message; `null` takes it. */
  refuseVideo?(path: string, file: Blob): string | null;
  /** The videos each `alternatePlan` lists; none by default. */
  alternates?: PlannedVideo[];
  /** Refuses a copy with this message; `null` takes it. */
  refuseAlternate?(path: string, file: Blob): string | null;
  errors?: Partial<Record<RequestType, WorkerError>>;
  /** Requests answered only on `release(type)`. */
  hold?: RequestType[];
  zipChunks?: Uint8Array[];
  report?: BundleReport;
}

/** A compile worker that answers the protocol from a script, asynchronously, as the real worker does. */
export class FakeWorker implements WorkerPort {
  readonly sent: ToWorker[] = [];
  /** The fonts taken since the last `open`. */
  fonts: FakeFont[] = [];
  /** The decoded media taken since the last `open`, in order. */
  media: { need: string; bytes: ArrayBuffer }[] = [];
  /** The re-encoded videos taken since the last `videoPlan`, in order. */
  videos: { path: string; file: Blob }[] = [];
  /** The copies taken since the last `alternatePlan`, in order. */
  alternates: { path: string; file: Blob }[] = [];
  /** The re-encoded pictures taken since the last `imagePlan`, in order. */
  images: { path: string; bytes: ArrayBuffer }[] = [];
  terminated = false;
  #script: FakeWorkerScript;
  #onMessage: (message: FromWorker) => void = () => {};
  #held = new Map<RequestType, (() => void)[]>();

  constructor(script: FakeWorkerScript = {}) {
    this.#script = script;
  }

  listen(onMessage: (message: FromWorker) => void, _onCrash: (message: string) => void) {
    this.#onMessage = onMessage;
    if (this.#script.loadError !== undefined) {
      const message = this.#script.loadError;
      this.#emit({ type: 'error', id: null, error: { kind: 'internal', message, diagnostics: [] } });
    } else if (!this.#script.holdReady) {
      this.becomeReady();
    }
  }

  becomeReady() {
    this.#emit({ type: 'ready', version: this.#script.version ?? '0.0.0-fake' });
  }

  #emit(message: FromWorker) {
    setTimeout(() => {
      if (!this.terminated) {
        this.#onMessage(message);
      }
    }, 0);
  }

  #needs(): FontNeed[] {
    const needs = this.#script.needs ?? [];
    return typeof needs === 'function' ? needs(this.fonts) : needs;
  }

  post(message: ToWorker) {
    this.sent.push(message);
    if (message.type === 'fontsNeeded') {
      this.#emit({ type: 'needs', id: message.id, needs: this.#needs() });
      this.#emit({ type: 'done', id: message.id, report: null });
    } else if (message.type === 'selectLocal') {
      if (this.#script.matchesLocal !== false) {
        this.#emit({ type: 'localPicks', id: message.id, picks: this.#script.pickLocal?.(message.fonts) ?? [] });
      }
      this.#emit({ type: 'done', id: message.id, report: null });
    } else if (
      message.type === 'open' ||
      message.type === 'finish' ||
      message.type === 'bundle' ||
      message.type === 'addFont' ||
      message.type === 'addMedia' ||
      message.type === 'imagePlan' ||
      message.type === 'addImage' ||
      message.type === 'videoPlan' ||
      message.type === 'addVideo' ||
      message.type === 'alternatePlan' ||
      message.type === 'addAlternate'
    ) {
      const type = message.type;
      if (type === 'open' || type === 'finish' || type === 'bundle') {
        this.#emit({ type: 'progress', id: message.id, phase: type });
      }
      const respond = () => this.#respond(message);
      if (this.#script.hold?.includes(type)) {
        this.#held.set(type, [...(this.#held.get(type) ?? []), respond]);
      } else {
        respond();
      }
    }
  }

  /** Sends the answers of held requests of `type`. */
  release(type: RequestType) {
    const held = this.#held.get(type) ?? [];
    this.#held.delete(type);
    for (const respond of held) {
      respond();
    }
  }

  #respond(message: Extract<ToWorker, { type: RequestType }>) {
    const { id, type } = message;
    const fail = this.#script.errors?.[type];
    if (fail) {
      this.#emit({ type: 'error', id, error: fail });
      return;
    }
    if (message.type === 'addFont') {
      const font: FakeFont = {
        source: message.source,
        bytes: message.bytes,
        unicodeRange: message.unicodeRange ?? null,
      };
      const refused = this.#script.refuseFont?.(font) ?? null;
      if (refused !== null) {
        this.#emit({ type: 'error', id, error: { kind: 'request', message: refused, diagnostics: [] } });
        return;
      }
      this.fonts.push(font);
      this.#emit({ type: 'needs', id, needs: this.#needs() });
      this.#emit({ type: 'done', id, report: null });
    } else if (message.type === 'addMedia') {
      const refused = this.#script.refuseMedia?.(message.need, message.bytes) ?? null;
      if (refused !== null) {
        this.#emit({ type: 'error', id, error: { kind: 'request', message: refused, diagnostics: [] } });
        return;
      }
      this.media.push({ need: message.need, bytes: message.bytes });
      this.#emit({ type: 'done', id, report: null });
    } else if (message.type === 'videoPlan') {
      this.videos = [];
      this.#emit({ type: 'videoPlan', id, videos: this.#script.videos ?? [] });
      this.#emit({ type: 'done', id, report: null });
    } else if (message.type === 'addVideo') {
      const refused = this.#script.refuseVideo?.(message.path, message.file as Blob) ?? null;
      if (refused !== null) {
        this.#emit({ type: 'error', id, error: { kind: 'request', message: refused, diagnostics: [] } });
        return;
      }
      this.videos.push({ path: message.path, file: message.file as Blob });
      this.#emit({ type: 'done', id, report: null });
    } else if (message.type === 'alternatePlan') {
      this.alternates = [];
      this.#emit({ type: 'alternatePlan', id, videos: this.#script.alternates ?? [] });
      this.#emit({ type: 'done', id, report: null });
    } else if (message.type === 'addAlternate') {
      const refused = this.#script.refuseAlternate?.(message.path, message.file as Blob) ?? null;
      if (refused !== null) {
        this.#emit({ type: 'error', id, error: { kind: 'request', message: refused, diagnostics: [] } });
        return;
      }
      this.alternates.push({ path: message.path, file: message.file as Blob });
      this.#emit({ type: 'done', id, report: null });
    } else if (message.type === 'imagePlan') {
      this.images = [];
      this.#emit({ type: 'imagePlan', id, images: this.#script.images ?? [] });
      this.#emit({ type: 'done', id, report: null });
    } else if (message.type === 'addImage') {
      const refused = this.#script.refuseImage?.(message.path, message.bytes) ?? null;
      if (refused !== null) {
        this.#emit({ type: 'error', id, error: { kind: 'request', message: refused, diagnostics: [] } });
        return;
      }
      this.images.push({ path: message.path, bytes: message.bytes });
      this.#emit({ type: 'done', id, report: null });
    } else if (type === 'open') {
      this.fonts = [];
      this.media = [];
      if (this.#script.diagnostics?.length) {
        this.#emit({ type: 'diagnostics', id, diagnostics: this.#script.diagnostics });
      }
      this.#emit({ type: 'needs', id, needs: this.#needs() });
      this.#emit({ type: 'mediaNeeds', id, needs: this.#script.mediaNeeds ?? [] });
      this.#emit({ type: 'done', id, report: null });
    } else if (type === 'finish') {
      this.#emit({ type: 'progress', id, phase: 'print' });
      // As the worker does: a finish sends the complete list, the open's diagnostics included.
      if (this.#script.diagnostics?.length) {
        this.#emit({ type: 'diagnostics', id, diagnostics: this.#script.diagnostics });
      }
      const script = this.#script.deck;
      const deck = typeof script === 'function' ? script(this.fonts) : script;
      if (deck) {
        // `postMessage` clones every `Blob`: each finish hands back new objects for the same media.
        const media = deck.media.map((medium) => ({
          ...medium,
          blob: medium.blob.slice(0, medium.blob.size, medium.mime),
        }));
        this.#emit({ type: 'deck', id, ...deck, media });
      }
      this.#emit({ type: 'done', id, report: null });
    } else {
      for (const chunk of this.#script.zipChunks ?? []) {
        this.#emit({ type: 'zipChunk', id, bytes: chunk.slice().buffer });
      }
      this.#emit({
        type: 'done',
        id,
        report: this.#script.report ?? { files: [], bytes: 0, singleFile: false, fontsLeftOut: [] },
      });
    }
  }

  terminate() {
    this.terminated = true;
  }
}
