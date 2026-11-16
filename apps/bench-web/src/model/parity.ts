// The parity view's table rows: level B per row against the golden reference, with the
// allowlist entry inline, and the summary of level C as the report holds it.
import type { AllowEntry, CompilerRow, ParityReport, ParityStatus, RowId } from '../schema';
import { outcomeRank } from './outcomes';
import { compareIds } from './rows';

export const PARITY_STATUSES = [
  'identical',
  'pixel-identical',
  'drift-better',
  'drift-same-class',
  'regressed',
  'missing',
  'new',
] as const satisfies readonly ParityStatus[];

export const PARITY_GLOSS: Record<ParityStatus, string> = {
  identical: 'the composed document and the render are equal',
  'pixel-identical': 'the render is equal, the document differs',
  'drift-better': 'the render differs and the class moved up',
  'drift-same-class': 'the render differs and the class stayed, within the erosion guards',
  regressed: 'the class moved down, or stayed and broke an erosion guard',
  missing: 'the run lacks a row the reference has',
  new: 'the run scores a row the reference declined or failed',
};

export interface ParityRow {
  id: RowId;
  row: CompilerRow;
  /** Rank of the run's class minus the reference's; null when the run has no class. */
  classDelta: number | null;
  /** `parity check` fails on the row. */
  failing: boolean;
  allowed: AllowEntry | null;
}

/** Level B's rows, filtered by status (any of; none means all), failing rows first, then by id. */
export function parityRows(report: ParityReport, statuses: readonly string[]): ParityRow[] {
  const compiler = report.compiler;
  if (!compiler) {
    return [];
  }
  const failing = new Set(compiler.failing);
  return Object.entries(compiler.rows)
    .filter(([, row]) => statuses.length === 0 || statuses.includes(row.status))
    .map(([id, row]) => ({
      id,
      row,
      classDelta: row.outcome === undefined ? null : outcomeRank(row.outcome) - outcomeRank(row.reference),
      failing: failing.has(id),
      allowed: row.allowed ?? null,
    }))
    .sort((a, b) => Number(b.failing) - Number(a.failing) || compareIds(a.id, b.id));
}

/** Level B's status counts in status order, zero counts included. */
export function parityCounts(report: ParityReport): { status: ParityStatus; rows: number }[] {
  const counts = report.compiler?.counts ?? {};
  return PARITY_STATUSES.map((status) => ({ status, rows: counts[status] ?? 0 }));
}
