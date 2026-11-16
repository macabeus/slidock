// The motion lane's figures for the Effects tab: which run it reads, each deck's transitions and steps by verdict,
// the coverage of the effect vocabulary native against fallback, the fallbacks to implement next, and the stage and
// transition rows with their frames and timelines. Every figure comes from a motion-lane results file (`motion@1`).
import type {
  Frontend,
  MotionItem,
  MotionOutcome,
  MotionPairReport,
  MotionVerdict,
  Results,
  RowRecord,
  RunEntry,
  RunsIndex,
  TimelineFidelity,
} from '../schema';
import { outcomeRank } from './outcomes';
import { NO_EFFECT, compareIds, rowEffects, rowKind } from './rows';

export { type MotionRowKind, NO_EFFECT, rowEffects, rowKind, slideOf, stageOf, stageSteps } from './rows';

/** The run the tab shows: the selected run when it is a motion run, else the newest motion run. */
export function motionRun(index: RunsIndex, selected: RunEntry): RunEntry | null {
  if (selected.lane === 'motion') {
    return selected;
  }
  return index.runs.findLast((run) => run.lane === 'motion') ?? null;
}

/** Where a motion run played: the renderer image, or the host's Chromium (`--renderer native`). */
export function motionRenderer(results: Results): 'container' | 'native' {
  return results.stamp.renderer.platformDigest.startsWith('native:') ? 'native' : 'container';
}

export const VERDICTS = [
  'native',
  'declared',
  'allowed',
  'accepted',
  'dropped',
  'mismatched',
] as const satisfies readonly MotionVerdict[];

export const VERDICT_LABELS: Record<MotionVerdict, string> = {
  native: 'native',
  declared: 'declared fallback',
  allowed: 'accepted fallback',
  accepted: 'accepted defect',
  dropped: 'dropped (not in the IR)',
  mismatched: 'mismatched',
};

export const MOTION_CLASSES = [
  'motion-mismatch',
  'motion-fallback',
  'motion-close',
  'motion-match',
] as const satisfies readonly MotionOutcome[];

/** A deck's verdicts, worst first. */
export interface DeckFigures {
  pair: string;
  frontend: Frontend;
  report: MotionPairReport;
}

export function deckFigures(results: Results): DeckFigures[] {
  return Object.entries(results.motionPairs ?? {})
    .filter((entry): entry is [string, MotionPairReport] => entry[1] !== undefined)
    .map(([pair, report]) => ({ pair, frontend: (pair.split(':')[1] ?? 'ir') as Frontend, report }))
    .sort((a, b) => b.report.mismatched - a.report.mismatched || a.pair.localeCompare(b.pair));
}

/** Items of every deck, totalled. */
export function verdictTotals(
  decks: DeckFigures[],
): Record<'native' | 'fallbackAllowed' | 'accepted' | 'dropped' | 'mismatched', number> {
  return decks.reduce(
    (sum, { report }) => ({
      native: sum.native + report.native,
      fallbackAllowed: sum.fallbackAllowed + report.fallbackAllowed,
      accepted: sum.accepted + report.accepted,
      dropped: sum.dropped + report.dropped,
      mismatched: sum.mismatched + report.mismatched,
    }),
    { native: 0, fallbackAllowed: 0, accepted: 0, dropped: 0, mismatched: 0 },
  );
}

/** How often an effect played, by how. */
export interface PlayCounts {
  native: number;
  fallback: number;
  unsupported: number;
}

export interface CoverageRow {
  /** The vocabulary effect the deck names, or `—` where it names none and the fallback plays. */
  effect: string;
  kind: 'transition' | 'build';
  byFrontend: Partial<Record<Frontend, PlayCounts>>;
  total: PlayCounts;
}

function add(counts: PlayCounts, played: 'native' | 'fallback' | 'unsupported'): void {
  counts[played] += 1;
}

const empty = (): PlayCounts => ({ native: 0, fallback: 0, unsupported: 0 });

/** The slide's timeline entries: `[static row id, fidelity]`. */
export function timelines(results: Results): [string, TimelineFidelity][] {
  return Object.entries(results.timelines ?? {}).filter(
    (entry): entry is [string, TimelineFidelity] => entry[1] !== undefined,
  );
}

/** Rows are effect × kind, columns frontends: every transition and step that played, native or not. */
export function coverage(results: Results): CoverageRow[] {
  const rows = new Map<string, CoverageRow>();
  const bump = (effect: string | null, kind: CoverageRow['kind'], frontend: Frontend, played: keyof PlayCounts) => {
    const name = effect ?? NO_EFFECT;
    const key = `${kind}\u0000${name}`;
    let row = rows.get(key);
    if (!row) {
      row = { effect: name, kind, byFrontend: {}, total: empty() };
      rows.set(key, row);
    }
    const cell = (row.byFrontend[frontend] ??= empty());
    add(cell, played);
    add(row.total, played);
  };
  for (const [id, fidelity] of timelines(results)) {
    const frontend = (id.split(':')[2] ?? 'ir') as Frontend;
    if (fidelity.transition) {
      bump(fidelity.transition.name, 'transition', frontend, fidelity.transition.effect.played);
    }
    for (const play of fidelity.plays ?? []) {
      bump(play.effect, 'build', frontend, play.played);
    }
  }
  return [...rows.values()].sort(
    (a, b) =>
      a.kind.localeCompare(b.kind) ||
      b.total.native +
        b.total.fallback +
        b.total.unsupported -
        (a.total.native + a.total.fallback + a.total.unsupported) ||
      a.effect.localeCompare(b.effect),
  );
}

/** A native effect the player played its fallback for: the effects to implement next. */
export interface ParetoEntry {
  native: string;
  fallback: string;
  kind: 'transition' | 'build';
  count: number;
  frontends: Frontend[];
}

export function fallbackPareto(results: Results): ParetoEntry[] {
  const entries = new Map<string, ParetoEntry>();
  const bump = (native: string, fallback: string, kind: ParetoEntry['kind'], frontend: Frontend) => {
    const key = `${kind}\u0000${native}`;
    let entry = entries.get(key);
    if (!entry) {
      entry = { native, fallback, kind, count: 0, frontends: [] };
      entries.set(key, entry);
    }
    entry.count += 1;
    if (!entry.frontends.includes(frontend)) {
      entry.frontends.push(frontend);
    }
  };
  for (const [id, fidelity] of timelines(results)) {
    const frontend = (id.split(':')[2] ?? 'ir') as Frontend;
    const transition = fidelity.transition;
    if (transition && transition.effect.played !== 'native') {
      bump(transition.effect.id, transition.effect.fallback, 'transition', frontend);
    }
    for (const play of fidelity.plays ?? []) {
      if (play.played !== 'native') {
        bump(play.native, play.fallback, 'build', frontend);
      }
    }
  }
  return [...entries.values()].sort((a, b) => b.count - a.count || a.native.localeCompare(b.native));
}

export interface MotionFilters {
  /** Motion classes (any of); empty for all. */
  classes: string[];
  kind: 'all' | 'stage' | 'transition';
  /** A deck (`<presentation>:<frontend>`). */
  deck: string | null;
  /** An effect name, or `—`. */
  effect: string | null;
}

export const NO_MOTION_FILTERS: MotionFilters = { classes: [], kind: 'all', deck: null, effect: null };

/** The motion rows the filters keep, worst class first, then by id. */
export function motionRows(results: Results, filters: MotionFilters): [string, RowRecord][] {
  return Object.entries(results.rows)
    .filter((entry): entry is [string, RowRecord] => entry[1] !== undefined)
    .filter(([id, row]) => {
      const kind = rowKind(id);
      if (kind === null || (filters.kind !== 'all' && kind !== filters.kind)) {
        return false;
      }
      if (filters.classes.length > 0 && !filters.classes.includes(row.outcome)) {
        return false;
      }
      if (filters.deck !== null && `${row.presentation}:${row.frontend}` !== filters.deck) {
        return false;
      }
      return filters.effect === null || rowEffects(results, id).includes(filters.effect);
    })
    .sort(([a, x], [b, y]) => outcomeRank(x.outcome) - outcomeRank(y.outcome) || compareIds(a, b));
}

/** The items of every deck that did not play natively, mismatched first. */
export function nonNativeItems(results: Results, verdict: MotionVerdict | null): (MotionItem & { pair: string })[] {
  const rank = new Map<string, number>(VERDICTS.map((name, i) => [name, -i]));
  return deckFigures(results)
    .flatMap(({ pair, report }) => report.items.map((item) => ({ ...item, pair })))
    .filter((item) => verdict === null || item.verdict === verdict)
    .sort(
      (a, b) =>
        (rank.get(a.verdict) ?? 0) - (rank.get(b.verdict) ?? 0) ||
        a.pair.localeCompare(b.pair) ||
        compareIds(`${a.slide}`, `${b.slide}`) ||
        a.item.localeCompare(b.item),
    );
}

/** A row's failed checks: each check's name, and what failed (`<check>: <detail>` in the record). */
export interface FailedCheck {
  check: string;
  detail: string;
}

export function failedChecks(row: RowRecord): FailedCheck[] {
  return (row.motion?.failures ?? []).map((failure) => {
    const at = failure.indexOf(': ');
    return at < 0 ? { check: failure, detail: '' } : { check: failure.slice(0, at), detail: failure.slice(at + 2) };
  });
}

/** A step's or transition's params as the deck writes them, for display: `dir: l, distance: offSlide`. */
export function paramsText(params: string | undefined): string | null {
  if (params === undefined) {
    return null;
  }
  try {
    const value: unknown = JSON.parse(params);
    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
      return params;
    }
    return Object.entries(value)
      .map(([key, v]) => `${key}: ${typeof v === 'string' ? v : JSON.stringify(v)}`)
      .join(', ');
  } catch {
    return params;
  }
}

/** `start → duration` in ms, or a dash. */
export function timing(value: { startMs: number; durationMs: number } | null): string {
  return value === null ? '—' : `${value.startMs} + ${value.durationMs}`;
}
