// The app's side of HEIC decoding: each photo a deck needs decoded goes to a worker (`decode-worker.ts`), one at a
// time, and comes back as a JPEG of its plan's output size, or with why this browser cannot decode it. A decode that
// takes longer than `DECODE_TIMEOUT_MS` is given up, its worker stopped (the next photo starts another).
import type { HeicPlan, MediaNeed } from '@slidock/types';

import type { DecodePath } from './heic';

/** A photo the browser decoded. */
export interface DecodedFile {
  bytes: ArrayBuffer;
  mime: 'image/jpeg';
  path: DecodePath;
  /** In milliseconds, in the worker. */
  elapsed: number;
}

/** Decodes the HEIC photos of a deck. */
export interface PhotoDecoder {
  /** Rejects with a `PhotoUndecodable` when this browser cannot decode it. */
  decode(need: MediaNeed & { plan: HeicPlan }): Promise<DecodedFile>;
  /** Stops the worker. */
  dispose(): void;
}

/** This browser cannot decode a photo: each path's reason. */
export class PhotoUndecodable extends Error {
  readonly reasons: readonly string[];

  constructor(reasons: readonly string[]) {
    super(reasons.join('; '));
    this.name = 'PhotoUndecodable';
    this.reasons = reasons;
  }
}

/** A request to the decode worker. */
export interface DecodeRequest {
  id: number;
  plan: HeicPlan;
  file: Blob;
}

/** The decode worker's answer. */
export type DecodeReply = ({ id: number; ok: true } & DecodedFile) | { id: number; ok: false; reasons: string[] };

/** What the decoder needs of a worker, so tests can use a fake. */
export interface DecodePort {
  postMessage(message: DecodeRequest): void;
  onmessage: ((event: MessageEvent<DecodeReply>) => void) | null;
  onerror: ((event: ErrorEvent) => void) | null;
  terminate(): void;
}

/** How long one photo may take to decode: a 12-megapixel photo takes well under a second. */
export const DECODE_TIMEOUT_MS = 60_000;

/** The timers a decoder runs its timeouts on, so tests can drive them. */
export interface Timers {
  setTimeout(run: () => void, ms: number): unknown;
  clearTimeout(timer: unknown): void;
}

const GLOBAL_TIMERS: Timers = {
  setTimeout: (run, ms) => globalThis.setTimeout(run, ms),
  clearTimeout: (timer) => globalThis.clearTimeout(timer as ReturnType<typeof globalThis.setTimeout>),
};

/**
 * Decodes through a worker that `start` makes on the first photo; a worker that fails, or a photo that takes longer
 * than `timeout` milliseconds, fails every pending photo and stops the worker.
 */
export function workerPhotoDecoder(
  start: () => DecodePort,
  timeout = DECODE_TIMEOUT_MS,
  timers: Timers = GLOBAL_TIMERS,
): PhotoDecoder {
  let port: DecodePort | null = null;
  let nextId = 1;
  const pending = new Map<number, { resolve(file: DecodedFile): void; reject(error: Error): void; timer: unknown }>();
  const failAll = (reason: string) => {
    for (const request of pending.values()) {
      timers.clearTimeout(request.timer);
      request.reject(new PhotoUndecodable([reason]));
    }
    pending.clear();
  };
  const stop = (reason: string) => {
    port?.terminate();
    port = null;
    failAll(reason);
  };
  const connect = (): DecodePort => {
    if (port) {
      return port;
    }
    const worker = start();
    worker.onmessage = ({ data }) => {
      const request = pending.get(data.id);
      pending.delete(data.id);
      if (request) {
        timers.clearTimeout(request.timer);
      }
      if (data.ok) {
        request?.resolve({ bytes: data.bytes, mime: data.mime, path: data.path, elapsed: data.elapsed });
      } else {
        request?.reject(new PhotoUndecodable(data.reasons));
      }
    };
    worker.onerror = (event) => {
      event.preventDefault();
      if (port === worker) {
        stop(`the decode worker failed (${event.message || 'no message'})`);
      }
    };
    port = worker;
    return worker;
  };
  return {
    decode(need) {
      const id = nextId++;
      return new Promise((resolve, reject) => {
        const timer = timers.setTimeout(
          () => stop(`decoding took longer than ${Math.round(timeout / 1000)} s`),
          timeout,
        );
        pending.set(id, { resolve, reject, timer });
        connect().postMessage({ id, plan: need.plan, file: need.blob });
      });
    },
    dispose() {
      stop('the decode worker was stopped');
    },
  };
}

/** Starts the app's decode worker. */
export function startDecodeWorker(): DecodePort {
  return new Worker(new URL('./decode-worker.ts', import.meta.url), { type: 'module' }) as unknown as DecodePort;
}
