// The fonts lane's figures for the Fonts tab: which run it reads, per deck and per family the glyphs by how they
// were drawn, and the rows. Every figure comes from a fonts-lane results file (`fonts@1`).
import type { FontSource, FontsPairReport, GlyphCounts, Results, RowRecord, RunEntry, RunsIndex } from '../schema';

/** Where a glyph's face came from, in resolution order, then the renderer's own fallback. */
export const GLYPH_SOURCES = [
  'embedded',
  'google',
  'local',
  'dropped',
  'substitute',
  'fallback',
] as const satisfies readonly (FontSource | 'fallback')[];
export type GlyphSource = (typeof GLYPH_SOURCES)[number];

export const GLYPH_SOURCE_LABELS: Record<GlyphSource, string> = {
  embedded: 'embedded',
  google: 'Google Fonts',
  local: 'installed',
  dropped: 'dropped',
  substitute: 'substitute',
  fallback: 'renderer fallback',
};

/**
 * Where a fonts run rendered: the renderer image with only Unicode's Last Resort face, or the host's Chromium
 * (`--renderer native`, stamped `native:<os>-<arch>`), whose installed fonts draw what the deck does not ship.
 */
export type FontsRenderer = 'container' | 'native';

export function fontsRenderer(results: Results): FontsRenderer {
  return results.stamp.renderer.platformDigest.startsWith('native:') ? 'native' : 'container';
}

/** How the run's glyphs were attributed, for the attribution panel. */
export const FONTS_RENDERER_NOTES: Record<FontsRenderer, string> = {
  container:
    "CSS.getPlatformFontsForNode in the renderer image, whose only font is Unicode's Last Resort face: every fallback glyph is one no shipped face draws.",
  native:
    "CSS.getPlatformFontsForNode in this computer's Chromium (--renderer native), not the no-fonts container: a glyph no shipped face draws still counts as fallback, but the fallback fonts named are this computer's.",
};

/** The rows the static-lane equivalence gate compared, or null when the run computed none. */
export function equivalenceCount(results: Results): number | null {
  const compared = Object.values(results.rows).filter((row) => row?.fonts?.equivalence !== undefined).length;
  return compared > 0 ? compared : null;
}

/** The run the tab shows: the selected run when it is a fonts run, else the newest fonts run. */
export function fontsRun(index: RunsIndex, selected: RunEntry): RunEntry | null {
  if (selected.lane === 'fonts') {
    return selected;
  }
  return index.runs.findLast((run) => run.lane === 'fonts') ?? null;
}

export function emptyCounts(): GlyphCounts {
  return { glyphs: 0, exact: 0, substitute: 0, fallback: 0, coverageGaps: 0, sources: {} };
}

/** Adds `other` into `into`. */
export function addCounts(into: GlyphCounts, other: GlyphCounts): GlyphCounts {
  into.glyphs += other.glyphs;
  into.exact += other.exact;
  into.substitute += other.substitute;
  into.fallback += other.fallback;
  into.coverageGaps += other.coverageGaps;
  for (const [source, glyphs] of Object.entries(other.sources)) {
    const key = source as FontSource;
    into.sources[key] = (into.sources[key] ?? 0) + (glyphs ?? 0);
  }
  return into;
}

/** `part / whole`, null over nothing. */
export function share(part: number, whole: number): number | null {
  return whole > 0 ? part / whole : null;
}

/** Glyphs per source, fallback last: the segments of a source-mix bar. */
export function sourceSegments(counts: GlyphCounts): { source: GlyphSource; count: number }[] {
  return GLYPH_SOURCES.map((source) => ({
    source,
    count: source === 'fallback' ? counts.fallback : (counts.sources[source] ?? 0),
  }));
}

export interface PairFigures {
  pair: string;
  rows: number;
  complete: number;
  failed: number;
  counts: GlyphCounts;
  report: FontsPairReport | null;
}

const pairOf = (row: RowRecord) => `${row.presentation}:${row.frontend}`;

/** Per deck (pair): its rows by class, their glyphs, and what it shipped; in pair order. */
export function pairFigures(results: Results): PairFigures[] {
  const pairs = new Map<string, PairFigures>();
  const entry = (pair: string): PairFigures => {
    let figures = pairs.get(pair);
    if (!figures) {
      figures = {
        pair,
        rows: 0,
        complete: 0,
        failed: 0,
        counts: emptyCounts(),
        report: results.fontPairs?.[pair] ?? null,
      };
      pairs.set(pair, figures);
    }
    return figures;
  };
  for (const pair of Object.keys(results.fontPairs ?? {})) {
    entry(pair);
  }
  for (const row of Object.values(results.rows)) {
    if (!row) {
      continue;
    }
    const figures = entry(pairOf(row));
    figures.rows += 1;
    if (row.outcome === 'fonts-complete') {
      figures.complete += 1;
    }
    if (row.fonts) {
      addCounts(figures.counts, row.fonts.total);
    } else {
      figures.failed += 1;
    }
  }
  return [...pairs.values()].sort((a, b) => (a.pair < b.pair ? -1 : a.pair > b.pair ? 1 : 0));
}

/** Every row's glyphs summed, and the shipped faces and bytes of every pair. */
export function laneTotals(pairs: readonly PairFigures[]) {
  const counts = emptyCounts();
  let rows = 0;
  let complete = 0;
  let faces = 0;
  let fontBytes = 0;
  let unresolved = 0;
  for (const pair of pairs) {
    addCounts(counts, pair.counts);
    rows += pair.rows;
    complete += pair.complete;
    faces += pair.report?.faces.length ?? 0;
    fontBytes += pair.report?.fontBytes ?? 0;
    unresolved += pair.report?.requested.filter((face) => face.source === null).length ?? 0;
  }
  return { counts, rows, complete, faces, fontBytes, unresolved };
}

/** The families of one pair (or of every pair), most glyphs first, ties by name. */
export function familyFigures(pairs: readonly PairFigures[], pair: string | null) {
  const families = new Map<string, GlyphCounts>();
  for (const figures of pairs) {
    if (pair !== null && figures.pair !== pair) {
      continue;
    }
    for (const [family, counts] of Object.entries(figures.report?.families ?? {})) {
      if (counts) {
        addCounts(families.get(family) ?? families.set(family, emptyCounts()).get(family)!, counts);
      }
    }
  }
  return [...families.entries()]
    .map(([family, counts]) => ({ family, counts }))
    .sort((a, b) => b.counts.glyphs - a.counts.glyphs || (a.family < b.family ? -1 : a.family > b.family ? 1 : 0));
}

/** The rows the tab lists: one pair's, or every row the renderer's fonts drew in or that failed; worst first. */
export function fontRows(results: Results, pair: string | null): [string, RowRecord][] {
  const rank = (row: RowRecord) => (row.fonts ? -row.fonts.total.fallback - row.fonts.total.coverageGaps / 1e6 : -1e9);
  return Object.entries(results.rows)
    .filter((entry): entry is [string, RowRecord] => entry[1] !== undefined)
    .filter(([, row]) => (pair === null ? row.outcome !== 'fonts-complete' : pairOf(row) === pair))
    .sort(([a, x], [b, y]) => rank(x) - rank(y) || (a < b ? -1 : a > b ? 1 : 0));
}

/** Bytes as `12.3 KB` (1024-based, one decimal), small counts as bytes. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  const units = ['KB', 'MB', 'GB'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}
