// The overview's aggregates. Class counts, medians and means come from the results file's own summary (the CLI's
// numbers); the UI only adds counts of rows by kind and the deltas between two summaries.
import type { Frontend, FrontendSummary, Outcome, Results, Summary } from '../schema';
import { FRONTEND_SSIM_DROP } from './compare';
import { OUTCOMES } from './outcomes';
import { isStepRow } from './rows';

export const FRONTENDS = ['pptx', 'keynote', 'odp', 'ir'] as const satisfies readonly Frontend[];

export const FRONTEND_LABELS: Record<Frontend, string> = {
  pptx: 'PPTX',
  keynote: 'Keynote',
  odp: 'ODP',
  ir: 'IR probes',
};

export function frontendLabel(frontend: string): string {
  return (FRONTENDS as readonly string[]).includes(frontend) ? FRONTEND_LABELS[frontend as Frontend] : frontend;
}

/** The frontends a summary covers, in display order. */
export function summaryFrontends(byFrontend: Summary['byFrontend']): [Frontend, FrontendSummary][] {
  return FRONTENDS.flatMap((frontend) => {
    const summary = byFrontend[frontend];
    return summary ? [[frontend, summary] as [Frontend, FrontendSummary]] : [];
  });
}

export interface Segment {
  outcome: Outcome;
  count: number;
}

/** A frontend's rows by class, best class first, empty classes left out. */
export function outcomeSegments(summary: FrontendSummary): Segment[] {
  return OUTCOMES.map((outcome) => ({ outcome, count: summary[outcome] })).filter((segment) => segment.count > 0);
}

export function totals(summary: Summary): FrontendSummary | null {
  const parts = summaryFrontends(summary.byFrontend).map(([, value]) => value);
  if (parts.length === 0) {
    return null;
  }
  const sum = (pick: (s: FrontendSummary) => number) => parts.reduce((total, part) => total + pick(part), 0);
  return {
    rows: sum((s) => s.rows),
    match: sum((s) => s.match),
    close: sum((s) => s.close),
    mismatch: sum((s) => s.mismatch),
    declined: sum((s) => s.declined),
    'render-error': sum((s) => s['render-error']),
    failed: sum((s) => s.failed),
    // A median over several frontends is not the sum of theirs; the summary has no overall one.
    ssim: { median: null, p10: null },
    textRecall: { mean: null },
  };
}

export interface Tiles {
  decks: number;
  slides: number;
  scored: number;
  steps: number;
  frontends: number;
  /** Matched rows over all rows of the summary; null over none. */
  matchShare: number | null;
}

/** The headline tiles of a results file. */
export function tiles(results: Results): Tiles {
  const records = Object.values(results.rows);
  const total = totals(results.summary);
  return {
    decks: new Set(records.map((record) => record.presentation)).size,
    slides: records.filter((record) => !isStepRow(record)).length,
    scored: total ? total.match + total.close + total.mismatch : 0,
    steps: records.filter(isStepRow).length,
    frontends: summaryFrontends(results.summary.byFrontend).length,
    matchShare: total && total.rows > 0 ? total.match / total.rows : null,
  };
}

export type Trend = 'improved' | 'regressed' | 'unchanged';

/**
 * The move a summary figure may make and still read as unchanged. The regression policy bounds only the SSIM median
 * and p10 per frontend (`FRONTEND_SSIM_DROP`); text recall and the match share have no such bound, so any move shows.
 */
export const FIGURE_TOLERANCE = {
  ssimMedian: FRONTEND_SSIM_DROP,
  ssimP10: FRONTEND_SSIM_DROP,
  textRecall: 0,
  matchShare: 0,
} as const;

export interface Figure {
  value: number | null;
  /** The value minus the base's, when both exist. */
  delta: number | null;
  trend: Trend;
}

/** A figure against its base: higher is better; a move within the tolerance is unchanged. */
export function figure(value: number | null, base: number | null | undefined, tolerance: number): Figure {
  if (value === null || base === null || base === undefined) {
    return { value, delta: null, trend: 'unchanged' };
  }
  const delta = value - base;
  const trend: Trend = delta > tolerance ? 'improved' : delta < -tolerance ? 'regressed' : 'unchanged';
  return { value, delta, trend };
}

export interface FidelityStrip {
  ssimMedian: Figure;
  ssimP10: Figure;
  textRecall: Figure;
  matchShare: Figure;
}

/** A frontend's fidelity figures against the base summary's (when one is given). */
export function fidelity(summary: FrontendSummary, base: FrontendSummary | undefined): FidelityStrip {
  const share = (s: FrontendSummary | undefined) => (s && s.rows > 0 ? s.match / s.rows : null);
  return {
    ssimMedian: figure(summary.ssim.median, base?.ssim.median, FIGURE_TOLERANCE.ssimMedian),
    ssimP10: figure(summary.ssim.p10, base?.ssim.p10, FIGURE_TOLERANCE.ssimP10),
    textRecall: figure(summary.textRecall.mean, base?.textRecall.mean, FIGURE_TOLERANCE.textRecall),
    matchShare: figure(share(summary), share(base), FIGURE_TOLERANCE.matchShare),
  };
}

/** The deciding decline codes, most rows first, ties by code. */
export function declineRanking(summary: Summary): { code: string; rows: number }[] {
  return Object.entries(summary.declines)
    .map(([code, rows]) => ({ code, rows }))
    .sort((a, b) => b.rows - a.rows || (a.code < b.code ? -1 : a.code > b.code ? 1 : 0));
}
