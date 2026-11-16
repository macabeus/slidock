// Two results files side by side, by the rules of `slidock-bench regression` (the Rust side
// is `slidock_bench_schema::compare` and `Stamp::incomparable`): the stamp fields that make the runs incomparable, how
// each row's class moved, the within-class erosion guards, the rows whose undrawn shadows or approximated gradients grew,
// the rows the base flags (kept out of acceptance) or the head newly flags, the per-frontend SSIM guard, and the rows
// that vanished or appeared.
import type { Results, RowId, RowRecord, Scores, Stamp } from '../schema';
import { formatNumber } from './format';
import { outcomeRank } from './outcomes';
import { compareIds, flagged } from './rows';

export type Movement = 'same' | 'regressed' | 'improved' | 'undeclined';

const SCORED = new Set(['match', 'close', 'mismatch']);

/** How a row's class moved; a declined row that is now scored is `undeclined`, its own bucket for review. */
export function movement(base: string, current: string): Movement {
  if (base === current) {
    return 'same';
  }
  if (base === 'declined' && SCORED.has(current)) {
    return 'undeclined';
  }
  return outcomeRank(current) > outcomeRank(base) ? 'improved' : 'regressed';
}

/** Within-class guards: a row may keep its class and still lose at most this much. */
export const EROSION = {
  ssimMean: -0.005,
  textRecall: -0.01,
  textP95Pm: 1.4,
  colorMean: 1,
  shadowWorstRmse: 0.01,
  gradientWorstRmse: 0.01,
} as const;

/** The shadow-not-drawn codes (Keynote, PPTX, ODP): a row whose count of any grows regressed, whatever its class did. */
export const SHADOW_NOT_DRAWN = ['KN1304', 'PX2111', 'OD9104'] as const;

/** The shadow-not-drawn codes whose count grew, as `<code> <base> → <current>`. */
export function shadowLosses(base: RowRecord, current: RowRecord): string[] {
  return SHADOW_NOT_DRAWN.flatMap((code) => {
    const b = base.diagnostics[code] ?? 0;
    const c = current.diagnostics[code] ?? 0;
    return c > b ? [`${code} ${b} → ${c}`] : [];
  });
}

/** The `shadow@1` guard of a row of unchanged class: its worst ring's RMSE may grow at most 0.01. */
export function shadowErosion(base: RowRecord, current: RowRecord): string[] {
  const b = base.shadow;
  const c = current.shadow;
  if (!b || !c || c.worstRmse - b.worstRmse <= EROSION.shadowWorstRmse) {
    return [];
  }
  return [`shadow.worstRmse ${formatNumber(b.worstRmse)} → ${formatNumber(c.worstRmse)} (${c.worstEl})`];
}

/** The gradient-approximated codes (Keynote, PPTX, ODP): a row whose count of any grows regressed, whatever its class
 * did. */
export const GRADIENT_APPROXIMATED = ['KN1301', 'PX2106', 'OD9103'] as const;

/** The gradient-approximated codes whose count grew, as `<code> <base> → <current>`. */
export function gradientLosses(base: RowRecord, current: RowRecord): string[] {
  return GRADIENT_APPROXIMATED.flatMap((code) => {
    const b = base.diagnostics[code] ?? 0;
    const c = current.diagnostics[code] ?? 0;
    return c > b ? [`${code} ${b} → ${c}`] : [];
  });
}

/** The `gradient@1` guards of a row of unchanged class: no region may go (a gradient gone from the IR without a word),
 * and its worst region's RMSE may grow at most 0.01. */
export function gradientErosion(base: RowRecord, current: RowRecord): string[] {
  const b = base.gradient;
  const c = current.gradient;
  const broken: string[] = [];
  const before = b?.elements ?? 0;
  const after = c?.elements ?? 0;
  if (after < before) {
    broken.push(`gradient regions ${before} → ${after}`);
  }
  if (b && c && c.worstRmse - b.worstRmse > EROSION.gradientWorstRmse) {
    broken.push(`gradient.worstRmse ${formatNumber(b.worstRmse)} → ${formatNumber(c.worstRmse)} (${c.worstEl})`);
  }
  return broken;
}

/** The erosion guards `current` breaks against `base`, as `<field> <base> → <current>`. */
export function erosionGuards(base: Scores, current: Scores): string[] {
  const n = formatNumber;
  const broken: string[] = [];
  if (current.ssim.mean - base.ssim.mean < EROSION.ssimMean) {
    broken.push(`ssim.mean ${n(base.ssim.mean)} → ${n(current.ssim.mean)}`);
  }
  if (current.text.recall - base.text.recall < EROSION.textRecall) {
    broken.push(`text.recall ${n(base.text.recall)} → ${n(current.text.recall)}`);
  }
  const bp = base.text.p95Pm;
  const cp = current.text.p95Pm;
  if (bp !== null && cp !== null && cp - bp > EROSION.textP95Pm) {
    broken.push(`text.p95Pm ${n(bp)} → ${n(cp)}`);
  }
  if (current.color.mean - base.color.mean > EROSION.colorMean) {
    broken.push(`color.mean ${n(base.color.mean)} → ${n(current.color.mean)}`);
  }
  return broken;
}

/** A pair key (`<presentation>:<frontend>`) of a row. */
export const pairOf = (record: RowRecord) => `${record.presentation}:${record.frontend}`;

/** The stamp fields that differ beyond the compiler for the pairs compared: non-empty means incomparable. */
export function incomparable(base: Stamp, head: Stamp, pairs: readonly string[]): string[] {
  const fields: string[] = [];
  const check = (name: string, equal: boolean) => {
    if (!equal) {
      fields.push(name);
    }
  };
  const same = (a: unknown, b: unknown) => JSON.stringify(sortKeys(a)) === JSON.stringify(sortKeys(b));
  check('harness', base.harness === head.harness);
  check('renderer.recipe', base.renderer.recipe === head.renderer.recipe);
  check('renderer.platformDigest', base.renderer.platformDigest === head.renderer.platformDigest);
  check('renderer.arch', base.renderer.arch === head.renderer.arch);
  check('fonts.fontSetId', base.fonts.fontSetId === head.fonts.fontSetId);
  check('metrics', same(base.metrics, head.metrics));
  check('classifier', base.classifier === head.classifier);
  check('deps', same(base.deps, head.deps));
  check('lane', base.lane === head.lane);
  for (const pair of pairs) {
    check(`corpus[${pair}]`, base.corpus[pair] === head.corpus[pair]);
  }
  return fields;
}

function sortKeys(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(sortKeys);
  }
  if (typeof value === 'object' && value !== null) {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([a], [b]) => compareIds(a, b))
        .map(([key, inner]) => [key, sortKeys(inner)]),
    );
  }
  return value;
}

/** A value of a flattened object: leaves by dotted path, arrays joined with `,`. */
export function flatten(value: unknown, prefix = '', into = new Map<string, string>()): Map<string, string> {
  if (Array.isArray(value)) {
    into.set(prefix, value.map((item) => (typeof item === 'object' ? JSON.stringify(item) : String(item))).join(','));
  } else if (typeof value === 'object' && value !== null) {
    for (const [key, inner] of Object.entries(value).sort(([a], [b]) => compareIds(a, b))) {
      flatten(inner, prefix === '' ? key : `${prefix}.${key}`, into);
    }
  } else {
    into.set(prefix, String(value));
  }
  return into;
}

export interface StampField {
  field: string;
  base: string | null;
  head: string | null;
}

/** Every stamp field whose value differs between the two runs, in path order. */
export function stampDiff(base: Stamp, head: Stamp): StampField[] {
  const a = flatten(base);
  const b = flatten(head);
  const fields = [...new Set([...a.keys(), ...b.keys()])].sort(compareIds);
  return fields
    .filter((field) => a.get(field) !== b.get(field))
    .map((field) => ({ field, base: a.get(field) ?? null, head: b.get(field) ?? null }));
}

/** Whether a stamp field falls under an incomparable name (`metrics` covers `metrics.ssim`; `corpus[p]` is `corpus.p`). */
export function blocks(field: string, names: readonly string[]): boolean {
  return names.some((name) => {
    const path = name.startsWith('corpus[') && name.endsWith(']') ? `corpus.${name.slice(7, -1)}` : name;
    return field === path || field.startsWith(`${path}.`);
  });
}

export interface RowChange {
  id: RowId;
  base: RowRecord;
  head: RowRecord;
  movement: Movement;
  /** Head's SSIM mean minus base's, when both are scored. */
  ssimDelta: number | null;
  guards: string[];
  /** The shadow-not-drawn counts that grew: such a row regressed whatever its class did. */
  losses: string[];
}

export interface Transition {
  from: string;
  to: string;
  rows: RowId[];
}

/** Per frontend: the largest allowed drop of the SSIM median or 10th percentile. */
export const FRONTEND_SSIM_DROP = 0.002;

/** A frontend whose SSIM median or p10, over the rows both runs hold (reference rows apart), dropped too far. */
export interface FrontendGuard {
  frontend: string;
  stat: 'median' | 'p10';
  base: number;
  head: number;
  /** As `slidock-bench regression` prints it: `<frontend> ssim <stat> <base> → <head>`. */
  text: string;
}

export interface Comparison {
  /** Stamp fields beyond the compiler that differ; non-empty means the runs are not comparable. */
  incomparable: string[];
  stamp: StampField[];
  /** Rows by (base class, head class), every pair that holds a row. */
  transitions: Transition[];
  regressed: RowChange[];
  improved: RowChange[];
  undeclined: RowChange[];
  /** Same class, but an erosion guard broke. */
  eroded: RowChange[];
  /** Changed rows the base flags reference or known loss: reported, kept out of acceptance. */
  flagged: RowChange[];
  /** Rows the head flags and the base does not: a change to review. */
  newlyFlagged: RowChange[];
  frontendGuards: FrontendGuard[];
  /** Base rows missing from a pair the head covered. */
  vanished: RowId[];
  /** Rows only the head holds. */
  added: RowId[];
  /** Base rows of pairs the head did not cover. */
  notRun: number;
  compared: number;
}

function byLargestDelta(a: RowChange, b: RowChange): number {
  const left = a.ssimDelta === null ? -1 : Math.abs(a.ssimDelta);
  const right = b.ssimDelta === null ? -1 : Math.abs(b.ssimDelta);
  return right - left || compareIds(a.id, b.id);
}

/** Results numbers live on a 1e-4 grid. */
const quantise = (value: number) => Math.round(value * 10_000) / 10_000 + 0;

/** The quantised SSIM median and nearest-rank 10th percentile over the scored records, as the results summary has them. */
export function ssimDistribution(records: readonly RowRecord[]): { median: number | null; p10: number | null } {
  const ssims = Float64Array.from(records.flatMap((record) => (record.scores ? [record.scores.ssim.mean] : []))).sort();
  if (ssims.length === 0) {
    return { median: null, p10: null };
  }
  const at = (index: number) => ssims[index] ?? Number.NaN;
  const mid = ssims.length >> 1;
  const median = ssims.length % 2 ? at(mid) : (at(mid - 1) + at(mid)) / 2;
  const p10 = at(Math.min(ssims.length - 1, Math.max(0, Math.ceil(0.1 * ssims.length) - 1)));
  return { median: quantise(median), p10: quantise(p10) };
}

/** The frontends whose SSIM median or p10 over `pairs` (reference rows apart) dropped by more than the guard. */
export function frontendGuards(pairs: readonly { base: RowRecord; head: RowRecord }[]): FrontendGuard[] {
  const guards: FrontendGuard[] = [];
  for (const frontend of new Set(pairs.map((pair) => pair.base.frontend))) {
    const rows = pairs.filter((pair) => pair.base.frontend === frontend && pair.base.reference !== true);
    if (rows.length === 0) {
      continue;
    }
    const before = ssimDistribution(rows.map((row) => row.base));
    const after = ssimDistribution(rows.map((row) => row.head));
    for (const stat of ['median', 'p10'] as const) {
      const base = before[stat];
      const head = after[stat];
      if (base !== null && head !== null && base - head > FRONTEND_SSIM_DROP) {
        guards.push({ frontend, stat, base, head, text: `${frontend} ssim ${stat} ${base} → ${head}` });
      }
    }
  }
  return guards;
}

/** The head run against the base run, bucketed as `slidock-bench regression` buckets it. */
export function compareResults(base: Results, head: Results): Comparison {
  const headPairs = new Set(Object.keys(head.stamp.corpus));
  const changes: RowChange[] = [];
  const counted: RowChange[] = [];
  const flaggedChanges: RowChange[] = [];
  const newlyFlagged: RowChange[] = [];
  const vanished: RowId[] = [];
  const counts = new Map<string, Transition>();
  const compared = new Set<string>();
  let notRun = 0;
  for (const id of Object.keys(base.rows).sort(compareIds)) {
    const before = base.rows[id];
    if (!before) {
      continue;
    }
    const after = head.rows[id];
    if (!after) {
      if (headPairs.has(pairOf(before))) {
        vanished.push(id);
      } else {
        notRun += 1;
      }
      continue;
    }
    compared.add(pairOf(before));
    const losses = [...shadowLosses(before, after), ...gradientLosses(before, after)];
    const moved = losses.length > 0 ? 'regressed' : movement(before.outcome, after.outcome);
    const guards =
      before.outcome === after.outcome && before.scores && after.scores
        ? [
            ...erosionGuards(before.scores, after.scores),
            ...shadowErosion(before, after),
            ...gradientErosion(before, after),
          ]
        : [];
    const ssimDelta = before.scores && after.scores ? after.scores.ssim.mean - before.scores.ssim.mean : null;
    const change: RowChange = { id, base: before, head: after, movement: moved, ssimDelta, guards, losses };
    changes.push(change);
    if (flagged(after) && !flagged(before)) {
      newlyFlagged.push(change);
    }
    if (moved !== 'same' || guards.length > 0) {
      (flagged(before) ? flaggedChanges : counted).push(change);
    }
    const key = `${before.outcome}\u0000${after.outcome}`;
    const transition = counts.get(key) ?? { from: before.outcome, to: after.outcome, rows: [] };
    transition.rows.push(id);
    counts.set(key, transition);
  }
  const added = Object.keys(head.rows).filter((id) => !(id in base.rows));
  const pick = (kind: Movement) => counted.filter((change) => change.movement === kind).sort(byLargestDelta);
  return {
    incomparable: incomparable(base.stamp, head.stamp, [...compared].sort(compareIds)),
    stamp: stampDiff(base.stamp, head.stamp),
    transitions: [...counts.values()].sort(
      (a, b) => outcomeRank(b.from) - outcomeRank(a.from) || outcomeRank(b.to) - outcomeRank(a.to),
    ),
    regressed: pick('regressed'),
    improved: pick('improved'),
    undeclined: pick('undeclined'),
    eroded: counted.filter((change) => change.guards.length > 0).sort(byLargestDelta),
    flagged: flaggedChanges.sort(byLargestDelta),
    newlyFlagged: newlyFlagged.sort(byLargestDelta),
    frontendGuards: frontendGuards(changes),
    vanished,
    added: added.sort(compareIds),
    notRun,
    compared: changes.length,
  };
}

/**
 * What `slidock-bench regression` exits with: 2 incomparable, 3 nothing compared, 1 a counted change, else 0. An
 * undecline counts, as it does without `--accept-undeclines`.
 */
export function regressionExit(comparison: Comparison): 0 | 1 | 2 | 3 {
  if (comparison.incomparable.length > 0) {
    return 2;
  }
  if (comparison.compared === 0) {
    return 3;
  }
  const failed =
    comparison.regressed.length > 0 ||
    comparison.improved.length > 0 ||
    comparison.undeclined.length > 0 ||
    comparison.eroded.length > 0 ||
    comparison.newlyFlagged.length > 0 ||
    comparison.vanished.length > 0 ||
    comparison.frontendGuards.length > 0;
  return failed ? 1 : 0;
}
