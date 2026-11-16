// The bench the dev mock and the sample report serve: the schema crate's synthetic files
// (`crates/slidock-bench-schema/tests/data`, made-up decks with no corpus content), indexed by run key the way
// `slidock-bench ui` serves a cache, plus the row details and images a real cache would hold.
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import { CLASSIFIER_THRESHOLDS } from '../src/model/thresholds.ts';
import type {
  AllowEntry,
  AppliedDiagnostic,
  CorpusManifest,
  ElementFinding,
  Explain,
  ImageKind,
  ImageRef,
  ParityReport,
  ParityView,
  Results,
  RowDetail,
  RowRecord,
  RunEntry,
  RunsIndex,
  Stamp,
  Status,
} from '../src/schema/index.ts';
import { type FramePhase, type SyntheticKind, syntheticFrame, syntheticImage } from './png.ts';

export const DATA_DIR = `${resolve(import.meta.dirname, '../../../crates/slidock-bench-schema/tests/data')}/`;

function read<T>(name: string, dir: string): T {
  return JSON.parse(readFileSync(`${dir}${name}`, 'utf8')) as T;
}

function sortKeys(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(sortKeys);
  }
  if (typeof value === 'object' && value !== null) {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
        .map(([key, inner]) => [key, sortKeys(inner)]),
    );
  }
  return value;
}

const sha256 = (text: string) => createHash('sha256').update(text).digest('hex');

/** `sha256Json`: the SHA-256 of the compact canonical JSON (keys in code-unit order). */
export const sha256Json = (value: unknown) => sha256(JSON.stringify(sortKeys(value)));

/** `Stamp::run_key`: the stamp without `partial` and what is stamped after the run. */
export function runKey(stamp: Stamp): string {
  const { partial: _partial, determinism: _determinism, parity: _parity, ...keyed } = stamp;
  return sha256Json(keyed);
}

export interface Fixture {
  manifest: CorpusManifest;
  runs: RunsIndex;
  results: Map<string, Results>;
  parity: Map<string, ParityView>;
  status: Status;
  /** The keys the aliases name. */
  latest: string;
  baseline: string;
}

/** Loads the synthetic bench. Throws when the files' run keys do not line up with the runs index. */
export function loadFixture(dir: string = DATA_DIR): Fixture {
  const manifest = read<CorpusManifest>('manifest.json', dir);
  const index = read<RunsIndex>('runs.json', dir);
  const results = new Map<string, Results>();
  for (const name of ['results', 'baseline-results', 'motion-results', 'host-results', 'fonts-results']) {
    const file = read<Results>(`${name}.json`, dir);
    results.set(runKey(file.stamp), file);
  }
  const listed = new Set(index.runs.map((run) => run.key));
  const latest = runKey(read<Results>('results.json', dir).stamp);
  const baseline = runKey(read<Results>('baseline-results.json', dir).stamp);
  for (const [name, key] of [
    ['results.json', latest],
    ['baseline-results.json', baseline],
  ] as const) {
    if (!listed.has(key)) {
      throw new Error(`${name}'s run key ${key} is not in runs.json`);
    }
  }
  // The host and fonts files are runs the index does not list: add them after its newest, as a cache would.
  const extra: RunEntry[] = [];
  let minute = 30;
  for (const file of results.values()) {
    const key = runKey(file.stamp);
    if (listed.has(key)) {
      continue;
    }
    extra.push({
      key,
      stamp: sha256Json(file.stamp),
      lane: file.stamp.lane,
      compiler: file.stamp.compiler,
      harness: file.stamp.harness,
      partial: file.stamp.partial,
      recordedAt: `2026-10-30T10:${String(minute).padStart(2, '0')}:00Z`,
      summary: file.summary,
    });
    minute += 15;
  }
  const runs: RunsIndex = { ...index, runs: [...index.runs, ...extra] };
  const report = read<ParityReport>('parity.json', dir);
  const allowlist = read<AllowEntry[]>('allow.json', dir);
  const parity = new Map<string, ParityView>([[report.run, { report, allowlist }]]);
  const head = results.get(latest);
  if (!head) {
    throw new Error('results.json is missing');
  }
  const thresholds = CLASSIFIER_THRESHOLDS[head.stamp.classifier];
  if (!thresholds) {
    throw new Error(`no thresholds for ${head.stamp.classifier}`);
  }
  const status: Status = {
    head: { commit: head.stamp.compiler.commit, content: head.stamp.compiler.content, harness: head.stamp.harness },
    baselines: ['static'],
    latest: { partial: head.stamp.partial },
    holdout: false,
    parity: true,
    thresholds,
  };
  return { manifest, runs, results, parity, status, latest, baseline };
}

/** A run reference (`latest`, `baseline` or a key) to the key. */
export function resolveKey(fixture: Fixture, run: string): string {
  return run === 'latest' ? fixture.latest : run === 'baseline' ? fixture.baseline : run;
}

const IMAGE_KINDS: readonly ImageKind[] = ['oracle', 'render', 'heatmap', 'golden', 'golden-diff'];

/** The images a row has: an oracle page always, a render and heatmap when it rendered, the golden's when parity drifted. */
export function rowImages(fixture: Fixture, run: string, id: string): ImageKind[] {
  const record = fixture.results.get(run)?.rows[id];
  if (!record) {
    return [];
  }
  const kinds: ImageKind[] = ['oracle'];
  if (record.render !== undefined) {
    kinds.push('render', 'heatmap');
    const status = fixture.parity.get(run)?.report.compiler?.rows[id]?.status;
    if (status !== undefined && status !== 'identical' && status !== 'pixel-identical') {
      kinds.push('golden', 'golden-diff');
    }
  }
  return kinds;
}

const FRAME_PHASES: readonly FramePhase[] = ['start', 'mid', 'end', 'still'];

/** The PNG of a row's image, or null when the row has none of that kind. */
export function rowImage(fixture: Fixture, run: string, id: string, kind: string): Buffer | null {
  const record = fixture.results.get(run)?.rows[id];
  if (record?.motion?.frames && (FRAME_PHASES as readonly string[]).includes(kind)) {
    if (kind === 'still' && !record.motion.endState) {
      return null;
    }
    return syntheticFrame(`${record.presentation}:${record.slide}`, record.outcome, kind as FramePhase);
  }
  if (!record || !(IMAGE_KINDS as readonly string[]).includes(kind)) {
    return null;
  }
  if (!rowImages(fixture, run, id).includes(kind as ImageKind)) {
    return null;
  }
  return syntheticImage(`${record.presentation}:${record.slide}`, record.outcome, kind as SyntheticKind);
}

const MESSAGES: Record<string, string> = {
  IR: 'A slide element failed to lower and is left out',
  MID: 'A text frame uses a line spacing rule the HTML backend approximates',
  PX: 'A SmartArt graphic shows as its fallback picture',
};

function diagnostics(record: RowRecord): AppliedDiagnostic[] {
  return Object.entries(record.diagnostics).map(([code, count], index) => {
    const family = /^[A-Z]+/.exec(code)?.[0] ?? '';
    return {
      code,
      severity: family === 'IR' ? 'error' : 'warning',
      message: MESSAGES[family] ?? 'A synthetic diagnostic',
      ...(count > 1 ? { count } : {}),
      ...(index % 2 === 1 ? { via: 'layout-1' } : {}),
    };
  });
}

function explain(run: string, record: RowRecord): Explain | null {
  const scores = record.scores;
  if (!scores || record.outcome === 'match') {
    return null;
  }
  const missing = scores.text.oracleWords - scores.text.matched;
  const elements: ElementFinding[] = [
    {
      el: 'e2',
      kind: 'text',
      name: 'Body',
      origin: 'slide',
      words: scores.text.oracleWords,
      moved: { dxPm: 2.4, dyPm: -1.1 },
      resized: { heightRatio: scores.text.heightRatioMedian ?? 1 },
      fonts: [],
      missing,
      extra: Math.max(0, scores.text.domWords - scores.text.matched - missing),
    },
    {
      el: 'e5',
      kind: 'image',
      origin: 'layout',
      words: 0,
      fonts: [],
      image: { ssim: scores.ssim.p1, deltaE: scores.color.p99 },
      missing: 0,
      extra: 0,
    },
  ];
  return {
    run,
    matchesRow: true,
    elements,
    missing: missing > 0 ? [{ text: 'A line of body text', xPm: 120, yPm: 410, el: 'e2' }] : [],
  };
}

/** The row's detail, as `GET /api/runs/<run>/rows/<row>` answers it. */
export function rowDetail(fixture: Fixture, run: string, id: string): RowDetail | null {
  const results = fixture.results.get(run);
  const record = results?.rows[id];
  if (!results || !record) {
    return null;
  }
  const presentation = fixture.manifest.presentations.find((p) => p.id === record.presentation);
  const source = presentation?.sources.find((s) => s.frontend === record.frontend);
  const losses = presentation?.knownLoss?.[record.frontend] ?? [];
  const kinds = rowImages(fixture, run, id);
  const images: { [kind in ImageKind]?: ImageRef } = {};
  for (const kind of IMAGE_KINDS) {
    if (kinds.includes(kind)) {
      images[kind] = {
        url: `api/png/${run}/${encodeURIComponent(id)}/${kind}`,
        from: kind === 'oracle' ? 'oracle-cache' : kind.startsWith('golden') ? 'golden-reference' : 'run',
      };
    } else if (kind === 'render' || kind === 'heatmap') {
      images[kind] = { missing: `slidock-bench run --only ${id}` };
    }
  }
  return {
    id,
    source: run === fixture.baseline ? 'baseline' : 'latest',
    run: { key: run, state: 'present' },
    record,
    knownLoss: losses.filter((loss) => record.knownLoss?.includes(loss.kind)),
    diagnostics: diagnostics(record),
    images,
    explain: explain(run, record),
    commands: {
      explain: `slidock-bench explain ${id} --from bench/.cache/runs/${run}`,
      run: `slidock-bench run --only ${id}`,
    },
    viewer: { available: false, file: source?.file ?? '' },
  };
}
