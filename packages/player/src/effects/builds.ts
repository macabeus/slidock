// Build recipes: each effect as keyframes on one target (an element, a paragraph or a text unit).
// Entrances are written forward, from hidden to the static render; an exit `E(dir)` plays as
// `reverse(entrance E(opposite(dir)))` and keeps its other params. Every entrance's first frame is invisible and its
// last frame is the static render, so a finished entrance leaves nothing behind. Transforms compose (`add`) over
// held emphases; opacity scales the element's own opacity.
import type { BuildEffect, StepKind } from '@slidock/types';

import {
  type ClipFrames,
  EDGE,
  blinds,
  checker,
  dirVector,
  dissolve,
  dissolveGrid,
  iris,
  oppositeDir,
  px,
  randomBars,
  sampleClip,
  split,
  strips,
  wipe,
} from './clip';
import { type Curve, readEasing } from './easing';
import { clockMask, softWipeMask, wedgeMask, wheelMask } from './masks';
import { samplePath } from './path';
import { type Paint, fontValue, paintProperty } from './properties';
import { generator } from './random';
import { type Tier2Track, family } from './vocab';

export interface Box {
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

export interface BuildContext {
  /** The target's box in slide pixels. */
  readonly box: Box;
  readonly slide: { readonly width: number; readonly height: number };
  /** A slide-space vector in the target's own transform space (its static rotation and flips undone). */
  readonly local: (dx: number, dy: number) => readonly [number, number];
  /** The target's own opacity: entrances fade to it. */
  readonly opacity: number;
  /** Seeds random bars, dissolve cells and `random`. */
  readonly seed: number;
  /** Whether the registered custom properties are usable (masks, colour change). */
  readonly properties: boolean;
  /** The dissolve grid's cell budget. */
  readonly cells: number;
}

/** One animation of a recipe; timing fields override the step's (tier-2 tracks carry their own). */
export interface Frames {
  readonly keyframes: Keyframe[];
  readonly composite?: CompositeOperation;
  readonly beginMs?: number;
  readonly durationMs?: number;
  readonly curve?: Curve;
  /** The `data-sd-mask` the target carries while it plays (`masks.ts`). */
  readonly mask?: string;
  /**
   * The colour or font the keyframes change (through `paintProperty`): they play on each element under the target
   * that paints it (`paint.ts`), from that element's own value when they have no first frame. Where none paints it,
   * or the registered properties are unusable, the player leaves these frames out.
   */
  readonly paint?: Paint;
  /** The frames' own fill in place of the step's: a switch's `forwards` (it fills no time before its begin). */
  readonly fill?: FillMode;
}

export interface Recipe {
  readonly frames: Frames[];
}

const PERSPECTIVE = 'perspective(800px)';

function translate(ctx: BuildContext, dx: number, dy: number): string {
  const [x, y] = ctx.local(dx, dy);
  return `translate(${px(x)}, ${px(y)})`;
}

function transformFrames(list: readonly (readonly [offset: number, transform: string])[]): Frames {
  return { keyframes: list.map(([offset, transform]) => ({ offset, transform })), composite: 'add' };
}

function fadeIn(ctx: BuildContext, until = 1): Frames {
  return {
    keyframes: [
      { offset: 0, opacity: 0 },
      { offset: until, opacity: ctx.opacity },
      ...(until < 1 ? [{ offset: 1, opacity: ctx.opacity }] : []),
    ],
  };
}

function clipFrames(clip: ClipFrames): Frames {
  return { keyframes: [...sampleClip(clip), { offset: 1, clipPath: 'none' }] };
}

/** The vector that puts the box just outside the slide, moving along `dir`'s components. */
function offSlide(ctx: BuildContext, vx: number, vy: number): readonly [number, number] {
  const { box, slide } = ctx;
  const dx = vx > 0 ? -(box.x + box.w + EDGE) : vx < 0 ? slide.width - box.x + EDGE : 0;
  const dy = vy > 0 ? -(box.y + box.h + EDGE) : vy < 0 ? slide.height - box.y + EDGE : 0;
  return [dx, dy];
}

function axisScale(axis: 'horz' | 'vert' | 'both', s: number): string {
  return axis === 'horz' ? `scale(${s}, 1)` : axis === 'vert' ? `scale(1, ${s})` : `scale(${s}, ${s})`;
}

function axisTurn(axis: 'horz' | 'vert' | 'both', deg: number): string {
  return axis === 'horz'
    ? `${PERSPECTIVE} rotate3d(0, 1, 0, ${deg}deg)`
    : axis === 'vert'
      ? `${PERSPECTIVE} rotate3d(1, 0, 0, ${deg}deg)`
      : `${PERSPECTIVE} rotate3d(1, 1, 0, ${deg}deg)`;
}

/** Entrances `random` picks from. */
const RANDOM_ENTRANCES: readonly BuildEffect[] = [
  { effect: 'fade' },
  { effect: 'flyIn', params: { dir: 'u', distance: 'offSlide' } },
  { effect: 'wipe', params: { dir: 'r', soft: false } },
  { effect: 'split', params: { orient: 'vert', inOut: 'out' } },
  { effect: 'iris', params: { shape: 'circle', inOut: 'out' } },
  { effect: 'blinds', params: { orient: 'horz' } },
  { effect: 'checker', params: { dir: 'r' } },
  { effect: 'randomBars', params: { orient: 'horz' } },
  { effect: 'strips', params: { corner: 'rd' } },
  { effect: 'zoom', params: { inOut: 'in', origin: 'object', slight: false, fade: true, overshoot: false } },
  { effect: 'peek', params: { dir: 'u', slow: false } },
];

/** The entrance `random` plays, the same on every replay for one seed. */
export function randomEntrance(seed: number): BuildEffect {
  const pick = Math.floor(generator(seed)() * RANDOM_ENTRANCES.length);
  return RANDOM_ENTRANCES[pick] ?? { effect: 'fade' };
}

/** The entrance recipe of an entrance-family effect, or `null` when it cannot play here. */
function entrance(effect: BuildEffect, ctx: BuildContext): Recipe | null {
  const { box } = ctx;
  switch (effect.effect) {
    case 'appear':
    case 'typewriter':
      return {
        frames: [
          {
            keyframes: [
              { offset: 0, opacity: 0 },
              { offset: 1, opacity: ctx.opacity },
            ],
          },
        ],
      };
    case 'fade':
      return { frames: [fadeIn(ctx)] };
    case 'dissolve': {
      const { cols, rows } = dissolveGrid(box.w / (box.h || 1), ctx.cells);
      return { frames: [clipFrames(dissolve(cols, rows, ctx.seed))] };
    }
    case 'flyIn': {
      const [vx, vy] = dirVector(effect.params.dir);
      if (effect.params.distance === 'slight') {
        const [dx, dy] = [-vx * box.w * 0.25, -vy * box.h * 0.25];
        return {
          frames: [
            transformFrames([
              [0, translate(ctx, dx, dy)],
              [1, translate(ctx, 0, 0)],
            ]),
            fadeIn(ctx),
          ],
        };
      }
      const [dx, dy] = offSlide(ctx, vx, vy);
      return {
        frames: [
          transformFrames([
            [0, translate(ctx, dx, dy)],
            [1, translate(ctx, 0, 0)],
          ]),
        ],
      };
    }
    case 'floatIn': {
      const dy = effect.params.dir === 'u' ? 40 : -40;
      return {
        frames: [
          transformFrames([
            [0, translate(ctx, 0, dy)],
            [1, translate(ctx, 0, 0)],
          ]),
          fadeIn(ctx),
        ],
      };
    }
    case 'peek': {
      const [vx, vy] = dirVector(effect.params.dir);
      const [tx, ty] = ctx.local(-vx * (box.w + EDGE), -vy * (box.h + EDGE));
      const w = box.w || 1;
      const h = box.h || 1;
      const window = (x: number, y: number): string => {
        const sx = (-x / w) * 100;
        const sy = (-y / h) * 100;
        return `polygon(${sx}% ${sy}%, ${100 + sx}% ${sy}%, ${100 + sx}% ${100 + sy}%, ${sx}% ${100 + sy}%)`;
      };
      return {
        frames: [
          {
            keyframes: [
              { offset: 0, transform: `translate(${px(tx)}, ${px(ty)})`, clipPath: window(tx, ty) },
              { offset: 1, transform: 'translate(0px, 0px)', clipPath: window(0, 0) },
              { offset: 1, transform: 'translate(0px, 0px)', clipPath: 'none' },
            ],
            composite: 'add',
          },
        ],
      };
    }
    case 'wipe':
      if (effect.params.soft && ctx.properties) {
        return { frames: [softWipeMask(effect.params.dir)] };
      }
      return { frames: [clipFrames(wipe(effect.params.dir))] };
    case 'split':
      return { frames: [clipFrames(split(effect.params.orient, effect.params.inOut))] };
    case 'iris':
      return { frames: [clipFrames(iris(effect.params.shape, effect.params.inOut, box.w / (box.h || 1)))] };
    case 'wheel':
      return ctx.properties ? { frames: [wheelMask(effect.params)] } : null;
    case 'clock':
      return ctx.properties ? { frames: [clockMask(effect.params)] } : null;
    case 'wedge':
      return ctx.properties ? { frames: [wedgeMask()] } : null;
    case 'blinds':
      return { frames: [clipFrames(blinds(effect.params.orient))] };
    case 'checker':
      return { frames: [clipFrames(checker(effect.params.dir))] };
    case 'randomBars':
      return { frames: [clipFrames(randomBars(effect.params.orient, ctx.seed))] };
    case 'strips':
      return { frames: [clipFrames(strips(effect.params.corner))] };
    case 'zoom': {
      const { inOut, origin, slight, fade, overshoot } = effect.params;
      const [cx, cy] =
        origin === 'slideCenter'
          ? ctx.local(ctx.slide.width / 2 - (box.x + box.w / 2), ctx.slide.height / 2 - (box.y + box.h / 2))
          : [0, 0];
      const about = (s: number) => `translate(${px(cx)}, ${px(cy)}) scale(${s}) translate(${px(-cx)}, ${px(-cy)})`;
      const from = inOut === 'in' ? (slight ? 0.7 : 0) : slight ? 1.3 : 4;
      const list: [number, string][] = [[0, about(from)]];
      if (overshoot) {
        list.push([0.75, about(inOut === 'in' ? 1.1 : 0.92)]);
      }
      list.push([1, about(1)]);
      const frames: Frames[] = [transformFrames(list)];
      if (fade || slight || inOut === 'out') {
        frames.push(fadeIn(ctx));
      }
      return { frames };
    }
    case 'grow':
    case 'stretch':
      return {
        frames: [
          transformFrames([
            [0, axisScale(effect.params.axis, 0)],
            [1, axisScale(effect.params.axis, 1)],
          ]),
        ],
      };
    case 'expand':
    case 'compress': {
      const from = effect.effect === 'expand' ? 0.7 : 1.4;
      return {
        frames: [
          transformFrames([
            [0, axisScale(effect.params.axis, from)],
            [1, axisScale(effect.params.axis, 1)],
          ]),
          fadeIn(ctx),
        ],
      };
    }
    case 'growTurn':
      return {
        frames: [
          transformFrames([
            [0, 'rotate(90deg) scale(0)'],
            [1, 'rotate(0deg) scale(1)'],
          ]),
        ],
      };
    case 'swivel':
    case 'flip':
      return {
        frames: [
          transformFrames([
            [0, axisTurn(effect.params.axis, 90)],
            [1, axisTurn(effect.params.axis, 0)],
          ]),
          fadeIn(ctx, 0.5),
        ],
      };
    case 'spinIn': {
      const deg = effect.params.cw ? -effect.params.deg : effect.params.deg;
      return {
        frames: [
          transformFrames([
            [0, `rotate(${deg}deg) scale(0)`],
            [1, 'rotate(0deg) scale(1)'],
          ]),
        ],
      };
    }
    case 'bounce':
    case 'drop': {
      const [, top] = offSlide(ctx, 0, 1);
      const h = box.h;
      const keys: [number, number][] =
        effect.effect === 'bounce'
          ? [
              [0, top],
              [0.45, 0],
              [0.6, -h * 0.25],
              [0.75, 0],
              [0.86, -h * 0.08],
              [1, 0],
            ]
          : [
              [0, top],
              [0.7, 0],
              [0.85, -h * 0.05],
              [1, 0],
            ];
      return { frames: [transformFrames(keys.map(([o, y]) => [o, translate(ctx, 0, y)]))] };
    }
    case 'pop':
      return {
        frames: [
          transformFrames([
            [0, 'scale(0)'],
            [0.7, 'scale(1.15)'],
            [1, 'scale(1)'],
          ]),
        ],
      };
    case 'boomerang':
    case 'sling':
    case 'whip':
    case 'swish': {
      // Curved approaches (approximations): from the side, turning and scaling into place.
      const curves: Record<typeof effect.effect, readonly (readonly [number, number, number, number, number])[]> = {
        // [offset, dx (box widths), dy (box heights), rotate (deg), scale]
        boomerang: [
          [0, 1.5, -0.5, -90, 0.3],
          [0.5, -0.4, 0.2, 20, 1.1],
          [1, 0, 0, 0, 1],
        ],
        sling: [
          [0, 0, -1.2, 0, 3],
          [0.6, 0, 0.1, 0, 0.9],
          [1, 0, 0, 0, 1],
        ],
        whip: [
          [0, -1.5, 0, 0, 1],
          [0.6, 0.15, 0, 0, 1],
          [1, 0, 0, 0, 1],
        ],
        swish: [
          [0, 1.2, -0.8, 30, 0.6],
          [0.5, 0.3, -0.3, 10, 0.9],
          [1, 0, 0, 0, 1],
        ],
      };
      const list = curves[effect.effect].map(
        ([o, dx, dy, deg, s]) =>
          [o, `${translate(ctx, dx * box.w, dy * box.h)} rotate(${deg}deg) scale(${s})`] as const,
      );
      return { frames: [transformFrames(list), fadeIn(ctx, 0.3)] };
    }
    case 'credits': {
      const [, dy] = offSlide(ctx, 0, -1);
      return {
        frames: [
          transformFrames([
            [0, translate(ctx, 0, dy)],
            [1, translate(ctx, 0, 0)],
          ]),
        ],
      };
    }
    case 'driftAndScale':
    case 'fadeAndMove': {
      const [vx, vy] = dirVector(effect.params.dir);
      const scale = effect.effect === 'driftAndScale' ? 0.9 : 1;
      return {
        frames: [
          transformFrames([
            [0, `${translate(ctx, -vx * box.w * 0.1, -vy * box.h * 0.1)} scale(${scale})`],
            [1, `${translate(ctx, 0, 0)} scale(1)`],
          ]),
          fadeIn(ctx),
        ],
      };
    }
    case 'random':
      return entrance(randomEntrance(ctx.seed), ctx);
    default:
      return null;
  }
}

function oscillate(offsets: number, amp: number, render: (value: number) => string): Frames {
  const list: [number, string][] = [[0, render(0)]];
  for (let i = 1; i < offsets; i++) {
    list.push([i / offsets, render(i % 2 === 1 ? amp : -amp)]);
  }
  list.push([1, render(0)]);
  return transformFrames(list);
}

/**
 * The recipe of an entrance-family effect played as an emphasis: Keynote's pop, bounce and flip actions. Each starts
 * and ends on the shown element, so a step that repeats plays on without a jump. An entrance effect with no action
 * (`null`) plays its fallback.
 */
function action(effect: BuildEffect, ctx: BuildContext): Recipe | null {
  switch (effect.effect) {
    case 'pop':
      // Up to 115 % and back.
      return {
        frames: [
          transformFrames([
            [0, 'scale(1)'],
            [0.7, 'scale(1.15)'],
            [1, 'scale(1)'],
          ]),
        ],
      };
    case 'bounce': {
      // The entrance's two bounces after it lands, over the whole step.
      const h = ctx.box.h;
      const keys: [number, number][] = [
        [0, 0],
        [0.27, -h * 0.25],
        [0.55, 0],
        [0.75, -h * 0.08],
        [1, 0],
      ];
      return { frames: [transformFrames(keys.map(([o, y]) => [o, translate(ctx, 0, y)]))] };
    }
    case 'flip':
      // A whole turn about its axis.
      return {
        frames: [
          transformFrames([
            [0, axisTurn(effect.params.axis, 0)],
            [1, axisTurn(effect.params.axis, 360)],
          ]),
        ],
      };
    default:
      return null;
  }
}

/** The emphasis recipe of an emphasis-family effect (played forward in every kind). */
function emphasis(effect: BuildEffect, ctx: BuildContext): Recipe | null {
  switch (effect.effect) {
    case 'pulse':
      return {
        frames: [
          transformFrames([
            [0, 'scale(1)'],
            [0.5, `scale(${effect.params.scale})`],
            [1, 'scale(1)'],
          ]),
        ],
      };
    case 'growShrink':
      return {
        frames: [
          transformFrames([
            [0, 'scale(1, 1)'],
            [1, `scale(${effect.params.scale.x}, ${effect.params.scale.y})`],
          ]),
        ],
      };
    case 'spin': {
      const deg = effect.params.cw ? effect.params.deg : -effect.params.deg;
      return {
        frames: [
          transformFrames([
            [0, 'rotate(0deg)'],
            [1, `rotate(${deg}deg)`],
          ]),
        ],
      };
    }
    case 'teeter':
      return { frames: [oscillate(4, 3 + 3 * effect.params.intensity, (a) => `rotate(${a}deg)`)] };
    case 'jiggle':
      return { frames: [oscillate(8, 2 + 4 * effect.params.intensity, (a) => `rotate(${a}deg)`)] };
    case 'wave': {
      const amp = 3 + 3 * effect.params.intensity;
      return {
        frames: [oscillate(4, amp, (a) => `${translate(ctx, 0, -Math.abs(a) * 2)} rotate(${a}deg)`)],
      };
    }
    case 'blink':
    case 'flash': {
      const n = effect.params.repeat;
      const keyframes: Keyframe[] = [{ offset: 0, opacity: ctx.opacity }];
      for (let i = 0; i < n; i++) {
        const start = i / n;
        const half = 0.5 / n;
        if (effect.effect === 'flash') {
          keyframes.push({ offset: start + half * 0.2, opacity: 0 }, { offset: start + half * 1.8, opacity: 0 });
        } else {
          keyframes.push({ offset: start + half, opacity: 0 });
        }
        keyframes.push({ offset: (i + 1) / n, opacity: ctx.opacity });
      }
      return { frames: [{ keyframes }] };
    }
    case 'boldFlash': {
      const n = effect.params.repeat;
      const bold = 'drop-shadow(0.6px 0 0 currentcolor) drop-shadow(-0.6px 0 0 currentcolor)';
      const keyframes: Keyframe[] = [{ offset: 0, filter: 'none' }];
      for (let i = 0; i < n; i++) {
        keyframes.push({ offset: (i + 0.5) / n, filter: bold }, { offset: (i + 1) / n, filter: 'none' });
      }
      // Added to the element's own filter (its printed shadow), which a replacing filter would drop.
      return { frames: [{ keyframes, composite: 'add' }] };
    }
    case 'transparency':
      return {
        frames: [
          {
            keyframes: [
              { offset: 0, opacity: ctx.opacity },
              { offset: 1, opacity: ctx.opacity * effect.params.alpha },
            ],
          },
        ],
      };
    case 'colorChange': {
      const paint = effect.params.property;
      return { frames: [{ keyframes: [{ offset: 1, [paintProperty(paint)]: effect.params.to }], paint }] };
    }
    default:
      return null;
  }
}

/** Sample points of a motion path: enough for curves, bounded. */
const PATH_SAMPLES = 48;

function motion(effect: Extract<BuildEffect, { effect: 'path' }>, ctx: BuildContext): Recipe {
  const samples = samplePath(effect.params.d, PATH_SAMPLES);
  const first = samples[0]?.angle ?? 0;
  return {
    frames: [
      transformFrames(
        samples.map((s, i) => [
          i / PATH_SAMPLES,
          `${translate(ctx, s.x, s.y)} rotate(${effect.params.autoRotate ? s.angle - first : 0}deg)`,
        ]),
      ),
    ],
  };
}

/** The keyframe value of one tier-2 frame, or of a switch. */
function trackValue(property: Exclude<Tier2Track['property'], 'clipPath' | 'set'>, value: unknown, ctx: BuildContext) {
  switch (property) {
    case 'opacity':
      return { opacity: (value as number) * ctx.opacity };
    case 'translate': {
      const offset = value as { x: number; y: number };
      return { transform: translate(ctx, offset.x, offset.y) };
    }
    case 'rotate':
      return { transform: `rotate(${value as number}deg)` };
    case 'scale': {
      const scale = value as { x: number; y: number };
      return { transform: `scale(${scale.x}, ${scale.y})` };
    }
    case 'blur':
      return { filter: `blur(${px(value as number)})` };
    case 'text':
    case 'fill':
    case 'line':
      return { [paintProperty(property)]: value as string };
    case 'bold':
    case 'italic':
    case 'underline':
      // Each value holds until the next frame.
      return { [paintProperty(property)]: fontValue(property, value as boolean), easing: 'steps(1, end)' };
  }
}

const PAINTS: ReadonlySet<string> = new Set<Paint>(['text', 'fill', 'line', 'bold', 'italic', 'underline']);

function isPaint(property: string): property is Paint {
  return PAINTS.has(property);
}

/** Tier 2: one animation per track, timed by the track. */
export function behaviors(tracks: readonly Tier2Track[], ctx: BuildContext): Recipe | null {
  const frames: Frames[] = [];
  for (const track of tracks) {
    const easing = (track as { easing?: unknown }).easing;
    const curve = easing === undefined ? null : readEasing(easing);
    if (easing !== undefined && curve === null) {
      return null;
    }
    const timing = {
      beginMs: track.beginMs,
      durationMs: track.durationMs,
      ...(curve === null ? {} : { curve }),
    };
    if (track.property === 'clipPath') {
      const reveal = entrance(track.reveal as BuildEffect, ctx);
      if (reveal === null) {
        return null;
      }
      for (const part of reveal.frames) {
        frames.push({
          keyframes: track.transition === 'in' ? part.keyframes : reverseKeyframes(part.keyframes),
          ...timing,
          ...(part.mask === undefined ? {} : { mask: part.mask }),
        });
      }
      continue;
    }
    // A switch shows its value from its begin on, with no backward fill, and holds it.
    const set = track.property === 'set';
    const property = set ? track.to.property : track.property;
    const keyframes = (
      set
        ? [
            { offset: 0, value: track.to.value },
            { offset: 1, value: track.to.value },
          ]
        : track.frames
    ).map((frame) => ({
      offset: frame.offset,
      ...trackValue(property, frame.value, ctx),
    }));
    // Transforms add to the element's placement, a blur to its own filter (its printed shadow).
    const added = property === 'translate' || property === 'rotate' || property === 'scale' || property === 'blur';
    frames.push({
      keyframes,
      ...(added ? { composite: 'add' as const } : {}),
      ...(isPaint(property) ? { paint: property } : {}),
      ...(set ? { fill: 'forwards' as const } : {}),
      ...timing,
    });
  }
  return { frames };
}

/** The keyframes played backwards (offsets mirrored, order reversed). */
export function reverseKeyframes(keyframes: readonly Keyframe[]): Keyframe[] {
  const count = keyframes.length;
  const reversed = keyframes
    .map((frame, i) => {
      const offset = typeof frame.offset === 'number' ? frame.offset : count > 1 ? i / (count - 1) : 1;
      return { ...frame, offset: 1 - offset };
    })
    .reverse();
  // An entrance ends on a duplicate keyframe that drops its clip or mask; reversed, the first one must hold at 0,
  // where WAAPI reads the last keyframe of a shared offset.
  if (reversed.length > 2 && reversed[0]?.offset === 0 && reversed[1]?.offset === 0) {
    reversed[1] = { ...reversed[1], offset: 0.001 };
  }
  return reversed;
}

/** The parameters of the entrance an exit reverses: directions turned around, everything else kept. */
export function exitParams(effect: BuildEffect): BuildEffect {
  switch (effect.effect) {
    case 'flyIn':
      return { effect: 'flyIn', params: { ...effect.params, dir: oppositeDir(effect.params.dir) } };
    case 'floatIn':
      return { effect: 'floatIn', params: { dir: effect.params.dir === 'u' ? 'd' : 'u' } };
    case 'peek':
      return { effect: 'peek', params: { ...effect.params, dir: oppositeDir(effect.params.dir) } };
    case 'wipe':
      return { effect: 'wipe', params: { ...effect.params, dir: oppositeDir(effect.params.dir) } };
    case 'checker':
      return { effect: 'checker', params: { dir: oppositeDir(effect.params.dir) } };
    case 'strips':
      return { effect: 'strips', params: { corner: oppositeDir(effect.params.corner) } };
    case 'driftAndScale':
    case 'fadeAndMove':
      return { effect: effect.effect, params: { dir: oppositeDir(effect.params.dir) } };
    default:
      return effect;
  }
}

/** Whether an emphasis keeps its end state after the step. */
export function holds(effect: BuildEffect): boolean {
  switch (effect.effect) {
    case 'growShrink':
    case 'spin':
    case 'transparency':
    case 'path':
      return effect.params.hold;
    case 'colorChange':
      return true;
    default:
      return false;
  }
}

/** The recipe a step of `kind` plays for `effect`, or `null` when it plays its fallback instead. */
export function buildRecipe(effect: BuildEffect, kind: StepKind, ctx: BuildContext): Recipe | null {
  const f = family(effect);
  if (f === 'motion' && effect.effect === 'path') {
    return motion(effect, ctx);
  }
  if (f === 'behaviors' && effect.effect === 'behaviors') {
    return behaviors(effect.params as readonly Tier2Track[], ctx);
  }
  if (f === 'emphasis') {
    return emphasis(effect, ctx);
  }
  if (f !== 'entrance') {
    return null;
  }
  if (kind === 'emphasis') {
    return action(effect, ctx);
  }
  if (kind === 'exit') {
    const recipe = entrance(exitParams(effect), ctx);
    return recipe === null
      ? null
      : { frames: recipe.frames.map((frames) => ({ ...frames, keyframes: reverseKeyframes(frames.keyframes) })) };
  }
  return entrance(effect, ctx);
}

/** The canonical step fallbacks: appear (an instant switch) and dissolve (an opacity fade); none plays nothing. */
export function fallbackRecipe(fallback: 'appear' | 'dissolve' | 'none', kind: StepKind, ctx: BuildContext): Recipe {
  if (fallback === 'none' || (kind !== 'entrance' && kind !== 'exit')) {
    return { frames: [] };
  }
  const recipe = fallback === 'appear' ? entrance({ effect: 'appear' }, ctx) : entrance({ effect: 'fade' }, ctx);
  const frames = recipe?.frames ?? [];
  return kind === 'exit'
    ? { frames: frames.map((f) => ({ ...f, keyframes: reverseKeyframes(f.keyframes) })) }
    : { frames };
}
