// The video worker: re-encodes each video it is sent (`video.ts`), posting its progress, and answers with the new
// file or why the original stays.
import type { PoolReply } from './compressor';
import { type VideoJob, type VideoOutcome, type VideoProgress, browserVideoEnvironment, compressVideo } from './video';

interface VideoScope {
  postMessage(message: PoolReply<VideoOutcome, VideoProgress>): void;
  onmessage: ((event: MessageEvent<{ id: number; job: VideoJob }>) => void) | null;
}

declare const self: VideoScope & typeof globalThis;

const env = browserVideoEnvironment(self);

self.onmessage = ({ data: { id, job } }) => {
  compressVideo(job, env, (progress) => self.postMessage({ id, progress })).then(
    (outcome) => self.postMessage({ id, outcome }),
    (error: unknown) =>
      self.postMessage({
        id,
        outcome: { kind: 'kept', reason: `the re-encode failed (${String(error)})`, elapsed: 0 },
      }),
  );
};
