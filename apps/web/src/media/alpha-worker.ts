// The copy worker: encodes the frames the page takes from a `<video>` into a WebM with alpha (`alpha.ts`), one copy
// per worker, answering each request in order.
import {
  type AlphaEncode,
  type AlphaReply,
  type AlphaRequest,
  browserAlphaEncoderEnvironment,
  startAlphaEncode,
} from './alpha';

interface AlphaScope {
  postMessage(message: AlphaReply): void;
  onmessage: ((event: MessageEvent<AlphaRequest>) => void) | null;
}

declare const self: AlphaScope & typeof globalThis;

const env = browserAlphaEncoderEnvironment(self);
let encode: AlphaEncode | null = null;
let queue = Promise.resolve();
/** Why an encode that already answered a frame failed: the next frame, or the finish, is answered with it. */
let failure: string | null = null;

const reason = (error: unknown) => (error instanceof Error ? error.message : String(error));

/**
 * Answers `request`. A frame is answered as soon as it is drawn (the page may then seek its `<video>` on: WebKit
 * recycles the frame's buffer as it seeks), and its encode goes on before the next request is taken.
 */
const answer = async (request: AlphaRequest): Promise<void> => {
  switch (request.type) {
    case 'start': {
      const started = await startAlphaEncode(request.spec, env);
      encode = typeof started === 'string' ? null : started;
      self.postMessage({ type: 'started', refused: typeof started === 'string' ? started : null });
      return;
    }
    case 'frame': {
      if (!encode || failure !== null) {
        request.frame.close();
        self.postMessage({ type: 'failed', reason: failure ?? 'the copy was not started' });
        return;
      }
      const added = encode.add(request.frame, request.timestamp, request.key);
      self.postMessage({ type: 'taken' });
      try {
        await added;
      } catch (error) {
        failure = reason(error);
      }
      return;
    }
    case 'finish': {
      if (!encode || failure !== null) {
        self.postMessage({ type: 'failed', reason: failure ?? 'the copy was not started' });
        return;
      }
      const file = await encode.finish(request.duration);
      self.postMessage({ type: 'finished', file, codec: encode.codec, transparentFrames: encode.transparentFrames });
    }
  }
};

self.onmessage = ({ data }) => {
  queue = queue.then(() =>
    answer(data).catch((error: unknown) => self.postMessage({ type: 'failed', reason: reason(error) })),
  );
};
