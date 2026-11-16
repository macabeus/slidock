import type { FromWorker, ToWorker } from '@slidock/types';

/** The app's side of the compile worker: what the client needs from a `Worker`, so tests can use a fake. */
export interface WorkerPort {
  post(message: ToWorker, transfer?: Transferable[]): void;
  /** Every message the worker sends. */
  listen(onMessage: (message: FromWorker) => void, onCrash: (message: string) => void): void;
  terminate(): void;
}

/** A port over a real `Worker`. A script error (the module or its wasm failed to load) reports as a crash. */
export function workerPort(worker: Worker): WorkerPort {
  return {
    post: (message, transfer = []) => worker.postMessage(message, transfer),
    listen(onMessage, onCrash) {
      worker.onmessage = (event: MessageEvent<FromWorker>) => onMessage(event.data);
      worker.onerror = (event: ErrorEvent) => {
        event.preventDefault();
        onCrash(event.message || 'The compile worker failed to start');
      };
      worker.onmessageerror = () => onCrash('A message from the compile worker could not be read');
    },
    terminate: () => worker.terminate(),
  };
}

/** Starts the app's compile worker. */
export function startCompileWorker(): WorkerPort {
  return workerPort(new Worker(new URL('./compile-worker.ts', import.meta.url), { type: 'module' }));
}
