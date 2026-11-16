import { describe, expect, it } from 'vitest';

import {
  NO_EFFECT,
  NO_MOTION_FILTERS,
  coverage,
  deckFigures,
  failedChecks,
  fallbackPareto,
  motionRenderer,
  motionRows,
  motionRun,
  nonNativeItems,
  paramsText,
  rowEffects,
  rowKind,
  slideOf,
  stageOf,
  verdictTotals,
} from '../src/model/effects';
import { filterRows, listCommand, rowsOf } from '../src/model/rows';
import type { Results } from '../src/schema';
import { NO_FILTERS } from '../src/state/url';
import { fixture } from './fixture';

function motionResults(): Results {
  const run = fixture.runs.runs.findLast((entry) => entry.lane === 'motion');
  const results = run ? fixture.results.get(run.key) : undefined;
  if (!results) {
    throw new Error('no motion results in the fixture');
  }
  return results;
}

describe('the motion lane model', () => {
  it('picks the selected motion run, else the newest one', () => {
    const motion = fixture.runs.runs.findLast((entry) => entry.lane === 'motion');
    const fonts = fixture.runs.runs.findLast((entry) => entry.lane === 'fonts');
    expect(motion && motionRun(fixture.runs, motion)).toBe(motion);
    expect(fonts && motionRun(fixture.runs, fonts)).toBe(motion);
    expect(motionRenderer(motionResults())).toBe('container');
  });

  it('reads row ids', () => {
    expect(rowKind('demo:kn-1:keynote#s2')).toBe('stage');
    expect(rowKind('demo:kn-1:keynote#t@250')).toBe('transition');
    expect(rowKind('demo:kn-1:keynote')).toBeNull();
    expect(slideOf('demo:kn-1:keynote#t@250')).toBe('demo:kn-1:keynote');
    expect(stageOf('demo:kn-1:keynote#s2')).toBe(2);
    expect(stageOf('demo:kn-1:keynote#t@250')).toBeNull();
  });

  it('totals each deck by verdict, worst first', () => {
    const decks = deckFigures(motionResults());
    expect(decks.map((d) => d.pair)).toEqual(['demo-keynote:keynote']);
    expect(verdictTotals(decks)).toEqual({ native: 1, fallbackAllowed: 1, accepted: 1, dropped: 0, mismatched: 1 });
  });

  it('counts every effect native against fallback, and ranks the fallbacks', () => {
    const results = motionResults();
    const rows = coverage(results);
    const dissolve = rows.find((row) => row.kind === 'transition' && row.effect === 'dissolve');
    expect(dissolve?.byFrontend.keynote).toEqual({ native: 1, fallback: 0, unsupported: 0 });
    // A step the deck names no effect for counts under the dash.
    const none = rows.find((row) => row.kind === 'build' && row.effect === NO_EFFECT);
    expect(none?.total).toEqual({ native: 0, fallback: 1, unsupported: 0 });
    expect(fallbackPareto(results)).toEqual([
      { native: 'keynote:apple:sparkle', fallback: 'dissolve', kind: 'build', count: 1, frontends: ['keynote'] },
    ]);
  });

  it('filters the rows by class, kind, deck and effect, worst first', () => {
    const results = motionResults();
    const all = motionRows(results, NO_MOTION_FILTERS);
    expect(all).toHaveLength(9);
    expect(all[0]?.[1].outcome).toBe('motion-mismatch');
    const transitions = motionRows(results, { ...NO_MOTION_FILTERS, kind: 'transition' });
    expect(transitions.map(([id]) => rowKind(id))).toEqual(['transition', 'transition', 'transition']);
    expect(motionRows(results, { ...NO_MOTION_FILTERS, classes: ['motion-fallback'] })).toHaveLength(1);
    expect(motionRows(results, { ...NO_MOTION_FILTERS, deck: 'other:pptx' })).toHaveLength(0);
    const pop = motionRows(results, { ...NO_MOTION_FILTERS, effect: 'pop' });
    expect(pop.map(([id]) => id)).toEqual([all[0]?.[0]]);
    expect(rowEffects(results, all[0]?.[0] ?? '')).toEqual(['pop']);
  });

  it('names the checks a row failed, and lists the items that did not play natively', () => {
    const results = motionResults();
    const [id, row] = motionRows(results, { ...NO_MOTION_FILTERS, classes: ['motion-mismatch'] })[0] ?? [];
    expect(id).toBeDefined();
    expect(row && failedChecks(row).map((c) => c.check)).toEqual(['end-state', 'reveal', 'moves']);
    expect(row && failedChecks(row)[0]).toEqual({
      check: 'end-state',
      detail: 'kn-1005.4 opacity ends at 0.6, settles at 1',
    });
    const mismatched = nonNativeItems(results, 'mismatched');
    expect(mismatched.map((item) => item.why)).toEqual(['duration 400 ms, the deck says 500 ms']);
    // An accepted defect is counted apart from the fallbacks.
    expect(nonNativeItems(results, null).map((item) => item.verdict)).toEqual(['mismatched', 'accepted', 'declared']);
  });

  it('shows the params of a step as the deck writes them', () => {
    const results = motionResults();
    const plays = Object.values(results.timelines ?? {}).flatMap((t) => t?.plays ?? []);
    const flying = plays.find((play) => play.params !== undefined);
    expect(paramsText(flying?.params)).toBe('dir: u, distance: offSlide');
    expect(flying?.delivery).toBe('byParagraph');
    expect(paramsText(undefined)).toBeNull();
  });

  it('opens the explorer on the rows of an effect', () => {
    const results = motionResults();
    const pop = filterRows(rowsOf(results), { ...NO_FILTERS, ref: true, fx: 'pop' });
    expect(pop.map((row) => row.id)).toEqual(
      motionRows(results, { ...NO_MOTION_FILTERS, effect: 'pop' }).map(([id]) => id),
    );
    expect(filterRows(rowsOf(results), { ...NO_FILTERS, ref: true, fx: 'nothing-plays-this' })).toHaveLength(0);
    // No `slidock-bench list` option filters by effect.
    expect(listCommand({ ...NO_FILTERS, fx: 'pop' }, null)).toBeNull();
  });
});
