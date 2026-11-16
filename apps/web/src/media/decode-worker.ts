// The HEIC decode worker: decodes each photo it is sent (`heic.ts`) and answers with its JPEG (transferred), or why
// this browser cannot decode it.
import { Undecodable, browserEnvironment, decodeHeic } from './heic';
import type { DecodeReply, DecodeRequest } from './photos';

interface DecodeScope {
  postMessage(message: DecodeReply, transfer?: Transferable[]): void;
  onmessage: ((event: MessageEvent<DecodeRequest>) => void) | null;
}

declare const self: DecodeScope & typeof globalThis;

const env = browserEnvironment(self);

self.onmessage = ({ data: { id, plan, file } }) => {
  decodeHeic(plan, file, env).then(
    ({ bytes, mime, path, elapsed }) => self.postMessage({ id, ok: true, bytes, mime, path, elapsed }, [bytes]),
    (error: unknown) =>
      self.postMessage({
        id,
        ok: false,
        reasons: error instanceof Undecodable ? [...error.reasons] : [String(error)],
      }),
  );
};
