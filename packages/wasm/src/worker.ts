// The compile worker: loads the wasm module and serves the app's requests (README.md). Import it as a module
// worker's entry: `new Worker(new URL('./compile-worker.ts', import.meta.url), { type: 'module' })` with
// `import '@slidock/wasm/worker';` in that file.
import init, { Compiler, memoryBytes, version } from '../pkg/slidock_wasm.js';
import type { SyncBlobReader } from './compiler';
import { serve } from './serve';
import type { WorkerScope } from './serve';

// The dedicated worker's globals, declared here so the entry type-checks under the DOM library too.
declare const self: WorkerScope;
declare const FileReaderSync: new () => SyncBlobReader;

serve(self, {
  module: init().then(() => ({ version, memoryBytes, create: () => new Compiler() })),
  reader: new FileReaderSync(),
});
