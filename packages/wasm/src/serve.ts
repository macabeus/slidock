// The compile worker's message loop (packages/wasm/README.md): requests run one at a time in the order received,
// each in its own task, so a `cancel` that arrives before its request starts finds it in the queue.
import type { FontFile, FromWorker, MediaHandle, MediaNeed, ToWorker, WorkerError } from '@slidock/types';

import { Compiler, StageError, blobSource } from './compiler';
import type { SyncBlobReader, WasmCompiler } from './compiler';

/** What the worker needs from its global scope (a `DedicatedWorkerGlobalScope`). */
export interface WorkerScope {
  postMessage(message: FromWorker, transfer: Transferable[]): void;
  addEventListener(type: 'message', listener: (event: MessageEvent<ToWorker>) => void): void;
}

/** The loaded wasm module. */
export interface CompilerModule {
  version(): string;
  /** The module's linear memory in bytes. */
  memoryBytes(): number;
  create(): WasmCompiler;
}

/** What the worker needs besides its scope. */
export interface ServeOptions {
  /** Resolves once the wasm module is loaded. */
  module: Promise<CompilerModule>;
  /** Reads `File` slices synchronously (`new FileReaderSync()`). */
  reader: SyncBlobReader;
  /** Runs `task` in a later task of the event loop (`setTimeout`). */
  later?: (task: () => void) => void;
}

type Request = Exclude<ToWorker, { type: 'cancel' }>;

function failure(kind: WorkerError['kind'], message: string): WorkerError {
  return { kind, message, diagnostics: [] };
}

/** Answers the app's requests on `scope` until the worker ends. */
export function serve(scope: WorkerScope, options: ServeOptions): void {
  const later = options.later ?? ((task: () => void) => void setTimeout(task, 0));
  const queue: Request[] = [];
  let compiler: Compiler | undefined;
  let loaded: CompilerModule | undefined;
  let unavailable: WorkerError | undefined;
  /** The open input, which the printed media are slices of. */
  let input: File | undefined;
  /** The media served for the open input by path: a path names the same bytes on every finish of one input. */
  let served = new Map<string, Blob>();
  let running = false;

  const post = (message: FromWorker, transfer: Transferable[] = []) => scope.postMessage(message, transfer);

  const schedule = () => {
    if (running || queue.length === 0 || (compiler === undefined && unavailable === undefined)) {
      return;
    }
    running = true;
    later(() => {
      const request = queue.shift();
      if (request !== undefined) {
        run(request);
      }
      running = false;
      schedule();
    });
  };

  const run = (request: Request) => {
    const { id } = request;
    if (unavailable !== undefined || compiler === undefined) {
      post({ type: 'error', id, error: unavailable ?? failure('internal', 'the compiler is not loaded') });
      return;
    }
    try {
      handle(compiler, request);
    } catch (thrown) {
      if (thrown instanceof StageError) {
        post({ type: 'error', id, error: thrown.error });
        return;
      }
      // A trap leaves the module's memory in an unknown state: no later request may use it.
      const message = thrown instanceof Error ? `${thrown.name}: ${thrown.message}` : String(thrown);
      unavailable = failure('internal', `the compiler crashed (${message}); start a new worker`);
      input = undefined;
      served = new Map();
      post({ type: 'error', id, error: failure('internal', message) });
    }
  };

  const handle = (compiler: Compiler, request: Request) => {
    switch (request.type) {
      case 'open': {
        post({ type: 'progress', id: request.id, phase: 'open' });
        input = undefined;
        served = new Map();
        const opened = compiler.open(
          request.file.name,
          blobSource(request.file, options.reader),
          request.fonts ?? true,
        );
        input = request.file;
        if (opened.diagnostics.length > 0) {
          post({ type: 'diagnostics', id: request.id, diagnostics: opened.diagnostics });
        }
        post({ type: 'needs', id: request.id, needs: opened.needs });
        const file = request.file;
        const media: MediaNeed[] = opened.media.map(({ id, mime, range, plan, refusal }) => ({
          id,
          mime,
          blob:
            range !== null
              ? file.slice(range.offset, range.offset + range.length, mime)
              : new Blob([compiler.mediaNeedBytes(id)], { type: mime }),
          plan,
          refusal,
        }));
        post({ type: 'mediaNeeds', id: request.id, needs: media });
        break;
      }
      case 'addMedia':
        compiler.addMedia(request.need, new Uint8Array(request.bytes));
        break;
      case 'fontsNeeded':
        post({ type: 'needs', id: request.id, needs: compiler.fontNeeds() });
        break;
      case 'addFont':
        post({
          type: 'needs',
          id: request.id,
          needs: compiler.addFont(new Uint8Array(request.bytes), request.source, request.unicodeRange ?? null),
        });
        break;
      case 'selectLocal':
        post({ type: 'localPicks', id: request.id, picks: compiler.selectLocal(request.fonts) });
        break;
      case 'googleFaces':
        post({ type: 'googleFaces', id: request.id, faces: compiler.googleFaces(request.css) });
        break;
      case 'finish': {
        post({ type: 'progress', id: request.id, phase: 'finish' });
        compiler.finish();
        post({ type: 'progress', id: request.id, phase: 'print' });
        const printed = compiler.print();
        if (printed.diagnostics.length > 0) {
          post({ type: 'diagnostics', id: request.id, diagnostics: printed.diagnostics });
        }
        const file = input;
        const media: MediaHandle[] = printed.media.map(({ path, mime, range }) => {
          let blob = served.get(path);
          if (blob === undefined) {
            blob =
              range !== null && file !== undefined
                ? file.slice(range.offset, range.offset + range.length, mime)
                : new Blob([compiler.mediaBytes(path)], { type: mime });
            served.set(path, blob);
          }
          return { path, mime, blob };
        });
        const fonts: FontFile[] = printed.fonts.map((rule, index) => ({
          ...rule,
          bytes: compiler.fontBytes(index).buffer,
        }));
        post(
          {
            type: 'deck',
            id: request.id,
            deck: printed.deck,
            slides: printed.slides,
            media,
            fonts,
            faces: printed.faces,
            fontIssues: printed.fontIssues,
            substitutes: printed.substitutes,
            page: printed.page,
          },
          fonts.map((font) => font.bytes as ArrayBuffer),
        );
        break;
      }
      case 'imagePlan':
        post({ type: 'imagePlan', id: request.id, images: compiler.imagePlan() });
        break;
      case 'addImage':
        compiler.addImage(request.path, new Uint8Array(request.bytes));
        break;
      case 'videoPlan':
        post({ type: 'videoPlan', id: request.id, videos: compiler.videoPlan() });
        break;
      case 'addVideo':
        compiler.addVideo(request.path, blobSource(request.file, options.reader));
        break;
      case 'alternatePlan':
        post({ type: 'alternatePlan', id: request.id, videos: compiler.alternatePlan() });
        break;
      case 'addAlternate':
        compiler.addAlternate(request.path, blobSource(request.file, options.reader));
        break;
      case 'memory':
        post({ type: 'memory', id: request.id, bytes: loaded?.memoryBytes() ?? 0 });
        break;
      case 'bundle': {
        post({ type: 'progress', id: request.id, phase: 'bundle' });
        const report = compiler.bundle(request.settings, (chunk) =>
          post({ type: 'zipChunk', id: request.id, bytes: chunk.buffer }, [chunk.buffer]),
        );
        post({ type: 'done', id: request.id, report });
        return;
      }
    }
    post({ type: 'done', id: request.id, report: null });
  };

  scope.addEventListener('message', ({ data }) => {
    if (data.type === 'cancel') {
      const index = queue.findIndex((request) => request.id === data.id);
      const [cancelled] = index >= 0 ? queue.splice(index, 1) : [];
      if (cancelled !== undefined) {
        post({ type: 'error', id: cancelled.id, error: failure('cancelled', 'cancelled before it started') });
      }
      return;
    }
    queue.push(data);
    schedule();
  });

  options.module.then(
    (module) => {
      loaded = module;
      compiler = new Compiler(module.create());
      post({ type: 'ready', version: module.version() });
      schedule();
    },
    (reason: unknown) => {
      unavailable = failure('internal', `the compiler did not load: ${String(reason)}`);
      post({ type: 'error', id: null, error: unavailable });
      schedule();
    },
  );
}
