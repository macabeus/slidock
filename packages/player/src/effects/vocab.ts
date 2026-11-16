// Reads a step's or a transition's effect in the common vocabulary (docs/format.md §5.2) from the deck JSON. The
// deck is untrusted input: every name, token and number is checked against the contract and its bounds
// (slidock-ir `effects::limits`), and anything else reads as unknown, so the step plays its canonical fallback.
import type { BuildEffect, HexColor, Keyframes, PathSeg, StepKind, Track, TransitionEffect } from '@slidock/types';

import { readEasing } from './easing';

/** The bounds of the IR's effect validator (`slidock_ir::effects::limits`), and `Ms::MAX`. */
export const LIMITS = {
  ms: 3_600_000,
  tracks: 32,
  keyframes: 64,
  pathSegments: 4_096,
  repeat: 1_000,
  spokes: 8,
  spinDegrees: 3_600,
  scale: 100,
  blurPx: 1_000,
  /** Px values of keyframe translations and paths: a generous bound on slide coordinates. */
  px: 1_000_000,
} as const;

type Check<T> = (value: unknown) => T | undefined;

const DIRS = ['l', 'r', 'u', 'd', 'lu', 'ru', 'ld', 'rd'] as const;
const DIR4 = ['l', 'r', 'u', 'd'] as const;
const DIAGONALS = ['lu', 'ru', 'ld', 'rd'] as const;

function token<T extends string>(...tokens: readonly T[]): Check<T> {
  return (value) =>
    typeof value === 'string' && (tokens as readonly string[]).includes(value) ? (value as T) : undefined;
}

const bool: Check<boolean> = (value) => (typeof value === 'boolean' ? value : undefined);

function num(min: number, max: number, options: { minExclusive?: boolean; int?: boolean } = {}): Check<number> {
  return (value) => {
    if (typeof value !== 'number' || !Number.isFinite(value) || value > max) {
      return undefined;
    }
    if (options.minExclusive === true ? value <= min : value < min) {
      return undefined;
    }
    if (options.int === true && !Number.isInteger(value)) {
      return undefined;
    }
    return value;
  };
}

const color: Check<string> = (value) => (typeof value === 'string' && /^#[0-9a-f]{6}$/.test(value) ? value : undefined);

/** An object with exactly these keys (optional ones may be absent), each passing its check. */
function shape<T extends object>(
  checks: { readonly [K in keyof T]-?: Check<T[K]> },
  optional: readonly string[] = [],
): Check<T> {
  return (value) => {
    if (typeof value !== 'object' || value === null || Array.isArray(value)) {
      return undefined;
    }
    const record = value as Record<string, unknown>;
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(record)) {
      if (!Object.hasOwn(checks, key)) {
        return undefined;
      }
    }
    for (const [key, check] of Object.entries(checks) as [string, Check<unknown>][]) {
      if (!Object.hasOwn(record, key)) {
        if (optional.includes(key)) {
          continue;
        }
        return undefined;
      }
      const read = check(record[key]);
      if (read === undefined) {
        return undefined;
      }
      out[key] = read;
    }
    return out as T;
  };
}

const unit = num(0, 1);
const ms = num(0, LIMITS.ms);
const px = num(-LIMITS.px, LIMITS.px);
const dirParams = shape({ dir: token(...DIRS) });
const dir4Params = shape({ dir: token(...DIR4) });
const orientParams = shape({ orient: token('horz', 'vert') });
const axisParams = shape({ axis: token('horz', 'vert', 'both') });
const wipeParams = shape({ dir: token(...DIRS), soft: bool });
const splitParams = shape({ orient: token('horz', 'vert'), inOut: token('in', 'out') });
const irisParams = shape({ shape: token('circle', 'diamond', 'box', 'plus'), inOut: token('in', 'out') });
const wheelParams = shape({ spokes: num(1, LIMITS.spokes, { int: true }), cw: bool });
const clockParams = shape({ cw: bool, from: token('12', '3', '6', '9') });
const stripsParams = shape({ corner: token(...DIAGONALS) });
const scale = shape<{ x: number; y: number }>({ x: num(0, LIMITS.scale), y: num(0, LIMITS.scale) });
const repeatParams = shape({ repeat: num(1, LIMITS.repeat, { int: true }) });
const intensityParams = shape({ intensity: unit });
const spinDeg = num(0, LIMITS.spinDegrees, { minExclusive: true });

function pathSeg(value: unknown): PathSeg | undefined {
  if (!Array.isArray(value) || value.length === 0) {
    return undefined;
  }
  const [op, ...rest] = value as unknown[];
  const arity = op === 'M' || op === 'L' ? 2 : op === 'C' ? 6 : op === 'Z' ? 0 : -1;
  if (arity < 0 || rest.length !== arity || !rest.every((n) => px(n) !== undefined)) {
    return undefined;
  }
  return value as PathSeg;
}

function path(value: unknown): PathSeg[] | undefined {
  if (!Array.isArray(value) || value.length === 0 || value.length > LIMITS.pathSegments) {
    return undefined;
  }
  const segments: PathSeg[] = [];
  for (const item of value as unknown[]) {
    const segment = pathSeg(item);
    if (segment === undefined) {
      return undefined;
    }
    segments.push(segment);
  }
  return segments[0]?.[0] === 'M' ? segments : undefined;
}

function easingValue(value: unknown): boolean {
  return readEasing(value) !== null;
}

/**
 * The IR's colour and font tracks (slidock-ir `Keyframes::{Text, Fill, Line, Bold, Italic, Underline}`) as a deck holds
 * them: colours resolved to `#rrggbb` by the deck builder, font switches on or off.
 */
export type PaintKeyframes =
  | ({ property: 'text' | 'fill' | 'line' } & Track<HexColor>)
  | ({ property: 'bold' | 'italic' | 'underline' } & Track<boolean>);

/** A switch (slidock-ir `SetTrack`) as a deck holds it: a colour resolved to `#rrggbb`. */
export interface SetKeyframes {
  readonly property: 'set';
  readonly beginMs: number;
  readonly durationMs: number;
  readonly to:
    | { readonly property: 'opacity'; readonly value: number }
    | { readonly property: 'text' | 'fill' | 'line'; readonly value: HexColor }
    | { readonly property: 'bold' | 'italic' | 'underline'; readonly value: boolean };
}

/** A tier-2 track the player plays. */
export type Tier2Track = Exclude<Keyframes, { property: 'set' }> | PaintKeyframes | SetKeyframes;

/** One tier-2 track: offsets rising from 0 to 1, values of its property; or a switch of one property at its begin. */
function track(value: unknown): Tier2Track | undefined {
  if (typeof value !== 'object' || value === null) {
    return undefined;
  }
  const record = value as Record<string, unknown>;
  const property = record.property;
  const common = ['property', 'beginMs', 'durationMs', 'easing'];
  if (ms(record.beginMs) === undefined || ms(record.durationMs) === undefined) {
    return undefined;
  }
  if ('easing' in record && !easingValue(record.easing)) {
    return undefined;
  }
  if (property === 'clipPath') {
    if (Object.keys(record).some((key) => ![...common, 'reveal', 'transition'].includes(key))) {
      return undefined;
    }
    if (token('in', 'out')(record.transition) === undefined || reveal(record.reveal) === undefined) {
      return undefined;
    }
    return value as Tier2Track;
  }
  // A switch (`set`) gives one of these properties a value: `to` in place of `frames`.
  const switched: Readonly<Record<string, Check<unknown>>> = {
    opacity: unit,
    text: color,
    fill: color,
    line: color,
    bold: bool,
    italic: bool,
    underline: bool,
  };
  const values: Readonly<Record<string, Check<unknown>>> = {
    ...switched,
    translate: shape({ x: px, y: px }),
    rotate: num(-LIMITS.spinDegrees, LIMITS.spinDegrees),
    scale: scale,
    blur: num(0, LIMITS.blurPx),
  };
  const set = property === 'set';
  const key = set ? (record.to as { property?: unknown } | null)?.property : property;
  const table = set ? switched : values;
  const check = typeof key === 'string' && Object.hasOwn(table, key) ? table[key] : undefined;
  if (check === undefined || Object.keys(record).some((name) => ![...common, set ? 'to' : 'frames'].includes(name))) {
    return undefined;
  }
  if (set) {
    return shape({ property: token(key as string), value: check })(record.to) === undefined
      ? undefined
      : (value as Tier2Track);
  }
  const frames = record.frames;
  if (!Array.isArray(frames) || frames.length < 2 || frames.length > LIMITS.keyframes) {
    return undefined;
  }
  let last = -1;
  for (const frame of frames as unknown[]) {
    const read = shape({ offset: unit, value: (v: unknown) => check(v) })(frame);
    if (read === undefined || read.offset < last) {
      return undefined;
    }
    last = read.offset;
  }
  const first = (frames[0] as { offset: number }).offset;
  return first === 0 && last === 1 ? (value as Tier2Track) : undefined;
}

function reveal(value: unknown): unknown {
  if (typeof value !== 'object' || value === null) {
    return undefined;
  }
  const { effect, params } = value as { effect?: unknown; params?: unknown };
  const keys = Object.keys(value);
  const checks: Readonly<Record<string, Check<unknown> | null>> = {
    wipe: wipeParams,
    split: splitParams,
    iris: irisParams,
    wheel: wheelParams,
    wedge: null,
    clock: clockParams,
    blinds: orientParams,
    checker: dir4Params,
    randomBars: orientParams,
    strips: stripsParams,
    dissolve: null,
  };
  if (typeof effect !== 'string' || !Object.hasOwn(checks, effect)) {
    return undefined;
  }
  const check = checks[effect] ?? null;
  if (check === null) {
    return keys.length === 1 ? value : undefined;
  }
  return keys.length === 2 && check(params) !== undefined ? value : undefined;
}

function behaviors(value: unknown): Tier2Track[] | undefined {
  if (!Array.isArray(value) || value.length === 0 || value.length > LIMITS.tracks) {
    return undefined;
  }
  const tracks: Tier2Track[] = [];
  for (const item of value as unknown[]) {
    const read = track(item);
    if (read === undefined) {
      return undefined;
    }
    tracks.push(read);
  }
  return tracks;
}

const BUILD_PARAMS: Readonly<Record<BuildEffect['effect'], Check<unknown> | null>> = {
  appear: null,
  fade: null,
  dissolve: null,
  flyIn: shape({ dir: token(...DIRS), distance: token('offSlide', 'slight') }),
  floatIn: shape({ dir: token('u', 'd') }),
  peek: shape({ dir: token(...DIR4), slow: bool }),
  wipe: wipeParams,
  split: splitParams,
  iris: irisParams,
  wheel: wheelParams,
  wedge: null,
  clock: clockParams,
  blinds: orientParams,
  checker: dir4Params,
  randomBars: orientParams,
  strips: stripsParams,
  zoom: shape({
    inOut: token('in', 'out'),
    origin: token('object', 'slideCenter'),
    slight: bool,
    fade: bool,
    overshoot: bool,
  }),
  grow: axisParams,
  expand: axisParams,
  compress: axisParams,
  stretch: axisParams,
  growTurn: null,
  swivel: axisParams,
  flip: axisParams,
  spinIn: shape({ deg: spinDeg, cw: bool }),
  bounce: null,
  drop: null,
  pop: null,
  boomerang: null,
  sling: null,
  whip: null,
  swish: null,
  typewriter: null,
  credits: null,
  driftAndScale: dirParams,
  fadeAndMove: dirParams,
  random: null,
  pulse: shape({ scale: num(0, LIMITS.scale, { minExclusive: true }) }),
  growShrink: shape({ scale, hold: bool }),
  spin: shape({ deg: spinDeg, cw: bool, hold: bool }),
  teeter: intensityParams,
  jiggle: intensityParams,
  wave: intensityParams,
  blink: repeatParams,
  flash: repeatParams,
  boldFlash: repeatParams,
  transparency: shape({ alpha: unit, hold: bool }),
  colorChange: shape({ property: token('fill', 'text', 'line'), to: color }),
  path: shape({ d: path, autoRotate: bool, hold: bool }),
  play: shape({ from: ms }),
  pause: null,
  stop: null,
  toggle: null,
  behaviors,
};

const TRANSITION_PARAMS: Readonly<Record<TransitionEffect['effect'], Check<unknown> | null>> = {
  none: null,
  cut: shape({ throughBlack: bool }),
  fade: (value) => {
    const read = shape<{ through: string; color?: string }>(
      { through: token('none', 'black', 'white', 'color'), color },
      ['color'],
    )(value);
    return read !== undefined && (read.through === 'color') === (read.color !== undefined) ? read : undefined;
  },
  dissolve: null,
  push: dir4Params,
  cover: dirParams,
  uncover: dirParams,
  wipe: wipeParams,
  split: splitParams,
  iris: irisParams,
  wheel: wheelParams,
  clock: clockParams,
  wedge: null,
  blinds: orientParams,
  checker: dir4Params,
  randomBars: orientParams,
  strips: stripsParams,
  comb: orientParams,
  zoom: shape({ inOut: token('in', 'out'), rotate: bool }),
  flip: dir4Params,
  cube: dir4Params,
  doors: orientParams,
  window: orientParams,
  gallery: dir4Params,
  conveyor: dir4Params,
  ferris: dir4Params,
  pan: dir4Params,
  switch: dir4Params,
  rotate3d: dir4Params,
  reveal: shape({ dir: token(...DIR4), throughBlack: bool }),
  flythrough: shape({ inOut: token('in', 'out'), bounce: bool }),
  swap: dir4Params,
  pageFlip: dir4Params,
  twist: dir4Params,
  mosaic: dir4Params,
  morph: shape({ by: token('object', 'word', 'char') }),
  magicMove: shape({ fadeUnmatched: bool }),
  random: null,
};

/** Why an effect did not read: the deck has none, names one this player does not know, or its params are wrong. */
export type Unread = 'absent' | 'unknown' | 'invalid';

export type Read<T> = { readonly effect: T } | { readonly unread: Unread; readonly name: string | null };

function readEffect<T>(
  table: Readonly<Record<string, Check<unknown> | null>>,
  value: { readonly effect?: unknown; readonly params?: unknown },
): Read<T> {
  const name = value.effect;
  if (name === undefined) {
    return { unread: 'absent', name: null };
  }
  if (typeof name !== 'string' || !Object.hasOwn(table, name)) {
    return { unread: 'unknown', name: typeof name === 'string' ? name : null };
  }
  const check = table[name] ?? null;
  if (check === null) {
    return value.params === undefined ? { effect: { effect: name } as T } : { unread: 'invalid', name };
  }
  const params = check(value.params);
  return params === undefined ? { unread: 'invalid', name } : { effect: { effect: name, params } as T };
}

export function readBuild(step: { readonly effect?: unknown; readonly params?: unknown }): Read<BuildEffect> {
  return readEffect<BuildEffect>(BUILD_PARAMS, step);
}

export function readTransition(transition: {
  readonly effect?: unknown;
  readonly params?: unknown;
}): Read<TransitionEffect> {
  return readEffect<TransitionEffect>(TRANSITION_PARAMS, transition);
}

export type Family = 'entrance' | 'emphasis' | 'motion' | 'media' | 'behaviors';

const EMPHASES = new Set<BuildEffect['effect']>([
  'pulse',
  'growShrink',
  'spin',
  'teeter',
  'jiggle',
  'wave',
  'blink',
  'flash',
  'boldFlash',
  'transparency',
  'colorChange',
]);

export function family(effect: BuildEffect): Family {
  if (EMPHASES.has(effect.effect)) {
    return 'emphasis';
  }
  switch (effect.effect) {
    case 'path':
      return 'motion';
    case 'play':
    case 'pause':
    case 'stop':
    case 'toggle':
      return 'media';
    case 'behaviors':
      return 'behaviors';
    default:
      return 'entrance';
  }
}

/** Whether a step of `kind` plays the effect (the IR's `BuildEffect::plays_in`). */
export function playsIn(effect: BuildEffect, kind: StepKind): boolean {
  const f = family(effect);
  if (f === 'entrance' || f === 'emphasis') {
    return kind === 'entrance' || kind === 'exit' || kind === 'emphasis';
  }
  if (f === 'motion') {
    return kind === 'motion';
  }
  if (f === 'media') {
    return kind === 'media';
  }
  return kind !== 'media';
}

/** Every build effect name, for the tests and the effect catalog. */
export const BUILD_EFFECTS = /* @__PURE__ */ Object.keys(BUILD_PARAMS) as BuildEffect['effect'][];
/** Every transition effect name. */
export const TRANSITION_EFFECTS = /* @__PURE__ */ Object.keys(TRANSITION_PARAMS) as TransitionEffect['effect'][];
