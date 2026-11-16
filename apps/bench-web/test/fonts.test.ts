import { describe, expect, it } from 'vitest';

import {
  addCounts,
  emptyCounts,
  equivalenceCount,
  familyFigures,
  fontRows,
  fontsRenderer,
  fontsRun,
  formatBytes,
  laneTotals,
  pairFigures,
  sourceSegments,
} from '../src/model/fonts';
import type { Results } from '../src/schema';
import { fixture } from './fixture';

function fontsResults(): Results {
  const run = fixture.runs.runs.findLast((entry) => entry.lane === 'fonts');
  const results = run ? fixture.results.get(run.key) : undefined;
  if (!results) {
    throw new Error('no fonts results in the fixture');
  }
  return results;
}

describe('the fonts lane model', () => {
  it('picks the selected fonts run, else the newest one', () => {
    const fonts = fixture.runs.runs.findLast((entry) => entry.lane === 'fonts');
    const other = fixture.runs.runs.find((entry) => entry.lane === 'static');
    expect(fonts && other && fontsRun(fixture.runs, other)?.key).toBe(fonts?.key);
    expect(fonts && fontsRun(fixture.runs, fonts)).toBe(fonts);
    expect(fontsRun({ ...fixture.runs, runs: fixture.runs.runs.filter((r) => r.lane !== 'fonts') }, other!)).toBeNull();
  });

  it('sums each deck and the lane from the rows, and the faces from the pair reports', () => {
    const results = fontsResults();
    const pairs = pairFigures(results);
    expect(pairs.map((pair) => pair.pair)).toEqual(['demo-basics:odp', 'demo-basics:pptx']);
    for (const pair of pairs) {
      const rows = Object.values(results.rows).filter((row) => `${row?.presentation}:${row?.frontend}` === pair.pair);
      expect(pair.rows).toBe(rows.length);
      expect(pair.counts.glyphs).toBe(rows.reduce((sum, row) => sum + (row?.fonts?.total.glyphs ?? 0), 0));
      expect(pair.counts.exact + pair.counts.substitute + pair.counts.fallback).toBe(pair.counts.glyphs);
    }
    const totals = laneTotals(pairs);
    expect(totals.faces).toBe(6);
    expect(totals.unresolved).toBe(2);
    const summaries = Object.values(results.summary.lanes?.fonts ?? {});
    expect(totals.counts.glyphs).toBe(summaries.reduce((sum, s) => sum + (s?.total.glyphs ?? 0), 0));
    expect(totals.complete).toBe(summaries.reduce((sum, s) => sum + (s?.complete ?? 0), 0));
  });

  it('orders families by glyphs and segments them by source, fallback last', () => {
    const pairs = pairFigures(fontsResults());
    const families = familyFigures(pairs, null);
    expect(families.map((f) => f.family)).toEqual(['Inter', 'Calibri']);
    const calibri = families[1]!.counts;
    const segments = sourceSegments(calibri);
    expect(segments.map((s) => s.source)).toEqual(['embedded', 'google', 'local', 'dropped', 'substitute', 'fallback']);
    expect(segments.reduce((sum, s) => sum + s.count, 0)).toBe(calibri.glyphs);
    expect(familyFigures(pairs, 'demo-basics:odp')[0]!.counts.glyphs).toBeLessThan(families[0]!.counts.glyphs);
  });

  it('lists the rows that fell back, worst first, or every row of one deck', () => {
    const results = fontsResults();
    const failing = fontRows(results, null);
    expect(failing.length).toBeGreaterThan(0);
    expect(failing.every(([, row]) => row.outcome !== 'fonts-complete')).toBe(true);
    const deck = fontRows(results, 'demo-basics:pptx');
    expect(deck.every(([, row]) => row.frontend === 'pptx')).toBe(true);
    expect(deck[0]?.[1].outcome).not.toBe('fonts-complete');
  });

  it("names the renderer from the stamp: a native run's fallback fonts are the host's", () => {
    const results = fontsResults();
    expect(fontsRenderer(results)).toBe('container');
    const native: Results = {
      ...results,
      stamp: { ...results.stamp, renderer: { ...results.stamp.renderer, platformDigest: 'native:macos-aarch64' } },
    };
    expect(fontsRenderer(native)).toBe('native');
  });

  it('counts the rows the equivalence gate compared, null when none', () => {
    const results = fontsResults();
    expect(equivalenceCount(results)).toBeNull();
    const [id, row] = Object.entries(results.rows).find((entry) => entry[1]?.fonts !== undefined)!;
    const compared: Results = {
      ...results,
      rows: {
        ...results.rows,
        [id]: {
          ...row!,
          fonts: {
            ...row!.fonts!,
            equivalence: { status: 'identical', staticRender: '0'.repeat(64), staticOutcome: 'match' },
          },
        },
      },
    };
    expect(equivalenceCount(compared)).toBe(1);
  });

  it('adds counts source by source and formats bytes', () => {
    const sum = addCounts(emptyCounts(), {
      glyphs: 3,
      exact: 2,
      substitute: 1,
      fallback: 0,
      coverageGaps: 0,
      sources: { google: 2, substitute: 1 },
    });
    addCounts(sum, { glyphs: 1, exact: 0, substitute: 0, fallback: 1, coverageGaps: 1, sources: {} });
    expect(sum).toEqual({
      glyphs: 4,
      exact: 2,
      substitute: 1,
      fallback: 1,
      coverageGaps: 1,
      sources: { google: 2, substitute: 1 },
    });
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(27_648)).toBe('27.0 KB');
    expect(formatBytes(5 * 1024 * 1024)).toBe('5.0 MB');
  });
});
