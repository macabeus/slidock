// The picture worker: re-encodes each picture it is sent (`compress.ts`) and answers with the new file (transferred),
// or why the original stays.
import { browserCompressEnvironment, compressImage } from './compress';
import type { CompressReply, CompressRequest } from './compressor';

interface CompressScope {
  postMessage(message: CompressReply, transfer?: Transferable[]): void;
  onmessage: ((event: MessageEvent<CompressRequest>) => void) | null;
}

declare const self: CompressScope & typeof globalThis;

const env = browserCompressEnvironment(self);

self.onmessage = ({ data: { id, job } }) => {
  compressImage(job, env).then(
    (outcome) => self.postMessage({ id, outcome }, outcome.kind === 'compressed' ? [outcome.bytes] : []),
    (error: unknown) =>
      self.postMessage({
        id,
        outcome: { kind: 'kept', reason: `the re-encode failed (${String(error)})`, elapsed: 0 },
      }),
  );
};
