// The bench UI's state, all of it in the URL fragment, so every view is a link. Keys are
// short and a value equal to its default is left out. Changing the tab, the run, the base run or opening a row
// pushes a history entry; filters, sort, the image mode and closing the drawer replace the current one.
import { type HistoryMode, writeHash } from '@slidock/ui';

export const TABS = ['overview', 'explorer', 'compare', 'parity', 'fonts', 'effects', 'methodology'] as const;
export type Tab = (typeof TABS)[number];

export const SORT_KEYS = ['id', 'outcome', 'ssim', 'p1', 'de', 'recall', 'p95'] as const;
export type SortKey = (typeof SORT_KEYS)[number];

export type SortDir = 'asc' | 'desc';

/** The row drawer's image comparison. */
export const IMAGE_MODES = ['side', 'slider', 'onion', 'difference', 'heatmap'] as const;
export type ImageMode = (typeof IMAGE_MODES)[number];

/** Which rows the explorer lists: slides only, motion steps only, or both. */
export const STEP_MODES = ['all', 'slide', 'steps'] as const;
export type StepMode = (typeof STEP_MODES)[number];

/** The explorer's filters. Values the UI does not know are kept: they match no row, so they show nothing. */
export interface Filters {
  /** Frontends (any of). */
  fe: string[];
  /** Outcomes (any of). */
  oc: string[];
  /** A presentation. */
  deck: string | null;
  /** Diagnostic codes (all of): a row matches a code through its decline, its own code or a diagnostic. */
  code: string[];
  /** The features of the deciding decline (any of). */
  feat: string[];
  /** A substring of the row id. */
  q: string;
  /** A motion row's effect (the vocabulary name its stage's steps or its transition play; `—` for none). */
  fx: string | null;
  /** Include reference and known-loss rows (`slidock-bench list --all`). */
  ref: boolean;
  step: StepMode;
}

export interface BenchState extends Filters {
  tab: Tab;
  /** A run key, `latest` or `baseline`. */
  run: string;
  /** The run compared against: a run key, `latest` or `baseline`; null for none. */
  base: string | null;
  sort: SortKey;
  dir: SortDir;
  /** The row open in the drawer. */
  row: string | null;
  img: ImageMode;
  /** Parity statuses (any of), on the parity tab. */
  ps: string[];
}

export const NO_FILTERS: Filters = {
  fe: [],
  oc: [],
  deck: null,
  code: [],
  feat: [],
  q: '',
  fx: null,
  ref: false,
  step: 'all',
};

export const DEFAULT_STATE: BenchState = {
  ...NO_FILTERS,
  tab: 'overview',
  run: 'latest',
  base: null,
  sort: 'id',
  dir: 'asc',
  row: null,
  img: 'side',
  ps: [],
};

function oneOf<T extends string>(values: readonly T[], raw: string | null, fallback: T): T {
  return raw !== null && (values as readonly string[]).includes(raw) ? (raw as T) : fallback;
}

function list(raw: string | null): string[] {
  if (raw === null || raw === '') {
    return [];
  }
  return [...new Set(raw.split(',').filter((value) => value !== ''))];
}

function text(raw: string | null): string | null {
  return raw === null || raw === '' ? null : raw;
}

/** The state a fragment names; malformed values read as their defaults. */
export function readState(params: URLSearchParams): BenchState {
  return {
    tab: oneOf(TABS, params.get('tab'), DEFAULT_STATE.tab),
    run: text(params.get('run')) ?? DEFAULT_STATE.run,
    base: text(params.get('base')),
    fe: list(params.get('fe')),
    oc: list(params.get('oc')),
    deck: text(params.get('deck')),
    code: list(params.get('code')),
    feat: list(params.get('feat')),
    q: params.get('q') ?? '',
    fx: text(params.get('fx')),
    ref: params.get('ref') === '1',
    step: oneOf(STEP_MODES, params.get('step'), DEFAULT_STATE.step),
    sort: oneOf(SORT_KEYS, params.get('sort'), DEFAULT_STATE.sort),
    dir: params.get('dir') === 'desc' ? 'desc' : 'asc',
    row: text(params.get('row')),
    img: oneOf(IMAGE_MODES, params.get('img'), DEFAULT_STATE.img),
    ps: list(params.get('ps')),
  };
}

type Updates = Record<string, string | null>;

function encode<K extends keyof BenchState>(key: K, value: BenchState[K]): string | null {
  const fallback = DEFAULT_STATE[key];
  if (Array.isArray(value)) {
    return value.length === 0 ? null : value.join(',');
  }
  if (typeof value === 'boolean') {
    return value ? '1' : null;
  }
  if (value === null || value === fallback || value === '') {
    return null;
  }
  return String(value);
}

/** The fragment keys a change sets (`null` clears a key back to its default). */
export function stateUpdates(change: Partial<BenchState>): Updates {
  const updates: Updates = {};
  for (const key of Object.keys(change) as (keyof BenchState)[]) {
    const value = change[key];
    if (value !== undefined) {
      updates[key] = encode(key, value);
    }
  }
  return updates;
}

/** Whether a change starts a new history entry: a new tab, run or base, or a row opened. */
export function historyFor(current: BenchState, change: Partial<BenchState>): HistoryMode {
  const moves =
    (change.tab !== undefined && change.tab !== current.tab) ||
    (change.run !== undefined && change.run !== current.run) ||
    (change.base !== undefined && change.base !== current.base) ||
    (change.row !== undefined && change.row !== null && change.row !== current.row);
  return moves ? 'push' : 'replace';
}

/** Applies a change to the fragment, pushing or replacing by `historyFor`. */
export function navigate(current: BenchState, change: Partial<BenchState>) {
  writeHash(stateUpdates(change), historyFor(current, change));
}

/** The explorer opened on exactly these filters (an overview preset): every other filter and the drawer cleared. */
export function explore(preset: Partial<Filters>): Partial<BenchState> {
  return { ...NO_FILTERS, ...preset, tab: 'explorer', row: null };
}
