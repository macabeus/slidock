import type {
  BundleReport,
  BundleSettings,
  Deck,
  DeckDiagnostic,
  FontNeed,
  MediaNeed,
  Phase,
  PlannedVideo,
  SlideTemplate,
  WorkerError,
} from '@slidock/types';

import { MediaUrls } from '../deck/media';
import { FontsController, type FontsEnvironment, type FontsHost } from '../fonts/controller';
import { FontRequests } from '../fonts/worker';
import type { AlphaCopier } from '../media/alpha';
import { type CompressOutcome, ENCODER_QUALITY, type ImageQuality } from '../media/compress';
import type { ImageCompressor, VideoCompressor } from '../media/compressor';
import { type DecodedPhotos, type PhotoOutcome, decodable, decodePhotos, explainPhotos } from '../media/needs';
import { type PhotoDecoder, PhotoUndecodable } from '../media/photos';
import type { VideoOutcome, VideoQuality } from '../media/video';
import { type AlternatesSummary, type MadeCopies, makeAlternates } from './alternates';
import { CompileClient, type CompiledDeck, type CompilerStatus, RequestError } from './client';
import { type DroppedItem, UnsupportedInput, fileFromDrop } from './input';
import type { WorkerPort } from './port';

/** The phases of a compile, in order, as the worker reports them. */
export const COMPILE_PHASES: readonly Exclude<Phase, 'bundle'>[] = ['open', 'finish', 'print'];

export type AppError =
  | { kind: 'unsupported'; message: string }
  | { kind: 'compile'; error: WorkerError }
  | { kind: 'compiler'; message: string };

export interface LoadedDeck {
  name: string;
  deck: Deck;
  slides: readonly SlideTemplate[];
  media: MediaUrls;
  /** Every diagnostic of the compile, in output order. */
  diagnostics: readonly DeckDiagnostic[];
  /** The faces no font provides (the fonts panel). */
  needs: readonly FontNeed[];
  /** The media handles as the worker sent them, for the presenter window. */
  compiled: CompiledDeck;
  /** What became of each HEIC photo the deck shows. */
  photos: readonly PhotoOutcome[];
}

export type View =
  | { kind: 'empty' }
  | { kind: 'preparing'; name: string }
  | {
      kind: 'compiling';
      name: string;
      phase: Phase | null;
      done: readonly Phase[];
      warnings: number;
      errors: number;
      /** The HEIC photos the browser decodes after `open`; `null` for a deck with none. */
      photos: { total: number; done: number; active: boolean } | null;
    }
  | ({ kind: 'deck' } & LoadedDeck)
  | { kind: 'error'; name: string | null; error: AppError };

/** What re-encoding the pictures of a download did. */
export interface ImagesSummary {
  /** The pictures the compiler planned. */
  planned: number;
  /** Those written smaller. */
  compressed: number;
  /** Their originals' bytes, and the bytes written in their place. */
  before: number;
  after: number;
  /** The planned pictures kept as they are, with why. */
  kept: readonly { path: string; reason: string }[];
  /** In milliseconds, from the plan to the last picture handed over. */
  elapsed: number;
}

/** What re-encoding the videos of a download did. */
export interface VideosSummary extends ImagesSummary {
  /** The re-encoded videos whose sound was carried over as it is, and those whose sound was re-encoded. */
  soundCopied: number;
  soundEncoded: number;
  /** Of `compressed`, the videos copied as they are without their sound (not re-encoded). */
  copied: number;
  /** The videos whose sound, never heard, was left out, and the bytes of that sound. */
  soundRemoved: number;
  soundRemovedBytes: number;
}

/** How far the videos' re-encode is: videos and frames done of all, when it started and when this was so. */
export interface VideosProgress {
  done: number;
  total: number;
  frames: number;
  totalFrames: number;
  /** `performance.now()` at the start, and now. */
  started: number;
  updated: number;
}

export type BundleState =
  | { state: 'idle' }
  | {
      state: 'running';
      bytes: number;
      images?: { done: number; total: number };
      videos?: VideosProgress;
      /** The copies of the videos only some browsers play, while they are made. */
      alternates?: VideosProgress;
    }
  | {
      state: 'done';
      report: BundleReport;
      blob: Blob;
      images?: ImagesSummary;
      videos?: VideosSummary;
      alternates?: AlternatesSummary;
    }
  | { state: 'failed'; error: WorkerError };

export interface SessionState {
  compiler: CompilerStatus;
  view: View;
  bundle: BundleState;
  /** The open deck's fonts flow; `null` while no deck is open. */
  fonts: FontsController | null;
}

const ERROR_SEVERITIES = new Set(['error', 'decline']);

/** A compressor for a browser with none: every picture stays as it is. */
const NO_IMAGE_COMPRESSOR: ImageCompressor = {
  compress: () => Promise.resolve({ kind: 'kept', reason: 'the app has no picture encoder', elapsed: 0 }),
  dispose: () => {},
};

/** A compressor for a browser with none: every video stays as it is. */
const NO_VIDEO_COMPRESSOR: VideoCompressor = {
  compress: () => Promise.resolve({ kind: 'kept', reason: 'the app has no video encoder', elapsed: 0 }),
  dispose: () => {},
};

/** A copier for a browser with none: every video that gets a copy is left without one. */
const NO_ALPHA_COPIER: AlphaCopier = {
  copy: () => Promise.resolve({ kind: 'kept', reason: 'the app makes no copies of videos', elapsed: 0 }),
  dispose: () => {},
};

/** A decoder for a browser with none: every photo stays omitted. */
const NO_PHOTO_DECODER: PhotoDecoder = {
  decode: () => Promise.reject(new PhotoUndecodable(['the app has no photo decoder'])),
  dispose: () => {},
};

function sameFamilies(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((family) => b.includes(family));
}

/**
 * The app's state around the compile worker, as an external store for `useSyncExternalStore`: open a file or a drop,
 * follow the compile, hold the deck and its media URLs, and bundle the zip. A new input while a compile runs, or a
 * cancel, stops the worker and starts a fresh one (a running request cannot be interrupted otherwise).
 */
export class DeckSession {
  #startWorker: () => WorkerPort;
  #fontsEnv: FontsEnvironment;
  #photoDecoder: () => PhotoDecoder;
  #imageCompressor: () => ImageCompressor;
  /** The open deck's picture compressor, started by its first compressed download. */
  #images: ImageCompressor | null = null;
  /** The open deck's re-encoded pictures, by path, size and quality: a download again reuses them. */
  #encoded = new Map<string, Promise<CompressOutcome>>();
  #videoCompressor: () => VideoCompressor;
  /** The open deck's video compressor, started by its first download with compressed videos. */
  #videos: VideoCompressor | null = null;
  /** The open deck's re-encoded videos, by path, size and quality: a download again reuses those that finished. */
  #encodedVideos = new Map<string, Promise<VideoOutcome>>();
  #alphaCopier: () => AlphaCopier;
  /** The open deck's copier, started by its first download with copies. */
  #copier: AlphaCopier | null = null;
  /** The open deck's copies, by path and size: a download again reuses those that finished. */
  #copies: MadeCopies = new Map();
  /** The open deck's videos that get a copy, as the download dialog first asked. */
  #alternatePlan: Promise<PlannedVideo[]> | null = null;
  /** Counts the downloads: a cancelled one drops its results. */
  #bundles = 0;
  /** The photo decoder of the running load: at most one, stopped by a cancel or another load. */
  #photos: PhotoDecoder | null = null;
  #client: CompileClient | null = null;
  #state: SessionState;
  #listeners = new Set<() => void>();
  #generation = 0;

  constructor(
    startWorker: () => WorkerPort,
    fonts: FontsEnvironment,
    photos: () => PhotoDecoder = () => NO_PHOTO_DECODER,
    images: () => ImageCompressor = () => NO_IMAGE_COMPRESSOR,
    videos: () => VideoCompressor = () => NO_VIDEO_COMPRESSOR,
    alternates: () => AlphaCopier = () => NO_ALPHA_COPIER,
  ) {
    this.#videoCompressor = videos;
    this.#alphaCopier = alternates;
    this.#startWorker = startWorker;
    this.#fontsEnv = fonts;
    this.#photoDecoder = photos;
    this.#imageCompressor = images;
    this.#state = {
      compiler: { state: 'loading' },
      view: { kind: 'empty' },
      bundle: { state: 'idle' },
      fonts: null,
    };
  }

  /** Starts the compile worker, unless it runs. `dispose` stops it; `connect` may start it again. */
  connect() {
    if (!this.#client) {
      this.#startClient();
    }
  }

  subscribe = (listener: () => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  getSnapshot = (): SessionState => this.#state;

  #set(patch: Partial<SessionState>) {
    const previous = this.#state.view;
    const previousFonts = this.#state.fonts;
    this.#state = { ...this.#state, ...patch };
    const view = this.#state.view;
    if (previous.kind === 'deck' && view !== previous && (view.kind !== 'deck' || view.media !== previous.media)) {
      previous.media.revoke();
    }
    if (previousFonts && this.#state.fonts !== previousFonts) {
      previousFonts.dispose();
    }
    if (this.#state.fonts && this.#state.fonts !== previousFonts) {
      this.#watchLeaveOut(this.#state.fonts);
    }
    for (const listener of this.#listeners) {
      listener();
    }
  }

  /**
   * A zip holds the families its build left out or included: a change in the fonts panel or the dialog forgets a built
   * one.
   */
  #watchLeaveOut(fonts: FontsController) {
    let { leaveOut, include } = fonts.getSnapshot();
    fonts.subscribe(() => {
      const next = fonts.getSnapshot();
      if (next.leaveOut !== leaveOut || next.include !== include) {
        ({ leaveOut, include } = next);
        this.resetBundle();
      }
    });
  }

  #startClient(): CompileClient {
    const client = new CompileClient(this.#startWorker(), (compiler) => {
      if (this.#client === client) {
        this.#set({ compiler });
      }
    });
    this.#client = client;
    this.#set({ compiler: client.status });
    return client;
  }

  #restartWorker() {
    this.#client?.terminate();
    this.#startClient();
  }

  get #worker(): CompileClient {
    return this.#client ?? this.#startClient();
  }

  /** Stops the running load's photo decoder: a stale decode is abandoned, its worker stopped. */
  #stopPhotos() {
    this.#photos?.dispose();
    this.#photos = null;
  }

  /** Forgets the open deck's re-encoded pictures and stops its picture workers. */
  #stopImages() {
    this.#images?.dispose();
    this.#images = null;
    this.#encoded = new Map();
    this.#videos?.dispose();
    this.#videos = null;
    this.#encodedVideos = new Map();
    this.#stopCopies();
    this.#alternatePlan = null;
  }

  /** Forgets the open deck's copies and stops its copier. */
  #stopCopies() {
    this.#copier?.dispose();
    this.#copier = null;
    this.#copies = new Map();
  }

  /**
   * The open deck's videos that get a copy other browsers play (for the download dialog), asked of the compiler once
   * per deck; none while no deck is open.
   */
  alternatePlan(): Promise<PlannedVideo[]> {
    if (this.#state.view.kind !== 'deck') {
      return Promise.resolve([]);
    }
    this.#alternatePlan ??= this.#worker.alternatePlan().catch(() => []);
    return this.#alternatePlan;
  }

  /** Makes the copies of the videos only some browsers play and hands them to the compiler (`alternates.ts`). */
  #alternates(current: () => boolean): Promise<AlternatesSummary | null> {
    const view = this.#state.view;
    if (view.kind !== 'deck') {
      return Promise.resolve(null);
    }
    this.#copier ??= this.#alphaCopier();
    return makeAlternates(
      this.#worker,
      new Map(view.compiled.media.map(({ path, blob }) => [path, blob])),
      this.#copier,
      this.#copies,
      current,
      (alternates) => this.#progress({ alternates }, current),
    );
  }

  /** Shows the running download's progress, `patch` over what it shows. */
  #progress(patch: Partial<Extract<BundleState, { state: 'running' }>>, current: () => boolean) {
    const bundle = this.#state.bundle;
    if (current() && bundle.state === 'running') {
      this.#set({ bundle: { ...bundle, ...patch } });
    }
  }

  /** Starts the worker again after it failed to load. */
  retryCompiler() {
    this.#generation += 1;
    this.#stopPhotos();
    this.#restartWorker();
    if (this.#state.view.kind === 'error' && this.#state.view.error.kind === 'compiler') {
      this.#set({ view: { kind: 'empty' } });
    }
  }

  /** Opens a file the user picked. */
  openFile(file: File) {
    return this.#load(file.name, () => fileFromDrop([{ kind: 'file', file }]));
  }

  /** Opens what a drop held (captured with `captureDrop`). */
  openDrop(items: readonly DroppedItem[]) {
    const first = items[0];
    const name = first ? (first.kind === 'file' ? first.file.name : first.entry.name) : null;
    return this.#load(name, () => fileFromDrop(items));
  }

  /** Stops the running compile and returns to the start. */
  cancel() {
    this.#generation += 1;
    this.#stopPhotos();
    this.#stopImages();
    if (this.#state.view.kind === 'compiling' || this.#state.view.kind === 'preparing') {
      this.#restartWorker();
    }
    this.#set({ view: { kind: 'empty' }, bundle: { state: 'idle' }, fonts: null });
  }

  /** Closes the deck (releasing its media URLs) and returns to the start. */
  close() {
    this.cancel();
  }

  async #load(name: string | null, input: () => Promise<File>) {
    const generation = ++this.#generation;
    const current = (): boolean => generation === this.#generation;
    this.#stopPhotos();
    this.#stopImages();
    if (this.#state.view.kind === 'compiling' || this.#state.view.kind === 'preparing') {
      this.#restartWorker();
    }
    this.#set({ view: { kind: 'preparing', name: name ?? 'presentation' }, bundle: { state: 'idle' }, fonts: null });

    let file: File;
    try {
      file = await input();
    } catch (error) {
      if (current()) {
        const message =
          error instanceof UnsupportedInput ? error.message : `The drop could not be read: ${String(error)}`;
        this.#set({ view: { kind: 'error', name, error: { kind: 'unsupported', message } } });
      }
      return;
    }
    if (!current()) {
      return;
    }

    const diagnostics: DeckDiagnostic[] = [];
    let needs: readonly FontNeed[] = [];
    let mediaNeeds: readonly MediaNeed[] = [];
    this.#set({
      view: { kind: 'compiling', name: file.name, phase: null, done: [], warnings: 0, errors: 0, photos: null },
    });
    const update = (patch: Partial<Extract<View, { kind: 'compiling' }>>) => {
      const view = this.#state.view;
      if (current() && view.kind === 'compiling') {
        this.#set({ view: { ...view, ...patch } });
      }
    };
    const handlers = {
      onProgress: (phase: Phase) => {
        const view = this.#state.view;
        if (view.kind === 'compiling') {
          update({ phase, done: view.phase && view.phase !== phase ? [...view.done, view.phase] : view.done });
        }
      },
      onDiagnostics: (batch: DeckDiagnostic[]) => {
        diagnostics.push(...batch);
        update({
          errors: diagnostics.filter((d) => ERROR_SEVERITIES.has(d.severity)).length,
          warnings: diagnostics.filter((d) => !ERROR_SEVERITIES.has(d.severity) && d.severity !== 'info').length,
        });
      },
      onNeeds: (list: FontNeed[]) => {
        needs = list;
      },
      onMediaNeeds: (list: MediaNeed[]) => {
        mediaNeeds = list;
      },
    };

    try {
      await this.#worker.open(file, handlers);
      if (!current()) {
        return;
      }
      const photos = await this.#decodePhotos(mediaNeeds, update, current);
      if (!current()) {
        return;
      }
      const compiled = await this.#worker.finish({ ...handlers, onDiagnostics: replacing(diagnostics, handlers) });
      if (!current()) {
        return;
      }
      const fonts = new FontsController(this.#fontsHost(generation, file, photos), this.#fontsEnv, needs);
      this.#set({
        view: {
          kind: 'deck',
          name: file.name,
          deck: compiled.deck,
          slides: compiled.slides,
          media: new MediaUrls(compiled.media),
          diagnostics: explainPhotos(diagnostics, photos.outcomes),
          needs,
          compiled,
          photos: photos.outcomes,
        },
        fonts,
      });
      void fonts.start();
    } catch (error) {
      if (!current()) {
        return;
      }
      if (error instanceof RequestError) {
        const compilerFailed = this.#state.compiler.state === 'failed';
        this.#set({
          view: {
            kind: 'error',
            name: file.name,
            error: compilerFailed
              ? { kind: 'compiler', message: error.error.message }
              : { kind: 'compile', error: error.error },
          },
        });
      } else {
        throw error;
      }
    }
  }

  /**
   * Has the browser decode the HEIC photos `needs` lists (off the main thread) and hands each decoded file to the
   * compiler for the next `finish`, showing the progress in the conversion view.
   */
  async #decodePhotos(
    needs: readonly MediaNeed[],
    update: (patch: Partial<Extract<View, { kind: 'compiling' }>>) => void,
    current: () => boolean,
  ): Promise<DecodedPhotos> {
    const total = decodable(needs);
    if (needs.length === 0) {
      return { outcomes: [], files: new Map() };
    }
    const show = (done: number, active: boolean) => {
      if (total > 0) {
        update({ photos: { total, done, active } });
      }
    };
    show(0, true);
    this.#stopPhotos();
    const decoder = this.#photoDecoder();
    this.#photos = decoder;
    try {
      return await decodePhotos(
        needs,
        decoder,
        (id, bytes) => this.#worker.addMedia(id, bytes),
        (done) => show(done, true),
        current,
      );
    } finally {
      if (this.#photos === decoder) {
        this.#photos = null;
      }
      decoder.dispose();
      show(total, false);
    }
  }

  /**
   * The open deck as the fonts flow sees it. Every call belongs to the load of `generation`: after another open, a
   * close or a cancel it fails, and its re-finish changes nothing. A finish sends the complete list of diagnostics, the
   * open's included, so a re-finish's replace the deck's.
   */
  #fontsHost(generation: number, file: File, photos: DecodedPhotos): FontsHost {
    const current = () => generation === this.#generation;
    const requests = () => {
      if (!current()) {
        throw new RequestError({ kind: 'cancelled', message: 'The deck was closed', diagnostics: [] });
      }
      return new FontRequests(this.#worker);
    };
    return {
      addFont: (bytes, source, unicodeRange) => requests().addFont(bytes, source, unicodeRange),
      selectLocal: (fonts) => requests().selectLocal(fonts),
      reopen: async () => {
        let needs: FontNeed[] = [];
        requests();
        await this.#worker.open(file, {
          onNeeds: (list) => (needs = list),
        });
        // A reopened deck has no media of the browser's: the decoded photos go to it again.
        for (const [id, bytes] of photos.files) {
          await this.#worker.addMedia(id, bytes.slice(0));
        }
        return needs;
      },
      refinish: async (needs) => {
        const diagnostics: DeckDiagnostic[] = [];
        requests();
        const compiled = await this.#worker.finish({ onDiagnostics: (batch) => diagnostics.push(...batch) });
        const view = this.#state.view;
        if (!current() || view.kind !== 'deck') {
          return;
        }
        const bundle = this.#state.bundle;
        this.#set({
          view: {
            ...view,
            deck: compiled.deck,
            slides: compiled.slides,
            media: view.media.holds(compiled.media) ? view.media : new MediaUrls(compiled.media),
            diagnostics: explainPhotos(diagnostics, photos.outcomes),
            needs,
            compiled,
          },
          // A zip built before belongs to the deck as it was.
          bundle: bundle.state === 'running' ? bundle : { state: 'idle' },
        });
      },
      slides: () => (this.#state.view.kind === 'deck' ? this.#state.view.slides : []),
      substitutes: () => (this.#state.view.kind === 'deck' ? (this.#state.view.compiled.substitutes ?? []) : []),
    };
  }

  /**
   * Has the browser re-encode the pictures the compiler plans (in the picture workers, at `quality`) and hands each
   * smaller file to the compiler for the bundle, showing the progress. A picture the browser keeps, or a file the
   * compiler refuses, stays as it is.
   */
  async #compressImages(quality: ImageQuality, current: () => boolean): Promise<ImagesSummary | null> {
    const view = this.#state.view;
    if (view.kind !== 'deck') {
      return null;
    }
    const start = performance.now();
    const files = new Map(view.compiled.media.map(({ path, blob }) => [path, blob]));
    const plan = await this.#worker.imagePlan();
    // A download cancelled while the compiler planned starts nothing.
    if (!current()) {
      return null;
    }
    const total = plan.length;
    let done = 0;
    const show = () => this.#progress({ images: { done, total } }, current);
    show();
    this.#images ??= this.#imageCompressor();
    const images = this.#images;
    const outcomes = await Promise.all(
      plan.map(async (image) => {
        const file = files.get(image.path);
        const key = `${image.path} ${image.targetWidth}x${image.targetHeight} ${quality}`;
        let outcome = this.#encoded.get(key);
        if (outcome === undefined) {
          outcome = file
            ? images.compress({ plan: image, file, quality: ENCODER_QUALITY[quality] })
            : Promise.resolve({ kind: 'kept', reason: 'the app holds no file of it', elapsed: 0 });
          this.#encoded.set(key, outcome);
        }
        const settled = await outcome;
        done += 1;
        show();
        return { image, outcome: settled };
      }),
    );
    const summary: ImagesSummary = { planned: total, compressed: 0, before: 0, after: 0, kept: [], elapsed: 0 };
    const kept: { path: string; reason: string }[] = [];
    for (const { image, outcome } of outcomes) {
      if (!current()) {
        return null;
      }
      summary.before += image.bytes;
      if (outcome.kind === 'kept') {
        summary.after += image.bytes;
        kept.push({ path: image.path, reason: outcome.reason });
        continue;
      }
      try {
        // A copy: the re-encoded file stays for the next download.
        await this.#worker.addImage(image.path, outcome.bytes.slice(0));
        summary.compressed += 1;
        summary.after += outcome.bytes.byteLength;
      } catch (error) {
        if (!(error instanceof RequestError) || error.error.kind !== 'request') {
          throw error;
        }
        summary.after += image.bytes;
        kept.push({ path: image.path, reason: error.error.message });
      }
    }
    return { ...summary, kept, elapsed: performance.now() - start };
  }

  /**
   * Has the browser re-encode the videos the compiler plans (in the video workers, at `quality`), showing their
   * progress by frames, and hands each smaller file to the compiler for the bundle. A video the plan or the browser
   * keeps, or a file the compiler refuses, stays as it is. A finished re-encode is kept for the next download; one a
   * cancel stopped is not.
   */
  async #compressVideos(quality: VideoQuality, current: () => boolean): Promise<VideosSummary | null> {
    const view = this.#state.view;
    if (view.kind !== 'deck') {
      return null;
    }
    const start = performance.now();
    const files = new Map(view.compiled.media.map(({ path, blob }) => [path, blob]));
    const plan = await this.#worker.videoPlan();
    // A download cancelled while the compiler planned starts nothing.
    if (!current()) {
      return null;
    }
    const work = plan.filter((video) => video.keep === null);
    const progress: VideosProgress = {
      done: plan.length - work.length,
      total: plan.length,
      frames: 0,
      totalFrames: work.reduce((sum, video) => sum + video.frames, 0),
      started: start,
      updated: start,
    };
    const frames = new Map<string, number>();
    const show = () => this.#progress({ videos: { ...progress, updated: performance.now() } }, current);
    show();
    this.#videos ??= this.#videoCompressor();
    const videos = this.#videos;
    // Only the chosen quality's re-encodes are kept: a deck's videos can be hundreds of MB at each quality.
    for (const key of [...this.#encodedVideos.keys()]) {
      if (!key.endsWith(` ${quality}`)) {
        this.#encodedVideos.delete(key);
      }
    }
    const outcomes = await Promise.all(
      plan.map(async (video) => {
        if (video.keep !== null) {
          return { video, outcome: { kind: 'kept', reason: video.keep, elapsed: 0 } as VideoOutcome };
        }
        const file = files.get(video.path);
        const key = `${video.path} ${video.targetWidth}x${video.targetHeight} ${quality}`;
        let outcome = this.#encodedVideos.get(key);
        if (outcome === undefined) {
          outcome = file
            ? videos.compress({ plan: video, file, quality }, ({ frames: done }) => {
                frames.set(video.path, done);
                progress.frames = [...frames.values()].reduce((sum, count) => sum + count, 0);
                show();
              })
            : Promise.resolve({ kind: 'kept', reason: 'the app holds no file of it', elapsed: 0 });
          this.#encodedVideos.set(key, outcome);
        }
        const settled = await outcome;
        // A video a cancel stopped is re-encoded by the next download.
        if (!current() && settled.kind === 'kept') {
          this.#encodedVideos.delete(key);
        }
        frames.set(video.path, video.frames);
        progress.frames = [...frames.values()].reduce((sum, count) => sum + count, 0);
        progress.done += 1;
        show();
        return { video, outcome: settled };
      }),
    );
    const summary: VideosSummary = {
      planned: plan.length,
      compressed: 0,
      before: 0,
      after: 0,
      kept: [],
      elapsed: 0,
      soundCopied: 0,
      soundEncoded: 0,
      copied: 0,
      soundRemoved: 0,
      soundRemovedBytes: 0,
    };
    const kept: { path: string; reason: string }[] = [];
    for (const { video, outcome } of outcomes) {
      if (!current()) {
        return null;
      }
      summary.before += video.bytes;
      if (outcome.kind === 'kept') {
        summary.after += video.bytes;
        kept.push({ path: video.path, reason: outcome.reason });
        continue;
      }
      try {
        await this.#worker.addVideo(video.path, outcome.file);
        summary.compressed += 1;
        summary.after += outcome.file.size;
        summary.soundCopied += outcome.audio === 'copied' ? 1 : 0;
        summary.soundEncoded += outcome.audio === 'encoded' ? 1 : 0;
        summary.copied += outcome.video === 'copied' ? 1 : 0;
        if (outcome.audio === 'removed') {
          summary.soundRemoved += 1;
          summary.soundRemovedBytes += video.audio?.bytes ?? 0;
        }
      } catch (error) {
        if (!(error instanceof RequestError) || error.error.kind !== 'request') {
          throw error;
        }
        summary.after += video.bytes;
        kept.push({ path: video.path, reason: error.error.message });
      }
    }
    return { ...summary, kept, elapsed: performance.now() - start };
  }

  /**
   * Bundles the open deck as a zip, collecting the streamed chunks into one `Blob`. With `compressImages`, the
   * pictures are re-encoded at `quality` first; with `compressVideos`, the videos at `videoQuality`, alongside.
   */
  async bundle(
    settings: BundleSettings,
    quality: ImageQuality = 'high',
    videoQuality: VideoQuality = 'high',
  ): Promise<void> {
    if (this.#state.view.kind !== 'deck' || this.#state.bundle.state === 'running') {
      return;
    }
    const generation = this.#generation;
    const download = ++this.#bundles;
    const current = () => generation === this.#generation && download === this.#bundles;
    const chunks: ArrayBuffer[] = [];
    let bytes = 0;
    const compress = settings.compressImages === true;
    const compressVideos = settings.compressVideos === true;
    const alternates = settings.alternateVideos === true;
    this.#set({
      bundle: {
        state: 'running',
        bytes: 0,
        ...(compress ? { images: { done: 0, total: 0 } } : {}),
        ...(compressVideos ? { videos: { done: 0, total: 0, frames: 0, totalFrames: 0, started: 0, updated: 0 } } : {}),
      },
    });
    try {
      const [images, videos, copies] = await Promise.all([
        compress ? this.#compressImages(quality, current) : null,
        compressVideos ? this.#compressVideos(videoQuality, current) : null,
        alternates ? this.#alternates(current) : null,
      ]);
      if (!current()) {
        return;
      }
      const report = await this.#worker.bundle(settings, (chunk) => {
        chunks.push(chunk);
        bytes += chunk.byteLength;
        if (current()) {
          this.#set({ bundle: { state: 'running', bytes } });
        }
      });
      if (!current()) {
        return;
      }
      const fonts = this.#state.fonts?.getSnapshot();
      // The families to leave out or include changed while it was built: the zip is not the one asked for now.
      this.#set({
        bundle:
          sameFamilies(fonts?.leaveOut ?? [], settings.leaveOut) && sameFamilies(fonts?.include ?? [], settings.include)
            ? {
                state: 'done',
                report,
                blob: new Blob(chunks, { type: 'application/zip' }),
                ...(images ? { images } : {}),
                ...(videos ? { videos } : {}),
                ...(copies ? { alternates: copies } : {}),
              }
            : { state: 'idle' },
      });
    } catch (error) {
      if (!current()) {
        return;
      }
      if (error instanceof RequestError) {
        this.#set({ bundle: { state: 'failed', error: error.error } });
      } else {
        throw error;
      }
    }
  }

  /**
   * Stops the running download: the videos and pictures being re-encoded are given up (those finished stay for the
   * next download) and the zip, if the worker is building it, is dropped.
   */
  cancelBundle() {
    if (this.#state.bundle.state !== 'running') {
      return;
    }
    this.#bundles += 1;
    this.#videos?.dispose();
    this.#videos = null;
    this.#copier?.dispose();
    this.#copier = null;
    this.#images?.dispose();
    this.#images = null;
    this.#encoded = new Map();
    this.#set({ bundle: { state: 'idle' } });
  }

  /** Forgets a finished or failed bundle. */
  resetBundle() {
    if (this.#state.bundle.state !== 'running') {
      this.#set({ bundle: { state: 'idle' } });
    }
  }

  /** Stops the worker and releases the deck's media URLs (the page is going away). */
  dispose() {
    this.#generation += 1;
    this.#stopImages();
    this.#client?.terminate();
    this.#client = null;
    this.#set({ view: { kind: 'empty' }, bundle: { state: 'idle' }, fonts: null });
  }
}

/**
 * A finish's diagnostics handler: a finish sends the complete list, the open's included, so its first batch replaces
 * what `list` holds before `handlers` adds it.
 */
function replacing(list: DeckDiagnostic[], handlers: { onDiagnostics(batch: DeckDiagnostic[]): void }) {
  let first = true;
  return (batch: DeckDiagnostic[]) => {
    if (first) {
      list.length = 0;
      first = false;
    }
    handlers.onDiagnostics(batch);
  };
}
