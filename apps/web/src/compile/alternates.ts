// The download's copies of the videos with alpha that only some browsers play (`media/alpha.ts`): the compiler
// plans them, the browser makes each one, the compiler checks it and the bundle offers it after the original.
import type { PlannedVideo } from '@slidock/types';

import type { AlphaCopier, AlphaOutcome } from '../media/alpha';
import { RequestError } from './client';
import type { VideosProgress } from './session';

/** What making the copies did. */
export interface AlternatesSummary {
  /** The videos that get a copy. */
  planned: number;
  /** Those the zip offers a copy of, and the copies' bytes. */
  made: number;
  bytes: number;
  /** The videos left with no copy, with why: they play only where the original plays. */
  kept: readonly { path: string; reason: string }[];
  /** In milliseconds, from the plan to the last copy handed over. */
  elapsed: number;
}

/** What the step needs of the compile worker. */
export interface AlternatesWorker {
  alternatePlan(): Promise<PlannedVideo[]>;
  addAlternate(path: string, file: Blob): Promise<void>;
}

/** The copies made for an open deck, by path and size: a download again reuses those that finished. */
export type MadeCopies = Map<string, Promise<AlphaOutcome>>;

/**
 * Has the browser make the copy of each video the compiler plans, showing their progress by frames, and hands each
 * one to the compiler for the bundle. A copy this browser cannot make, or a file the compiler refuses, leaves the
 * video with none. Returns `null` once `current` says the download was dropped.
 */
export async function makeAlternates(
  worker: AlternatesWorker,
  files: ReadonlyMap<string, Blob>,
  copier: AlphaCopier,
  made: MadeCopies,
  current: () => boolean,
  show: (progress: VideosProgress) => void,
): Promise<AlternatesSummary | null> {
  const start = performance.now();
  const plan = await worker.alternatePlan();
  if (!current()) {
    return null;
  }
  const progress: VideosProgress = {
    done: 0,
    total: plan.length,
    frames: 0,
    totalFrames: plan.reduce((sum, video) => sum + video.frames, 0),
    started: start,
    updated: start,
  };
  const frames = new Map<string, number>();
  const update = () => show({ ...progress, updated: performance.now() });
  if (plan.length > 0) {
    update();
  }
  const outcomes = await Promise.all(
    plan.map(async (video) => {
      const key = `${video.path} ${video.targetWidth}x${video.targetHeight}`;
      let outcome = made.get(key);
      if (outcome === undefined) {
        const file = files.get(video.path);
        outcome = file
          ? copier.copy({ plan: video, file }, ({ frames: done }) => {
              frames.set(video.path, done);
              progress.frames = [...frames.values()].reduce((sum, count) => sum + count, 0);
              update();
            })
          : Promise.resolve({ kind: 'kept', reason: 'the app holds no file of it', elapsed: 0 });
        made.set(key, outcome);
      }
      const settled = await outcome;
      // A copy a cancel stopped is made by the next download.
      if (!current() && settled.kind === 'kept') {
        made.delete(key);
      }
      frames.set(video.path, video.frames);
      progress.frames = [...frames.values()].reduce((sum, count) => sum + count, 0);
      progress.done += 1;
      update();
      return { video, outcome: settled };
    }),
  );
  const summary = { planned: plan.length, made: 0, bytes: 0 };
  const kept: { path: string; reason: string }[] = [];
  for (const { video, outcome } of outcomes) {
    if (!current()) {
      return null;
    }
    if (outcome.kind === 'kept') {
      kept.push({ path: video.path, reason: outcome.reason });
      continue;
    }
    try {
      await worker.addAlternate(video.path, outcome.file);
      summary.made += 1;
      summary.bytes += outcome.file.size;
    } catch (error) {
      if (!(error instanceof RequestError) || error.error.kind !== 'request') {
        throw error;
      }
      kept.push({ path: video.path, reason: error.error.message });
    }
  }
  return { ...summary, kept, elapsed: performance.now() - start };
}
