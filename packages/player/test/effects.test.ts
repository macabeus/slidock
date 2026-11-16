// @vitest-environment happy-dom
import type { BuildEffect, StepKind, TransitionEffect } from '@slidock/types';
import { describe, expect, it } from 'vitest';

import { type BuildContext, buildRecipe, exitParams, fallbackRecipe, reverseKeyframes } from '../src/effects/builds';
import {
  barSet,
  blinds,
  cellBudget,
  checker,
  dissolve,
  dissolveGrid,
  iris,
  randomBars,
  sampleClip,
  split,
  strips,
  wipe,
} from '../src/effects/clip';
import { cssEasing, evaluate, readEasing } from '../src/effects/easing';
import { maskCss } from '../src/effects/masks';
import { flatten, samplePath } from '../src/effects/path';
import { ALLOWED_KEYFRAME_KEYS, allowedKeyframes, paintProperty } from '../src/effects/properties';
import { permutation } from '../src/effects/random';
import { type TransitionScene, transitionRecipe } from '../src/effects/transitions';
import { BUILD_EFFECTS, LIMITS, TRANSITION_EFFECTS, family, readBuild, readTransition } from '../src/effects/vocab';

const ctx: BuildContext = {
  box: { x: 100, y: 50, w: 200, h: 100 },
  slide: { width: 960, height: 540 },
  local: (dx, dy) => [dx, dy],
  opacity: 1,
  seed: 7,
  properties: true,
  cells: 288,
};

/** One valid instance of every build effect. */
const BUILDS: Record<BuildEffect['effect'], BuildEffect> = {
  appear: { effect: 'appear' },
  fade: { effect: 'fade' },
  dissolve: { effect: 'dissolve' },
  flyIn: { effect: 'flyIn', params: { dir: 'r', distance: 'offSlide' } },
  floatIn: { effect: 'floatIn', params: { dir: 'u' } },
  peek: { effect: 'peek', params: { dir: 'd', slow: false } },
  wipe: { effect: 'wipe', params: { dir: 'lu', soft: true } },
  split: { effect: 'split', params: { orient: 'horz', inOut: 'in' } },
  iris: { effect: 'iris', params: { shape: 'plus', inOut: 'in' } },
  wheel: { effect: 'wheel', params: { spokes: 3, cw: false } },
  wedge: { effect: 'wedge' },
  clock: { effect: 'clock', params: { cw: true, from: '6' } },
  blinds: { effect: 'blinds', params: { orient: 'vert' } },
  checker: { effect: 'checker', params: { dir: 'u' } },
  randomBars: { effect: 'randomBars', params: { orient: 'horz' } },
  strips: { effect: 'strips', params: { corner: 'ld' } },
  zoom: { effect: 'zoom', params: { inOut: 'out', origin: 'slideCenter', slight: false, fade: true, overshoot: true } },
  grow: { effect: 'grow', params: { axis: 'horz' } },
  expand: { effect: 'expand', params: { axis: 'both' } },
  compress: { effect: 'compress', params: { axis: 'vert' } },
  stretch: { effect: 'stretch', params: { axis: 'horz' } },
  growTurn: { effect: 'growTurn' },
  swivel: { effect: 'swivel', params: { axis: 'vert' } },
  flip: { effect: 'flip', params: { axis: 'horz' } },
  spinIn: { effect: 'spinIn', params: { deg: 720, cw: false } },
  bounce: { effect: 'bounce' },
  drop: { effect: 'drop' },
  pop: { effect: 'pop' },
  boomerang: { effect: 'boomerang' },
  sling: { effect: 'sling' },
  whip: { effect: 'whip' },
  swish: { effect: 'swish' },
  typewriter: { effect: 'typewriter' },
  credits: { effect: 'credits' },
  driftAndScale: { effect: 'driftAndScale', params: { dir: 'ld' } },
  fadeAndMove: { effect: 'fadeAndMove', params: { dir: 'r' } },
  random: { effect: 'random' },
  pulse: { effect: 'pulse', params: { scale: 1.1 } },
  growShrink: { effect: 'growShrink', params: { scale: { x: 1.5, y: 1.5 }, hold: true } },
  spin: { effect: 'spin', params: { deg: 360, cw: true, hold: false } },
  teeter: { effect: 'teeter', params: { intensity: 0.2 } },
  jiggle: { effect: 'jiggle', params: { intensity: 1 } },
  wave: { effect: 'wave', params: { intensity: 0.5 } },
  blink: { effect: 'blink', params: { repeat: 3 } },
  flash: { effect: 'flash', params: { repeat: 2 } },
  boldFlash: { effect: 'boldFlash', params: { repeat: 1 } },
  transparency: { effect: 'transparency', params: { alpha: 0.5, hold: true } },
  colorChange: { effect: 'colorChange', params: { property: 'text', to: '#ff0000' } },
  path: {
    effect: 'path',
    params: { d: [['M', 0, 0], ['C', 10, 10, 20, 10, 30, 0], ['Z']], autoRotate: true, hold: true },
  },
  play: { effect: 'play', params: { from: 500 } },
  pause: { effect: 'pause' },
  stop: { effect: 'stop' },
  toggle: { effect: 'toggle' },
  behaviors: {
    effect: 'behaviors',
    params: [
      {
        property: 'opacity',
        beginMs: 0,
        durationMs: 100,
        frames: [
          { offset: 0, value: 0 },
          { offset: 1, value: 1 },
        ],
      },
      {
        property: 'translate',
        beginMs: 0,
        durationMs: 100,
        frames: [
          { offset: 0, value: { x: 1, y: 2 } },
          { offset: 1, value: { x: 0, y: 0 } },
        ],
      },
      {
        property: 'rotate',
        beginMs: 0,
        durationMs: 100,
        easing: 'out',
        frames: [
          { offset: 0, value: 90 },
          { offset: 1, value: 0 },
        ],
      },
      {
        property: 'scale',
        beginMs: 0,
        durationMs: 100,
        frames: [
          { offset: 0, value: { x: 0, y: 0 } },
          { offset: 1, value: { x: 1, y: 1 } },
        ],
      },
      {
        property: 'blur',
        beginMs: 50,
        durationMs: 50,
        frames: [
          { offset: 0, value: 4 },
          { offset: 1, value: 0 },
        ],
      },
      { property: 'clipPath', beginMs: 0, durationMs: 100, reveal: { effect: 'dissolve' }, transition: 'in' },
    ],
  },
};

const D4 = { dir: 'l' } as const;
const TRANSITIONS: Record<TransitionEffect['effect'], TransitionEffect> = {
  none: { effect: 'none' },
  cut: { effect: 'cut', params: { throughBlack: true } },
  fade: { effect: 'fade', params: { through: 'color', color: '#123456' } },
  dissolve: { effect: 'dissolve' },
  push: { effect: 'push', params: { dir: 'u' } },
  cover: { effect: 'cover', params: { dir: 'rd' } },
  uncover: { effect: 'uncover', params: { dir: 'l' } },
  wipe: { effect: 'wipe', params: { dir: 'd', soft: true } },
  split: { effect: 'split', params: { orient: 'vert', inOut: 'out' } },
  iris: { effect: 'iris', params: { shape: 'circle', inOut: 'in' } },
  wheel: { effect: 'wheel', params: { spokes: 8, cw: true } },
  clock: { effect: 'clock', params: { cw: false, from: '12' } },
  wedge: { effect: 'wedge' },
  blinds: { effect: 'blinds', params: { orient: 'horz' } },
  checker: { effect: 'checker', params: { dir: 'l' } },
  randomBars: { effect: 'randomBars', params: { orient: 'vert' } },
  strips: { effect: 'strips', params: { corner: 'ru' } },
  comb: { effect: 'comb', params: { orient: 'vert' } },
  zoom: { effect: 'zoom', params: { inOut: 'out', rotate: true } },
  flip: { effect: 'flip', params: D4 },
  cube: { effect: 'cube', params: { dir: 'u' } },
  doors: { effect: 'doors', params: { orient: 'horz' } },
  window: { effect: 'window', params: { orient: 'vert' } },
  gallery: { effect: 'gallery', params: D4 },
  conveyor: { effect: 'conveyor', params: D4 },
  ferris: { effect: 'ferris', params: D4 },
  pan: { effect: 'pan', params: D4 },
  switch: { effect: 'switch', params: D4 },
  rotate3d: { effect: 'rotate3d', params: D4 },
  reveal: { effect: 'reveal', params: { dir: 'r', throughBlack: true } },
  flythrough: { effect: 'flythrough', params: { inOut: 'in', bounce: true } },
  swap: { effect: 'swap', params: D4 },
  pageFlip: { effect: 'pageFlip', params: D4 },
  twist: { effect: 'twist', params: D4 },
  mosaic: { effect: 'mosaic', params: D4 },
  morph: { effect: 'morph', params: { by: 'word' } },
  magicMove: { effect: 'magicMove', params: { fadeUnmatched: false } },
  random: { effect: 'random' },
};

function scene(): TransitionScene {
  const out = document.createElement('div');
  const into = document.createElement('div');
  return {
    out,
    into,
    size: { width: 960, height: 540 },
    seed: 3,
    properties: true,
    cells: 288,
    clone: () => document.createElement('div'),
    overlay: () => document.createElement('div'),
  };
}

const KINDS: StepKind[] = ['entrance', 'exit', 'emphasis', 'motion'];

describe('vocabulary', () => {
  it('reads every build and transition effect of the contract', () => {
    expect(Object.keys(BUILDS).sort()).toEqual([...BUILD_EFFECTS].sort());
    expect(Object.keys(TRANSITIONS).sort()).toEqual([...TRANSITION_EFFECTS].sort());
    for (const effect of Object.values(BUILDS)) {
      expect(readBuild(effect), effect.effect).toEqual({ effect });
    }
    for (const effect of Object.values(TRANSITIONS)) {
      expect(readTransition(effect), effect.effect).toEqual({ effect });
    }
  });

  it('refuses unknown names, unknown or missing params, wrong tokens and out-of-bound values', () => {
    const bad: [Record<string, unknown>, string][] = [
      [{ effect: 'vortex' }, 'unknown'],
      [{ effect: 'toString' }, 'unknown'],
      [{ effect: 7 }, 'unknown'],
      [{ effect: 'fade', params: {} }, 'invalid'],
      [{ effect: 'flyIn', params: { dir: 'left', distance: 'offSlide' } }, 'invalid'],
      [{ effect: 'flyIn', params: { dir: 'l' } }, 'invalid'],
      [{ effect: 'flyIn', params: { dir: 'l', distance: 'offSlide', speed: 2 } }, 'invalid'],
      [{ effect: 'wheel', params: { spokes: 9, cw: true } }, 'invalid'],
      [{ effect: 'wheel', params: { spokes: 2.5, cw: true } }, 'invalid'],
      [{ effect: 'spin', params: { deg: 3601, cw: true, hold: true } }, 'invalid'],
      [{ effect: 'spin', params: { deg: 0, cw: true, hold: true } }, 'invalid'],
      [{ effect: 'blink', params: { repeat: 1001 } }, 'invalid'],
      [{ effect: 'pulse', params: { scale: Number.NaN } }, 'invalid'],
      [{ effect: 'growShrink', params: { scale: { x: 101, y: 1 }, hold: true } }, 'invalid'],
      [{ effect: 'colorChange', params: { property: 'text', to: '#FF0000' } }, 'invalid'],
      [{ effect: 'path', params: { d: [['L', 1, 1]], autoRotate: false, hold: false } }, 'invalid'],
      [
        {
          effect: 'path',
          params: {
            d: [
              ['M', 0, 0],
              ['Q', 1, 1, 2, 2],
            ],
            autoRotate: false,
            hold: false,
          },
        },
        'invalid',
      ],
      [{ effect: 'behaviors', params: [] }, 'invalid'],
      [{ effect: 'behaviors', params: [{ property: 'color', beginMs: 0, durationMs: 1, frames: [] }] }, 'invalid'],
      [
        {
          effect: 'behaviors',
          params: [
            {
              property: 'opacity',
              beginMs: 0,
              durationMs: 1,
              frames: [
                { offset: 0.5, value: 0 },
                { offset: 1, value: 1 },
              ],
            },
          ],
        },
        'invalid',
      ],
    ];
    for (const [value, unread] of bad) {
      const read = readBuild(value);
      expect('unread' in read ? read.unread : 'read', JSON.stringify(value)).toBe(unread);
    }
    expect(readBuild({})).toEqual({ unread: 'absent', name: null });
    expect('unread' in readTransition({ effect: 'fade', params: { through: 'color' } })).toBe(true);
    expect('unread' in readTransition({ effect: 'fade', params: { through: 'black', color: '#000000' } })).toBe(true);
  });

  it('bounds paths and tier-2 tracks at the IR limits', () => {
    const segments = (n: number) => [['M', 0, 0], ...Array.from({ length: n - 1 }, () => ['L', 1, 1])];
    const path = (n: number) =>
      readBuild({ effect: 'path', params: { d: segments(n), autoRotate: false, hold: false } });
    expect('effect' in path(LIMITS.pathSegments)).toBe(true);
    expect('effect' in path(LIMITS.pathSegments + 1)).toBe(false);
    const track = {
      property: 'opacity',
      beginMs: 0,
      durationMs: 1,
      frames: [
        { offset: 0, value: 0 },
        { offset: 1, value: 1 },
      ],
    };
    expect('effect' in readBuild({ effect: 'behaviors', params: Array(LIMITS.tracks).fill(track) })).toBe(true);
    expect('effect' in readBuild({ effect: 'behaviors', params: Array(LIMITS.tracks + 1).fill(track) })).toBe(false);
  });
});

describe('recipes', () => {
  it('animate only the allowed properties, for every effect in every kind it plays in', () => {
    for (const effect of Object.values(BUILDS)) {
      for (const kind of KINDS) {
        const recipe = buildRecipe(effect, kind, ctx);
        for (const frames of recipe?.frames ?? []) {
          expect(allowedKeyframes(frames.keyframes), `${effect.effect} ${kind}`).toBe(true);
        }
      }
    }
    for (const effect of Object.values(TRANSITIONS)) {
      for (const part of transitionRecipe(effect, scene())?.parts ?? []) {
        expect(allowedKeyframes(part.keyframes), effect.effect).toBe(true);
      }
    }
    // Negative control: a layout property is refused.
    expect(allowedKeyframes([{ left: '0px' }])).toBe(false);
    expect(ALLOWED_KEYFRAME_KEYS.has('color')).toBe(false);
  });

  it('gives every entrance-family effect a recipe, and media none', () => {
    for (const effect of Object.values(BUILDS)) {
      const f = family(effect);
      const recipe = buildRecipe(effect, f === 'motion' ? 'motion' : f === 'media' ? 'media' : 'entrance', ctx);
      expect(recipe === null, effect.effect).toBe(f === 'media');
    }
  });

  it("plays an entrance effect as an emphasis only as Keynote's action, from the shown element and back to it", () => {
    const transforms = (effect: BuildEffect) =>
      (buildRecipe(effect, 'emphasis', ctx)?.frames ?? []).map((frames) =>
        frames.keyframes.map((k) => [k.offset, k.transform, k.opacity]),
      );
    // Pop: up to 115 % and back.
    expect(transforms(BUILDS.pop)).toEqual([
      [
        [0, 'scale(1)', undefined],
        [0.7, 'scale(1.15)', undefined],
        [1, 'scale(1)', undefined],
      ],
    ]);
    // Bounce: the entrance's two bounces after it lands (a quarter and 8 % of its height), over the whole step.
    expect(transforms(BUILDS.bounce)).toEqual([
      [
        [0, 'translate(0px, 0px)', undefined],
        [0.27, 'translate(0px, -25px)', undefined],
        [0.55, 'translate(0px, 0px)', undefined],
        [0.75, 'translate(0px, -8px)', undefined],
        [1, 'translate(0px, 0px)', undefined],
      ],
    ]);
    // Flip: a whole turn about its axis.
    expect(transforms({ effect: 'flip', params: { axis: 'vert' } })).toEqual([
      [
        [0, 'perspective(800px) rotate3d(1, 0, 0, 0deg)', undefined],
        [1, 'perspective(800px) rotate3d(1, 0, 0, 360deg)', undefined],
      ],
    ]);
    // Every action starts and ends where the element shows (a whole turn is no turn), moves in between and never
    // fades it; every other entrance effect has no action, and plays its fallback.
    const still = (t: unknown) => String(t).replace(/(-?[\d.]+)deg/g, (_, deg: string) => `${Number(deg) % 360}deg`);
    const actions: string[] = [];
    for (const effect of Object.values(BUILDS)) {
      if (family(effect) !== 'entrance') {
        continue;
      }
      const recipe = buildRecipe(effect, 'emphasis', ctx);
      if (recipe === null) {
        continue;
      }
      actions.push(effect.effect);
      for (const frames of recipe.frames) {
        const [first, ...rest] = frames.keyframes;
        const last = rest.at(-1);
        expect([first?.offset, last?.offset], effect.effect).toEqual([0, 1]);
        expect(still(first?.transform), effect.effect).toBe(still(last?.transform));
        expect(
          rest.some((k) => k.transform !== first?.transform),
          `${effect.effect} moves`,
        ).toBe(true);
        expect(
          frames.keyframes.every((k) => k.opacity === undefined),
          effect.effect,
        ).toBe(true);
      }
    }
    expect(actions.sort()).toEqual(['bounce', 'flip', 'pop']);
    expect(buildRecipe(BUILDS.pop, 'entrance', ctx)?.frames[0]?.keyframes[0]).toMatchObject({ transform: 'scale(0)' });
  });

  it('plays conic masks only where the registered properties work', () => {
    for (const name of ['wheel', 'clock', 'wedge'] as const) {
      expect(buildRecipe(BUILDS[name], 'entrance', { ...ctx, properties: false }), name).toBeNull();
      expect(transitionRecipe(TRANSITIONS[name], { ...scene(), properties: false }), name).toBeNull();
    }
    // A soft wipe falls back to a hard one.
    const soft = buildRecipe(BUILDS.wipe, 'entrance', { ...ctx, properties: false });
    expect(soft?.frames[0]?.mask).toBeUndefined();
    expect(soft?.frames[0]?.keyframes[0]?.clipPath).toBeDefined();
    expect(maskCss('')).toContain('[data-sd-mask="wheel-3-ccw"]');
  });

  it('changes a colour through its registered property, on what paints it', () => {
    for (const property of ['text', 'fill', 'line'] as const) {
      const effect: BuildEffect = { effect: 'colorChange', params: { property, to: '#ff0000' } };
      // The player plays these frames on the elements that paint `property`, or the fallback where none does.
      expect(buildRecipe(effect, 'emphasis', ctx)?.frames).toEqual([
        { keyframes: [{ offset: 1, [paintProperty(property)]: '#ff0000' }], paint: property },
      ]);
    }
    expect(paintProperty('text')).toBe('--sd-anim-color');
    expect(paintProperty('fill')).toBe('--sd-anim-fill');
  });

  it('plays colour tracks interpolated and font tracks switched, each on its hook', () => {
    const color = (property: 'text' | 'fill' | 'line') => ({
      property,
      beginMs: 0,
      durationMs: 400,
      frames: [
        { offset: 0, value: '#000000' },
        { offset: 0.5, value: '#ff0000' },
        { offset: 1, value: '#00ff00' },
      ],
    });
    const font = (property: 'bold' | 'italic' | 'underline') => ({
      property,
      beginMs: 100,
      durationMs: 400,
      frames: [
        { offset: 0, value: true },
        { offset: 0.5, value: false },
        { offset: 1, value: true },
      ],
    });
    const tracks = [
      color('text'),
      color('fill'),
      color('line'),
      font('bold'),
      font('italic'),
      font('underline'),
      {
        property: 'opacity',
        beginMs: 0,
        durationMs: 400,
        frames: [
          { offset: 0, value: 1 },
          { offset: 1, value: 0.5 },
        ],
      },
    ];
    const read = readBuild({ effect: 'behaviors', params: tracks });
    expect('effect' in read).toBe(true);
    const effect = { effect: 'behaviors', params: tracks } as unknown as BuildEffect;
    const recipe = buildRecipe(effect, 'emphasis', ctx);
    expect(recipe?.frames.map((f) => f.paint)).toEqual([
      'text',
      'fill',
      'line',
      'bold',
      'italic',
      'underline',
      undefined,
    ]);
    expect(recipe?.frames[1]?.keyframes).toEqual([
      { offset: 0, '--sd-anim-fill': '#000000' },
      { offset: 0.5, '--sd-anim-fill': '#ff0000' },
      { offset: 1, '--sd-anim-fill': '#00ff00' },
    ]);
    // A font switch holds each value until the next frame.
    expect(recipe?.frames[3]?.keyframes).toEqual([
      { offset: 0, '--sd-anim-bold': 'bold', easing: 'steps(1, end)' },
      { offset: 0.5, '--sd-anim-bold': 'normal', easing: 'steps(1, end)' },
      { offset: 1, '--sd-anim-bold': 'bold', easing: 'steps(1, end)' },
    ]);
    expect(recipe?.frames[4]?.keyframes[0]).toMatchObject({ '--sd-anim-italic': 'italic' });
    expect(recipe?.frames[5]?.keyframes[1]).toMatchObject({ '--sd-anim-underline': 'none' });
    expect(recipe?.frames[3]).toMatchObject({ beginMs: 100, durationMs: 400 });
    for (const frames of recipe?.frames ?? []) {
      expect(allowedKeyframes(frames.keyframes)).toBe(true);
    }
  });

  it('reads colour tracks as #rrggbb and font tracks as booleans, failing closed otherwise', () => {
    const track = (property: string, a: unknown, b: unknown) => ({
      effect: 'behaviors',
      params: [
        {
          property,
          beginMs: 0,
          durationMs: 100,
          frames: [
            { offset: 0, value: a },
            { offset: 1, value: b },
          ],
        },
      ],
    });
    expect('effect' in readBuild(track('fill', '#000000', '#a0b1c2'))).toBe(true);
    expect('effect' in readBuild(track('underline', false, true))).toBe(true);
    // An HSL shift is resolved by the deck builder; a deck never holds one.
    expect(readBuild(track('fill', '#000000', { h: 10, s: 0, l: 0 }))).toMatchObject({ unread: 'invalid' });
    expect(readBuild(track('line', 'red', '#000000'))).toMatchObject({ unread: 'invalid' });
    expect(readBuild(track('text', '#FFFFFF', '#000000'))).toMatchObject({ unread: 'invalid' });
    expect(readBuild(track('bold', 1, 0))).toMatchObject({ unread: 'invalid' });
    expect(readBuild(track('fontSize', 10, 20))).toMatchObject({ unread: 'invalid' });
  });

  it('reads a switch of one paint, font or opacity with its value, failing closed otherwise', () => {
    const set = (to: unknown, extra: object = {}) => ({
      effect: 'behaviors',
      params: [{ property: 'set', beginMs: 100, durationMs: 0, to, ...extra }],
    });
    for (const to of [
      { property: 'bold', value: true },
      { property: 'underline', value: false },
      { property: 'fill', value: '#a0b1c2' },
      { property: 'opacity', value: 0.5 },
    ]) {
      expect('effect' in readBuild(set(to)), JSON.stringify(to)).toBe(true);
    }
    for (const [to, extra] of [
      [{ property: 'bold', value: 1 }, {}],
      [{ property: 'text', value: 'red' }, {}],
      [{ property: 'opacity', value: 2 }, {}],
      // A transform is a track, never a switch.
      [{ property: 'translate', value: { x: 0, y: 0 } }, {}],
      [{ property: 'set', value: true }, {}],
      [{ property: 'bold', value: true, extra: 1 }, {}],
      [{ property: 'bold' }, {}],
      [null, {}],
      [{ property: 'bold', value: true }, { frames: [] }],
    ] as const) {
      expect(readBuild(set(to, extra)), JSON.stringify([to, extra])).toMatchObject({ unread: 'invalid' });
    }
    expect(readBuild({ effect: 'behaviors', params: [{ property: 'set', beginMs: 0, durationMs: 0 }] })).toMatchObject({
      unread: 'invalid',
    });
  });

  it('plays a switch at its begin with no backward fill, and holds it', () => {
    const tracks = [
      { property: 'set', beginMs: 100, durationMs: 400, to: { property: 'bold', value: true } },
      { property: 'set', beginMs: 0, durationMs: 0, to: { property: 'opacity', value: 0.5 } },
    ];
    const effect = { effect: 'behaviors', params: tracks } as unknown as BuildEffect;
    const recipe = buildRecipe(effect, 'emphasis', { ...ctx, opacity: 0.8 });
    expect(recipe?.frames[0]).toEqual({
      keyframes: [
        { offset: 0, '--sd-anim-bold': 'bold', easing: 'steps(1, end)' },
        { offset: 1, '--sd-anim-bold': 'bold', easing: 'steps(1, end)' },
      ],
      paint: 'bold',
      fill: 'forwards',
      beginMs: 100,
      durationMs: 400,
    });
    expect(recipe?.frames[1]).toMatchObject({
      keyframes: [
        { offset: 0, opacity: 0.4 },
        { offset: 1, opacity: 0.4 },
      ],
      fill: 'forwards',
    });
    // A track keeps the step's fill.
    const track = {
      property: 'opacity',
      beginMs: 0,
      durationMs: 100,
      frames: [
        { offset: 0, value: 1 },
        { offset: 1, value: 0 },
      ],
    };
    expect(
      buildRecipe({ effect: 'behaviors', params: [track] } as unknown as BuildEffect, 'emphasis', ctx)?.frames[0]?.fill,
    ).toBeUndefined();
  });

  it("reads only the vocabulary's own names: a name on Object.prototype fails closed", () => {
    const frames = [
      { offset: 0, value: 0 },
      { offset: 1, value: 1 },
    ];
    for (const property of ['constructor', 'toString', '__proto__', 'hasOwnProperty', 'valueOf']) {
      const track = { property, beginMs: 0, durationMs: 100, frames };
      expect(readBuild({ effect: 'behaviors', params: [track] }), property).toMatchObject({ unread: 'invalid' });
      const set = { property: 'set', beginMs: 0, durationMs: 0, to: { property, value: true } };
      expect(readBuild({ effect: 'behaviors', params: [set] }), `set ${property}`).toMatchObject({ unread: 'invalid' });
      const clip = {
        property: 'clipPath',
        beginMs: 0,
        durationMs: 100,
        transition: 'in',
        reveal: { effect: property, params: {} },
      };
      expect(readBuild({ effect: 'behaviors', params: [clip] }), `reveal ${property}`).toMatchObject({
        unread: 'invalid',
      });
      // A key the shape does not name fails, even one every object inherits.
      expect(readBuild({ effect: 'spin', params: { deg: 90, cw: true, hold: false, [property]: 1 } })).toMatchObject({
        unread: 'invalid',
      });
      expect(readBuild({ effect: property }), `effect ${property}`).toMatchObject({ unread: 'unknown' });
    }
  });

  it('never returns a recipe that plays nothing for an effect it knows', () => {
    for (const effect of Object.values(BUILDS)) {
      for (const kind of KINDS) {
        for (const properties of [true, false]) {
          const recipe = buildRecipe(effect, kind, { ...ctx, properties });
          if (recipe !== null) {
            expect(recipe.frames.length, `${effect.effect} ${kind} ${properties}`).toBeGreaterThan(0);
          }
        }
      }
    }
  });

  it("adds a filter effect to the element's own filter, so its printed shadow stays", () => {
    const filters = (effect: BuildEffect) =>
      (buildRecipe(effect, 'emphasis', ctx)?.frames ?? []).filter((frames) =>
        frames.keyframes.some((keyframe) => keyframe.filter !== undefined),
      );
    for (const effect of [BUILDS.boldFlash, BUILDS.behaviors]) {
      const found = filters(effect);
      expect(found.length, effect.effect).toBe(1);
      expect(found[0]?.composite, effect.effect).toBe('add');
    }
  });

  it('turns a page about its leading edge, so it travels along dir', () => {
    const last = (dir: 'l' | 'r' | 'u' | 'd') => {
      const s = scene();
      const part = transitionRecipe({ effect: 'pageFlip', params: { dir } }, s)?.parts.find((p) => p.target === s.out);
      return String(part?.keyframes.at(-1)?.transform);
    };
    // Moving left, the spine is the left edge and the free right edge swings towards the viewer.
    expect(last('l')).toContain('translateX(-480px) rotateY(-90deg)');
    expect(last('r')).toContain('translateX(480px) rotateY(90deg)');
    expect(last('u')).toContain('translateY(-270px) rotateX(90deg)');
    expect(last('d')).toContain('translateY(270px) rotateX(-90deg)');
  });

  it('plays an exit as the reversed entrance with the opposite direction', () => {
    expect(exitParams({ effect: 'flyIn', params: { dir: 'lu', distance: 'slight' } })).toEqual({
      effect: 'flyIn',
      params: { dir: 'rd', distance: 'slight' },
    });
    expect(exitParams({ effect: 'strips', params: { corner: 'ru' } })).toEqual({
      effect: 'strips',
      params: { corner: 'ld' },
    });
    expect(exitParams({ effect: 'floatIn', params: { dir: 'u' } })).toEqual({
      effect: 'floatIn',
      params: { dir: 'd' },
    });
    // Other params name the entrance being reversed and stay.
    expect(exitParams(BUILDS.zoom)).toEqual(BUILDS.zoom);
    const exit = buildRecipe({ effect: 'flyIn', params: { dir: 'l', distance: 'offSlide' } }, 'exit', ctx);
    const entrance = buildRecipe({ effect: 'flyIn', params: { dir: 'r', distance: 'offSlide' } }, 'entrance', ctx);
    expect(exit?.frames[0]?.keyframes).toEqual(reverseKeyframes(entrance?.frames[0]?.keyframes ?? []));
    // Flying out to the left ends left of the slide.
    expect(exit?.frames[0]?.keyframes.at(-1)?.transform).toBe('translate(-302px, 0px)');
  });

  it('starts every entrance hidden and ends it on the static render', () => {
    const fade = buildRecipe(BUILDS.fade, 'entrance', { ...ctx, opacity: 0.6 });
    expect(fade?.frames[0]?.keyframes).toEqual([
      { offset: 0, opacity: 0 },
      { offset: 1, opacity: 0.6 },
    ]);
    const wipeIn = buildRecipe(BUILDS.split, 'entrance', ctx)?.frames[0]?.keyframes ?? [];
    expect(wipeIn.at(-1)).toEqual({ offset: 1, clipPath: 'none' });
    const fly = buildRecipe(BUILDS.flyIn, 'entrance', ctx)?.frames[0]?.keyframes ?? [];
    expect(fly[0]?.transform).toBe('translate(-302px, 0px)');
    expect(fly.at(-1)?.transform).toBe('translate(0px, 0px)');
  });

  it('keeps the reversed first keyframe at 0, where WAAPI would read the duplicate', () => {
    const reversed = reverseKeyframes([
      { offset: 0, clipPath: 'inset(0 100% 0 0)' },
      { offset: 1, clipPath: 'inset(0 0 0 0)' },
      { offset: 1, clipPath: 'none' },
    ]);
    expect(reversed.map((k) => [k.offset, k.clipPath])).toEqual([
      [0, 'none'],
      [0.001, 'inset(0 0 0 0)'],
      [1, 'inset(0 100% 0 0)'],
    ]);
  });

  it('plays the canonical fallbacks: appear switches, dissolve fades, none plays nothing', () => {
    expect(fallbackRecipe('dissolve', 'entrance', ctx).frames[0]?.keyframes).toEqual([
      { offset: 0, opacity: 0 },
      { offset: 1, opacity: 1 },
    ]);
    expect(fallbackRecipe('dissolve', 'exit', ctx).frames[0]?.keyframes).toEqual([
      { offset: 0, opacity: 1 },
      { offset: 1, opacity: 0 },
    ]);
    expect(fallbackRecipe('none', 'entrance', ctx).frames).toEqual([]);
    expect(fallbackRecipe('dissolve', 'emphasis', ctx).frames).toEqual([]);
  });
});

describe('clip generators', () => {
  const vertices = (clip: string) => clip.split(',').length;

  it('keep a constant vertex count across samples, so WAAPI interpolates them', () => {
    for (const clip of [
      blinds('horz'),
      randomBars('vert', 1),
      checker('r'),
      strips('rd'),
      dissolve(8, 4, 5),
      split('vert', 'in'),
      iris('circle', 'in', 16 / 9),
      iris('plus', 'out', 1),
      wipe('ru'),
    ]) {
      const counts = new Set(sampleClip(clip).map((frame) => vertices(frame.clipPath)));
      expect(counts.size).toBe(1);
    }
  });

  it('reveal from the trailing side of the motion', () => {
    expect(wipe('r').at(0.25)).toBe('inset(0% 75% 0% 0%)');
    expect(wipe('l').at(0.25)).toBe('inset(0% 0% 0% 75%)');
    expect(wipe('d').at(0.5)).toBe('inset(0% 0% 50% 0%)');
    expect(wipe('rd').at(0)).toBe('polygon(0% 0%, 0% 0%, 0% 0%, 0% 0%, 0% 0%)');
    expect(wipe('lu').at(1)).toBe('polygon(100% 100%, 0% 100%, 0% 0%, 0% 0%, 100% 0%)');
  });

  it('collapse unrevealed bars and join bars with zero-width bridges', () => {
    expect(
      barSet([
        [0, 0, 10, 10],
        [20, 0, 30, 10],
      ]),
    ).toBe('polygon(0% 0%, 10% 0%, 10% 10%, 0% 10%, 0% 0%, 20% 0%, 30% 0%, 30% 10%, 20% 10%, 20% 0%, 0% 0%)');
    expect(blinds('horz', 2).at(0)).toBe(
      barSet([
        [0, 0, 100, 0],
        [0, 50, 100, 50],
      ]),
    );
  });

  it('order random bars and dissolve cells from the seed', () => {
    expect(randomBars('horz', 9).at(0.3)).toBe(randomBars('horz', 9).at(0.3));
    expect(randomBars('horz', 9).at(0.3)).not.toBe(randomBars('horz', 10).at(0.3));
    expect(permutation(5, 1)).toEqual(permutation(5, 1));
  });

  it('cap dissolve cells by device', () => {
    expect(cellBudget(2)).toBe(128);
    expect(cellBudget(4)).toBe(288);
    expect(cellBudget(16)).toBe(576);
    expect(cellBudget(undefined)).toBe(288);
    const { cols, rows } = dissolveGrid(16 / 9, 576);
    expect([cols, rows]).toEqual([32, 18]);
    const small = dissolveGrid(16 / 9, 128);
    expect(small.cols * small.rows).toBeLessThanOrEqual(128);
    const odd = dissolveGrid(Number.NaN, 10_000);
    expect(odd.cols * odd.rows).toBeLessThanOrEqual(576);
  });
});

describe('easing and paths', () => {
  it('maps the IR curves to CSS', () => {
    expect(cssEasing(readEasing('in-out') ?? { kind: 'accelDecel', accel: 0, decel: 0 })).toBe(
      'cubic-bezier(0.42, 0, 0.58, 1)',
    );
    expect(cssEasing(readEasing({ bezier: [0.25, 0.1, 0.25, 1] }) ?? { kind: 'accelDecel', accel: 0, decel: 0 })).toBe(
      'cubic-bezier(0.25, 0.1, 0.25, 1)',
    );
    expect(readEasing('ease')).toBeNull();
    expect(readEasing({ bezier: [1.5, 0, 0, 1] })).toBeNull();
    expect(readEasing({ accelDecel: { accel: 0.7, decel: 0.5 } })).toBeNull();
  });

  it('accelerates and decelerates through three segments', () => {
    const curve = { kind: 'accelDecel', accel: 0.2, decel: 0.3 } as const;
    expect(evaluate(curve, 0)).toBe(0);
    expect(evaluate(curve, 1)).toBe(1);
    let last = 0;
    for (let i = 1; i <= 100; i++) {
      const v = evaluate(curve, i / 100);
      expect(v).toBeGreaterThanOrEqual(last);
      last = v;
    }
    // The middle runs at constant speed 1 / (1 - (a + d) / 2).
    const speed = 1 / (1 - 0.25);
    expect(evaluate(curve, 0.5) - evaluate(curve, 0.4)).toBeCloseTo(0.1 * speed);
  });

  it('flattens and resamples paths at even arc lengths', () => {
    expect(flatten([['M', 0, 0], ['L', 10, 0], ['Z']])).toEqual([
      { x: 0, y: 0 },
      { x: 10, y: 0 },
      { x: 0, y: 0 },
    ]);
    const samples = samplePath(
      [
        ['M', 0, 0],
        ['L', 100, 0],
        ['L', 100, 100],
      ],
      4,
    );
    expect(samples.map((s) => [s.x, s.y, s.angle])).toEqual([
      [0, 0, 0],
      [50, 0, 0],
      [100, 0, 0],
      [100, 50, 90],
      [100, 100, 90],
    ]);
  });
});
