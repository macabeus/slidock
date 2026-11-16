import { describe, expect, it } from 'vitest';

import {
  type Action,
  type MachineModel,
  type Position,
  everyHidden,
  initialPosition,
  numbering,
  reduce,
  slideForNumber,
} from '../src/machine';

/** Slides as `s` (shown), `h` (hidden), with an optional click-group count: `s2` is shown with 2 groups. */
function model(spec: string, options: Partial<Pick<MachineModel, 'builds' | 'loop'>> = {}): MachineModel {
  return {
    slides: spec
      .split(' ')
      .map((token) => ({ hidden: token.startsWith('h'), clickGroups: Number(token.slice(1) || 0) })),
    builds: options.builds ?? false,
    loop: options.loop ?? false,
  };
}

function at(slide: number, step = 0, presenting = true, blank: Position['blank'] = null): Position {
  return { slide, step, presenting, blank };
}

const next: Action = { type: 'next' };
const previous: Action = { type: 'previous' };

describe('position machine', () => {
  it.each<[string, string, Position, Action, Position, string]>([
    ['presenting skips a hidden slide forward', 's h s', at(0), next, at(2), 'transition'],
    ['presenting skips a hidden slide back', 's h s', at(2), previous, at(0), 'cut'],
    ['viewing steps through hidden slides', 's h s', at(0, 0, false), next, at(1, 0, false), 'cut'],
    ['next plays the next click group', 's2 s', at(0, 0), next, at(0, 1), 'build'],
    ['next after the last group moves on', 's2 s', at(0, 2), next, at(1, 0), 'transition'],
    ['previous cuts to the prior step', 's2 s', at(0, 2), previous, at(0, 1), 'cut'],
    ['previous from step 0 lands fully built', 's2 s', at(1, 0), previous, at(0, 2), 'cut'],
    ['viewing without builds pins the step', 's2 s3', at(0, 2, false), next, at(1, 3, false), 'cut'],
    ['first lands on the first shown slide', 'h s2 s', at(2), { type: 'first' }, at(1, 0), 'cut'],
    ['last lands on the last shown slide at step 0', 's s2 h', at(0), { type: 'last' }, at(1, 0), 'cut'],
    ['goTo clamps the slide', 's s', at(0), { type: 'goTo', slide: 9 }, at(1), 'cut'],
    ['goTo clamps the step', 's2', at(0), { type: 'goTo', slide: 0, step: 7 }, at(0, 2), 'cut'],
    ['goTo can reach a hidden slide', 's h', at(0), { type: 'goTo', slide: 1 }, at(1), 'cut'],
    [
      'goTo while viewing pins the step',
      's s2',
      at(0, 0, false),
      { type: 'goTo', slide: 1, step: 0 },
      at(1, 2, false),
      'cut',
    ],
    ['next while blanked only unblanks', 's s', at(0, 0, true, 'black'), next, at(0), 'none'],
    ['previous while blanked only unblanks', 's s', at(1, 0, true, 'white'), previous, at(1), 'none'],
    ['blank sets the screen', 's', at(0), { type: 'blank', kind: 'white' }, at(0, 0, true, 'white'), 'none'],
    [
      'toggleBlank clears the same kind',
      's',
      at(0, 0, true, 'black'),
      { type: 'toggleBlank', kind: 'black' },
      at(0),
      'none',
    ],
    [
      'toggleBlank swaps kinds',
      's',
      at(0, 0, true, 'black'),
      { type: 'toggleBlank', kind: 'white' },
      at(0, 0, true, 'white'),
      'none',
    ],
    ['present starts on the current slide', 's s', at(1, 0, false), { type: 'present' }, at(1, 0), 'cut'],
    [
      'present from a hidden slide takes the next shown',
      's h s',
      at(1, 0, false),
      { type: 'present' },
      at(2, 0),
      'cut',
    ],
    [
      'present from a hidden last slide takes the previous shown',
      's h',
      at(1, 0, false),
      { type: 'present' },
      at(0, 0),
      'cut',
    ],
    ['present starts at step 0', 's2', at(0, 2, false), { type: 'present' }, at(0, 0), 'cut'],
    ['stop shows the slide fully built', 's2', at(0, 1, true, 'black'), { type: 'stop' }, at(0, 2, false), 'cut'],
  ])('%s', (_name, spec, from, action, expected, motion) => {
    const move = reduce(model(spec), from, action);
    expect(move.position).toEqual(expected);
    expect(move.motion).toBe(motion);
  });

  it('returns the same object when nothing moves', () => {
    const m = model('s h');
    for (const [from, action] of [
      [at(0), next],
      [at(0), previous],
      [at(0), { type: 'first' }],
      [at(0), { type: 'last' }],
      [at(0), { type: 'present' }],
      [at(0, 0, false), { type: 'stop' }],
      [at(0), { type: 'goTo', slide: 0 }],
    ] as const) {
      const move = reduce(m, from, action);
      expect(move.position).toBe(from);
      expect(move.motion).toBe('none');
    }
  });

  it('refuses to present when every slide is hidden', () => {
    const m = model('h h');
    const from = at(0, 0, false);
    expect(everyHidden(m)).toBe(true);
    expect(reduce(m, from, { type: 'present' }).position).toBe(from);
  });

  it('loops from the last shown slide when asked', () => {
    expect(reduce(model('h s s', { loop: true }), at(2), next).position).toEqual(at(1));
    expect(reduce(model('h s s'), at(2), next).position).toEqual(at(2));
  });

  it('plays builds while viewing when asked', () => {
    const m = model('s2 s', { builds: true });
    expect(reduce(m, at(0, 0, false), next)).toEqual({ position: at(0, 1, false), motion: 'build' });
    expect(reduce(m, at(0, 1, true), { type: 'stop' }).position).toEqual(at(0, 1, false));
  });

  it('an empty deck never moves', () => {
    const m = model('s');
    const empty = { ...m, slides: [] };
    const from = at(0);
    for (const action of [next, previous, { type: 'present' }, { type: 'goTo', slide: 3 }] as const) {
      expect(reduce(empty, from, action).position).toBe(from);
    }
  });

  it('starts presenting on the first shown slide', () => {
    expect(initialPosition(model('h s2 s'), true)).toEqual(at(1, 0));
    expect(initialPosition(model('s2 s'), false)).toEqual(at(0, 2, false));
  });
});

describe('numbering', () => {
  const slides = [{ hidden: false }, { hidden: true }, { hidden: false }, { hidden: false }];

  it('counts shown slides only', () => {
    expect(numbering(slides)).toEqual([1, null, 2, 3]);
  });

  it('goes to a show number', () => {
    expect(slideForNumber(slides, 1)).toBe(0);
    expect(slideForNumber(slides, 2)).toBe(2);
    expect(slideForNumber(slides, 3)).toBe(3);
    expect(slideForNumber(slides, 4)).toBeNull();
    expect(slideForNumber(slides, 0)).toBeNull();
    expect(slideForNumber(slides, 1.5)).toBeNull();
  });
});
