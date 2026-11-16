// The timeline compiler: click groups, each step's start inside its group, its repeats,
// auto-reverse, stagger and after effect, and the state a slide is in after a number of groups. Pure: it reads only
// the deck's steps (and, for stagger, how many units each step's delivery split into), so a slide's state depends
// only on (slide, step).
//
// Group 0 is the run of automatic steps at the start; every `click` step opens a group, and the with- and
// after-previous steps that follow join it. A step starts at its absolute `offsetMs` when the compiler gave one (PPTX),
// else with the previous step's start (with previous) or at its end (after previous), plus its delay. A step's end
// includes its repeats, auto-reverse and stagger; an endless repeat counts one round toward its end.
import type { AfterEffect, BuildEffect, DeckStep, StaggerOrder } from '@slidock/types';

import { type Curve, LINEAR, evaluate, readEasing } from './effects/easing';
import { pointAt } from './effects/path';
import { type Paint, fontValue } from './effects/properties';
import { permutation } from './effects/random';
import { LIMITS, type Read, type Tier2Track, type Unread, playsIn, readBuild } from './effects/vocab';

/** Steps a slide plays at most (slidock-ir `STEPS_PER_SLIDE`); the rest are ignored. */
export const STEPS_PER_SLIDE = 2_000;
/** Units (paragraphs, words, characters) one step staggers at most. */
export const UNITS_PER_STEP = 10_000;

/** Why a step's timing or effect did not read; the step then plays a declared default. */
export type Issue =
  | { readonly kind: 'effect'; readonly unread: Unread; readonly name: string | null }
  | { readonly kind: 'cannotPlay'; readonly name: string }
  | { readonly kind: 'timing'; readonly field: string }
  /** A tier-2 colour or font track the target cannot play here: left out, the step's other tracks play. */
  | { readonly kind: 'track'; readonly property: string };

export interface ScheduledStep {
  readonly step: DeckStep;
  /** The step's index in the slide's timeline. */
  readonly index: number;
  /** What it plays: its effect in the vocabulary, or `null` for its canonical fallback. */
  readonly effect: BuildEffect | null;
  /** Its start inside its group, delay included, in ms. */
  readonly startMs: number;
  /** When it (with every repeat and its stagger) ends; one round for an endless repeat. */
  readonly endMs: number;
  /** One iteration, in ms. */
  readonly durationMs: number;
  /** WAAPI iterations (auto-reverse doubles them); `Infinity` for an endless repeat. */
  readonly iterations: number;
  readonly direction: PlaybackDirection;
  /** Until when an endless repeat runs. */
  readonly loop: 'click' | 'slide' | null;
  readonly curve: Curve;
  /** Each unit's start after `startMs`, in unit order (one entry for a whole target). */
  readonly unitDelays: readonly number[];
  readonly after: AfterEffect;
  readonly issues: readonly Issue[];
}

export interface Group {
  readonly steps: readonly ScheduledStep[];
  /** When its last step ends (one round of an endless repeat). */
  readonly durationMs: number;
}

/** How many units a step's delivery splits its target into (1 when it is built whole). */
export type UnitCounter = (step: DeckStep, index: number) => number;

/** The steps a slide plays: its timeline up to the bound. */
export function bounded(timeline: readonly DeckStep[]): readonly DeckStep[] {
  return timeline.length > STEPS_PER_SLIDE ? timeline.slice(0, STEPS_PER_SLIDE) : timeline;
}

function readMs(value: unknown, field: string, issues: Issue[]): number {
  if (typeof value === 'number' && Number.isFinite(value) && value >= 0 && value <= LIMITS.ms) {
    return value;
  }
  issues.push({ kind: 'timing', field });
  return 0;
}

/** The effect a step plays: its vocabulary effect when it reads and plays in the step's kind. */
export function stepEffect(step: DeckStep): { effect: BuildEffect | null; issue: Issue | null } {
  const read: Read<BuildEffect> = readBuild(step);
  if ('unread' in read) {
    return {
      effect: null,
      issue: read.unread === 'absent' ? null : { kind: 'effect', unread: read.unread, name: read.name },
    };
  }
  if (!playsIn(read.effect, step.kind)) {
    return { effect: null, issue: { kind: 'cannotPlay', name: read.effect.effect } };
  }
  return { effect: read.effect, issue: null };
}

/** The stagger slot of each of `n` units in `order` (units in the same slot start together). */
export function staggerLevels(n: number, order: StaggerOrder): number[] {
  const count = Math.max(0, Math.min(UNITS_PER_STEP, Math.floor(n)));
  if (typeof order === 'object') {
    const levels = new Array<number>(count).fill(0);
    permutation(count, order.random >>> 0).forEach((unit, slot) => (levels[unit] = slot));
    return levels;
  }
  const center = (count - 1) / 2;
  const fromCenter = (i: number) => Math.floor(Math.abs(i - center));
  const widest = count === 0 ? 0 : fromCenter(0);
  return Array.from({ length: count }, (_, i) => {
    switch (order) {
      case 'backward':
        return count - 1 - i;
      case 'fromCenter':
        return fromCenter(i);
      case 'fromEdges':
        return widest - fromCenter(i);
      case 'forward':
        return i;
    }
  });
}

function readOrder(value: unknown): StaggerOrder | null {
  if (value === 'forward' || value === 'backward' || value === 'fromCenter' || value === 'fromEdges') {
    return value;
  }
  if (typeof value === 'object' && value !== null && Object.keys(value).length === 1) {
    const seed = (value as { random?: unknown }).random;
    if (typeof seed === 'number' && Number.isInteger(seed) && seed >= 0 && seed <= 0xffffffff) {
      return { random: seed };
    }
  }
  return null;
}

function readAfter(value: unknown, issues: Issue[]): AfterEffect {
  if (value === undefined || value === 'keep') {
    return 'keep';
  }
  if (value === 'hide') {
    return 'hide';
  }
  if (typeof value === 'object' && value !== null && Object.keys(value).length === 1) {
    const dim = (value as { dim?: unknown }).dim;
    if (typeof dim === 'string' && /^#[0-9a-f]{6}$/.test(dim)) {
      return { dim };
    }
  }
  issues.push({ kind: 'timing', field: 'after' });
  return 'keep';
}

function readRepeat(value: unknown, issues: Issue[]): { rounds: number; loop: 'click' | 'slide' | null } {
  if (value === undefined) {
    return { rounds: 1, loop: null };
  }
  if (value === 'untilNextClick') {
    return { rounds: 1, loop: 'click' };
  }
  if (value === 'untilEndOfSlide') {
    return { rounds: 1, loop: 'slide' };
  }
  if (typeof value === 'object' && value !== null && Object.keys(value).length === 1) {
    const count = (value as { count?: unknown }).count;
    if (typeof count === 'number' && Number.isFinite(count) && count > 0 && count <= LIMITS.repeat) {
      return { rounds: count, loop: null };
    }
  }
  issues.push({ kind: 'timing', field: 'repeat' });
  return { rounds: 1, loop: null };
}

function readStagger(value: unknown, duration: number, issues: Issue[]): { perUnit: number; order: StaggerOrder } {
  const none = { perUnit: 0, order: 'forward' as const };
  if (value === undefined) {
    return none;
  }
  if (typeof value !== 'object' || value === null) {
    issues.push({ kind: 'timing', field: 'stagger' });
    return none;
  }
  const { perUnit: amount, order: rawOrder } = value as { perUnit?: unknown; order?: unknown };
  const order = readOrder(rawOrder);
  let perUnit: number | null = null;
  if (typeof amount === 'object' && amount !== null && Object.keys(amount).length === 1) {
    const ms = (amount as { ms?: unknown }).ms;
    const fraction = (amount as { fraction?: unknown }).fraction;
    if (typeof ms === 'number' && Number.isFinite(ms) && ms >= 0 && ms <= LIMITS.ms) {
      perUnit = ms;
    } else if (typeof fraction === 'number' && Number.isFinite(fraction) && fraction >= 0 && fraction <= 1) {
      perUnit = fraction * duration;
    }
  }
  if (perUnit === null || order === null) {
    issues.push({ kind: 'timing', field: 'stagger' });
    return none;
  }
  return { perUnit, order };
}

/** Schedules one step inside its group, after `previous` (the step before it in the group, if any). */
function scheduleStep(
  step: DeckStep,
  index: number,
  previous: ScheduledStep | null,
  units: UnitCounter,
): ScheduledStep {
  const issues: Issue[] = [];
  const { effect, issue } = stepEffect(step);
  if (issue !== null) {
    issues.push(issue);
  }
  const delay = readMs(step.delayMs, 'delayMs', issues);
  let duration = readMs(step.durationMs, 'durationMs', issues);
  let start: number;
  if (step.offsetMs !== undefined) {
    start = readMs(step.offsetMs, 'offsetMs', issues);
  } else if (step.trigger === 'withPrevious') {
    start = (previous?.startMs ?? 0) + delay;
  } else if (step.trigger === 'afterPrevious') {
    start = (previous?.endMs ?? 0) + delay;
  } else {
    start = delay;
  }
  let curve = LINEAR;
  if (step.easing !== undefined) {
    const read = readEasing(step.easing);
    if (read === null) {
      issues.push({ kind: 'timing', field: 'easing' });
    } else {
      curve = read;
    }
  }
  const repeats = readRepeat(step.repeat, issues);
  const autoReverse = step.autoReverse === true;
  const after = readAfter(step.after, issues);
  const count = Math.max(1, Math.min(UNITS_PER_STEP, Math.floor(units(step, index)) || 1));
  let { perUnit, order } = readStagger(step.stagger, duration, issues);
  // A typewriter types its characters one after another over the step's duration.
  if (effect?.effect === 'typewriter') {
    perUnit = duration / count;
    order = 'forward';
    duration = 0;
  }
  const unitDelays = staggerLevels(count, order).map((level) => level * perUnit);
  const span = Math.max(0, ...unitDelays);
  const iterations = repeats.loop !== null ? Infinity : repeats.rounds * (autoReverse ? 2 : 1);
  const active = repeats.loop !== null ? duration * (autoReverse ? 2 : 1) : duration * iterations;
  return {
    step,
    index,
    effect,
    startMs: start,
    endMs: start + active + span,
    durationMs: duration,
    iterations,
    direction: autoReverse ? 'alternate' : 'normal',
    loop: repeats.loop,
    curve,
    unitDelays,
    after,
    issues,
  };
}

/** The slide's groups: group 0 (automatic, maybe empty) and one per click step. */
export function groups(timeline: readonly DeckStep[], units: UnitCounter = () => 1): Group[] {
  const raw: { step: DeckStep; index: number }[][] = [[]];
  bounded(timeline).forEach((step, index) => {
    if (step.trigger === 'click') {
      raw.push([{ step, index }]);
    } else {
      raw[raw.length - 1]?.push({ step, index });
    }
  });
  return raw.map((entries) => {
    let previous: ScheduledStep | null = null;
    const steps = entries.map(({ step, index }) => {
      const scheduled = scheduleStep(step, index, previous, units);
      previous = scheduled;
      return scheduled;
    });
    return { steps, durationMs: Math.max(0, ...steps.map((s) => s.endMs)) };
  });
}

/** The slide's click groups: its `click` steps. */
export function clickGroupCount(timeline: readonly DeckStep[]): number {
  return bounded(timeline).filter((step) => step.trigger === 'click').length;
}

/** A build unit: a whole element, one paragraph of it, or its shape without its text. */
export interface Unit {
  readonly element: string;
  readonly paragraph: number | null;
  readonly background?: boolean;
}

export function unitKey(unit: Unit): string {
  if (unit.background === true) {
    return `${unit.element}\u0000bg`;
  }
  return unit.paragraph === null ? unit.element : `${unit.element}\u0000${unit.paragraph}`;
}

/** Paragraphs one step addresses at most; a wider (or malformed) range addresses the whole element. */
const PARAGRAPHS_PER_STEP = 10_000;

/** The units a step builds: its element, its background, or each paragraph of its range. */
export function stepUnits(step: DeckStep): Unit[] {
  const element = step.target.element;
  if (step.target.background === true) {
    return [{ element, paragraph: null, background: true }];
  }
  const range: unknown = step.target.paragraphs;
  if (!Array.isArray(range) || range.length !== 2) {
    return [{ element, paragraph: null }];
  }
  const [first, last] = range as unknown[];
  if (
    typeof first !== 'number' ||
    typeof last !== 'number' ||
    !Number.isInteger(first) ||
    !Number.isInteger(last) ||
    first < 0 ||
    last < first ||
    last - first >= PARAGRAPHS_PER_STEP
  ) {
    return [{ element, paragraph: null }];
  }
  const units: Unit[] = [];
  for (let p = first; p <= last; p++) {
    units.push({ element, paragraph: p });
  }
  return units;
}

/**
 * A transform a step left, in the order the steps played: a translation (slide px, played in the element's space with
 * the turns and scales of the first `under` transforms undone, as the step that left it saw them), a turn (degrees,
 * clockwise) or a scale.
 */
export type HoldOp =
  | { readonly op: 'translate'; readonly x: number; readonly y: number; readonly under: number }
  | { readonly op: 'rotate'; readonly deg: number }
  | { readonly op: 'scale'; readonly x: number; readonly y: number };

/** What emphases and motion paths leave on a unit. */
export interface Hold {
  /** Transforms, composed in the order their animations played. */
  readonly transforms: readonly HoldOp[];
  readonly opacity: number;
  /** The blur (px) a tier-2 track left. */
  readonly blur: number;
  /** The colours and font switches colour and font changes left, as their registered properties' values. */
  readonly paint: Readonly<Partial<Record<Paint, string>>>;
}

export const NO_HOLD: Hold = { transforms: [], opacity: 1, blur: 0, paint: {} };

export function isIdentity(hold: Hold): boolean {
  return hold.transforms.length === 0 && hold.opacity === 1 && hold.blur === 0 && Object.keys(hold.paint).length === 0;
}

/** The most transforms a hold keeps; past it, further transforms are not held. */
const MAX_TRANSFORMS = 256;

function identity(op: HoldOp): boolean {
  return op.op === 'rotate' ? op.deg === 0 : op.op === 'scale' ? op.x === 1 && op.y === 1 : op.x === 0 && op.y === 0;
}

/**
 * `base` with `ops` composed after its transforms: a transform of the kind of the last one merges into it, and one
 * that leaves nothing (a path back where it started) is not held.
 */
function withTransforms(base: Hold, ops: readonly HoldOp[]): Hold {
  const list = [...base.transforms];
  for (const op of ops) {
    const last = list.at(-1);
    let merged: HoldOp | null = null;
    if (last?.op === 'rotate' && op.op === 'rotate') {
      merged = { op: 'rotate', deg: last.deg + op.deg };
    } else if (last?.op === 'scale' && op.op === 'scale') {
      merged = { op: 'scale', x: last.x * op.x, y: last.y * op.y };
    } else if (last?.op === 'translate' && op.op === 'translate') {
      merged = { ...last, x: last.x + op.x, y: last.y + op.y };
    }
    if (merged !== null) {
      list.pop();
      if (!identity(merged)) {
        list.push(merged);
      }
    } else if (!identity(op) && list.length < MAX_TRANSFORMS) {
      list.push(op);
    }
  }
  return { ...base, transforms: list };
}

/** Two frame values `k` of the way from `a` to `b`: numbers, points and `#rrggbb` colours (in sRGB) mix, switches step. */
function mix(a: unknown, b: unknown, k: number): unknown {
  if (typeof a === 'number') {
    return a + ((b as number) - a) * k;
  }
  if (typeof a === 'string') {
    const channel = (hex: unknown, i: number) => Number.parseInt((hex as string).slice(i, i + 2), 16);
    return `#${[1, 3, 5]
      .map((i) =>
        Math.round(mix(channel(a, i), channel(b, i), k) as number)
          .toString(16)
          .padStart(2, '0'),
      )
      .join('')}`;
  }
  if (typeof a === 'object' && a !== null) {
    const [p, q] = [a, b] as { x: number; y: number }[];
    return { x: mix(p?.x, q?.x, k), y: mix(p?.y, q?.y, k) };
  }
  return k < 1 ? a : b;
}

/**
 * A track's value `at` ms into its step, its easing applied (`null`: its last frame); `undefined` for a switch that
 * has not begun.
 */
function valueAt(track: Tier2Track, at: number | null, curve: Curve): unknown {
  if (track.property === 'set') {
    return at === null || at >= track.beginMs ? track.to.value : undefined;
  }
  const frames = track.property === 'clipPath' ? [] : track.frames;
  if (at === null) {
    return frames.at(-1)?.value;
  }
  const { beginMs, durationMs } = track;
  const q = durationMs > 0 ? Math.min(1, Math.max(0, (at - beginMs) / durationMs)) : +(at >= beginMs);
  const u = evaluate((track.easing === undefined ? null : readEasing(track.easing)) ?? curve, q);
  let i = 0;
  while (i < frames.length - 2 && (frames[i + 1]?.offset ?? 1) <= u) {
    i++;
  }
  const [a, b] = [frames[i], frames[i + 1]];
  if (a === undefined || b === undefined) {
    return a?.value;
  }
  return mix(
    a.value,
    b.value,
    b.offset > a.offset ? Math.min(1, Math.max(0, (u - a.offset) / (b.offset - a.offset))) : 1,
  );
}

/**
 * The hold tier-2 tracks leave `at` ms into their step (`null`: each on its last frame, as they play once, on their
 * own timing). Transform tracks compose in their order, opacity scales the element's, the last blur, colour and font
 * switch stay. `null` when every track ends at rest.
 */
function behaviorsHold(tracks: readonly Tier2Track[], base: Hold, at: number | null, curve: Curve): Hold | null {
  let hold = base;
  const ops: HoldOp[] = [];
  const under = base.transforms.length;
  for (const track of tracks) {
    const value = valueAt(track, at, curve) as never;
    if (value === undefined) {
      continue;
    }
    const property = track.property === 'set' ? track.to.property : track.property;
    switch (property) {
      case 'text':
      case 'fill':
      case 'line':
        hold = { ...hold, paint: { ...hold.paint, [property]: value } };
        break;
      case 'bold':
      case 'italic':
      case 'underline':
        hold = { ...hold, paint: { ...hold.paint, [property]: fontValue(property, value) } };
        break;
      case 'opacity':
        hold = { ...hold, opacity: base.opacity * value };
        break;
      case 'blur':
        hold = { ...hold, blur: value };
        break;
      case 'translate':
        ops.push({ op: 'translate', ...(value as { x: number; y: number }), under });
        break;
      case 'rotate':
        ops.push({ op: 'rotate', deg: value });
        break;
      case 'scale':
        ops.push({ op: 'scale', ...(value as { x: number; y: number }) });
        break;
    }
  }
  const rest =
    ops.every(identity) && hold.opacity === base.opacity && hold.blur === base.blur && hold.paint === base.paint;
  return rest ? null : withTransforms(hold, ops);
}

/**
 * Where a finished step left its iteration progress (0 to 1), its repeats and auto-reverse counted; an endless repeat
 * ends a round, as its end counts it.
 */
export function endProgress(step: Pick<ScheduledStep, 'iterations' | 'direction'>): number {
  const alternate = step.direction === 'alternate';
  const n = Number.isFinite(step.iterations) ? step.iterations : alternate ? 2 : 1;
  if (n <= 0) {
    return 0;
  }
  const whole = Math.floor(n);
  const fraction = n - whole;
  if (fraction === 0) {
    return alternate && whole % 2 === 0 ? 0 : 1;
  }
  return alternate && whole % 2 === 1 ? 1 - fraction : fraction;
}

/**
 * The hold a step leaves on its unit on top of `base`, or `null` when it leaves none. A step that holds ends frozen
 * where its last iteration ends, its repeats included (SMIL `fill="freeze"`).
 */
export function holdAfter(step: ScheduledStep, base: Hold): Hold | null {
  const effect = step.effect;
  if (effect === null) {
    return null;
  }
  const end = endProgress(step);
  if (effect.effect === 'behaviors') {
    // A finite repeat repeats the whole step: it ends where its last round's progress leaves each track.
    const whole = Number.isFinite(step.iterations) && step.iterations !== 1 && step.durationMs > 0 && end < 1;
    return behaviorsHold(
      effect.params as readonly Tier2Track[],
      base,
      whole ? end * step.durationMs : null,
      step.curve,
    );
  }
  const p = evaluate(step.curve, end);
  switch (effect.effect) {
    case 'growShrink': {
      const { hold, scale } = effect.params;
      return hold ? withTransforms(base, [{ op: 'scale', x: 1 + (scale.x - 1) * p, y: 1 + (scale.y - 1) * p }]) : null;
    }
    case 'spin':
      return effect.params.hold
        ? withTransforms(base, [{ op: 'rotate', deg: (effect.params.cw ? 1 : -1) * effect.params.deg * p }])
        : null;
    case 'transparency':
      return effect.params.hold ? { ...base, opacity: base.opacity * (1 + (effect.params.alpha - 1) * p) } : null;
    case 'colorChange':
      return { ...base, paint: { ...base.paint, [effect.params.property]: effect.params.to } };
    case 'path': {
      if (!effect.params.hold) {
        return null;
      }
      const start = pointAt(effect.params.d, 0);
      const at = pointAt(effect.params.d, p);
      const ops: HoldOp[] = [{ op: 'translate', x: at.x, y: at.y, under: base.transforms.length }];
      if (effect.params.autoRotate) {
        ops.push({ op: 'rotate', deg: at.angle - start.angle });
      }
      return withTransforms(base, ops);
    }
    default:
      return null;
  }
}

export interface SlideState {
  /** Hidden units, by key. */
  readonly hidden: Map<string, Unit>;
  /** Dimmed units (their text takes the colour), by key. */
  readonly dimmed: Map<string, { readonly unit: Unit; readonly color: string }>;
  /** Units an emphasis or a motion path left changed, by key. */
  readonly holds: Map<string, { readonly unit: Unit; readonly hold: Hold }>;
}

/**
 * The state after the first `played` groups have played (0: before group 0, the state a slide mounts in for its
 * transition). A unit whose first step is an entrance starts hidden; an entrance shows it (and clears a dim), an exit
 * or the after effect `hide` hides it, `dim` dims its text. Emphases and paths that hold leave their end state, except
 * the steps in `running` (by index): an endless repeat still running leaves its hold once it ends.
 */
export function slideState(
  timeline: readonly DeckStep[],
  played: number,
  running: ReadonlySet<number> = new Set(),
): SlideState {
  const steps = bounded(timeline);
  const hidden = new Map<string, Unit>();
  const dimmed = new Map<string, { unit: Unit; color: string }>();
  const holds = new Map<string, { unit: Unit; hold: Hold }>();
  const seen = new Set<string>();
  for (const step of steps) {
    for (const unit of stepUnits(step)) {
      const key = unitKey(unit);
      if (!seen.has(key) && step.kind === 'entrance') {
        hidden.set(key, unit);
      }
      seen.add(key);
    }
  }
  for (const group of groups(steps).slice(0, Math.max(0, played))) {
    for (const scheduled of group.steps) {
      const { step } = scheduled;
      for (const unit of stepUnits(step)) {
        const key = unitKey(unit);
        if (step.kind === 'entrance') {
          hidden.delete(key);
          dimmed.delete(key);
        } else if (step.kind === 'exit') {
          hidden.set(key, unit);
        }
        const hold = running.has(scheduled.index) ? null : holdAfter(scheduled, holds.get(key)?.hold ?? NO_HOLD);
        if (hold !== null) {
          holds.set(key, { unit, hold });
        }
        if (scheduled.after === 'hide') {
          hidden.set(key, unit);
        } else if (typeof scheduled.after === 'object') {
          dimmed.set(key, { unit, color: scheduled.after.dim });
        }
      }
    }
  }
  return { hidden, dimmed, holds };
}
