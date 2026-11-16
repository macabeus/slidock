// The explorer's logic over a results file: row ids taken apart, the filters (those of `slidock-bench list` and a
// few it lacks), the count each filter option would leave, the sort, and the `list` command that prints the same
// rows when the filters allow one. Nothing here computes a score; it filters and orders what the records say.
import type { Results, RowId, RowRecord, StepPlay, TimelineFidelity } from '../schema';
import type { Filters, SortDir, SortKey } from '../state/url';
import { outcomeRank } from './outcomes';

export type RowStepKind = { kind: 'static' } | { kind: 'stage'; stage: number } | { kind: 'transition'; pm: number };

export interface RowIdParts {
  presentation: string;
  slide: string;
  frontend: string;
  step: RowStepKind;
}

/** `<presentation>:<slide>:<frontend>[#s<k>|#t@<pm>]` taken apart, or null when the id has another shape. */
export function parseRowId(id: string): RowIdParts | null {
  const match = /^([^:#]+):([^:#]+):([^:#]+)(?:#s(0|[1-9]\d*)|#t@([1-9]\d{0,2}))?$/.exec(id);
  if (!match) {
    return null;
  }
  const [, presentation = '', slide = '', frontend = '', stage, pm] = match;
  const step: RowStepKind =
    stage !== undefined
      ? { kind: 'stage', stage: Number(stage) }
      : pm !== undefined
        ? { kind: 'transition', pm: Number(pm) }
        : { kind: 'static' };
  return { presentation, slide, frontend, step };
}

/** The step part of a row for display: `stage 2`, `transition 50%`, or null for a static row. */
export function stepLabel(step: RowStepKind): string | null {
  switch (step.kind) {
    case 'static':
      return null;
    case 'stage':
      return `stage ${step.stage}`;
    case 'transition':
      return `transition ${step.pm / 10}%`;
  }
}

export interface Row {
  id: RowId;
  record: RowRecord;
  /** A motion row's effects (`rowEffects`); none for the others. */
  effects?: string[];
}

export function rowsOf(results: Pick<Results, 'rows'> & Partial<Pick<Results, 'timelines'>>): Row[] {
  return Object.entries(results.rows).map(([id, record]) => ({ id, record, effects: rowEffects(results, id) }));
}

/** The marker of an item whose deck names no vocabulary effect. */
export const NO_EFFECT = '—';

/** A motion row's place: a stage or a transition frame. */
export type MotionRowKind = 'stage' | 'transition';

export function rowKind(id: string): MotionRowKind | null {
  if (/#s\d+$/.test(id)) {
    return 'stage';
  }
  return /#t@\d+$/.test(id) ? 'transition' : null;
}

/** The slide's static row id: the row id without its step. */
export function slideOf(id: string): string {
  return id.replace(/#.*$/, '');
}

export function stageOf(id: string): number | null {
  const match = /#s(\d+)$/.exec(id);
  return match ? Number(match[1]) : null;
}

/** The steps of a stage (click group). */
export function stageSteps(fidelity: TimelineFidelity, stage: number | null): StepPlay[] {
  return (fidelity.plays ?? []).filter((play) => stage === null || play.group === stage);
}

/** The effects a row shows: its stage's steps' (their effect, or `—` where none), or its transition's. */
export function rowEffects(results: Partial<Pick<Results, 'timelines'>>, id: string): string[] {
  const fidelity = results.timelines?.[slideOf(id)];
  const kind = rowKind(id);
  if (!fidelity || kind === null) {
    return [];
  }
  if (kind === 'transition') {
    return fidelity.transition ? [fidelity.transition.name ?? NO_EFFECT] : [];
  }
  return [...new Set(stageSteps(fidelity, stageOf(id)).map((play) => play.effect ?? NO_EFFECT))];
}

/** Reference and known-loss rows, which `slidock-bench list` and acceptance leave out unless asked. */
export function flagged(record: RowRecord): boolean {
  return record.reference === true || (record.knownLoss?.length ?? 0) > 0;
}

/** The codes a row matches, as `slidock-bench list --code` reads them: its decline, its own code, its diagnostics. */
export function codesOf(record: RowRecord): string[] {
  return [
    ...new Set([
      ...(record.declined ? [record.declined.code] : []),
      ...(record.code ? [record.code] : []),
      ...Object.keys(record.diagnostics),
    ]),
  ];
}

export function isStepRow(record: RowRecord): boolean {
  return record.stage !== undefined || record.transitionPm !== undefined;
}

export type Facet = 'fe' | 'oc' | 'deck' | 'code' | 'feat' | 'step';

function keeps(row: Row, filters: Filters, except?: Facet): boolean {
  const { record } = row;
  if (!filters.ref && flagged(record)) {
    return false;
  }
  if (except !== 'fe' && filters.fe.length > 0 && !filters.fe.includes(record.frontend)) {
    return false;
  }
  if (except !== 'oc' && filters.oc.length > 0 && !filters.oc.includes(record.outcome)) {
    return false;
  }
  if (except !== 'deck' && filters.deck !== null && record.presentation !== filters.deck) {
    return false;
  }
  if (except !== 'code' && filters.code.length > 0) {
    const codes = codesOf(record);
    if (!filters.code.every((code) => codes.includes(code))) {
      return false;
    }
  }
  if (except !== 'feat' && filters.feat.length > 0 && !filters.feat.includes(record.declined?.feature ?? '')) {
    return false;
  }
  if (except !== 'step' && filters.step !== 'all' && isStepRow(record) !== (filters.step === 'steps')) {
    return false;
  }
  if (filters.q !== '' && !row.id.toLowerCase().includes(filters.q.toLowerCase())) {
    return false;
  }
  return filters.fx === null || (row.effects ?? []).includes(filters.fx);
}

export function filterRows(rows: readonly Row[], filters: Filters): Row[] {
  return rows.filter((row) => keeps(row, filters));
}

export type FacetCounts = Record<Facet, Map<string, number>>;

/**
 * Per filter, each option's row count: what choosing the option leaves. An any-of filter (frontend, outcome, feature)
 * or a single choice (deck, step) counts its options under all the other filters; the all-of code filter counts them
 * under every filter, its own included, since choosing a code narrows the rows it already keeps.
 */
export function facetCounts(rows: readonly Row[], filters: Filters): FacetCounts {
  const counts: FacetCounts = {
    fe: new Map(),
    oc: new Map(),
    deck: new Map(),
    code: new Map(),
    feat: new Map(),
    step: new Map(),
  };
  const add = (facet: Facet, key: string) => counts[facet].set(key, (counts[facet].get(key) ?? 0) + 1);
  for (const row of rows) {
    const { record } = row;
    if (keeps(row, filters, 'fe')) {
      add('fe', record.frontend);
    }
    if (keeps(row, filters, 'oc')) {
      add('oc', record.outcome);
    }
    if (keeps(row, filters, 'deck')) {
      add('deck', record.presentation);
    }
    if (keeps(row, filters)) {
      for (const code of codesOf(record)) {
        add('code', code);
      }
    }
    if (keeps(row, filters, 'feat') && record.declined?.feature !== undefined) {
      add('feat', record.declined.feature);
    }
    if (keeps(row, filters, 'step')) {
      add('step', isStepRow(record) ? 'steps' : 'slide');
    }
  }
  return counts;
}

/** UTF-16 code-unit order, which `results.json` and `slidock-bench list` use. */
export const compareIds = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);

const SORT_VALUE: Record<Exclude<SortKey, 'id'>, (record: RowRecord) => number | null | undefined> = {
  outcome: (record) => outcomeRank(record.outcome),
  ssim: (record) => record.scores?.ssim.mean,
  p1: (record) => record.scores?.ssim.p1,
  de: (record) => record.scores?.color.mean,
  recall: (record) => record.scores?.text.recall,
  p95: (record) => record.scores?.text.p95Pm,
};

/** The rows in `sort` order; rows without the value come last in either direction, and ties keep id order. */
export function sortRows(rows: readonly Row[], sort: SortKey, dir: SortDir): Row[] {
  const sign = dir === 'asc' ? 1 : -1;
  if (sort === 'id') {
    return [...rows].sort((a, b) => sign * compareIds(a.id, b.id));
  }
  const value = SORT_VALUE[sort];
  return [...rows].sort((a, b) => {
    const left = value(a.record);
    const right = value(b.record);
    const leftMissing = left === null || left === undefined;
    const rightMissing = right === null || right === undefined;
    if (leftMissing !== rightMissing) {
      return leftMissing ? 1 : -1;
    }
    return sign * ((left ?? 0) - (right ?? 0)) || compareIds(a.id, b.id);
  });
}

/**
 * A run's folder in the bench cache, quoted for a POSIX shell, for the commands the UI prints. The page does not know
 * where the cache is, so the shell resolves it: `SLIDOCK_BENCH_CACHE`, or its default `bench/.cache`.
 */
export const runDirPath = (run: string, file = '') =>
  `"\${SLIDOCK_BENCH_CACHE:-bench/.cache}/runs/${run}${file === '' ? '' : `/${file}`}"`;

/** A run's results file, as `runDirPath` names it. */
export const runResultsPath = (run: string) => runDirPath(run, 'results.json');

/**
 * The `slidock-bench list` command that prints the same rows, or null when a filter has no `list` option (several
 * frontends, outcomes or codes, a deck, a feature, a search, a step mode or an effect). `results` is the results file it reads; null reads
 * the committed baseline.
 */
export function listCommand(filters: Filters, results: string | null): string | null {
  if (filters.fe.length > 1 || filters.oc.length > 1 || filters.code.length > 1) {
    return null;
  }
  if (
    filters.deck !== null ||
    filters.feat.length > 0 ||
    filters.q !== '' ||
    filters.step !== 'all' ||
    filters.fx !== null
  ) {
    return null;
  }
  return [
    'slidock-bench list',
    ...(results === null ? [] : [`--results ${results}`]),
    ...(filters.oc[0] !== undefined ? [`--class ${filters.oc[0]}`] : []),
    ...(filters.code[0] !== undefined ? [`--code ${filters.code[0]}`] : []),
    ...(filters.fe[0] !== undefined ? [`--frontend ${filters.fe[0]}`] : []),
    ...(filters.ref ? ['--all'] : []),
  ].join(' ');
}

/** The rows' ids in display order, for previous/next navigation from the drawer. */
export function neighbours(order: readonly RowId[], current: RowId): { previous: RowId | null; next: RowId | null } {
  const index = order.indexOf(current);
  if (index < 0) {
    return { previous: null, next: null };
  }
  return { previous: order[index - 1] ?? null, next: order[index + 1] ?? null };
}
