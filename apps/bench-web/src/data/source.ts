// Imported by the build's dev mock too, so its imports name their files.
import type { ImageKind, ParityView, Results, RowDetail, RowId, RunsIndex, Status } from '../schema/index.ts';

/**
 * What a report holds beyond its runs. A view whose data a report does not hold is left out; the API serves whatever
 * the bench holds, and has no scope.
 */
export interface ReportScope {
  /** Some run carries a parity view. */
  parity: boolean;
  /**
   * A published report (`slidock-bench report --public`): the presentations cleared for publication only, with the
   * images of rows of these classes; its readers have no bench, so the views show no command.
   */
  public: { images: string[] } | null;
}

/**
 * Where the UI reads a bench from: `slidock-bench ui`'s read-only API, or the data inlined in a single-file report.
 * Runs are named by their key (aliases are resolved against the runs index first). `null` means the bench holds no
 * such thing (the API's 404); a failed read rejects.
 */
export interface BenchSource {
  readonly kind: 'api' | 'report';
  readonly scope?: ReportScope;
  /** HEAD's ids and the classifier thresholds; null when the source does not know HEAD (a report). */
  status(): Promise<Status | null>;
  runs(): Promise<RunsIndex>;
  results(run: string): Promise<Results | null>;
  /** The row's images, diagnostics and findings; null when only its record is known. */
  row(run: string, row: RowId): Promise<RowDetail | null>;
  parity(run: string): Promise<ParityView | null>;
  /** Where a row's image is served, or null when the source cannot hold it. */
  imageUrl(run: string, row: RowId, kind: ImageKind): string | null;
}

/** The API's paths, relative to the page so the UI also works under a path prefix. */
export const API = {
  status: () => 'api/status',
  runs: () => 'api/runs',
  results: (run: string) => `api/runs/${encodeURIComponent(run)}/results`,
  row: (run: string, row: RowId) => `api/runs/${encodeURIComponent(run)}/rows/${encodeURIComponent(row)}`,
  parity: (run: string) => `api/runs/${encodeURIComponent(run)}/parity`,
  png: (run: string, row: RowId, kind: ImageKind) =>
    `api/png/${encodeURIComponent(run)}/${encodeURIComponent(row)}/${kind}`,
} as const;

export class ApiError extends Error {
  readonly status: number;
  readonly path: string;

  constructor(path: string, status: number) {
    super(`GET ${path} answered ${status}`);
    this.name = 'ApiError';
    this.path = path;
    this.status = status;
  }
}

type Fetch = (input: string, init?: RequestInit) => Promise<Response>;

/** Reads `slidock-bench ui`'s API. */
export function apiSource(fetchImpl: Fetch = (input, init) => fetch(input, init)): BenchSource {
  async function get<T>(path: string, optional: true): Promise<T | null>;
  async function get<T>(path: string, optional: false): Promise<T>;
  async function get<T>(path: string, optional: boolean): Promise<T | null> {
    const response = await fetchImpl(path, { headers: { accept: 'application/json' } });
    if (response.status === 404 && optional) {
      return null;
    }
    if (!response.ok) {
      throw new ApiError(path, response.status);
    }
    return (await response.json()) as T;
  }
  return {
    kind: 'api',
    status: () => get<Status>(API.status(), true),
    runs: () => get<RunsIndex>(API.runs(), false),
    results: (run) => get<Results>(API.results(run), true),
    row: (run, row) => get<RowDetail>(API.row(run, row), true),
    parity: (run) => get<ParityView>(API.parity(run), true),
    imageUrl: (run, row, kind) => API.png(run, row, kind),
  };
}
