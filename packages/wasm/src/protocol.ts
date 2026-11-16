// Messages between the app and the compile worker (README.md). The protocol's types are generated from Rust into
// `@slidock/types`; `pkg/` holds the wasm-pack output (`pnpm wasm:build`).
export type { FromWorker, ToWorker } from '@slidock/types';

/** A minimal worker's first message: `ready` with its version, or an error (`FromWorker` is the full protocol). */
export type WorkerMessage = { type: 'ready'; version: string } | { type: 'error'; message: string };
