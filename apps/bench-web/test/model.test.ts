import { describe, expect, it } from 'vitest';

import {
  type Comparison,
  blocks,
  compareResults,
  erosionGuards,
  frontendGuards,
  gradientErosion,
  gradientLosses,
  incomparable,
  movement,
  regressionExit,
  shadowErosion,
  shadowLosses,
  ssimDistribution,
  stampDiff,
} from '../src/model/compare';
import { formatDelta, formatPercent, formatPointsDelta } from '../src/model/format';
import { laneFigures } from '../src/model/lanes';
import { outcomeGloss, outcomeRank } from '../src/model/outcomes';
import { declineRanking, fidelity, figure, outcomeSegments, summaryFrontends, tiles } from '../src/model/overview';
import { parityCounts, parityRows } from '../src/model/parity';
import { filterRows, rowsOf } from '../src/model/rows';
import { latestByLane, resolveRun } from '../src/model/runs';
import { thresholdsFor } from '../src/model/thresholds';
import type { Results, RowRecord, Scores } from '../src/schema';
import { NO_FILTERS } from '../src/state/url';
import { baseline, fixture, latest } from './fixture';

describe('overview', () => {
  const results = latest();

  it("draws the summary's own class counts, and each segment opens exactly its rows", () => {
    for (const [frontend, summary] of summaryFrontends(results.summary.byFrontend)) {
      const segments = outcomeSegments(summary);
      expect(segments.reduce((sum, segment) => sum + segment.count, 0)).toBe(summary.rows);
      for (const { outcome, count } of segments) {
        // The overview's preset: this frontend and class, flagged rows included.
        const rows = filterRows(rowsOf(results), { ...NO_FILTERS, fe: [frontend], oc: [outcome], ref: true });
        expect(rows.length, `${frontend} ${outcome}`).toBe(count);
      }
    }
  });

  it('counts the headline tiles', () => {
    const counted = tiles(results);
    expect(counted.decks).toBe(3);
    expect(counted.slides).toBe(Object.keys(results.rows).length);
    expect(counted.steps).toBe(0);
    expect(counted.frontends).toBe(3);
    const records = Object.values(results.rows);
    expect(counted.scored).toBe(records.filter((r) => ['match', 'close', 'mismatch'].includes(r.outcome)).length);
    expect(counted.matchShare).toBeCloseTo(records.filter((r) => r.outcome === 'match').length / records.length);
  });

  it('marks a figure improved or regressed only beyond its tolerance', () => {
    expect(figure(0.97, 0.969, 0.002).trend).toBe('unchanged');
    expect(figure(0.975, 0.97, 0.002).trend).toBe('improved');
    expect(figure(0.96, 0.97, 0.002).trend).toBe('regressed');
    expect(figure(0.96, null, 0.002)).toEqual({ value: 0.96, delta: null, trend: 'unchanged' });
    const odp = results.summary.byFrontend.odp;
    const base = baseline().summary.byFrontend.odp;
    expect(odp && base).toBeTruthy();
    if (odp && base) {
      const strip = fidelity(odp, base);
      expect(strip.ssimMedian.delta).toBeCloseTo((odp.ssim.median ?? 0) - (base.ssim.median ?? 0));
    }
  });

  it("bounds only the SSIM figures, by regression's frontend guard; text recall and match share show any move", () => {
    const summary = latest().summary.byFrontend.pptx;
    if (!summary) {
      throw new Error('the fixture has no PPTX summary');
    }
    const at = (median: number, recall: number) => ({
      ...summary,
      ssim: { median, p10: summary.ssim.p10 },
      textRecall: { mean: recall },
    });
    const strip = fidelity(at(0.9581, 0.995), at(0.96, 1));
    expect(strip.ssimMedian.trend).toBe('unchanged');
    expect(strip.textRecall.trend).toBe('regressed');
    expect(fidelity(at(0.9579, 1), at(0.96, 1)).ssimMedian.trend).toBe('regressed');
    expect(fidelity(at(0.96, 1.0001), at(0.96, 1)).textRecall.trend).toBe('improved');
  });

  it('ranks declines by rows then code', () => {
    expect(
      declineRanking({ byFrontend: {}, reference: {}, declines: { PX2: 1, PX1: 1, IR9: 4 } }).map((d) => d.code),
    ).toEqual(['IR9', 'PX1', 'PX2']);
  });

  it('formats deltas on the results grid with a true minus', () => {
    expect(formatDelta(0.00211)).toBe('+0.0021');
    expect(formatDelta(-0.0105)).toBe('−0.0105');
    expect(formatDelta(0.00001)).toBe('±0');
    expect(formatPercent(0.875)).toBe('87.5%');
    expect(formatPointsDelta(-0.2)).toBe('−20.0 pp');
  });
});

describe('outcomes', () => {
  it('rank as classifier@2 moves them, unknown classes lowest', () => {
    const order = ['failed', 'render-error', 'declined', 'mismatch', 'close', 'match'];
    for (let i = 1; i < order.length; i += 1) {
      expect(outcomeRank(order[i] ?? '')).toBeGreaterThan(outcomeRank(order[i - 1] ?? ''));
    }
    expect(outcomeRank('something-new')).toBeLessThan(outcomeRank('failed'));
  });

  it("quote the classifier's thresholds, and none for an unknown classifier", () => {
    const stamp = latest().stamp;
    const other = { classifier: stamp.classifier, harness: '0'.repeat(64) };
    const thresholds = thresholdsFor(other, null);
    expect(outcomeGloss('match', thresholds)).toContain('SSIM mean ≥ 0.97');
    expect(outcomeGloss('close', thresholds)).toContain('words placed within 11‰');
    expect(thresholdsFor({ ...other, classifier: 'classifier@3' }, fixture.status)).toBeNull();
    expect(outcomeGloss('match', null)).not.toContain('0.97');
  });

  it("read the server's thresholds for a run of HEAD's harness, the classifier's own for any other", () => {
    const stamp = latest().stamp;
    const served = structuredClone(fixture.status);
    served.thresholds.match.ssimMean = 0.975;
    expect(served.head.harness).toBe(stamp.harness);
    expect(thresholdsFor(stamp, served)?.match.ssimMean).toBe(0.975);
    expect(thresholdsFor({ ...stamp, classifier: 'classifier@3' }, served)?.match.ssimMean).toBe(0.975);
    expect(thresholdsFor({ ...stamp, harness: '0'.repeat(64) }, served)?.match.ssimMean).toBe(0.97);
    expect(thresholdsFor(stamp, null)?.match.ssimMean).toBe(0.97);
  });
});

describe('runs and lanes', () => {
  it('resolve latest to the newest static run and baseline to the newest that wrote it', () => {
    expect(resolveRun(fixture.runs, 'latest')?.key).toBe(fixture.latest);
    expect(resolveRun(fixture.runs, 'baseline')?.key).toBe(fixture.baseline);
    expect(resolveRun(fixture.runs, 'nope')).toBeNull();
  });

  it("sum each lane's checks over frontends", () => {
    const byLane = new Map(latestByLane(fixture.runs).map((run) => [run.lane, run]));
    expect([...byLane.keys()]).toEqual(['static', 'host', 'fonts', 'motion']);
    const host = byLane.get('host');
    const hostSummary = host?.summary.lanes?.host ?? {};
    const parts = Object.values(hostSummary);
    expect(host && laneFigures('host', host.summary)).toEqual([
      {
        label: 'identical',
        pass: parts.reduce((s, p) => s + (p?.identical ?? 0), 0),
        rows: parts.reduce((s, p) => s + (p?.rows ?? 0), 0),
      },
    ]);
    const motion = byLane.get('motion');
    expect(motion && laneFigures('motion', motion.summary).map((f) => f.label)).toEqual([
      'end states identical',
      'transition frames match',
      'effects native',
    ]);
    expect(laneFigures('shell', latest().summary)).toEqual([]);
  });
});

describe('compare', () => {
  it('moves classes by rank and isolates undeclines', () => {
    expect(movement('match', 'match')).toBe('same');
    expect(movement('close', 'match')).toBe('improved');
    expect(movement('match', 'mismatch')).toBe('regressed');
    expect(movement('declined', 'mismatch')).toBe('undeclined');
    expect(movement('declined', 'failed')).toBe('regressed');
    expect(movement('failed', 'render-error')).toBe('improved');
  });

  it('breaks the erosion guards past their tolerances only', () => {
    const scores = (ssim: number, recall: number, p95: number | null, color: number): Scores => ({
      ssim: { mean: ssim, p1: 0.5 },
      color: { mean: color, p99: 9 },
      pixel: 1,
      text: {
        oracleWords: 1,
        domWords: 1,
        matched: 1,
        recall,
        precision: 1,
        medianPm: 1,
        p95Pm: p95,
        heightRatioMedian: 1,
      },
    });
    expect(erosionGuards(scores(0.97, 1, 1, 2), scores(0.966, 0.995, 2.3, 2.9))).toEqual([]);
    expect(erosionGuards(scores(0.97, 1, 1, 2), scores(0.96, 0.98, 2.5, 3.5))).toEqual([
      'ssim.mean 0.97 → 0.96',
      'text.recall 1 → 0.98',
      'text.p95Pm 1 → 2.5',
      'color.mean 2 → 3.5',
    ]);
    expect(erosionGuards(scores(0.97, 1, null, 2), scores(0.97, 1, 9, 2))).toEqual([]);
  });

  it('erodes shadow rings past their guard and regresses rows whose undrawn shadows grew', () => {
    const base = baseline();
    const head = latest();
    const id = Object.keys(base.rows).find((key) => {
      const row = base.rows[key];
      return row?.scores && head.rows[key]?.outcome === row.outcome;
    });
    if (id === undefined) {
      throw new Error('the fixtures hold no scored row of unchanged class');
    }
    const shadowed = (row: RowRecord, worstRmse: number, kn1304: number): RowRecord => ({
      ...row,
      diagnostics: { ...row.diagnostics, KN1304: kn1304 },
      shadow: { elements: 1, worstRmse, meanRmse: worstRmse, worstEl: 'kn-1' },
    });
    const before = shadowed(base.rows[id] as RowRecord, 0.02, 1);
    expect(shadowErosion(before, shadowed(before, 0.03, 1))).toEqual([]);
    expect(shadowErosion(before, shadowed(before, 0.0301, 1))).toEqual(['shadow.worstRmse 0.02 → 0.0301 (kn-1)']);
    expect(shadowLosses(before, shadowed(before, 0.02, 0))).toEqual([]);
    expect(shadowLosses(before, shadowed(before, 0.02, 2))).toEqual(['KN1304 1 → 2']);

    const after = shadowed(head.rows[id] as RowRecord, 0.05, 2);
    const comparison = compareResults(
      { ...base, rows: { ...base.rows, [id]: before } },
      { ...head, rows: { ...head.rows, [id]: after } },
    );
    const change = comparison.regressed.find((c) => c.id === id);
    expect(change?.losses).toEqual(['KN1304 1 → 2']);
    expect(change?.guards).toContain('shadow.worstRmse 0.02 → 0.05 (kn-1)');
  });

  it('erodes gradient regions past their guard or when one goes, and regresses rows whose approximations grew', () => {
    const base = baseline();
    const head = latest();
    const id = Object.keys(base.rows).find((key) => {
      const row = base.rows[key];
      return row?.scores && head.rows[key]?.outcome === row.outcome;
    });
    if (id === undefined) {
      throw new Error('the fixtures hold no scored row of unchanged class');
    }
    const graded = (row: RowRecord, elements: number, worstRmse: number, kn1301: number): RowRecord => {
      const { gradient: _, ...rest } = row;
      return {
        ...rest,
        diagnostics: { ...row.diagnostics, KN1301: kn1301 },
        ...(elements > 0 ? { gradient: { elements, worstRmse, meanRmse: worstRmse, worstEl: 'bg' } } : {}),
      };
    };
    const before = graded(base.rows[id] as RowRecord, 2, 0.02, 1);
    expect(gradientErosion(before, graded(before, 2, 0.03, 1))).toEqual([]);
    expect(gradientErosion(before, graded(before, 3, 0.0301, 1))).toEqual(['gradient.worstRmse 0.02 → 0.0301 (bg)']);
    expect(gradientErosion(before, graded(before, 1, 0.02, 1))).toEqual(['gradient regions 2 → 1']);
    expect(gradientErosion(before, graded(before, 0, 0, 1))).toEqual(['gradient regions 2 → 0']);
    expect(gradientLosses(before, graded(before, 2, 0.02, 0))).toEqual([]);
    expect(gradientLosses(before, graded(before, 2, 0.02, 3))).toEqual(['KN1301 1 → 3']);

    const after = graded(head.rows[id] as RowRecord, 1, 0.05, 2);
    const comparison = compareResults(
      { ...base, rows: { ...base.rows, [id]: before } },
      { ...head, rows: { ...head.rows, [id]: after } },
    );
    const change = comparison.regressed.find((c) => c.id === id);
    expect(change?.losses).toEqual(['KN1301 1 → 2']);
    expect(change?.guards).toEqual(
      expect.arrayContaining(['gradient regions 2 → 1', 'gradient.worstRmse 0.02 → 0.05 (bg)']),
    );
  });

  it('compares the baseline with the latest run', () => {
    const base = baseline();
    const head = latest();
    const comparison = compareResults(base, head);
    expect(comparison.incomparable).toEqual([]);
    expect(comparison.compared).toBe(Object.keys(base.rows).length);
    const moved = comparison.transitions.reduce((sum, t) => sum + t.rows.length, 0);
    expect(moved).toBe(comparison.compared);
    for (const change of comparison.regressed) {
      expect(outcomeRank(change.head.outcome)).toBeLessThan(outcomeRank(change.base.outcome));
    }
    const deltas = comparison.regressed.map((change) => Math.abs(change.ssimDelta ?? -1));
    expect([...deltas].sort((a, b) => b - a)).toEqual(deltas);
    expect(comparison.eroded.every((change) => change.movement === 'same' && change.guards.length > 0)).toBe(true);
    expect(comparison.stamp.map((field) => field.field)).toContain('compiler.commit');
  });

  it('buckets the baseline against the latest run by the regression policy', () => {
    // The recorded buckets of the regression policy (Methodology) over the schema crate's baseline-results.json and
    // results.json.
    const expected = {
      regressed: [
        'demo-basics:pp-9002:pptx',
        'demo-basics:pp-9003:pptx',
        'demo-keynote:kn-1006:keynote',
        'demo-keynote:kn-1007:keynote',
        'demo-tables:od-p912:odp',
        'demo-tables:od-p914:odp',
      ],
      improved: [],
      undeclined: [],
      eroded: [
        'demo-basics:od-p902:odp',
        'demo-basics:od-p906:odp',
        'demo-basics:pp-9001:pptx',
        'demo-basics:pp-9005:pptx',
        'demo-basics:pp-9006:pptx',
        'demo-tables:pp-9102:pptx',
        'demo-tables:pp-9103:pptx',
      ],
      flagged: [
        'demo-basics:od-p901:odp',
        'demo-basics:od-p903:odp',
        'demo-basics:od-p905:odp',
        'demo-tables:od-p911:odp',
      ],
      newlyFlagged: [],
      frontendGuards: ['odp ssim median 0.9407 → 0.931'],
    };
    const comparison = compareResults(baseline(), latest());
    const ids = (changes: Comparison['regressed']) => changes.map((change) => change.id).sort();
    expect({
      regressed: ids(comparison.regressed),
      improved: ids(comparison.improved),
      undeclined: ids(comparison.undeclined),
      eroded: ids(comparison.eroded),
      flagged: ids(comparison.flagged),
      newlyFlagged: ids(comparison.newlyFlagged),
      frontendGuards: comparison.frontendGuards.map((guard) => guard.text),
    }).toEqual(expected);
    expect(comparison.eroded.find((change) => change.id === 'demo-basics:od-p902:odp')?.guards).toEqual([
      'ssim.mean 0.9622 → 0.9256',
      'text.recall 0.9756 → 0.9459',
    ]);
    expect(regressionExit(comparison)).toBe(1);
  });

  it('keeps changes of rows the base flags out of acceptance, and counts rows the head newly flags', () => {
    const flag = (record: RowRecord | undefined) => {
      if (record) {
        record.knownLoss = ['source'];
      }
    };
    const plain = (record: RowRecord) => record.reference !== true && !record.knownLoss?.length;
    const before = compareResults(baseline(), latest());
    const regressed = before.regressed[0]?.id ?? '';
    const unchanged = Object.keys(latest().rows).find((id) => {
      const record = latest().rows[id];
      return record && plain(record) && before.transitions.some((t) => t.from === t.to && t.rows.includes(id));
    });
    expect(regressed && unchanged).toBeTruthy();

    const base = structuredClone(baseline());
    const head = structuredClone(latest());
    flag(base.rows[regressed]);
    flag(head.rows[regressed]);
    flag(head.rows[unchanged ?? '']);
    const comparison = compareResults(base, head);
    expect(comparison.regressed.map((change) => change.id)).not.toContain(regressed);
    expect(comparison.flagged.map((change) => change.id)).toContain(regressed);
    expect(comparison.newlyFlagged.map((change) => change.id)).toEqual([unchanged]);

    // A flag only the head carries does not keep the row's own change out of acceptance.
    const flaggedHead = structuredClone(latest());
    flag(flaggedHead.rows[regressed]);
    const moved = compareResults(baseline(), flaggedHead);
    expect(moved.regressed.map((change) => change.id)).toContain(regressed);
    expect(moved.newlyFlagged.map((change) => change.id)).toEqual([regressed]);
  });

  it('fails a frontend whose SSIM median or p10 drops by more than 0.002, reference rows apart', () => {
    const template = Object.values(latest().rows).find((record) => record.scores);
    const scores = template?.scores;
    if (!template || !scores) {
      throw new Error('the fixture has no scored row');
    }
    const record = (ssim: number, reference: boolean): RowRecord => {
      const { reference: _, ...rest } = template;
      return {
        ...rest,
        ...(reference ? { reference: true } : {}),
        frontend: 'odp',
        scores: { ...scores, ssim: { mean: ssim, p1: 0.5 } },
      };
    };
    const distribution = ssimDistribution([0.9, 0.95, 0.93, 0.99].map((ssim) => record(ssim, false)));
    expect(distribution).toEqual({ median: 0.94, p10: 0.9 });
    expect(ssimDistribution([])).toEqual({ median: null, p10: null });
    const guards = (base: number[], head: number[], reference = false) =>
      frontendGuards(
        base.map((ssim, index) => ({ base: record(ssim, reference), head: record(head[index] ?? 0, reference) })),
      ).map((guard) => guard.text);
    expect(guards([0.95, 0.96, 0.97], [0.95, 0.9581, 0.97])).toEqual([]);
    expect(guards([0.95, 0.96, 0.97], [0.95, 0.9579, 0.97])).toEqual(['odp ssim median 0.96 → 0.9579']);
    expect(guards([0.95, 0.96, 0.97], [0.9, 0.96, 0.97])).toEqual(['odp ssim p10 0.95 → 0.9']);
    expect(guards([0.95, 0.96, 0.97], [0.5, 0.5, 0.5], true)).toEqual([]);
  });

  it('exits as regression does: 2 incomparable, 3 nothing compared, 1 a counted change, 0 otherwise', () => {
    const base = baseline();
    expect(regressionExit(compareResults(base, base))).toBe(0);
    const empty: Results = { ...base, rows: {} };
    expect(regressionExit(compareResults(empty, empty))).toBe(3);
    const other = structuredClone(base);
    other.stamp.harness = '0'.repeat(64);
    expect(regressionExit(compareResults(base, other))).toBe(2);
  });

  it('finds vanished rows in covered pairs and counts the rest as not run', () => {
    const base = baseline();
    const head = structuredClone(latest());
    const [first, second] = Object.keys(head.rows);
    expect(first && second).toBeTruthy();
    if (!first || !second) {
      return;
    }
    delete head.rows[first];
    const pair = `${base.rows[second]?.presentation}:${base.rows[second]?.frontend}`;
    const uncovered = Object.values(base.rows).filter((r) => `${r.presentation}:${r.frontend}` === pair).length;
    delete head.stamp.corpus[pair];
    for (const [id, record] of Object.entries(head.rows)) {
      if (`${record.presentation}:${record.frontend}` === pair) {
        delete head.rows[id];
      }
    }
    const comparison = compareResults(base, head);
    expect(comparison.notRun).toBe(uncovered);
    expect(comparison.vanished.length + comparison.notRun + comparison.compared).toBe(Object.keys(base.rows).length);
  });

  it('names the stamp fields that make two runs incomparable', () => {
    const base = baseline().stamp;
    const head = structuredClone(latest().stamp);
    head.renderer.recipe = '0'.repeat(64);
    head.metrics.ssim = 'ssim@2';
    const pair = Object.keys(head.corpus)[0] ?? '';
    head.corpus[pair] = '1'.repeat(64);
    const names = incomparable(base, head, [pair]);
    expect(names).toEqual(['renderer.recipe', 'metrics', `corpus[${pair}]`]);
    const diff = stampDiff(base, head).map((field) => field.field);
    expect(diff.filter((field) => blocks(field, names))).toEqual([`corpus.${pair}`, 'metrics.ssim', 'renderer.recipe']);
    expect(blocks('compiler.commit', names)).toBe(false);
  });
});

describe('parity', () => {
  const view = fixture.parity.get(fixture.latest);

  it('lists failing rows first, filters by status and carries allowlist entries inline', () => {
    expect(view).toBeDefined();
    if (!view) {
      return;
    }
    const rows = parityRows(view.report, []);
    const failing = new Set(view.report.compiler?.failing);
    expect(rows.length).toBe(Object.keys(view.report.compiler?.rows ?? {}).length);
    const firstPassing = rows.findIndex((row) => !row.failing);
    expect(rows.slice(0, firstPassing).every((row) => failing.has(row.id))).toBe(true);
    expect(rows.slice(firstPassing).some((row) => row.failing)).toBe(false);
    expect(parityRows(view.report, ['regressed']).every((row) => row.row.status === 'regressed')).toBe(true);
    const allowed = rows.find((row) => row.allowed !== null);
    expect(allowed?.id).toBe(view.allowlist[0]?.row);
    const counts = parityCounts(view.report);
    expect(counts.reduce((sum, c) => sum + c.rows, 0)).toBe(rows.length);
  });
});
