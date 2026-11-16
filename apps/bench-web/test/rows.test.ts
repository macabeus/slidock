import { execFileSync } from 'node:child_process';
import { describe, expect, it } from 'vitest';

import {
  codesOf,
  facetCounts,
  filterRows,
  listCommand,
  neighbours,
  parseRowId,
  rowsOf,
  runDirPath,
  runResultsPath,
  sortRows,
  stepLabel,
} from '../src/model/rows';
import type { RowRecord } from '../src/schema';
import { NO_FILTERS } from '../src/state/url';
import { fixture, latest } from './fixture';

function record(overrides: Partial<RowRecord>): RowRecord {
  return {
    presentation: 'demo',
    slide: 's1',
    frontend: 'pptx',
    oracle: { page: 1 },
    outcome: 'match',
    diagnostics: {},
    content: 'c',
    ...overrides,
  };
}

describe('row ids', () => {
  it('take apart into presentation, slide, frontend and step', () => {
    expect(parseRowId('demo-a:pp-9001:pptx')).toEqual({
      presentation: 'demo-a',
      slide: 'pp-9001',
      frontend: 'pptx',
      step: { kind: 'static' },
    });
    expect(parseRowId('demo-b:kn-1:keynote#s12')?.step).toEqual({ kind: 'stage', stage: 12 });
    expect(parseRowId('demo-c:od-p1:odp#t@250')?.step).toEqual({ kind: 'transition', pm: 250 });
    expect(stepLabel({ kind: 'transition', pm: 250 })).toBe('transition 25%');
    expect(stepLabel({ kind: 'stage', stage: 0 })).toBe('stage 0');
  });

  it('refuse other shapes', () => {
    for (const bad of ['demo:pp-1', 'demo:pp-1:pptx#s', 'demo:pp-1:pptx#s01', 'demo:pp-1:pptx#t@0', 'a:b:c:d']) {
      expect(parseRowId(bad), bad).toBeNull();
    }
  });
});

describe('filters', () => {
  const rows = rowsOf(latest());

  it('hide reference and known-loss rows unless asked, as slidock-bench list does', () => {
    const flaggedCount = rows.filter((row) => (row.record.knownLoss?.length ?? 0) > 0).length;
    expect(flaggedCount).toBeGreaterThan(0);
    expect(filterRows(rows, NO_FILTERS)).toHaveLength(rows.length - flaggedCount);
    expect(filterRows(rows, { ...NO_FILTERS, ref: true })).toHaveLength(rows.length);
  });

  it('match a code through the decline, the own code or any diagnostic, and need every code', () => {
    const row = record({ declined: { code: 'PX1' }, code: 'IR9', diagnostics: { MID2: 1 } });
    expect(codesOf(row).sort()).toEqual(['IR9', 'MID2', 'PX1']);
    const list = [{ id: 'demo:s1:pptx', record: row }];
    expect(filterRows(list, { ...NO_FILTERS, code: ['PX1', 'MID2'] })).toHaveLength(1);
    expect(filterRows(list, { ...NO_FILTERS, code: ['PX1', 'NOPE'] })).toHaveLength(0);
  });

  it('let an unknown outcome or frontend match nothing rather than everything', () => {
    expect(filterRows(rows, { ...NO_FILTERS, ref: true, oc: ['weird'] })).toHaveLength(0);
    expect(filterRows(rows, { ...NO_FILTERS, ref: true, fe: ['pdf'] })).toHaveLength(0);
  });

  it('split slides from motion steps', () => {
    const motion = fixture.runs.runs.findLast((run) => run.lane === 'motion');
    const results = motion ? fixture.results.get(motion.key) : undefined;
    expect(results).toBeDefined();
    const all = rowsOf(results ?? latest());
    const steps = filterRows(all, { ...NO_FILTERS, step: 'steps' });
    const slides = filterRows(all, { ...NO_FILTERS, step: 'slide' });
    expect(steps.length).toBeGreaterThan(0);
    expect(steps.length + slides.length).toBe(all.length);
    expect(steps.every((row) => row.id.includes('#'))).toBe(true);
  });

  it('count each option under the other filters', () => {
    const counts = facetCounts(rows, { ...NO_FILTERS, ref: true, fe: ['odp'], oc: ['close'] });
    // The frontend facet ignores the frontend filter: every frontend's close rows.
    const closes = rows.filter((row) => row.record.outcome === 'close');
    expect([...counts.fe.values()].reduce((a, b) => a + b, 0)).toBe(closes.length);
    // The outcome facet ignores the outcome filter: every ODP row by class.
    const odp = rows.filter((row) => row.record.frontend === 'odp');
    expect([...counts.oc.values()].reduce((a, b) => a + b, 0)).toBe(odp.length);
  });

  it('count each code by the rows choosing it leaves, since codes are all of', () => {
    let checked = 0;
    for (const results of fixture.results.values()) {
      const all = rowsOf(results);
      const codes = [...new Set(all.flatMap((row) => codesOf(row.record)))];
      for (const selected of [[], ...codes.map((code) => [code])]) {
        const filters = { ...NO_FILTERS, ref: true, code: selected };
        const counts = facetCounts(all, filters).code;
        for (const code of codes.filter((option) => !selected.includes(option))) {
          const leaves = filterRows(all, { ...filters, code: [...selected, code] }).length;
          expect(counts.get(code) ?? 0, `${selected.join(',')} + ${code}`).toBe(leaves);
          checked += 1;
        }
      }
    }
    expect(checked).toBeGreaterThan(0);
  });

  it("filter and count by the deciding decline's feature, any of", () => {
    const list = [
      {
        id: 'demo:s1:pptx',
        record: record({ outcome: 'declined', declined: { code: 'PX1', feature: 'element.chart' } }),
      },
      {
        id: 'demo:s2:pptx',
        record: record({ outcome: 'declined', declined: { code: 'PX2', feature: 'element.ole' } }),
      },
      { id: 'demo:s3:pptx', record: record({ outcome: 'declined', declined: { code: 'PX1' } }) },
      { id: 'demo:s4:pptx', record: record({}) },
    ];
    expect(filterRows(list, { ...NO_FILTERS, feat: ['element.chart'] }).map((row) => row.id)).toEqual(['demo:s1:pptx']);
    expect(filterRows(list, { ...NO_FILTERS, feat: ['element.chart', 'element.ole'] })).toHaveLength(2);
    const counts = facetCounts(list, { ...NO_FILTERS, feat: ['element.chart'], code: ['PX1'] });
    expect([...counts.feat]).toEqual([['element.chart', 1]]);
    expect(counts.code.get('PX1')).toBe(1);
    expect([...facetCounts(list, NO_FILTERS).feat]).toEqual([
      ['element.chart', 1],
      ['element.ole', 1],
    ]);
  });
});

describe('sort', () => {
  const list = [
    { id: 'b:s:pptx', record: record({ outcome: 'failed' }) },
    {
      id: 'a:s:pptx',
      record: record({
        outcome: 'close',
        scores: {
          ssim: { mean: 0.9, p1: 0.5 },
          color: { mean: 2, p99: 3 },
          pixel: 1,
          text: {
            oracleWords: 1,
            domWords: 1,
            matched: 1,
            recall: 1,
            precision: 1,
            medianPm: 1,
            p95Pm: 1,
            heightRatioMedian: 1,
          },
        },
      }),
    },
    {
      id: 'c:s:pptx',
      record: record({
        outcome: 'match',
        scores: {
          ssim: { mean: 0.99, p1: 0.9 },
          color: { mean: 1, p99: 2 },
          pixel: 1,
          text: {
            oracleWords: 1,
            domWords: 1,
            matched: 1,
            recall: 1,
            precision: 1,
            medianPm: null,
            p95Pm: null,
            heightRatioMedian: null,
          },
        },
      }),
    },
  ];

  it('puts rows without the value last in both directions', () => {
    expect(sortRows(list, 'ssim', 'asc').map((row) => row.id)).toEqual(['a:s:pptx', 'c:s:pptx', 'b:s:pptx']);
    expect(sortRows(list, 'ssim', 'desc').map((row) => row.id)).toEqual(['c:s:pptx', 'a:s:pptx', 'b:s:pptx']);
    expect(sortRows(list, 'p95', 'desc').map((row) => row.id)).toEqual(['a:s:pptx', 'b:s:pptx', 'c:s:pptx']);
  });

  it('sorts outcomes by severity rank and ids in code-unit order', () => {
    expect(sortRows(list, 'outcome', 'asc').map((row) => row.record.outcome)).toEqual(['failed', 'close', 'match']);
    expect(sortRows(list, 'id', 'desc').map((row) => row.id)).toEqual(['c:s:pptx', 'b:s:pptx', 'a:s:pptx']);
  });
});

describe('the list command', () => {
  it('prints the filters slidock-bench list has, reading the baseline or a results file', () => {
    expect(listCommand({ ...NO_FILTERS, oc: ['declined'], code: ['PX1'], fe: ['pptx'], ref: true }, null)).toBe(
      'slidock-bench list --class declined --code PX1 --frontend pptx --all',
    );
    expect(listCommand(NO_FILTERS, 'bench/.cache/runs/k/results.json')).toBe(
      'slidock-bench list --results bench/.cache/runs/k/results.json',
    );
  });

  it('is absent when a filter has no list option', () => {
    expect(listCommand({ ...NO_FILTERS, fe: ['pptx', 'odp'] }, null)).toBeNull();
    expect(listCommand({ ...NO_FILTERS, deck: 'demo' }, null)).toBeNull();
    expect(listCommand({ ...NO_FILTERS, q: 'kn' }, null)).toBeNull();
    expect(listCommand({ ...NO_FILTERS, step: 'steps' }, null)).toBeNull();
    expect(listCommand({ ...NO_FILTERS, feat: ['element.chart'] }, null)).toBeNull();
  });

  it("names a run's files in the cache the shell resolves, SLIDOCK_BENCH_CACHE or its default", () => {
    const echo = (word: string, env: Record<string, string>) =>
      execFileSync('/bin/sh', ['-c', `printf %s ${word}`], { env: { PATH: '/usr/bin:/bin', ...env } }).toString();
    expect(echo(runResultsPath('k'), {})).toBe('bench/.cache/runs/k/results.json');
    expect(echo(runResultsPath('k'), { SLIDOCK_BENCH_CACHE: '/private bench' })).toBe(
      '/private bench/runs/k/results.json',
    );
    expect(echo(runDirPath('k'), { SLIDOCK_BENCH_CACHE: '/private bench' })).toBe('/private bench/runs/k');
  });
});

describe('neighbours', () => {
  it('follow the display order', () => {
    expect(neighbours(['a', 'b', 'c'], 'b')).toEqual({ previous: 'a', next: 'c' });
    expect(neighbours(['a', 'b', 'c'], 'a')).toEqual({ previous: null, next: 'b' });
    expect(neighbours(['a'], 'z')).toEqual({ previous: null, next: null });
  });
});
