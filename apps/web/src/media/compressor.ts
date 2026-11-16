// The app's side of picture compression: each picture to re-encode goes to one of a few workers
// (`compress-worker.ts`), and comes back as a smaller file or with why the original stays. A picture that takes longer
// than `COMPRESS_TIMEOUT_MS` is given up (kept), its worker stopped (the next picture starts another).
import type { CompressJob, CompressOutcome } from './compress';
import type { VideoJob, VideoOutcome, VideoProgress } from './video';

/** Re-encodes a deck's pictures. */
export interface ImageCompressor {
  compress(job: CompressJob): Promise<CompressOutcome>;
  /** Stops the workers; pending pictures are kept. */
  dispose(): void;
}

/** A request to a picture worker. */
export interface CompressRequest {
  id: number;
  job: CompressJob;
}

/** A picture worker's answer. */
export interface CompressReply {
  id: number;
  outcome: CompressOutcome;
}

/** What the compressor needs of a worker, so tests can use a fake. */
export interface CompressPort {
  postMessage(message: CompressRequest): void;
  onmessage: ((event: MessageEvent<CompressReply>) => void) | null;
  onerror: ((event: ErrorEvent) => void) | null;
  terminate(): void;
}

/** How long one picture may take: a 50-megapixel PNG takes a few seconds. */
export const COMPRESS_TIMEOUT_MS = 60_000;

/** Workers at most: each holds a decoded picture and a canvas or two (about 200 MB for a 50-megapixel one). */
export const MAX_WORKERS = 4;

/** The workers for `cores` logical cores: one fewer, at least one, at most `MAX_WORKERS`. */
export function workerCount(cores: number | undefined): number {
  return Math.max(1, Math.min(MAX_WORKERS, (cores ?? 2) - 1));
}

/** The timers a compressor runs its timeouts on, so tests can drive them. */
export interface Timers {
  setTimeout(run: () => void, ms: number): unknown;
  clearTimeout(timer: unknown): void;
}

const GLOBAL_TIMERS: Timers = {
  setTimeout: (run, ms) => globalThis.setTimeout(run, ms),
  clearTimeout: (timer) => globalThis.clearTimeout(timer as ReturnType<typeof globalThis.setTimeout>),
};

interface Slot<Request, Reply> {
  port: PoolPort<Request, Reply> | null;
  busy: boolean;
}

interface Pending<Job, Outcome, Progress> {
  job: Job;
  onProgress: ((progress: Progress) => void) | undefined;
  resolve(outcome: Outcome): void;
}

/** What a pool needs of a worker, so tests can use a fake. */
export interface PoolPort<Request, Reply> {
  postMessage(message: Request): void;
  onmessage: ((event: MessageEvent<Reply>) => void) | null;
  onerror: ((event: ErrorEvent) => void) | null;
  terminate(): void;
}

/** A worker's answer: the outcome, or progress before it. */
export type PoolReply<Outcome, Progress> =
  { id: number; outcome: Outcome; progress?: undefined } | { id: number; progress: Progress; outcome?: undefined };

/** How a pool names what it does, and what it makes of a job it gives up. */
export interface PoolOptions<Outcome> {
  size: number;
  /** In milliseconds: how long a job may go without an answer or progress. */
  timeout: number;
  timers?: Timers;
  /** The outcome of a job the pool gives up, with why. */
  kept(reason: string): Outcome;
  /** What a job is (`picture`, `video`), in those reasons. */
  noun: string;
  /** Why a job that ran out of time is given up. */
  expired: string;
}

/** Runs jobs through a few workers. */
export interface WorkerPool<Job, Outcome, Progress> {
  run(job: Job, onProgress?: (progress: Progress) => void): Promise<Outcome>;
  /** Stops the workers; pending jobs are given up. */
  dispose(): void;
}

/**
 * Runs jobs through up to `size` workers that `start` makes as jobs arrive, one job per worker at a time, in the order
 * asked. A worker that fails, or a job that goes `timeout` milliseconds without an answer or progress, gives that job
 * up and stops the worker.
 */
export function workerPool<Job, Outcome, Progress = never>(
  start: () => PoolPort<{ id: number; job: Job }, PoolReply<Outcome, Progress>>,
  options: PoolOptions<Outcome>,
): WorkerPool<Job, Outcome, Progress> {
  type Request = { id: number; job: Job };
  type Reply = PoolReply<Outcome, Progress>;
  const { timeout, kept, noun } = options;
  const timers = options.timers ?? GLOBAL_TIMERS;
  const slots: Slot<Request, Reply>[] = Array.from({ length: Math.max(1, options.size) }, () => ({
    port: null,
    busy: false,
  }));
  const queue: Pending<Job, Outcome, Progress>[] = [];
  let nextId = 1;
  let disposed = false;
  /** Stops each job running, giving it up. */
  const running = new Set<() => void>();
  const stoppedReason = `the ${noun} workers were stopped`;

  const run = (slot: Slot<Request, Reply>, pending: Pending<Job, Outcome, Progress>) => {
    slot.busy = true;
    const id = nextId++;
    let port = slot.port;
    if (!port) {
      port = start();
      slot.port = port;
    }
    const worker = port;
    let settled = false;
    const finish = (outcome: Outcome, stop: boolean) => {
      if (settled) {
        return;
      }
      settled = true;
      running.delete(stopped);
      timers.clearTimeout(timer);
      if (stop) {
        worker.terminate();
        if (slot.port === worker) {
          slot.port = null;
        }
      }
      slot.busy = false;
      pending.resolve(outcome);
      next();
    };
    const stopped = () => finish(kept(stoppedReason), true);
    running.add(stopped);
    const expire = () => finish(kept(options.expired), true);
    let timer = timers.setTimeout(expire, timeout);
    worker.onmessage = ({ data }) => {
      if (data.id !== id) {
        return;
      }
      if (data.outcome !== undefined) {
        finish(data.outcome, false);
        return;
      }
      timers.clearTimeout(timer);
      timer = timers.setTimeout(expire, timeout);
      pending.onProgress?.(data.progress as Progress);
    };
    worker.onerror = (event) => {
      event.preventDefault();
      finish(kept(`the ${noun} worker failed (${event.message || 'no message'})`), true);
    };
    worker.postMessage({ id, job: pending.job });
  };

  const next = () => {
    for (const slot of slots) {
      if (disposed || queue.length === 0) {
        return;
      }
      if (!slot.busy) {
        run(slot, queue.shift()!);
      }
    }
  };

  return {
    run(job, onProgress) {
      if (disposed) {
        return Promise.resolve(kept(stoppedReason));
      }
      return new Promise((resolve) => {
        queue.push({ job, onProgress, resolve });
        next();
      });
    },
    dispose() {
      disposed = true;
      for (const stop of [...running]) {
        stop();
      }
      for (const slot of slots) {
        slot.port?.terminate();
        slot.port = null;
      }
      for (const pending of queue.splice(0)) {
        pending.resolve(kept(stoppedReason));
      }
    },
  };
}

/**
 * Re-encodes through up to `size` workers that `start` makes as pictures arrive, one picture per worker at a time, in
 * the order asked. A worker that fails, or a picture that takes longer than `timeout` milliseconds, keeps that
 * picture and stops the worker.
 */
export function workerImageCompressor(
  start: () => CompressPort,
  size: number,
  timeout = COMPRESS_TIMEOUT_MS,
  timers: Timers = GLOBAL_TIMERS,
): ImageCompressor {
  const pool = workerPool<CompressJob, CompressOutcome>(
    start as () => PoolPort<CompressRequest, PoolReply<CompressOutcome, never>>,
    {
      size,
      timeout,
      timers,
      kept: (reason) => ({ kind: 'kept', reason, elapsed: 0 }),
      noun: 'picture',
      expired: `re-encoding took longer than ${Math.round(timeout / 1000)} s`,
    },
  );
  return { compress: (job) => pool.run(job), dispose: () => pool.dispose() };
}

/** Starts one of the app's picture workers. */
export function startCompressWorker(): CompressPort {
  return new Worker(new URL('./compress-worker.ts', import.meta.url), { type: 'module' }) as unknown as CompressPort;
}

/** Re-encodes a deck's videos. */
export interface VideoCompressor {
  compress(job: VideoJob, onProgress?: (progress: VideoProgress) => void): Promise<VideoOutcome>;
  /** Stops the workers; pending videos are kept. */
  dispose(): void;
}

/** How long a video may go without progress: a frame decoded and encoded. */
export const VIDEO_STALL_MS = 60_000;

/** Video workers at most: each runs the browser's decoder and encoder, which use the hardware where it has them. */
export const MAX_VIDEO_WORKERS = 2;

/** A video worker: its requests and answers. */
export type VideoPort = PoolPort<{ id: number; job: VideoJob }, PoolReply<VideoOutcome, VideoProgress>>;

/**
 * Re-encodes videos through up to `size` workers, one video per worker at a time, in the order asked. A worker that
 * fails, or a video that goes `stall` milliseconds without progress, keeps that video and stops the worker.
 */
export function workerVideoCompressor(
  start: () => VideoPort,
  size = MAX_VIDEO_WORKERS,
  stall = VIDEO_STALL_MS,
  timers: Timers = GLOBAL_TIMERS,
): VideoCompressor {
  const pool = workerPool<VideoJob, VideoOutcome, VideoProgress>(start, {
    size,
    timeout: stall,
    timers,
    kept: (reason) => ({ kind: 'kept', reason, elapsed: 0 }),
    noun: 'video',
    expired: `re-encoding made no progress for ${Math.round(stall / 1000)} s`,
  });
  return { compress: (job, onProgress) => pool.run(job, onProgress), dispose: () => pool.dispose() };
}

/** Starts one of the app's video workers. */
export function startVideoWorker(): VideoPort {
  return new Worker(new URL('./video-worker.ts', import.meta.url), { type: 'module' }) as unknown as VideoPort;
}
