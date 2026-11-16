import type { Lane, RunEntry, RunsIndex } from '../schema';
import { formatTime, short } from './format';

export const LANES = ['static', 'host', 'fonts', 'motion', 'shell'] as const satisfies readonly Lane[];

export const LANE_LABELS: Record<Lane, string> = {
  static: 'Static',
  host: 'Host',
  fonts: 'Fonts',
  motion: 'Motion',
  shell: 'Shell',
};

/** What each lane checks. */
export const LANE_GLOSS: Record<Lane, string> = {
  static: "Each slide's static render against its oracle: the lane regression and parity read.",
  host: 'The slide mounted in a shadow root on the player stage must equal its standalone render, pixel for pixel.',
  fonts: 'The deck rendered from the faces slidock ships (no system fonts), with glyph attribution per text node.',
  motion:
    "Each deck played by its own player, animations paused and seeked: every transition and build against the deck's timeline, its still renders and the probes' expectations.",
  shell: 'The downloaded zip opened from file:// must show exactly what was scored.',
};

/**
 * The run a reference names: `latest` is the newest static run, `baseline` the newest static run that wrote the
 * committed baseline, anything else a run key. Null when the index has no such run.
 */
export function resolveRun(index: RunsIndex, ref: string): RunEntry | null {
  const runs = index.runs;
  if (ref === 'latest') {
    return runs.findLast((run) => run.lane === 'static') ?? null;
  }
  if (ref === 'baseline') {
    return runs.findLast((run) => run.lane === 'static' && run.baseline === true) ?? null;
  }
  return runs.find((run) => run.key === ref) ?? null;
}

/** The newest run of each lane, in lane order. */
export function latestByLane(index: RunsIndex): RunEntry[] {
  return LANES.flatMap((lane) => {
    const run = index.runs.findLast((entry) => entry.lane === lane);
    return run ? [run] : [];
  });
}

/** `2026-10-30 09:30 UTC · static · 5b05b05`, the run's time, lane and commit. */
export function runLabel(run: RunEntry): string {
  const dirty = run.compiler.dirty ? '+dirty' : '';
  return `${formatTime(run.recordedAt)} · ${run.lane} · ${short(run.compiler.commit)}${dirty}`;
}
