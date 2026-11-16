import type {
  BundleReport,
  BundleSettings,
  DeckDiagnostic,
  FontNeed,
  FromWorker,
  LocalPick,
  MediaNeed,
  Phase,
  PlannedImage,
  PlannedVideo,
  ToWorker,
  WorkerError,
} from '@slidock/types';

import type { WorkerPort } from './port';

type DeckMessage = Extract<FromWorker, { type: 'deck' }>;

/** A compiled deck as the worker hands it over. */
export type CompiledDeck = Omit<DeckMessage, 'type' | 'id'>;

/** Messages a request receives before it ends. */
export interface RequestHandlers {
  onProgress?(phase: Phase): void;
  onDiagnostics?(diagnostics: DeckDiagnostic[]): void;
  onNeeds?(needs: FontNeed[]): void;
  /** The media the app may decode (HEIC photos), after `open`. */
  onMediaNeeds?(needs: MediaNeed[]): void;
  onLocalPicks?(picks: LocalPick[]): void;
}

/** A request that ended with the worker's `error`, or the worker that died under it. */
export class RequestError extends Error {
  readonly error: WorkerError;

  constructor(error: WorkerError) {
    super(error.message);
    this.name = 'RequestError';
    this.error = error;
  }
}

interface Pending extends RequestHandlers {
  resolve(message: Extract<FromWorker, { type: 'done' }>): void;
  reject(error: RequestError): void;
  onDeck?(message: DeckMessage): void;
  onChunk?(bytes: ArrayBuffer): void;
  onImagePlan?(images: PlannedImage[]): void;
  onVideoPlan?(videos: PlannedVideo[]): void;
  onAlternatePlan?(videos: PlannedVideo[]): void;
}

/** A request as the app sends it, before the client numbers it. */
export type Request = ToWorker extends infer M ? (M extends { id: number } ? Omit<M, 'id'> : never) : never;

/** What a request may pass besides its handlers. */
export interface RequestOptions extends RequestHandlers {
  /** Moved to the worker with the message (an `addFont`'s bytes). */
  transfer?: Transferable[];
}

export type CompilerStatus =
  { state: 'loading' } | { state: 'ready'; version: string } | { state: 'failed'; message: string };

/**
 * Talks to one compile worker (packages/wasm/README.md): numbers requests, routes the worker's messages to them,
 * and ends each with its `done` or `error`. Requests sent before the worker is ready wait for it; a worker that
 * fails to load or crashes fails every request.
 */
export class CompileClient {
  #port: WorkerPort;
  #nextId = 1;
  #pending = new Map<number, Pending>();
  #queue: { message: ToWorker; transfer: Transferable[] }[] = [];
  #status: CompilerStatus = { state: 'loading' };
  #onStatus: (status: CompilerStatus) => void;

  constructor(port: WorkerPort, onStatus: (status: CompilerStatus) => void = () => {}) {
    this.#port = port;
    this.#onStatus = onStatus;
    port.listen(
      (message) => this.#receive(message),
      (message) => this.#crash(message),
    );
  }

  get status(): CompilerStatus {
    return this.#status;
  }

  /** Opens a presentation (replacing any open one) and runs the passes that need no font. */
  open(file: File, handlers: RequestHandlers = {}): Promise<void> {
    return this.request({ type: 'open', file }, handlers).then(() => undefined);
  }

  /**
   * Adds the app's decode of a media need for the next `finish`, moving `bytes` to the worker. A file the compiler
   * refuses rejects with its `request` error.
   */
  addMedia(need: string, bytes: ArrayBuffer): Promise<void> {
    return this.request({ type: 'addMedia', need, bytes }, { transfer: [bytes] }).then(() => undefined);
  }

  /** Runs the passes from bind-fonts and prints the deck. */
  finish(handlers: RequestHandlers = {}): Promise<CompiledDeck> {
    let deck: DeckMessage | undefined;
    return this.#send({ type: 'finish' }, { ...handlers, onDeck: (message) => (deck = message) }).then(() => {
      if (!deck) {
        throw new RequestError({ kind: 'internal', message: 'finish ended without a deck', diagnostics: [] });
      }
      const { type: _type, id: _id, ...compiled } = deck;
      return compiled;
    });
  }

  /** The pictures of the last finished deck the app may re-encode smaller, forgetting those handed over before. */
  imagePlan(): Promise<PlannedImage[]> {
    let images: PlannedImage[] | undefined;
    return this.#send({ type: 'imagePlan' }, { onImagePlan: (list) => (images = list) }).then(() => {
      if (!images) {
        throw new RequestError({ kind: 'internal', message: 'imagePlan ended without a plan', diagnostics: [] });
      }
      return images;
    });
  }

  /**
   * Hands over the app's re-encode of a planned picture, moving `bytes` to the worker, for the bundles with
   * `compressImages`. A file the compiler refuses rejects with its `request` error.
   */
  addImage(path: string, bytes: ArrayBuffer): Promise<void> {
    return this.request({ type: 'addImage', path, bytes }, { transfer: [bytes] }).then(() => undefined);
  }

  /** The videos of the last finished deck, each with its planned size or why it stays, forgetting those handed over. */
  videoPlan(): Promise<PlannedVideo[]> {
    let videos: PlannedVideo[] | undefined;
    return this.#send({ type: 'videoPlan' }, { onVideoPlan: (list) => (videos = list) }).then(() => {
      if (!videos) {
        throw new RequestError({ kind: 'internal', message: 'videoPlan ended without a plan', diagnostics: [] });
      }
      return videos;
    });
  }

  /**
   * Hands over the app's re-encode of a planned video (the worker reads the `Blob` as it checks and bundles it), for
   * the bundles with `compressVideos`. A file the compiler refuses rejects with its `request` error.
   */
  addVideo(path: string, file: Blob): Promise<void> {
    return this.request({ type: 'addVideo', path, file }).then(() => undefined);
  }

  /** The videos of the last finished deck that get a copy other browsers play, forgetting the copies handed over. */
  alternatePlan(): Promise<PlannedVideo[]> {
    let videos: PlannedVideo[] | undefined;
    return this.#send({ type: 'alternatePlan' }, { onAlternatePlan: (list) => (videos = list) }).then(() => {
      if (!videos) {
        throw new RequestError({ kind: 'internal', message: 'alternatePlan ended without a plan', diagnostics: [] });
      }
      return videos;
    });
  }

  /**
   * Hands over the app's copy of a video (the worker reads the `Blob` as it checks and bundles it), for the bundles
   * with `alternateVideos`. A file the compiler refuses rejects with its `request` error.
   */
  addAlternate(path: string, file: Blob): Promise<void> {
    return this.request({ type: 'addAlternate', path, file }).then(() => undefined);
  }

  /** Bundles the last finished deck as a zip; `onChunk` receives its bytes in order. */
  bundle(settings: BundleSettings, onChunk: (bytes: ArrayBuffer) => void, handlers: RequestHandlers = {}) {
    return this.#send({ type: 'bundle', settings }, { ...handlers, onChunk }).then((done) => {
      if (!done.report) {
        throw new RequestError({ kind: 'internal', message: 'bundle ended without a report', diagnostics: [] });
      }
      return done.report satisfies BundleReport;
    });
  }

  /** Stops the worker; pending requests fail as cancelled. */
  terminate() {
    this.#port.terminate();
    this.#failAll({ kind: 'cancelled', message: 'The compile worker was stopped', diagnostics: [] });
    this.#queue = [];
  }

  /** Sends any request; resolves with its `done`, or rejects with its `error` as a `RequestError`. */
  request(request: Request, options: RequestOptions = {}): Promise<Extract<FromWorker, { type: 'done' }>> {
    return this.#send(request, options);
  }

  #send(
    request: Request,
    { transfer = [], ...handlers }: RequestOptions & Omit<Pending, 'resolve' | 'reject'>,
  ): Promise<Extract<FromWorker, { type: 'done' }>> {
    if (this.#status.state === 'failed') {
      return Promise.reject(new RequestError({ kind: 'internal', message: this.#status.message, diagnostics: [] }));
    }
    const id = this.#nextId++;
    const message = { ...request, id } as ToWorker;
    return new Promise((resolve, reject) => {
      this.#pending.set(id, { ...handlers, resolve, reject });
      if (this.#status.state === 'ready') {
        this.#port.post(message, transfer);
      } else {
        this.#queue.push({ message, transfer });
      }
    });
  }

  #setStatus(status: CompilerStatus) {
    this.#status = status;
    this.#onStatus(status);
  }

  #receive(message: FromWorker) {
    if (message.type === 'ready') {
      this.#setStatus({ state: 'ready', version: message.version });
      for (const { message: queued, transfer } of this.#queue) {
        this.#port.post(queued, transfer);
      }
      this.#queue = [];
      return;
    }
    if (message.type === 'error' && message.id === null) {
      this.#crash(message.error.message);
      return;
    }
    const id = message.id;
    const pending = id === null ? undefined : this.#pending.get(id);
    if (id === null || !pending) {
      return;
    }
    switch (message.type) {
      case 'progress':
        pending.onProgress?.(message.phase);
        break;
      case 'diagnostics':
        pending.onDiagnostics?.(message.diagnostics);
        break;
      case 'needs':
        pending.onNeeds?.(message.needs);
        break;
      case 'mediaNeeds':
        pending.onMediaNeeds?.(message.needs);
        break;
      case 'localPicks':
        pending.onLocalPicks?.(message.picks);
        break;
      case 'deck':
        pending.onDeck?.(message);
        break;
      case 'imagePlan':
        pending.onImagePlan?.(message.images);
        break;
      case 'videoPlan':
        pending.onVideoPlan?.(message.videos);
        break;
      case 'alternatePlan':
        pending.onAlternatePlan?.(message.videos);
        break;
      case 'zipChunk':
        pending.onChunk?.(message.bytes);
        break;
      case 'done':
        this.#pending.delete(id);
        pending.resolve(message);
        break;
      case 'error':
        this.#pending.delete(id);
        pending.reject(new RequestError(message.error));
        break;
    }
  }

  #crash(message: string) {
    this.#setStatus({ state: 'failed', message });
    this.#failAll({ kind: 'internal', message, diagnostics: [] });
    this.#queue = [];
  }

  #failAll(error: WorkerError) {
    const pending = [...this.#pending.values()];
    this.#pending.clear();
    for (const request of pending) {
      request.reject(new RequestError(error));
    }
  }
}
