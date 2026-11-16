// @vitest-environment happy-dom
import type { DeckSlide, DeckStep } from '@slidock/types';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { htmlDeck } from '../src/deck';
import type { ScheduledStep } from '../src/schedule';
import {
  type MountedSlide,
  type SlideEnv,
  applyState,
  cutLoops,
  finishAll,
  mountSlide,
  playGroup,
  restoreSplits,
} from '../src/slide';
import { deck, slide, slideHtml, step } from './fixtures';

interface Recorded {
  readonly target: Element;
  readonly keyframes: Keyframe[];
  readonly options: KeyframeAnimationOptions;
  state: 'running' | 'finished' | 'cancelled';
}

let recorded: Recorded[] = [];
let original: typeof Element.prototype.animate;

beforeEach(() => {
  recorded = [];
  original = Element.prototype.animate;
  Element.prototype.animate = function (this: Element, keyframes: Keyframe[], options: KeyframeAnimationOptions) {
    const entry: Recorded = { target: this, keyframes, options, state: 'running' };
    recorded.push(entry);
    const iterations = options.iterations ?? 1;
    return {
      finished: new Promise<void>(() => undefined),
      effect: { getComputedTiming: () => ({ endTime: Number.isFinite(iterations) ? 1 : Infinity }) },
      finish: () => {
        if (!Number.isFinite(iterations)) {
          throw new Error('InvalidStateError');
        }
        entry.state = 'finished';
      },
      cancel: () => {
        entry.state = 'cancelled';
      },
    } as unknown as Animation;
  } as typeof Element.prototype.animate;
  document.body.replaceChildren();
});

afterEach(() => {
  Element.prototype.animate = original;
});

const issues: [ScheduledStep, unknown][] = [];

function env(overrides: Partial<SlideEnv> = {}): SlideEnv {
  return {
    size: { width: 960, height: 540 },
    reduced: false,
    media: { muted: true, onError: () => undefined },
    properties: true,
    cells: 128,
    onIssue: (s, issue) => issues.push([s, issue]),
    ...overrides,
  };
}

function mounted(timeline: DeckStep[], body = ''): { entry: MountedSlide; s: DeckSlide } {
  const s = slide('s1', { timeline });
  const source = htmlDeck(document, deck([s]), new Map([['s1', slideHtml('s1', body)]]));
  const entry = mountSlide(document, source, s, 0, { width: 960, height: 540 });
  document.body.append(entry.layer);
  return { entry, s };
}

function rich(element: string, fields: Record<string, unknown>): DeckStep {
  return step(element, fields as Partial<DeckStep>);
}

const live = () => recorded.filter((r) => r.state === 'running');
const el = (entry: MountedSlide, id: string) =>
  Array.from(entry.root.querySelectorAll('[data-el]')).find((e) => e.getAttribute('data-el') === id) as Element;

describe('runtime: printed shadows', () => {
  it("holds a blur after the element's printed shadow, which the held frame would otherwise replace", () => {
    const blur = {
      property: 'blur',
      beginMs: 0,
      durationMs: 100,
      frames: [
        { offset: 0, value: 0 },
        { offset: 1, value: 2 },
      ],
    };
    const timeline = [rich('s1.box', { kind: 'emphasis', effect: 'behaviors', params: [blur] })];
    const shadow = 'drop-shadow(0pt 3.287pt 7.14pt #00000080)';
    const box = `<div data-el="s1.box" style="position:absolute;left:0pt;top:0pt;width:10pt;height:10pt;filter:${shadow}"></div>`;
    const { entry } = mounted(timeline, box);
    // Played through the click that plays the step.
    applyState(entry, timeline, 2);
    const held = recorded.find((r) => r.target === el(entry, 's1.box'));
    expect(held?.keyframes[0]?.filter).toMatch(/^drop-shadow\(.+\)\s*blur\(2px\)$/);
  });
});

describe('runtime: playing groups', () => {
  it('plays the vocabulary effect, or the fallback with a reported issue', () => {
    issues.length = 0;
    const timeline = [
      rich('s1.title', { effect: 'flyIn', params: { dir: 'u', distance: 'offSlide' } }),
      rich('s1.title', { trigger: 'withPrevious', kind: 'emphasis', effect: 'vortex', fallback: 'none' }),
    ];
    const { entry } = mounted(timeline);
    applyState(entry, timeline, 1);
    const started = playGroup(entry, timeline, 1, env());
    expect(started.length).toBe(1);
    expect(recorded[0]?.keyframes[0]).toMatchObject({ offset: 0 });
    expect(Object.keys(recorded[0]?.keyframes[0] ?? {})).toContain('transform');
    expect(recorded[0]?.options).toMatchObject({ duration: 500, fill: 'both', composite: 'add' });
    expect(issues.map(([, issue]) => issue)).toEqual([{ kind: 'effect', unread: 'unknown', name: 'vortex' }]);
  });

  it('removes the pre-hide of the units its entrances show, and hides an exit at its end', () => {
    const timeline = [step('s1.title'), rich('s1.title', { kind: 'exit', trigger: 'afterPrevious', delayMs: 100 })];
    const { entry } = mounted(timeline);
    applyState(entry, timeline, 1);
    expect(el(entry, 's1.title').getAttribute('data-sd-state')).toBe('hidden');
    playGroup(entry, timeline, 1, env());
    expect(el(entry, 's1.title').hasAttribute('data-sd-state')).toBe(false);
    const hide = recorded.find((r) => r.keyframes.length === 1 && r.keyframes[0]?.opacity === 0);
    expect(hide?.options).toMatchObject({ delay: 1100, duration: 0, fill: 'forwards' });
  });

  it('plays a switch from its begin with no backward fill, holding it until the state takes over', () => {
    const timeline = [
      rich('s1.title', {
        kind: 'emphasis',
        fallback: 'none',
        delayMs: 50,
        effect: 'behaviors',
        params: [{ property: 'set', beginMs: 100, durationMs: 400, to: { property: 'opacity', value: 0.5 } }],
      }),
    ];
    const { entry } = mounted(timeline);
    applyState(entry, timeline, 1);
    expect(playGroup(entry, timeline, 1, env()).length).toBe(1);
    expect(recorded.at(-1)?.keyframes).toEqual([
      { offset: 0, opacity: 0.5 },
      { offset: 1, opacity: 0.5 },
    ]);
    expect(recorded.at(-1)?.options).toMatchObject({ delay: 150, duration: 400, fill: 'forwards' });
    applyState(entry, timeline, 2, { keepLoops: true });
    expect(recorded.at(-1)?.keyframes).toEqual([{ opacity: 0.5 }]);
  });

  it('links a switched paint with the value it shows now, under its changes, and covers none of them', () => {
    const set = (value: string, beginMs: number) => ({
      property: 'set',
      beginMs,
      durationMs: 500,
      to: { property: 'text', value },
    });
    const timeline = [
      rich('s1.title', {
        kind: 'emphasis',
        fallback: 'none',
        effect: 'behaviors',
        durationMs: 1000,
        params: [set('#dc2626', 0), set('#7c3aed', 500)],
      }),
    ];
    const { entry } = mounted(timeline);
    applyState(entry, timeline, 1);
    recorded = [];
    playGroup(entry, timeline, 1, env());
    const title = el(entry, 's1.title') as HTMLElement;
    expect(title.getAttribute('data-sd-paint')).toBe('text');
    expect(title.style.getPropertyValue('--sd-anim-color')).not.toBe('');
    // Only the two switches animate the paint, in their order: nothing created after the first covers it.
    expect(recorded.map((r) => [r.keyframes[0]?.['--sd-anim-color'], r.options.delay, r.options.fill])).toEqual([
      ['#dc2626', 0, 'forwards'],
      ['#7c3aed', 500, 'forwards'],
    ]);
    applyState(entry, timeline, 2, { keepLoops: true });
    expect(title.getAttribute('data-sd-paint')).toBeNull();
    expect(title.getAttribute('data-sd-hold')).toBe('text');
    expect(title.style.getPropertyValue('--sd-held-text')).toBe('#7c3aed');
    expect(title.style.getPropertyValue('--sd-anim-color')).toBe('');
  });

  it('repeats a tier-2 step as a whole: each track plays its part of every round, shown before and held after', () => {
    const timeline = [
      rich('s1.title', {
        kind: 'emphasis',
        fallback: 'none',
        effect: 'behaviors',
        durationMs: 500,
        repeat: { count: 3 },
        params: [
          {
            property: 'rotate',
            beginMs: 100,
            durationMs: 200,
            frames: [
              { offset: 0, value: 0 },
              { offset: 1, value: 90 },
            ],
          },
          { property: 'set', beginMs: 250, durationMs: 250, to: { property: 'opacity', value: 0.5 } },
        ],
      }),
    ];
    const { entry } = mounted(timeline);
    applyState(entry, timeline, 1);
    recorded = [];
    playGroup(entry, timeline, 1, env());
    const [turn, set] = recorded;
    expect(turn?.options).toMatchObject({ delay: 0, duration: 500, iterations: 3, fill: 'both' });
    expect(turn?.keyframes.map((k) => [k.offset, k.transform])).toEqual([
      [0, 'rotate(0deg)'],
      [0.2, 'rotate(0deg)'],
      [0.6, 'rotate(90deg)'],
      [1, 'rotate(90deg)'],
    ]);
    expect(set?.options).toMatchObject({ delay: 0, duration: 500, iterations: 3, fill: 'forwards' });
    expect(set?.keyframes.map((k) => [k.offset, k.opacity])).toEqual([
      [0, 1],
      [0.5, 0.5],
      [1, 0.5],
      [1, 0.5],
    ]);
    expect(set?.keyframes[0]?.easing).toBe('steps(1, end)');
  });

  it('a repeat cut by the next click leaves its hold before that group plays, and the group plays from it', () => {
    const spin = (deg: number, fields: Record<string, unknown> = {}) =>
      rich('s1.title', {
        kind: 'emphasis',
        fallback: 'none',
        effect: 'spin',
        params: { deg, cw: true, hold: true },
        ...fields,
      });
    const timeline = [spin(90, { repeat: 'untilNextClick' }), spin(45)];
    const { entry } = mounted(timeline);
    applyState(entry, timeline, 1);
    playGroup(entry, timeline, 1, env());
    applyState(entry, timeline, 2, { keepLoops: true });
    const held = () =>
      recorded
        .filter((r) => r.state !== 'cancelled' && r.keyframes.length === 1 && 'transform' in r.keyframes[0]!)
        .map((r) => r.keyframes[0]?.transform);
    expect(held()).toEqual([]);
    playGroup(entry, timeline, 2, env());
    expect(entry.loops.size).toBe(0);
    expect(held()).toEqual(['rotate(90deg) rotate(0deg)']);
    recorded = [];
    applyState(entry, timeline, 3, { keepLoops: true });
    expect(held()).toEqual(['rotate(135deg) rotate(0deg)']);
  });

  it("hides an appear shown for a while (Flash Once) when its duration ends, and reduced motion's at once", () => {
    const timeline = [rich('s1.title', { effect: 'appear', fallback: 'appear', durationMs: 500, after: 'hide' })];
    const hideOf = (reduced: boolean) => {
      const { entry } = mounted(timeline);
      applyState(entry, timeline, 1);
      recorded = [];
      playGroup(entry, timeline, 1, env({ reduced }));
      const shown = recorded.find((r) => r.keyframes.some((k) => k.opacity !== 0));
      expect(shown?.options).toMatchObject({ delay: 0, duration: 0 });
      return recorded.find((r) => r.keyframes.length === 1 && r.keyframes[0]?.opacity === 0)?.options;
    };
    expect(hideOf(false)).toMatchObject({ delay: 500, duration: 0, fill: 'forwards' });
    expect(hideOf(true)).toMatchObject({ delay: 0, duration: 0, fill: 'forwards' });
  });

  it('holds an endless repeat that holds once it stops running, never under it', () => {
    const timeline = [
      rich('s1.title', {
        kind: 'emphasis',
        fallback: 'none',
        effect: 'growShrink',
        params: { scale: { x: 2, y: 2 }, hold: true },
        repeat: 'untilEndOfSlide',
      }),
    ];
    const { entry } = mounted(timeline);
    applyState(entry, timeline, 1);
    playGroup(entry, timeline, 1, env());
    const held = () =>
      recorded.filter((r) => r.state !== 'cancelled' && r.keyframes.length === 1 && 'transform' in r.keyframes[0]!);
    applyState(entry, timeline, 2, { keepLoops: true });
    expect(entry.loops.size).toBe(1);
    expect(held()).toEqual([]);
    applyState(entry, timeline, 2);
    expect(entry.loops.size).toBe(0);
    expect(held().map((r) => r.keyframes[0]?.transform)).toEqual([expect.stringContaining('scale(2, 2)')]);
  });

  it('loops an endless tier-2 step whole while the slide shows; a cut holds where its one round ends', () => {
    // LibreOffice's blast: a scale to 1.1 that holds, its own track 300 ms of a 500 ms step.
    const blast = (repeat: string) =>
      rich('s1.title', {
        kind: 'emphasis',
        fallback: 'none',
        effect: 'behaviors',
        durationMs: 500,
        repeat,
        params: [
          {
            property: 'scale',
            beginMs: 100,
            durationMs: 300,
            frames: [
              { offset: 0, value: { x: 1, y: 1 } },
              { offset: 1, value: { x: 1.1, y: 1.1 } },
            ],
          },
        ],
      });
    const spin = rich('s1.title', {
      kind: 'emphasis',
      fallback: 'none',
      effect: 'spin',
      params: { deg: 90, cw: true, hold: false },
    });
    const held = () =>
      recorded
        .filter((r) => r.state !== 'cancelled' && r.keyframes.length === 1 && 'transform' in r.keyframes[0]!)
        .map((r) => r.keyframes[0]?.transform);
    for (const repeat of ['untilNextClick', 'untilEndOfSlide']) {
      recorded = [];
      const timeline = [blast(repeat), spin];
      const { entry } = mounted(timeline);
      applyState(entry, timeline, 1);
      expect(playGroup(entry, timeline, 1, env()), `${repeat}: nothing to wait for`).toEqual([]);
      const loop = recorded.find((r) => r.keyframes.some((k) => k.transform !== undefined));
      // One round is the whole step: the track plays from 100 to 400 ms of every 500 ms round.
      expect(loop?.options).toMatchObject({ delay: 0, duration: 500, iterations: Infinity });
      expect(loop?.keyframes.map((k) => k.offset)).toEqual([0, 0.2, 0.8, 1]);
      expect(entry.loops.size).toBe(1);
      applyState(entry, timeline, 2, { keepLoops: true });
      expect(held(), `${repeat}: no hold under the running loop`).toEqual([]);
      playGroup(entry, timeline, 2, env());
      // The next click ends an untilNextClick loop; an untilEndOfSlide loop runs on until the slide ends.
      expect(entry.loops.size, repeat).toBe(repeat === 'untilNextClick' ? 0 : 1);
      expect(loop?.state).toBe(repeat === 'untilNextClick' ? 'cancelled' : 'running');
      applyState(entry, timeline, 3, { keepLoops: true });
      expect(held().length, repeat).toBe(repeat === 'untilNextClick' ? 1 : 0);
      cutLoops(entry, timeline);
      expect(entry.loops.size).toBe(0);
      expect(loop?.state).toBe('cancelled');
      expect(held(), `${repeat}: cut, it holds its one round's end`).toEqual([
        expect.stringContaining('scale(1.1, 1.1)'),
      ]);
    }
  });

  it("plays a tier-2 step's endless repeat once in reduced motion, as an appear of nothing", () => {
    const timeline = [
      rich('s1.title', {
        kind: 'emphasis',
        fallback: 'none',
        effect: 'behaviors',
        durationMs: 500,
        repeat: 'untilEndOfSlide',
        params: [
          {
            property: 'opacity',
            beginMs: 0,
            durationMs: 500,
            frames: [
              { offset: 0, value: 1 },
              { offset: 1, value: 0.5 },
            ],
          },
        ],
      }),
    ];
    const { entry } = mounted(timeline);
    applyState(entry, timeline, 1);
    playGroup(entry, timeline, 1, env({ reduced: true }));
    expect(entry.loops.size).toBe(0);
  });

  it('runs endless repeats beside the group: not waited for, cancelled by the next click, kept to the slide end', () => {
    const timeline = [
      rich('s1.title', {
        kind: 'emphasis',
        fallback: 'none',
        effect: 'pulse',
        params: { scale: 1.2 },
        repeat: 'untilNextClick',
      }),
      rich('s1.title', {
        kind: 'emphasis',
        fallback: 'none',
        trigger: 'withPrevious',
        effect: 'teeter',
        params: { intensity: 1 },
        repeat: 'untilEndOfSlide',
      }),
      rich('s1.title', {
        kind: 'emphasis',
        fallback: 'none',
        effect: 'spin',
        params: { deg: 90, cw: true, hold: false },
      }),
    ];
    const { entry } = mounted(timeline);
    applyState(entry, timeline, 1);
    expect(playGroup(entry, timeline, 1, env())).toEqual([]);
    expect(entry.loops.size).toBe(2);
    finishAll(entry.animations);
    expect(live().length).toBe(2);
    applyState(entry, timeline, 2, { keepLoops: true });
    expect(live().length).toBe(2);
    playGroup(entry, timeline, 2, env());
    expect([...entry.loops.values()].map((step) => step.loop)).toEqual(['slide']);
    applyState(entry, timeline, 3);
    expect(entry.loops.size).toBe(0);
    expect(live().length).toBe(0);
  });

  it('staggers word units and puts the text back with the state', () => {
    const timeline = [
      rich('s1.title', { delivery: 'byWord', effect: 'fade', stagger: { perUnit: { ms: 100 }, order: 'backward' } }),
    ];
    const { entry } = mounted(timeline);
    applyState(entry, timeline, 1);
    const before = entry.root.innerHTML;
    playGroup(entry, timeline, 1, env());
    const spans = entry.root.querySelectorAll('[data-sd-unit]');
    expect(Array.from(spans, (s) => s.textContent)).toEqual(['One', 'Two']);
    expect(recorded.map((r) => r.options.delay)).toEqual([100, 0]);
    applyState(entry, timeline, 2);
    expect(entry.root.querySelectorAll('[data-sd-unit]').length).toBe(0);
    applyState(entry, timeline, 1);
    expect(entry.root.innerHTML).toBe(before);
    playGroup(entry, timeline, 1, env());
    restoreSplits(entry);
    expect(entry.root.querySelectorAll('[data-sd-unit]').length).toBe(0);
  });

  it('makes builds appear and skips emphases under reduced motion', () => {
    const timeline = [
      rich('s1.title', { effect: 'flyIn', params: { dir: 'u', distance: 'offSlide' }, durationMs: 800 }),
      rich('s1.title', {
        trigger: 'withPrevious',
        kind: 'emphasis',
        fallback: 'none',
        effect: 'spin',
        params: { deg: 90, cw: true, hold: true },
      }),
    ];
    const { entry } = mounted(timeline);
    applyState(entry, timeline, 1);
    playGroup(entry, timeline, 1, env({ reduced: true }));
    expect(recorded.length).toBe(1);
    expect(recorded[0]?.options.duration).toBe(0);
    expect(recorded[0]?.keyframes).toEqual([
      { offset: 0, opacity: 0 },
      { offset: 1, opacity: 1 },
    ]);
  });

  it('plays the fallback of an effect that needs registered properties when they are missing', () => {
    issues.length = 0;
    const timeline = [rich('s1.title', { effect: 'wheel', params: { spokes: 2, cw: true } })];
    const { entry } = mounted(timeline);
    applyState(entry, timeline, 1);
    playGroup(entry, timeline, 1, env({ properties: false }));
    expect(recorded[0]?.keyframes).toEqual([
      { offset: 0, opacity: 0 },
      { offset: 1, opacity: 1 },
    ]);
    expect(issues.map(([, issue]) => issue)).toEqual([{ kind: 'cannotPlay', name: 'wheel' }]);
    expect(el(entry, 's1.title').hasAttribute('data-sd-mask')).toBe(false);
  });

  it('marks a mask effect for its rule while it plays, and the state clears it', () => {
    const timeline = [rich('s1.title', { effect: 'clock', params: { cw: true, from: '3' } })];
    const { entry } = mounted(timeline);
    applyState(entry, timeline, 1);
    playGroup(entry, timeline, 1, env());
    expect(el(entry, 's1.title').getAttribute('data-sd-mask')).toBe('clock-3-cw');
    expect(recorded[0]?.keyframes).toEqual([
      { offset: 0, '--sd-a': '0deg' },
      { offset: 1, '--sd-a': '360deg' },
    ]);
    applyState(entry, timeline, 2);
    expect(el(entry, 's1.title').hasAttribute('data-sd-mask')).toBe(false);
  });
});

describe('runtime: state at a step', () => {
  it('fast-forwards holds as finished animations, and holds dimmed text as a plain declaration', () => {
    const timeline = [
      rich('s1.title', {
        kind: 'emphasis',
        fallback: 'none',
        effect: 'spin',
        params: { deg: 90, cw: true, hold: true },
      }),
      rich('s1.title', {
        kind: 'emphasis',
        fallback: 'none',
        effect: 'transparency',
        params: { alpha: 0.5, hold: true },
        after: { dim: '#808080' },
      }),
    ];
    const { entry } = mounted(timeline);
    applyState(entry, timeline, 2);
    const held = recorded.filter((r) => r.state === 'finished');
    expect(held.length).toBe(1);
    expect(held[0]?.keyframes[0]?.transform).toBe('rotate(90deg) rotate(0deg)');
    expect(el(entry, 's1.title').getAttribute('data-sd-paint')).toBeNull();
    recorded = [];
    applyState(entry, timeline, 3);
    const both = recorded.filter((r) => r.state === 'finished');
    expect(both[0]?.keyframes[0]).toMatchObject({ opacity: 0.5 });
    expect(both[0]?.keyframes[0]).not.toHaveProperty('--sd-anim-color');
    const title = el(entry, 's1.title') as HTMLElement;
    // The dim is held: no link (no stacking context of its own), no animation, no registered value inline.
    expect(title.getAttribute('data-sd-paint')).toBeNull();
    expect(title.getAttribute('data-sd-hold')).toBe('text');
    expect(title.style.getPropertyValue('--sd-held-text')).toBe('#808080');
    expect(title.style.getPropertyValue('--sd-anim-color')).toBe('');
    // Back at step 0, nothing is held.
    recorded = [];
    applyState(entry, timeline, 1);
    expect(recorded.length).toBe(0);
    expect(title.getAttribute('data-sd-paint')).toBeNull();
    expect(title.getAttribute('data-sd-hold')).toBeNull();
    expect(title.style.getPropertyValue('--sd-held-text')).toBe('');
  });

  it('targets a background step at the shape without its text', () => {
    const timeline = [step('s1.title', { target: { element: 's1.title', paragraphs: null, background: true } })];
    const { entry } = mounted(timeline);
    const title = el(entry, 's1.title');
    const shape = document.createElement('svg');
    title.prepend(shape);
    applyState(entry, timeline, 1);
    expect(shape.getAttribute('data-sd-state')).toBe('hidden');
    expect(title.querySelector('[data-p="0"]')?.hasAttribute('data-sd-state')).toBe(false);
  });
});
