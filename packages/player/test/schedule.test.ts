import type { DeckStep } from '@slidock/types';
import { describe, expect, it } from 'vitest';

import {
  STEPS_PER_SLIDE,
  clickGroupCount,
  endProgress,
  groups,
  slideState,
  staggerLevels,
  stepUnits,
  unitKey,
} from '../src/schedule';
import { NO_HOLD, isIdentity } from '../src/schedule';
import { step } from './fixtures';

const keys = (map: Map<string, unknown>) => [...map.keys()].sort();

/** A step with vocabulary and timing fields the fixture type does not spell out. */
function rich(element: string, fields: Record<string, unknown>): DeckStep {
  return step(element, fields as Partial<DeckStep>);
}

const timings = (timeline: DeckStep[], group = 1, units?: (s: DeckStep, i: number) => number) =>
  groups(timeline, units)[group]?.steps.map((s) => [s.startMs, s.endMs]);

describe('schedule: groups and triggers', () => {
  it('group 0 holds the leading automatic steps; each click opens a group', () => {
    const timeline = [
      step('a', { trigger: 'afterPrevious' }),
      step('b'),
      step('c', { trigger: 'withPrevious' }),
      step('d'),
    ];
    const all = groups(timeline);
    expect(all.map((g) => g.steps.map((s) => s.step.target.element))).toEqual([['a'], ['b', 'c'], ['d']]);
    expect(clickGroupCount(timeline)).toBe(2);
    expect(groups([]).length).toBe(1);
  });

  it('starts with-previous steps with the previous start and after-previous ones at its end', () => {
    const timeline = [
      step('a', { delayMs: 100, durationMs: 400 }),
      step('b', { trigger: 'withPrevious', delayMs: 50, durationMs: 1000 }),
      step('c', { trigger: 'afterPrevious', delayMs: 250, durationMs: 500 }),
    ];
    expect(timings(timeline)).toEqual([
      [100, 500],
      [150, 1150],
      [1400, 1900],
    ]);
    expect(groups(timeline)[1]?.durationMs).toBe(1900);
  });

  it('uses the absolute offset when the compiler gave one, instead of chaining', () => {
    const timeline = [
      step('a', { durationMs: 400 }),
      rich('b', { trigger: 'afterPrevious', delayMs: 250, durationMs: 500, offsetMs: 1000 }),
      step('c', { trigger: 'afterPrevious', durationMs: 100 }),
    ];
    // Negative control: without the offset, b chains at a's end plus its delay.
    expect(timings(timeline)).toEqual([
      [0, 400],
      [1000, 1500],
      [1500, 1600],
    ]);
    const chained = [timeline[0] as DeckStep, step('b', { trigger: 'afterPrevious', delayMs: 250, durationMs: 500 })];
    expect(timings(chained)?.[1]).toEqual([650, 1150]);
  });

  it('chains after-previous on the end that includes repeats, auto-reverse and stagger', () => {
    const base = { durationMs: 100 };
    const ends = (fields: Record<string, unknown>, units = 1) =>
      timings(
        [rich('a', { ...base, ...fields }), step('b', { trigger: 'afterPrevious', durationMs: 10 })],
        1,
        (_s, i) => (i === 0 ? units : 1),
      );
    expect(ends({})).toEqual([
      [0, 100],
      [100, 110],
    ]);
    expect(ends({ repeat: { count: 2.5 } })?.[1]).toEqual([250, 260]);
    expect(ends({ autoReverse: true })?.[1]).toEqual([200, 210]);
    expect(ends({ repeat: { count: 2 }, autoReverse: true })?.[1]).toEqual([400, 410]);
    expect(ends({ stagger: { perUnit: { ms: 30 }, order: 'forward' } }, 4)?.[1]).toEqual([190, 200]);
    expect(ends({ stagger: { perUnit: { fraction: 0.5 }, order: 'forward' } }, 3)?.[1]).toEqual([200, 210]);
  });

  it('counts one round of an endless repeat toward its end and marks how long it runs', () => {
    const [click] = groups([rich('a', { repeat: 'untilNextClick', durationMs: 300 })]).slice(1);
    expect(click?.steps[0]).toMatchObject({ endMs: 300, iterations: Infinity, loop: 'click' });
    const [slide] = groups([rich('a', { repeat: 'untilEndOfSlide', autoReverse: true, durationMs: 300 })]).slice(1);
    expect(slide?.steps[0]).toMatchObject({ endMs: 600, loop: 'slide', direction: 'alternate' });
  });

  it('reads iterations and direction for WAAPI', () => {
    const [, group] = groups([rich('a', { repeat: { count: 3 }, autoReverse: true })]);
    expect(group?.steps[0]).toMatchObject({ iterations: 6, direction: 'alternate', loop: null });
  });

  it('spreads a typewriter over its duration, one character after another', () => {
    const [, group] = groups([rich('t', { effect: 'typewriter', durationMs: 1000 })], () => 4);
    expect(group?.steps[0]).toMatchObject({ durationMs: 0, unitDelays: [0, 250, 500, 750], endMs: 750 });
  });

  it('plays at most STEPS_PER_SLIDE steps', () => {
    const timeline = Array.from({ length: STEPS_PER_SLIDE + 5 }, (_, i) => step(`e${i}`));
    expect(clickGroupCount(timeline)).toBe(STEPS_PER_SLIDE);
    expect(groups(timeline).length).toBe(STEPS_PER_SLIDE + 1);
  });
});

describe('schedule: untrusted timing fails closed to declared defaults', () => {
  it('reads negative, non-finite or over-long times as 0, with an issue', () => {
    const [, group] = groups([step('a', { delayMs: -5, durationMs: Number.NaN })]);
    expect(group?.steps[0]?.startMs).toBe(0);
    expect(group?.durationMs).toBe(0);
    expect(group?.steps[0]?.issues).toEqual([
      { kind: 'timing', field: 'delayMs' },
      { kind: 'timing', field: 'durationMs' },
    ]);
    const [, long] = groups([step('a', { durationMs: 3_600_001 })]);
    expect(long?.steps[0]?.durationMs).toBe(0);
    const [, ok] = groups([step('a', { durationMs: 3_600_000 })]);
    expect(ok?.steps[0]?.issues).toEqual([]);
  });

  it('plays once when the repeat is out of the contract', () => {
    for (const repeat of [
      { count: 0 },
      { count: 1001 },
      { count: Number.POSITIVE_INFINITY },
      'forever',
      { count: 2, x: 1 },
    ]) {
      const [, group] = groups([rich('a', { repeat })]);
      expect(group?.steps[0]).toMatchObject({ iterations: 1, loop: null });
      expect(group?.steps[0]?.issues).toContainEqual({ kind: 'timing', field: 'repeat' });
    }
    const [, max] = groups([rich('a', { repeat: { count: 1000 } })]);
    expect(max?.steps[0]?.iterations).toBe(1000);
  });

  it('drops an unreadable stagger, easing or after effect', () => {
    const [, group] = groups(
      [
        rich('a', { stagger: { perUnit: { pct: 10 }, order: 'forward' } }),
        rich('b', { trigger: 'withPrevious', stagger: { perUnit: { ms: 10 }, order: 'sideways' } }),
        rich('c', { trigger: 'withPrevious', easing: 'ease-in' }),
        rich('d', { trigger: 'withPrevious', easing: { bezier: [2, 0, 0, 1] } }),
        rich('e', { trigger: 'withPrevious', after: { dim: 'red' } }),
      ],
      () => 3,
    );
    const steps = group?.steps ?? [];
    expect(steps[0]?.unitDelays).toEqual([0, 0, 0]);
    expect(steps[1]?.unitDelays).toEqual([0, 0, 0]);
    expect(steps[2]?.curve.kind).toBe('bezier');
    expect(steps.map((s) => s.issues.map((i) => (i.kind === 'timing' ? i.field : i.kind)))).toEqual([
      ['stagger'],
      ['stagger'],
      ['easing'],
      ['easing'],
      ['after'],
    ]);
    expect(steps[4]?.after).toBe('keep');
  });

  it('reads an unknown effect or one that cannot play in the step kind as its fallback', () => {
    const [, group] = groups([
      rich('a', { effect: 'vortex' }),
      rich('b', { trigger: 'withPrevious', effect: 'flyIn', params: { dir: 'left', distance: 'offSlide' } }),
      rich('c', { trigger: 'withPrevious', kind: 'media', effect: 'fade' }),
      rich('d', { trigger: 'withPrevious', effect: 'fade' }),
      step('e', { trigger: 'withPrevious' }),
    ]);
    expect(group?.steps.map((s) => s.effect?.effect ?? null)).toEqual([null, null, null, 'fade', null]);
    expect(group?.steps.map((s) => s.issues)).toEqual([
      [{ kind: 'effect', unread: 'unknown', name: 'vortex' }],
      [{ kind: 'effect', unread: 'invalid', name: 'flyIn' }],
      [{ kind: 'cannotPlay', name: 'fade' }],
      [],
      [],
    ]);
  });

  it('reads a malformed paragraph range as the whole element', () => {
    for (const paragraphs of [[2, 1], [-1, 3], [0.5, 1], [0, 20_000], 'all', [0]] as unknown[]) {
      expect(stepUnits(step('t', { target: { element: 't', paragraphs } as DeckStep['target'] }))).toEqual([
        { element: 't', paragraph: null },
      ]);
    }
  });
});

describe('schedule: stagger orders', () => {
  it('orders units forward, backward, from the centre and from the edges', () => {
    expect(staggerLevels(5, 'forward')).toEqual([0, 1, 2, 3, 4]);
    expect(staggerLevels(5, 'backward')).toEqual([4, 3, 2, 1, 0]);
    expect(staggerLevels(5, 'fromCenter')).toEqual([2, 1, 0, 1, 2]);
    expect(staggerLevels(4, 'fromCenter')).toEqual([1, 0, 0, 1]);
    expect(staggerLevels(5, 'fromEdges')).toEqual([0, 1, 2, 1, 0]);
    expect(staggerLevels(4, 'fromEdges')).toEqual([0, 1, 1, 0]);
    expect(staggerLevels(0, 'forward')).toEqual([]);
  });

  it('shuffles with a seed: the same order on every replay, another for another seed', () => {
    const a = staggerLevels(16, { random: 7 });
    expect(staggerLevels(16, { random: 7 })).toEqual(a);
    expect([...a].sort((x, y) => x - y)).toEqual(Array.from({ length: 16 }, (_, i) => i));
    expect(staggerLevels(16, { random: 8 })).not.toEqual(a);
    expect(a).not.toEqual(staggerLevels(16, 'forward'));
  });

  it('scales unit delays by the per-unit amount', () => {
    const [, group] = groups([rich('a', { stagger: { perUnit: { ms: 100 }, order: 'fromCenter' } })], () => 3);
    expect(group?.steps[0]?.unitDelays).toEqual([100, 0, 100]);
  });
});

describe('schedule: state at a step', () => {
  it('hides a unit whose first step is an entrance until it plays', () => {
    const timeline = [step('a'), step('b', { kind: 'exit' }), step('a', { kind: 'exit' })];
    expect(keys(slideState(timeline, 0).hidden)).toEqual(['a']);
    expect(keys(slideState(timeline, 1).hidden)).toEqual(['a']);
    expect(keys(slideState(timeline, 2).hidden)).toEqual([]);
    expect(keys(slideState(timeline, 3).hidden)).toEqual(['b']);
    expect(keys(slideState(timeline, 4).hidden)).toEqual(['a', 'b']);
  });

  it('does not pre-hide a unit whose first step is not an entrance', () => {
    const timeline = [step('a', { kind: 'emphasis' }), step('a')];
    expect(keys(slideState(timeline, 0).hidden)).toEqual([]);
  });

  it('builds paragraph ranges unit by unit, and a background apart from its element', () => {
    const timeline = [
      step('t', { target: { element: 't', paragraphs: [0, 1] } }),
      step('t', { target: { element: 't', paragraphs: [2, 2] } }),
      step('t', { target: { element: 't', paragraphs: null, background: true } }),
    ];
    const p = (n: number) => unitKey({ element: 't', paragraph: n });
    const bg = unitKey({ element: 't', paragraph: null, background: true });
    expect(keys(slideState(timeline, 1).hidden)).toEqual([p(0), p(1), p(2), bg].sort());
    expect(keys(slideState(timeline, 2).hidden)).toEqual([p(2), bg].sort());
    expect(keys(slideState(timeline, 3).hidden)).toEqual([bg]);
    expect(keys(slideState(timeline, 4).hidden)).toEqual([]);
  });

  it('applies after effects: hide hides, dim recolours until the unit enters again', () => {
    const timeline = [
      rich('a', { kind: 'emphasis', after: 'hide' }),
      rich('b', { kind: 'emphasis', after: { dim: '#808080' } }),
      step('b'),
    ];
    expect(keys(slideState(timeline, 1).hidden)).toEqual([]);
    expect(keys(slideState(timeline, 2).hidden)).toEqual(['a']);
    expect([...slideState(timeline, 3).dimmed.values()].map((d) => d.color)).toEqual(['#808080']);
    expect(keys(slideState(timeline, 4).dimmed)).toEqual([]);
    // Negative control: keep leaves both alone.
    const kept = [rich('a', { kind: 'emphasis', after: 'keep' })];
    expect(keys(slideState(kept, 2).hidden)).toEqual([]);
    expect(keys(slideState(kept, 2).dimmed)).toEqual([]);
  });

  it('keeps the end state of emphases that hold, and composes them', () => {
    const spin = (deg: number, hold: boolean) =>
      rich('a', { kind: 'emphasis', effect: 'spin', params: { deg, cw: true, hold } });
    const timeline = [
      spin(90, true),
      spin(45, true),
      spin(30, false),
      rich('a', { kind: 'emphasis', effect: 'growShrink', params: { scale: { x: 2, y: 0.5 }, hold: true } }),
      rich('a', { kind: 'emphasis', effect: 'transparency', params: { alpha: 0.5, hold: true } }),
      rich('a', {
        kind: 'motion',
        effect: 'path',
        params: {
          d: [
            ['M', 0, 0],
            ['L', 100, 50],
          ],
          autoRotate: false,
          hold: true,
        },
      }),
    ];
    const hold = (played: number) => slideState(timeline, played).holds.get('a')?.hold;
    expect(hold(1)).toBeUndefined();
    // Turns of one kind in a row merge; transforms compose in the order they played.
    expect(hold(2)?.transforms).toEqual([{ op: 'rotate', deg: 90 }]);
    expect(hold(3)?.transforms).toEqual([{ op: 'rotate', deg: 135 }]);
    expect(hold(4)?.transforms).toEqual([{ op: 'rotate', deg: 135 }]);
    expect(hold(5)?.transforms).toEqual([
      { op: 'rotate', deg: 135 },
      { op: 'scale', x: 2, y: 0.5 },
    ]);
    expect(hold(6)?.opacity).toBeCloseTo(0.5);
    // The path played with the turn and the scale before it undone.
    expect(hold(7)?.transforms.at(-1)).toEqual({ op: 'translate', x: 100, y: 50, under: 2 });
    // A path back where it started, or turns that cancel, leave no transform.
    const back = [
      rich('b', {
        kind: 'motion',
        effect: 'path',
        params: { d: [['M', 0, 0], ['L', 50, 0], ['Z']], autoRotate: false, hold: true },
      }),
      spin(90, true),
      rich('b', { kind: 'emphasis', effect: 'spin', params: { deg: 90, cw: false, hold: true } }),
    ];
    expect(slideState(back, 2).holds.get('b')?.hold.transforms).toEqual([]);
    expect(
      slideState(
        [back[1] as DeckStep, back[2] as DeckStep].map((s) => ({ ...s, target: { element: 'b', paragraphs: null } })),
        3,
      ).holds.get('b')?.hold.transforms,
    ).toEqual([]);
  });

  it('holds where a fractional or reversed repeat stops', () => {
    expect(endProgress({ iterations: 1, direction: 'normal' })).toBe(1);
    expect(endProgress({ iterations: 2, direction: 'alternate' })).toBe(0);
    expect(endProgress({ iterations: 2.5, direction: 'normal' })).toBe(0.5);
    expect(endProgress({ iterations: 1.25, direction: 'alternate' })).toBeCloseTo(0.75);
    const half = slideState(
      [
        rich('a', {
          kind: 'emphasis',
          effect: 'spin',
          params: { deg: 100, cw: false, hold: true },
          repeat: { count: 1.5 },
        }),
      ],
      2,
    );
    expect(half.holds.get('a')?.hold.transforms[0]).toMatchObject({ op: 'rotate', deg: expect.closeTo(-50) });
    const back = slideState(
      [rich('a', { kind: 'emphasis', effect: 'spin', params: { deg: 100, cw: true, hold: true }, autoReverse: true })],
      2,
    );
    expect(back.holds.get('a')?.hold.transforms).toEqual([]);
  });

  it('holds where tier-2 tracks end, in the order they compose, and adds later holds after them', () => {
    const track = (property: string, from: unknown, to: unknown) => ({
      property,
      beginMs: 0,
      durationMs: 500,
      frames: [
        { offset: 0, value: from },
        { offset: 1, value: to },
      ],
    });
    const timeline = [
      rich('a', { kind: 'emphasis', effect: 'spin', params: { deg: 30, cw: true, hold: true } }),
      rich('a', {
        kind: 'emphasis',
        effect: 'behaviors',
        params: [
          track('translate', { x: 0, y: 0 }, { x: 160, y: 0 }),
          track('rotate', 0, 90),
          track('scale', { x: 1, y: 1 }, { x: 1.5, y: 1.5 }),
          track('translate', { x: 0, y: 0 }, { x: 0, y: 20 }),
          track('opacity', 1, 0.5),
          track('blur', 0, 2),
        ],
      }),
      rich('a', { kind: 'emphasis', effect: 'growShrink', params: { scale: { x: 2, y: 2 }, hold: true } }),
      // Each track holds its own end: the second scale composes after the first.
      rich('b', {
        kind: 'emphasis',
        effect: 'behaviors',
        params: [
          track('scale', { x: 1, y: 1 }, { x: 1.2, y: 1.2 }),
          track('scale', { x: 1.2, y: 1.2 }, { x: 1, y: 1 }),
        ],
      }),
    ];
    const hold = (played: number) => slideState(timeline, played).holds.get('a')?.hold;
    // Every track of the step played with the spin before it undone.
    expect(hold(3)).toMatchObject({
      opacity: 0.5,
      blur: 2,
      transforms: [
        { op: 'rotate', deg: 30 },
        { op: 'translate', x: 160, y: 0, under: 1 },
        { op: 'rotate', deg: 90 },
        { op: 'scale', x: 1.5, y: 1.5 },
        { op: 'translate', x: 0, y: 20, under: 1 },
      ],
    });
    expect(hold(4)?.transforms.at(-1)).toEqual({ op: 'scale', x: 2, y: 2 });
    expect(slideState(timeline, 5).holds.get('b')?.hold.transforms).toEqual([{ op: 'scale', x: 1.2, y: 1.2 }]);
    // Tracks that all end at rest hold nothing.
    const still = [
      rich('c', {
        kind: 'emphasis',
        effect: 'behaviors',
        params: [
          {
            property: 'rotate',
            beginMs: 0,
            durationMs: 500,
            frames: [
              { offset: 0, value: 0 },
              { offset: 0.5, value: 20 },
              { offset: 1, value: 0 },
            ],
          },
          track('opacity', 0.4, 1),
        ],
      }),
    ];
    expect(slideState(still, 2).holds.get('c')).toBeUndefined();
  });

  it('holds the colours and font switches colour changes and tier-2 tracks leave', () => {
    const color = (property: string, to: string) =>
      rich('a', { kind: 'emphasis', effect: 'colorChange', params: { property, to } });
    const track = (property: string, from: unknown, to: unknown) => ({
      property,
      beginMs: 0,
      durationMs: 400,
      frames: [
        { offset: 0, value: from },
        { offset: 1, value: to },
      ],
    });
    const timeline = [
      color('fill', '#ff0000'),
      color('line', '#00ff00'),
      color('text', '#0000ff'),
      rich('a', {
        kind: 'emphasis',
        effect: 'behaviors',
        params: [track('fill', '#000000', '#111111'), track('bold', false, true), track('underline', true, false)],
      }),
    ];
    const paint = (played: number) => slideState(timeline, played).holds.get('a')?.hold.paint;
    expect(paint(2)).toEqual({ fill: '#ff0000' });
    expect(paint(4)).toEqual({ fill: '#ff0000', line: '#00ff00', text: '#0000ff' });
    // A later change of the same paint replaces it; font switches hold as their CSS values.
    expect(paint(5)).toEqual({ fill: '#111111', line: '#00ff00', text: '#0000ff', bold: 'bold', underline: 'none' });
    expect(isIdentity({ ...NO_HOLD, paint: { fill: '#ff0000' } })).toBe(false);
  });

  it('holds where a held repeat ends, an endless one after a round, and nothing while it still runs', () => {
    const spin = (fields: Record<string, unknown>) =>
      rich('a', { kind: 'emphasis', effect: 'spin', params: { deg: 90, cw: true, hold: true }, ...fields });
    const turn = (step: DeckStep, running?: ReadonlySet<number>) =>
      slideState([step], 2, running).holds.get('a')?.hold.transforms;
    expect(turn(spin({ repeat: 'untilNextClick' }))).toEqual([{ op: 'rotate', deg: 90 }]);
    expect(turn(spin({ repeat: 'untilEndOfSlide' }))).toEqual([{ op: 'rotate', deg: 90 }]);
    // A round of an auto-reversed repeat ends where it started.
    expect(turn(spin({ repeat: 'untilEndOfSlide', autoReverse: true }))).toEqual([]);
    expect(turn(spin({ repeat: 'untilEndOfSlide' }), new Set([0]))).toBeUndefined();
    expect(turn(spin({ repeat: 'untilEndOfSlide' }), new Set([1]))).toEqual([{ op: 'rotate', deg: 90 }]);
    // A held tier-2 scale that repeats (LibreOffice's blast) ends frozen at its held value.
    const blast = rich('a', {
      kind: 'emphasis',
      effect: 'behaviors',
      params: [
        {
          property: 'scale',
          beginMs: 0,
          durationMs: 500,
          frames: [
            { offset: 0, value: { x: 1, y: 1 } },
            { offset: 1, value: { x: 1.1, y: 1.1 } },
          ],
        },
      ],
    });
    for (const repeat of ['untilEndOfSlide', 'untilNextClick', { count: 3 }]) {
      expect(turn({ ...blast, repeat } as DeckStep), JSON.stringify(repeat)).toEqual([{ op: 'scale', x: 1.1, y: 1.1 }]);
    }
  });

  it('holds a repeated tier-2 step where its last round leaves each track', () => {
    const track = (property: string, from: unknown, to: unknown, beginMs = 0) => ({
      property,
      beginMs,
      durationMs: 500,
      frames: [
        { offset: 0, value: from },
        { offset: 1, value: to },
      ],
    });
    const held = (fields: Record<string, unknown>, params: unknown[]) =>
      slideState(
        [rich('a', { kind: 'emphasis', effect: 'behaviors', durationMs: 1000, params, ...fields })],
        2,
      ).holds.get('a')?.hold;
    const scale = track('scale', { x: 1, y: 1 }, { x: 2, y: 2 });
    // 1.5 rounds end halfway through the step: 500 ms in, the scale (0–500 ms) has ended, the fill (500–1000) not begun.
    expect(held({ repeat: { count: 1.5 } }, [scale, track('fill', '#000000', '#ffffff', 500)])).toMatchObject({
      transforms: [{ op: 'scale', x: 2, y: 2 }],
      paint: { fill: '#000000' },
    });
    // 1.25 rounds: 250 ms in, the scale is halfway, colours mix in sRGB, a switch that has not begun holds nothing.
    const quarter = held({ repeat: { count: 1.25 } }, [
      scale,
      track('text', '#000000', '#64c8fa'),
      { property: 'set', beginMs: 300, durationMs: 100, to: { property: 'bold', value: true } },
    ]);
    expect(quarter?.paint).toEqual({ text: '#32647d' });
    expect(quarter?.transforms[0]).toMatchObject({ op: 'scale', x: expect.closeTo(1.5, 6) });
    expect(held({ repeat: { count: 1.25 } }, [scale])?.paint).toEqual({});
    // An auto-reversed repeat ends where the step starts.
    expect(held({ repeat: { count: 2 }, autoReverse: true }, [scale])).toBeUndefined();
    // Whole rounds end on every track's last frame; a step that plays once holds its tracks' last frames too.
    expect(held({ repeat: { count: 3 } }, [scale])?.transforms).toEqual([{ op: 'scale', x: 2, y: 2 }]);
    expect(held({ autoReverse: true, repeat: 'untilEndOfSlide' }, [scale])?.transforms).toEqual([
      { op: 'scale', x: 2, y: 2 },
    ]);
  });

  it('holds what a switch switches to', () => {
    const set = (to: Record<string, unknown>) => ({ property: 'set', beginMs: 0, durationMs: 500, to });
    const state = slideState(
      [
        rich('a', {
          kind: 'emphasis',
          effect: 'behaviors',
          params: [
            set({ property: 'bold', value: true }),
            set({ property: 'underline', value: false }),
            set({ property: 'fill', value: '#123456' }),
            set({ property: 'opacity', value: 0.5 }),
          ],
        }),
      ],
      2,
    );
    expect(state.holds.get('a')?.hold).toMatchObject({
      opacity: 0.5,
      paint: { bold: 'bold', underline: 'none', fill: '#123456' },
    });
  });
});
