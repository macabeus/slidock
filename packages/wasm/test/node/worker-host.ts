// The compile worker (`src/serve.ts` over the built module) inside a Node `worker_threads` Worker, as `compile.ts`
// drives it. Node has no `File` read synchronously, so the input is a `NodeFile` (a file descriptor and a range)
// read with `readSync` by a `FileReaderSync` stand-in, and the media handles that are slices of it go back to the
// parent as `{ range: [start, end] }`.
import type { FromWorker, ToWorker } from '@slidock/types';
import { closeSync, fstatSync, openSync, readFileSync, readSync } from 'node:fs';
import { parentPort } from 'node:worker_threads';

import { Compiler, initSync, memoryBytes, version } from '../../pkg/slidock_wasm.js';
import type { SyncBlobReader } from '../../src/compiler';
import { serve } from '../../src/serve';

/** A range of an open file, standing in for a `File` and its slices. */
class NodeFile {
  readonly fd: number;
  readonly start: number;
  readonly end: number;
  readonly name: string;
  readonly type: string;

  constructor(fd: number, start: number, end: number, name: string, type = '') {
    this.fd = fd;
    this.start = start;
    this.end = end;
    this.name = name;
    this.type = type;
  }

  get size(): number {
    return this.end - this.start;
  }

  slice(start = 0, end = this.size, type = ''): NodeFile {
    const clamp = (at: number) => Math.min(Math.max(at, 0), this.size);
    return new NodeFile(this.fd, this.start + clamp(start), this.start + clamp(end), this.name, type);
  }
}

const reader: SyncBlobReader = {
  readAsArrayBuffer(blob: Blob): ArrayBuffer {
    const file = blob as unknown as NodeFile;
    const bytes = new Uint8Array(file.size);
    let done = 0;
    while (done < bytes.length) {
      const read = readSync(file.fd, bytes, done, bytes.length - done, file.start + done);
      if (read === 0) {
        throw new RangeError(`${file.name}: the file ended at ${file.start + done}`);
      }
      done += read;
    }
    return bytes.buffer;
  },
};

/** What the parent sends: the protocol, with the `open` file given by path. */
export type HostRequest =
  Exclude<ToWorker, { type: 'open' }> | { type: 'open'; id: number; path: string; name: string };

/** What the parent receives: the protocol (media slices of the input as ranges) and the module's memory size. */
export type HostReply = FromWorker | { type: 'memory'; bytes: number };

const port = parentPort;
if (port === null) {
  throw new Error('worker-host.ts runs in a worker_threads Worker');
}
const wasm = initSync({ module: readFileSync(new URL('../../pkg/slidock_wasm_bg.wasm', import.meta.url)) });
let fd: number | undefined;
let listener: ((event: MessageEvent<ToWorker>) => void) | undefined;

serve(
  {
    postMessage(message: FromWorker, transfer: Transferable[]) {
      if (message.type === 'deck') {
        const media = message.media.map((handle) => {
          const blob: unknown = handle.blob;
          return blob instanceof NodeFile ? { ...handle, blob: { range: [blob.start, blob.end] } } : handle;
        });
        port.postMessage({ ...message, media }, transfer as ArrayBuffer[]);
      } else {
        port.postMessage(message, transfer as ArrayBuffer[]);
      }
      if (message.type === 'done' || message.type === 'error') {
        port.postMessage({ type: 'memory', bytes: wasm.memory.buffer.byteLength } satisfies HostReply);
      }
    },
    addEventListener(_type, next) {
      listener = next;
    },
  },
  { module: Promise.resolve({ version, memoryBytes, create: () => new Compiler() }), reader },
);

port.on('message', (request: HostRequest) => {
  let message: ToWorker;
  if (request.type === 'open') {
    if (fd !== undefined) {
      closeSync(fd);
    }
    fd = openSync(request.path, 'r');
    const file = new NodeFile(fd, 0, fstatSync(fd).size, request.name);
    message = { type: 'open', id: request.id, file: file as unknown as File };
  } else {
    message = request;
  }
  listener?.({ data: message } as MessageEvent<ToWorker>);
});
