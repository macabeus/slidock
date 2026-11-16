// Every effect of the vocabulary (docs/format.md §5.2) with the params the suite plays it with: one entry per effect,
// plus the variants that exercise each direction, shape and mode.
import type { BuildEffect, Keyframes, StepKind, TransitionEffect } from '@slidock/types';

/**
 * The effect suite's tests whose middle frame carries a projective transform (perspective) where it is measured:
 * WebKit measures those by geometry, and `perspective.spec.ts` checks that the page draws that geometry.
 */
export const PROJECTIVE: readonly string[] = [
  ...['swivel', 'flip'].flatMap((effect) =>
    ['horz', 'vert', 'both'].flatMap((axis) => [`build entrance ${effect}-${axis}`, `build exit ${effect}-${axis}`]),
  ),
  ...['flip-l', 'flip-u', 'cube-r', 'cube-d', 'gallery', 'rotate3d', 'twist'].map((name) => `transition ${name}`),
];

/** A side of the stage or of an element, in the deck's direction tokens. */
export type Side = 'l' | 'r' | 'u' | 'd' | 'lu' | 'ru' | 'ld' | 'rd';

/** The side opposite `side`. */
export function opposite(side: Side): Side {
  const flip: Record<string, string> = { l: 'r', r: 'l', u: 'd', d: 'u' };
  return side.replace(/[lrud]/g, (c) => flip[c] ?? c) as Side;
}

/**
 * What a frame partway through an effect must show, independent of the recipe. `dir` is the direction the content
 * moves, so an entrance's content still sits on the side it comes from, `opposite(dir)`.
 * - `side`: where the visible content (a build) or the incoming slide (a transition) sits, relative to where it
 *   ends;
 * - `spread`: whether it gathers at the centre or at the edges of where it ends;
 * - `axis`: the axis it is squeezed or stretched along (the other keeps its extent).
 * An exit is the reversed entrance with the opposite direction: a direction (`dir`, `corner`) or a
 * delivery order shows it on the other side, while the other params keep the entrance's picture.
 */
export interface Mid {
  readonly side?: Side;
  readonly spread?: 'center' | 'edges';
  readonly axis?: 'horz' | 'vert' | 'both';
  /** The side the exit shows when it is not the entrance's (or its opposite, by the rule above). */
  readonly exitSide?: Side;
  /** When 50% shows nothing to measure (a face edge-on, still off-stage): the fraction to measure at. */
  readonly at?: number;
}

/** What an emphasis or motion step shows on its target's computed style at 50%. */
export interface Pose {
  readonly rotate?: 'cw' | 'ccw';
  /** The side the target has moved to. */
  readonly moved?: Side;
  readonly scaleX?: 'grows' | 'shrinks';
  readonly scaleY?: 'grows' | 'shrinks';
  readonly fades?: boolean;
  readonly recolors?: boolean;
  readonly filters?: boolean;
}

export interface BuildCase {
  readonly name: string;
  readonly effect: BuildEffect;
  readonly kind: StepKind;
  /** The element the step builds. */
  readonly target?: 'box' | 'text' | 'rot';
  readonly delivery?: 'all' | 'byParagraph' | 'byWord' | 'byChar';
  /** The middle frame shows no motion (an instant switch). */
  readonly still?: boolean;
  /** Its first unit switches at time 0, so the first frame already shows it. */
  readonly instantStart?: boolean;
  /** Where the middle frame and the pose are taken, for an effect that is back at its start at 50% (default 0.5). */
  readonly middle?: number;
  readonly mid?: Mid;
  readonly pose?: Pose;
  /** It plays its canonical fallback (with an RT0006 diagnostic) instead of a recipe. */
  readonly fallback?: boolean;
}

const zoom = (
  inOut: 'in' | 'out',
  extra: Partial<Extract<BuildEffect, { effect: 'zoom' }>['params']> = {},
): BuildEffect => ({
  effect: 'zoom',
  params: { inOut, origin: 'object', slight: false, fade: false, overshoot: false, ...extra },
});

/** Entrance-family effects; each also plays as an exit. */
export const ENTRANCES: readonly BuildCase[] = [
  { name: 'appear', effect: { effect: 'appear' }, kind: 'entrance', still: true, instantStart: true },
  { name: 'fade', effect: { effect: 'fade' }, kind: 'entrance' },
  { name: 'dissolve', effect: { effect: 'dissolve' }, kind: 'entrance' },
  ...(['l', 'r', 'u', 'd', 'lu', 'ru', 'ld', 'rd'] as const).map((dir): BuildCase => ({
    name: `flyIn-${dir}`,
    effect: { effect: 'flyIn', params: { dir, distance: 'offSlide' } },
    kind: 'entrance',
    mid: { side: opposite(dir) },
  })),
  {
    name: 'flyIn-slight',
    effect: { effect: 'flyIn', params: { dir: 'u', distance: 'slight' } },
    kind: 'entrance',
    mid: { side: 'd' },
  },
  {
    name: 'flyIn-rotated',
    effect: { effect: 'flyIn', params: { dir: 'r', distance: 'offSlide' } },
    kind: 'entrance',
    target: 'rot',
    mid: { side: 'l' },
  },
  { name: 'floatIn-u', effect: { effect: 'floatIn', params: { dir: 'u' } }, kind: 'entrance', mid: { side: 'd' } },
  { name: 'floatIn-d', effect: { effect: 'floatIn', params: { dir: 'd' } }, kind: 'entrance', mid: { side: 'u' } },
  ...(['l', 'r', 'u', 'd'] as const).map((dir): BuildCase => ({
    name: `peek-${dir}`,
    effect: { effect: 'peek', params: { dir, slow: false } },
    kind: 'entrance',
    mid: { side: opposite(dir) },
  })),
  ...(['l', 'r', 'u', 'd', 'lu', 'rd'] as const).map((dir): BuildCase => ({
    name: `wipe-${dir}`,
    effect: { effect: 'wipe', params: { dir, soft: false } },
    kind: 'entrance',
    mid: { side: opposite(dir) },
  })),
  {
    name: 'wipe-soft',
    effect: { effect: 'wipe', params: { dir: 'r', soft: true } },
    kind: 'entrance',
    mid: { side: 'l' },
  },
  ...(['horz', 'vert'] as const).flatMap((orient) =>
    (['in', 'out'] as const).map((inOut): BuildCase => ({
      name: `split-${orient}-${inOut}`,
      effect: { effect: 'split', params: { orient, inOut } },
      kind: 'entrance',
      mid: { spread: inOut === 'out' ? 'center' : 'edges' },
    })),
  ),
  ...(['circle', 'diamond', 'box', 'plus'] as const).flatMap((shape) =>
    (['in', 'out'] as const).map((inOut): BuildCase => ({
      name: `iris-${shape}-${inOut}`,
      effect: { effect: 'iris', params: { shape, inOut } },
      kind: 'entrance',
      mid: { spread: inOut === 'out' ? 'center' : 'edges' },
    })),
  ),
  // A sweep from 12 o'clock clockwise has covered the right half at 50%.
  {
    name: 'wheel-1',
    effect: { effect: 'wheel', params: { spokes: 1, cw: true } },
    kind: 'entrance',
    mid: { side: 'r' },
  },
  { name: 'wheel-4-ccw', effect: { effect: 'wheel', params: { spokes: 4, cw: false } }, kind: 'entrance' },
  { name: 'wedge', effect: { effect: 'wedge' }, kind: 'entrance', mid: { side: 'u' } },
  {
    name: 'clock-12-cw',
    effect: { effect: 'clock', params: { cw: true, from: '12' } },
    kind: 'entrance',
    mid: { side: 'r' },
  },
  {
    name: 'clock-9-ccw',
    effect: { effect: 'clock', params: { cw: false, from: '9' } },
    kind: 'entrance',
    mid: { side: 'd' },
  },
  { name: 'blinds-horz', effect: { effect: 'blinds', params: { orient: 'horz' } }, kind: 'entrance' },
  { name: 'blinds-vert', effect: { effect: 'blinds', params: { orient: 'vert' } }, kind: 'entrance' },
  { name: 'checker-r', effect: { effect: 'checker', params: { dir: 'r' } }, kind: 'entrance' },
  { name: 'checker-d', effect: { effect: 'checker', params: { dir: 'd' } }, kind: 'entrance' },
  { name: 'randomBars-horz', effect: { effect: 'randomBars', params: { orient: 'horz' } }, kind: 'entrance' },
  { name: 'randomBars-vert', effect: { effect: 'randomBars', params: { orient: 'vert' } }, kind: 'entrance' },
  // Strips move toward their corner, so they start from the opposite one.
  ...(['lu', 'ru', 'ld', 'rd'] as const).map((corner): BuildCase => ({
    name: `strips-${corner}`,
    effect: { effect: 'strips', params: { corner } },
    kind: 'entrance',
    mid: { side: opposite(corner) },
  })),
  { name: 'zoom-in', effect: zoom('in'), kind: 'entrance', mid: { spread: 'center' } },
  { name: 'zoom-out', effect: zoom('out'), kind: 'entrance', mid: { spread: 'edges' } },
  // The element sits right of the slide centre it zooms from.
  {
    name: 'zoom-slide-center',
    effect: zoom('in', { origin: 'slideCenter', fade: true }),
    kind: 'entrance',
    mid: { side: 'l', spread: 'center' },
  },
  { name: 'zoom-slight-overshoot', effect: zoom('in', { slight: true, overshoot: true }), kind: 'entrance' },
  ...(['grow', 'expand', 'compress', 'stretch', 'swivel', 'flip'] as const).flatMap((effect) =>
    (['horz', 'vert', 'both'] as const).map((axis): BuildCase => ({
      name: `${effect}-${axis}`,
      effect: { effect, params: { axis } },
      kind: 'entrance',
      mid: { axis },
    })),
  ),
  { name: 'growTurn', effect: { effect: 'growTurn' }, kind: 'entrance', mid: { spread: 'center' } },
  {
    name: 'spinIn',
    effect: { effect: 'spinIn', params: { deg: 360, cw: true } },
    kind: 'entrance',
    mid: { spread: 'center' },
  },
  // Bounce and drop come from above, credits from below, pop grows in place.
  ...(
    [
      ['bounce', { side: 'u' }],
      ['drop', { side: 'u' }],
      ['pop', { spread: 'center' }],
      ['boomerang', undefined],
      ['sling', undefined],
      ['whip', undefined],
      ['swish', undefined],
      ['credits', { side: 'd' }],
      ['random', undefined],
    ] as const
  ).map(([effect, mid]): BuildCase => ({
    name: effect,
    effect: { effect },
    kind: 'entrance',
    ...(mid ? { mid } : {}),
  })),
  // Characters type from the start of the text; an exit removes them in the same order.
  {
    name: 'typewriter',
    effect: { effect: 'typewriter' },
    kind: 'entrance',
    target: 'text',
    instantStart: true,
    mid: { side: 'u', exitSide: 'd' },
  },
  {
    name: 'driftAndScale',
    effect: { effect: 'driftAndScale', params: { dir: 'u' } },
    kind: 'entrance',
    mid: { side: 'd' },
  },
  {
    name: 'fadeAndMove',
    effect: { effect: 'fadeAndMove', params: { dir: 'l' } },
    kind: 'entrance',
    mid: { side: 'r' },
  },
  {
    name: 'behaviors',
    effect: {
      effect: 'behaviors',
      params: [
        {
          property: 'opacity',
          beginMs: 0,
          durationMs: 1000,
          frames: [
            { offset: 0, value: 0 },
            { offset: 1, value: 1 },
          ],
        },
        {
          property: 'translate',
          beginMs: 0,
          durationMs: 1000,
          frames: [
            { offset: 0, value: { x: 0, y: 60 } },
            { offset: 1, value: { x: 0, y: 0 } },
          ],
        },
        {
          property: 'clipPath',
          beginMs: 0,
          durationMs: 1000,
          reveal: { effect: 'wipe', params: { dir: 'r', soft: false } },
          transition: 'in',
        },
      ],
    },
    kind: 'entrance',
    // Rising from below while a wipe reveals it from the left.
    mid: { side: 'ld' },
  },
  { name: 'fade-byParagraph', effect: { effect: 'fade' }, kind: 'entrance', target: 'text', delivery: 'byParagraph' },
  {
    name: 'flyIn-byWord',
    effect: { effect: 'flyIn', params: { dir: 'u', distance: 'slight' } },
    kind: 'entrance',
    target: 'text',
    delivery: 'byWord',
  },
  { name: 'fade-byChar', effect: { effect: 'fade' }, kind: 'entrance', target: 'text', delivery: 'byChar' },
  // Tier-2 reveals drawn with a mask: hidden at first, uncovered from the left (a soft wipe moving right), or around
  // the centre (a wheel).
  {
    name: 'behaviors-soft-wipe',
    effect: { effect: 'behaviors', params: [reveal({ effect: 'wipe', params: { dir: 'r', soft: true } })] },
    kind: 'entrance',
    mid: { side: 'l' },
  },
  {
    name: 'behaviors-wheel',
    effect: { effect: 'behaviors', params: [reveal({ effect: 'wheel', params: { spokes: 1, cw: true } })] },
    kind: 'entrance',
  },
];

function reveal(effect: Extract<Keyframes, { property: 'clipPath' }>['reveal']): Keyframes {
  return { property: 'clipPath', beginMs: 0, durationMs: 1000, reveal: effect, transition: 'in' };
}

/** A tier-2 exit: its tracks fade and slide the target out. */
export const EXIT_BEHAVIORS: BuildCase = {
  name: 'behaviors',
  effect: {
    effect: 'behaviors',
    params: [
      {
        property: 'opacity',
        beginMs: 0,
        durationMs: 1000,
        frames: [
          { offset: 0, value: 1 },
          { offset: 1, value: 0 },
        ],
      },
      {
        property: 'rotate',
        beginMs: 200,
        durationMs: 800,
        frames: [
          { offset: 0, value: 0 },
          { offset: 1, value: 45 },
        ],
      },
      {
        property: 'clipPath',
        beginMs: 0,
        durationMs: 1000,
        reveal: { effect: 'iris', params: { shape: 'circle', inOut: 'out' } },
        transition: 'out',
      },
    ],
  },
  kind: 'exit',
};

/** Exits that play an emphasis recipe (PowerPoint's Flash Once as an exit): it plays, then the target hides. */
export const EMPHASIS_EXITS: readonly BuildCase[] = [
  // Two blinks are back on at 50%; a quarter of the way is the first one's dark half.
  { name: 'blink-exit', effect: { effect: 'blink', params: { repeat: 2 } }, kind: 'exit', middle: 0.25 },
  {
    name: 'spin-exit',
    effect: { effect: 'spin', params: { deg: 90, cw: true, hold: false } },
    kind: 'exit',
    pose: { rotate: 'cw' },
  },
];

/** Emphasis and motion effects. */
export const EMPHASES: readonly BuildCase[] = [
  {
    name: 'pulse',
    effect: { effect: 'pulse', params: { scale: 1.2 } },
    kind: 'emphasis',
    pose: { scaleX: 'grows', scaleY: 'grows' },
  },
  {
    name: 'growShrink',
    effect: { effect: 'growShrink', params: { scale: { x: 1.5, y: 0.5 }, hold: false } },
    kind: 'emphasis',
    pose: { scaleX: 'grows', scaleY: 'shrinks' },
  },
  {
    name: 'growShrink-hold',
    effect: { effect: 'growShrink', params: { scale: { x: 1.5, y: 0.5 }, hold: true } },
    kind: 'emphasis',
    pose: { scaleX: 'grows', scaleY: 'shrinks' },
  },
  {
    name: 'spin',
    effect: { effect: 'spin', params: { deg: 90, cw: true, hold: false } },
    kind: 'emphasis',
    pose: { rotate: 'cw' },
  },
  {
    name: 'spin-hold',
    effect: { effect: 'spin', params: { deg: 90, cw: false, hold: true } },
    kind: 'emphasis',
    pose: { rotate: 'ccw' },
  },
  { name: 'teeter', effect: { effect: 'teeter', params: { intensity: 0.5 } }, kind: 'emphasis' },
  { name: 'jiggle', effect: { effect: 'jiggle', params: { intensity: 0.5 } }, kind: 'emphasis' },
  { name: 'wave', effect: { effect: 'wave', params: { intensity: 0.5 } }, kind: 'emphasis' },
  { name: 'blink', effect: { effect: 'blink', params: { repeat: 2 } }, kind: 'emphasis', middle: 0.25 },
  { name: 'flash', effect: { effect: 'flash', params: { repeat: 1 } }, kind: 'emphasis', pose: { fades: true } },
  {
    name: 'boldFlash',
    effect: { effect: 'boldFlash', params: { repeat: 1 } },
    kind: 'emphasis',
    target: 'text',
    pose: { filters: true },
  },
  {
    name: 'transparency',
    effect: { effect: 'transparency', params: { alpha: 0.3, hold: false } },
    kind: 'emphasis',
    pose: { fades: true },
  },
  {
    name: 'transparency-hold',
    effect: { effect: 'transparency', params: { alpha: 0.3, hold: true } },
    kind: 'emphasis',
    pose: { fades: true },
  },
  {
    name: 'colorChange-text',
    effect: { effect: 'colorChange', params: { property: 'text', to: '#dc2626' } },
    kind: 'emphasis',
    target: 'text',
    pose: { recolors: true },
  },
  // Fill colours have no compiler hook: the step plays its fallback, `none`.
  {
    name: 'colorChange-fill',
    effect: { effect: 'colorChange', params: { property: 'fill', to: '#dc2626' } },
    kind: 'emphasis',
    still: true,
    fallback: true,
  },
  // Keynote's actions: entrance effects played as emphases, from the shown element and back to it.
  { name: 'action-pop', effect: { effect: 'pop' }, kind: 'emphasis', pose: { scaleX: 'grows', scaleY: 'grows' } },
  { name: 'action-bounce', effect: { effect: 'bounce' }, kind: 'emphasis', middle: 0.27, pose: { moved: 'u' } },
  // A quarter of the step turns the box away from the viewer.
  ...(['horz', 'vert'] as const).map((axis): BuildCase => ({
    name: `action-flip-${axis}`,
    effect: { effect: 'flip', params: { axis } },
    kind: 'emphasis',
    middle: 0.25,
    pose: axis === 'horz' ? { scaleX: 'shrinks' } : { scaleY: 'shrinks' },
  })),
  // An entrance effect with no action plays its fallback, `none`.
  { name: 'fade-as-emphasis', effect: { effect: 'fade' }, kind: 'emphasis', still: true, fallback: true },
  // Tier-2 tracks that end away from rest hold their end (the state after keeps it).
  {
    name: 'behaviors-hold',
    effect: {
      effect: 'behaviors',
      params: [
        {
          property: 'scale',
          beginMs: 0,
          durationMs: 1000,
          frames: [
            { offset: 0, value: { x: 1, y: 1 } },
            { offset: 1, value: { x: 1.3, y: 1.3 } },
          ],
        },
        {
          property: 'rotate',
          beginMs: 0,
          durationMs: 1000,
          frames: [
            { offset: 0, value: 0 },
            { offset: 1, value: 20 },
          ],
        },
        {
          property: 'translate',
          beginMs: 0,
          durationMs: 1000,
          frames: [
            { offset: 0, value: { x: 0, y: 0 } },
            { offset: 1, value: { x: 40, y: 0 } },
          ],
        },
      ],
    },
    kind: 'emphasis',
    pose: { rotate: 'cw', scaleX: 'grows', scaleY: 'grows' },
  },
  {
    name: 'path',
    effect: {
      effect: 'path',
      params: {
        d: [
          ['M', 0, 0],
          ['C', 60, -80, 160, -80, 220, 0],
        ],
        autoRotate: false,
        hold: false,
      },
    },
    kind: 'motion',
    pose: { moved: 'ru' },
  },
  {
    name: 'path-hold-rotate',
    effect: {
      effect: 'path',
      params: {
        d: [
          ['M', 0, 0],
          ['L', 120, 60],
          ['L', 240, 0],
        ],
        autoRotate: true,
        hold: true,
      },
    },
    kind: 'motion',
    pose: { moved: 'rd' },
  },
];

/** Media commands animate nothing on screen; they are checked by the unit tests. */
export const MEDIA: readonly BuildEffect['effect'][] = ['play', 'pause', 'stop', 'toggle'];

export interface TransitionCase {
  readonly name: string;
  readonly effect: TransitionEffect;
  /** The first frame is not the outgoing slide (a cut through black starts black). */
  readonly startsElsewhere?: boolean;
  /** No motion at all (a plain cut). */
  readonly instant?: boolean;
  /** Where the middle frame is taken, for a transition that has arrived at 50% (default 0.5). */
  readonly middle?: number;
  /** Where the incoming slide shows partway through (its background colour, against where it ends). */
  readonly mid?: Mid;
  /** Where the incoming layer has still to travel from, read from its computed transform at 50%. */
  readonly pose?: Pose;
}

export const TRANSITIONS: readonly TransitionCase[] = [
  { name: 'none', effect: { effect: 'none' }, instant: true },
  { name: 'cut', effect: { effect: 'cut', params: { throughBlack: false } }, instant: true },
  // Black for the first half, the incoming slide from 50%.
  {
    name: 'cut-black',
    effect: { effect: 'cut', params: { throughBlack: true } },
    startsElsewhere: true,
    middle: 0.25,
  },
  { name: 'fade', effect: { effect: 'fade', params: { through: 'none' } } },
  { name: 'fade-black', effect: { effect: 'fade', params: { through: 'black' } } },
  { name: 'fade-white', effect: { effect: 'fade', params: { through: 'white' } } },
  { name: 'fade-color', effect: { effect: 'fade', params: { through: 'color', color: '#7c3aed' } } },
  { name: 'dissolve', effect: { effect: 'dissolve' } },
  // The incoming slide moves along `dir`: partway it still sits on the side it comes from.
  ...(['l', 'r', 'u', 'd'] as const).map((dir): TransitionCase => ({
    name: `push-${dir}`,
    effect: { effect: 'push', params: { dir } },
    mid: { side: opposite(dir) },
  })),
  ...(['l', 'rd'] as const).map((dir): TransitionCase => ({
    name: `cover-${dir}`,
    effect: { effect: 'cover', params: { dir } },
    mid: { side: opposite(dir) },
  })),
  // The outgoing slide moves along `dir`, uncovering the incoming one from the opposite side.
  ...(['r', 'lu'] as const).map((dir): TransitionCase => ({
    name: `uncover-${dir}`,
    effect: { effect: 'uncover', params: { dir } },
    mid: { side: opposite(dir) },
  })),
  { name: 'wipe-r', effect: { effect: 'wipe', params: { dir: 'r', soft: false } }, mid: { side: 'l' } },
  { name: 'wipe-soft-d', effect: { effect: 'wipe', params: { dir: 'd', soft: true } }, mid: { side: 'u' } },
  {
    name: 'split-vert-out',
    effect: { effect: 'split', params: { orient: 'vert', inOut: 'out' } },
    mid: { spread: 'center' },
  },
  {
    name: 'split-horz-in',
    effect: { effect: 'split', params: { orient: 'horz', inOut: 'in' } },
    mid: { spread: 'edges' },
  },
  {
    name: 'iris-circle-out',
    effect: { effect: 'iris', params: { shape: 'circle', inOut: 'out' } },
    mid: { spread: 'center' },
  },
  {
    name: 'iris-diamond-in',
    effect: { effect: 'iris', params: { shape: 'diamond', inOut: 'in' } },
    mid: { spread: 'edges' },
  },
  {
    name: 'iris-plus-out',
    effect: { effect: 'iris', params: { shape: 'plus', inOut: 'out' } },
    mid: { spread: 'center' },
  },
  { name: 'wheel-3', effect: { effect: 'wheel', params: { spokes: 3, cw: true } } },
  // From 3 o'clock counter-clockwise, the sweep has covered the top half at 50%.
  { name: 'clock-3-ccw', effect: { effect: 'clock', params: { cw: false, from: '3' } }, mid: { side: 'u' } },
  { name: 'wedge', effect: { effect: 'wedge' }, mid: { side: 'u' } },
  { name: 'blinds', effect: { effect: 'blinds', params: { orient: 'vert' } } },
  { name: 'checker', effect: { effect: 'checker', params: { dir: 'd' } } },
  { name: 'randomBars', effect: { effect: 'randomBars', params: { orient: 'horz' } } },
  { name: 'strips', effect: { effect: 'strips', params: { corner: 'ld' } }, mid: { side: 'ru' } },
  { name: 'comb-horz', effect: { effect: 'comb', params: { orient: 'horz' } } },
  { name: 'comb-vert', effect: { effect: 'comb', params: { orient: 'vert' } } },
  { name: 'zoom-in', effect: { effect: 'zoom', params: { inOut: 'in', rotate: false } } },
  { name: 'zoom-out', effect: { effect: 'zoom', params: { inOut: 'out', rotate: false } } },
  { name: 'newsflash', effect: { effect: 'zoom', params: { inOut: 'in', rotate: true } } },
  // A flip turns the pair like a cube does: the incoming face's leading edge (on the `dir` side) is the nearer one, so
  // in perspective it shows larger on that side. Edge-on at 50%, it is measured at 75%.
  ...(['l', 'u'] as const).map((dir): TransitionCase => ({
    name: `flip-${dir}`,
    effect: { effect: 'flip', params: { dir } },
    mid: { side: dir, at: 0.75 },
  })),
  ...(['r', 'd'] as const).map((dir): TransitionCase => ({
    name: `cube-${dir}`,
    effect: { effect: 'cube', params: { dir } },
    mid: { side: opposite(dir) },
  })),
  { name: 'doors', effect: { effect: 'doors', params: { orient: 'vert' } }, mid: { spread: 'center' } },
  { name: 'window', effect: { effect: 'window', params: { orient: 'horz' } }, mid: { spread: 'center' } },
  // Moving left, each of these brings the incoming slide in from the right; a page turned leftwards hinges on its
  // left edge and uncovers the next page from the right; rotate3d and twist turn like a flip. Switch fades in, so its
  // incoming layer's own transform says where it comes from.
  ...(
    [
      ['gallery', { side: 'r' }],
      ['conveyor', { side: 'r' }],
      ['ferris', { side: 'r' }],
      ['pan', { side: 'r' }],
      ['switch', undefined],
      ['rotate3d', { side: 'l' }],
      ['swap', { side: 'r' }],
      ['pageFlip', { side: 'r' }],
      ['twist', { side: 'l' }],
      ['mosaic', { side: 'r' }],
    ] as const
  ).map(([effect, mid]): TransitionCase => ({
    name: effect,
    effect: { effect, params: { dir: 'l' } },
    ...(mid === undefined ? { pose: { moved: 'r' } } : { mid }),
  })),
  // Still off-stage at 50% on its long arc: measured at 75%, coming up from below.
  { name: 'ferris-u', effect: { effect: 'ferris', params: { dir: 'u' } }, mid: { side: 'd', at: 0.75 } },
  { name: 'reveal', effect: { effect: 'reveal', params: { dir: 'l', throughBlack: false } } },
  { name: 'reveal-black', effect: { effect: 'reveal', params: { dir: 'l', throughBlack: true } } },
  { name: 'flythrough-in', effect: { effect: 'flythrough', params: { inOut: 'in', bounce: false } } },
  { name: 'flythrough-out-bounce', effect: { effect: 'flythrough', params: { inOut: 'out', bounce: true } } },
  { name: 'morph', effect: { effect: 'morph', params: { by: 'object' } } },
  { name: 'magicMove', effect: { effect: 'magicMove', params: { fadeUnmatched: true } } },
  { name: 'random', effect: { effect: 'random' } },
];
