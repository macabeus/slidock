// Imported by the build's sample report too, so its imports name their files.
import type { ImageKind, ParityView, Results, RowDetail, RowId, RunsIndex, Status } from '../schema/index.ts';
import type { BenchSource } from './source.ts';

export const REPORT_SCHEMA = 'slidock-bench-report@1';

/**
 * What `slidock-bench report` inlines into the single-file report: the runs index and the results of the runs it
 * covers, with whatever row details, parity views and images it chose to carry. Keys of `rows` are
 * `<run> <row>`, keys of `images` are `<run> <row> <kind>`; an image value is any URL an `<img>` loads (a `data:`
 * URI, or a path relative to the report file).
 */
export interface ReportPayload {
  schema: typeof REPORT_SCHEMA;
  status: Status | null;
  runs: RunsIndex;
  results: { [run: string]: Results };
  parity: { [run: string]: ParityView };
  rows: { [key: string]: RowDetail };
  images: { [key: string]: string };
  /**
   * A published report: the classes whose rows carry images (`slidock-bench report --public`). Its one run is named
   * `public`, and its payload leaves out every run key, hash and cache key the types above name (the stamp's, the
   * entry's, the records' and the details'), the stamp's corpus and font set, the details' source, run and commands,
   * and the baseline mark (`crates/slidock-bench/src/public.rs`); the views read none of them in a published report.
   */
  public?: { images: string[] };
}

export const rowKey = (run: string, row: RowId) => `${run} ${row}`;
export const imageKey = (run: string, row: RowId, kind: ImageKind) => `${run} ${row} ${kind}`;

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Checks the payload's envelope (the schema name and the shape of each section); its contents are trusted. */
export function parseReportPayload(value: unknown): ReportPayload {
  if (!isObject(value) || value.schema !== REPORT_SCHEMA) {
    throw new Error(`the report data is not ${REPORT_SCHEMA}`);
  }
  const runs = value.runs;
  if (!isObject(runs) || !Array.isArray(runs.runs)) {
    throw new Error('the report data has no runs index');
  }
  for (const section of ['results', 'parity', 'rows', 'images'] as const) {
    if (!isObject(value[section])) {
      throw new Error(`the report data has no ${section} section`);
    }
  }
  if (
    value.public !== undefined &&
    !(
      isObject(value.public) &&
      Array.isArray(value.public.images) &&
      value.public.images.every((c) => typeof c === 'string')
    )
  ) {
    throw new Error('the report data has a malformed public section');
  }
  if (value.status !== null && !isObject(value.status)) {
    throw new Error('the report data has a malformed status');
  }
  return value as unknown as ReportPayload;
}

/** Reads a report's inlined data. */
export function reportSource(payload: ReportPayload): BenchSource {
  return {
    kind: 'report',
    scope: { parity: Object.keys(payload.parity).length > 0, public: payload.public ?? null },
    status: () => Promise.resolve(payload.status),
    runs: () => Promise.resolve(payload.runs),
    results: (run) => Promise.resolve(payload.results[run] ?? null),
    row: (run, row) => Promise.resolve(payload.rows[rowKey(run, row)] ?? null),
    parity: (run) => Promise.resolve(payload.parity[run] ?? null),
    imageUrl: (run, row, kind) => payload.images[imageKey(run, row, kind)] ?? null,
  };
}
