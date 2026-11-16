// Transition recipes: keyframes on the outgoing layer (`out`), the incoming one (`in`), and the
// temporary layers some recipes add (a clone of a layer, a colour overlay), which the runner removes afterwards.
// `dir` is the direction the content moves. Every recipe starts on the outgoing slide and ends on the incoming one
// with no transform, clip or mask left, so the end state is the incoming slide's static render.
import type { TransitionEffect } from '@slidock/types';

import {
  type ClipFrames,
  EDGE,
  blinds,
  checker,
  dirVector,
  dissolve,
  dissolveGrid,
  iris,
  px,
  randomBars,
  sampleClip,
  split,
  strips,
  wipe,
} from './clip';
import { type MaskFrames, clockMask, softWipeMask, wedgeMask, wheelMask } from './masks';
import { generator } from './random';

export interface TransitionScene {
  readonly out: HTMLElement | null;
  readonly into: HTMLElement;
  readonly size: { readonly width: number; readonly height: number };
  readonly seed: number;
  /** Whether the registered custom properties are usable. */
  readonly properties: boolean;
  readonly cells: number;
  /** A temporary copy of a layer, above the incoming one when it copies `in`. */
  clone(layer: HTMLElement): HTMLElement;
  /** A temporary layer of one colour, above both slides. */
  overlay(color: string): HTMLElement;
}

export interface TransitionPart {
  readonly target: HTMLElement | Element;
  readonly keyframes: Keyframe[];
  readonly composite?: CompositeOperation;
  /** The `data-sd-mask` the target carries while it plays (`masks.ts`). */
  readonly mask?: string;
  /**
   * The span of the transition's time the part plays in, as fractions (0 ≤ start < end ≤ 1), the curve whole on it,
   * held before and after. Without it the part plays the whole transition, its offsets on the curve's progress.
   */
  readonly window?: readonly [number, number];
}

export interface TransitionPlan {
  readonly parts: TransitionPart[];
  /** The incoming slide plays beneath the outgoing one. */
  readonly under?: boolean;
  /** Linear where the transition names no easing; else ease-in-out. */
  readonly easing?: 'linear';
  /** Nodes the plan added to a slide, removed when the transition ends. */
  readonly temporary?: readonly Element[];
}

const DISTANCE = 2000;
const PERSPECTIVE = `perspective(${DISTANCE}px)`;

function move(x: number, y: number): string {
  return `translate(${px(x)}, ${px(y)})`;
}

function frames(list: readonly (readonly [number, Keyframe])[]): Keyframe[] {
  return list.map(([offset, frame]) => ({ offset, ...frame }));
}

function clipIn(scene: TransitionScene, clip: ClipFrames): TransitionPlan {
  return { parts: [{ target: scene.into, keyframes: [...sampleClip(clip), { offset: 1, clipPath: 'none' }] }] };
}

function maskIn(scene: TransitionScene, mask: MaskFrames): TransitionPlan | null {
  return scene.properties ? { parts: [{ target: scene.into, ...mask }] } : null;
}

/** Shows `in` at the half way point (a cut inside a longer transition). */
const SWAP_IN: Keyframe[] = frames([
  [0, { opacity: 0 }],
  [0.5, { opacity: 0 }],
  [0.5, { opacity: 1 }],
  [1, { opacity: 1 }],
]);

function throughColor(scene: TransitionScene, color: string): TransitionPlan {
  return {
    parts: [
      { target: scene.into, keyframes: SWAP_IN },
      {
        target: scene.overlay(color),
        keyframes: frames([
          [0, { opacity: 0 }],
          [0.5, { opacity: 1 }],
          [1, { opacity: 0 }],
        ]),
      },
    ],
    easing: 'linear',
  };
}

function outPart(scene: TransitionScene, keyframes: Keyframe[], composite?: CompositeOperation): TransitionPart[] {
  return scene.out === null
    ? []
    : [{ target: scene.out, keyframes, ...(composite === undefined ? {} : { composite }) }];
}

/** Bars of `orient`, the even or the odd ones, shifted by (sx, sy) percent. */
function combBars(orient: 'horz' | 'vert', odd: boolean, sx: number, sy: number, count = 8): string {
  const size = 100 / count;
  const pts: string[] = [];
  const rects: [number, number, number, number][] = [];
  for (let i = odd ? 1 : 0; i < count; i += 2) {
    rects.push(orient === 'horz' ? [0, i * size, 100, (i + 1) * size] : [i * size, 0, (i + 1) * size, 100]);
  }
  for (const [x0, y0, x1, y1] of rects) {
    pts.push(
      `${x0 + sx}% ${y0 + sy}%`,
      `${x1 + sx}% ${y0 + sy}%`,
      `${x1 + sx}% ${y1 + sy}%`,
      `${x0 + sx}% ${y1 + sy}%`,
      `${x0 + sx}% ${y0 + sy}%`,
    );
  }
  for (let i = rects.length - 2; i >= 0; i--) {
    const [x0, y0] = rects[i] as [number, number, number, number];
    pts.push(`${x0 + sx}% ${y0 + sy}%`);
  }
  return `polygon(${pts.join(', ')})`;
}

/** Recipes `random` picks from. */
const RANDOM: readonly TransitionEffect[] = [
  { effect: 'fade', params: { through: 'none' } },
  { effect: 'push', params: { dir: 'l' } },
  { effect: 'cover', params: { dir: 'l' } },
  { effect: 'uncover', params: { dir: 'l' } },
  { effect: 'wipe', params: { dir: 'r', soft: false } },
  { effect: 'split', params: { orient: 'vert', inOut: 'out' } },
  { effect: 'iris', params: { shape: 'circle', inOut: 'out' } },
  { effect: 'blinds', params: { orient: 'horz' } },
  { effect: 'checker', params: { dir: 'r' } },
  { effect: 'randomBars', params: { orient: 'horz' } },
  { effect: 'strips', params: { corner: 'rd' } },
  { effect: 'zoom', params: { inOut: 'in', rotate: false } },
  { effect: 'dissolve' },
];

export function randomTransition(seed: number): TransitionEffect {
  return RANDOM[Math.floor(generator(seed)() * RANDOM.length)] ?? { effect: 'dissolve' };
}

/** The recipe of a transition, or `null` when it plays its fallback (morph and magic move run elsewhere). */
export function transitionRecipe(effect: TransitionEffect, scene: TransitionScene): TransitionPlan | null {
  const { width: W, height: H } = scene.size;
  const aspect = W / (H || 1);
  switch (effect.effect) {
    case 'none':
      return { parts: [] };
    case 'cut':
      return effect.params.throughBlack
        ? {
            parts: [
              { target: scene.into, keyframes: SWAP_IN },
              {
                target: scene.overlay('#000'),
                keyframes: frames([
                  [0, { opacity: 1 }],
                  [0.5, { opacity: 1 }],
                  [0.5, { opacity: 0 }],
                  [1, { opacity: 0 }],
                ]),
              },
            ],
            easing: 'linear',
          }
        : { parts: [] };
    case 'fade':
      if (effect.params.through === 'none') {
        return {
          parts: [
            {
              target: scene.into,
              keyframes: frames([
                [0, { opacity: 0 }],
                [1, { opacity: 1 }],
              ]),
            },
          ],
          easing: 'linear',
        };
      }
      return throughColor(
        scene,
        effect.params.through === 'black'
          ? '#000'
          : effect.params.through === 'white'
            ? '#fff'
            : (effect.params.color ?? '#000'),
      );
    case 'dissolve': {
      const { cols, rows } = dissolveGrid(aspect, scene.cells);
      return { ...clipIn(scene, dissolve(cols, rows, scene.seed)), easing: 'linear' };
    }
    case 'push': {
      const [vx, vy] = dirVector(effect.params.dir);
      return {
        parts: [
          {
            target: scene.into,
            keyframes: frames([
              [0, { transform: move(-vx * (W + EDGE), -vy * (H + EDGE)) }],
              [1, { transform: move(0, 0) }],
            ]),
          },
          ...outPart(
            scene,
            frames([
              [0, { transform: move(0, 0) }],
              [1, { transform: move(vx * W, vy * H) }],
            ]),
          ),
        ],
      };
    }
    case 'cover': {
      const [vx, vy] = dirVector(effect.params.dir);
      return {
        parts: [
          {
            target: scene.into,
            keyframes: frames([
              [0, { transform: move(-vx * (W + EDGE), -vy * (H + EDGE)) }],
              [1, { transform: move(0, 0) }],
            ]),
          },
        ],
      };
    }
    case 'uncover': {
      const [vx, vy] = dirVector(effect.params.dir);
      return {
        parts: outPart(
          scene,
          frames([
            [0, { transform: move(0, 0) }],
            [1, { transform: move(vx * W, vy * H) }],
          ]),
        ),
        under: true,
      };
    }
    case 'wipe':
      return effect.params.soft && scene.properties
        ? { parts: [{ target: scene.into, ...softWipeMask(effect.params.dir) }] }
        : clipIn(scene, wipe(effect.params.dir));
    case 'split':
      return clipIn(scene, split(effect.params.orient, effect.params.inOut));
    case 'iris':
      return clipIn(scene, iris(effect.params.shape, effect.params.inOut, aspect));
    case 'wheel':
      return maskIn(scene, wheelMask(effect.params));
    case 'clock':
      return maskIn(scene, clockMask(effect.params));
    case 'wedge':
      return maskIn(scene, wedgeMask());
    case 'blinds':
      return clipIn(scene, blinds(effect.params.orient));
    case 'checker':
      return clipIn(scene, checker(effect.params.dir));
    case 'randomBars':
      return { ...clipIn(scene, randomBars(effect.params.orient, scene.seed)), easing: 'linear' };
    case 'strips':
      return clipIn(scene, strips(effect.params.corner));
    case 'comb': {
      const horz = effect.params.orient === 'horz';
      const half = (target: HTMLElement, odd: boolean): TransitionPart => {
        const sign = odd ? 1 : -1;
        const [tx, ty] = horz ? [sign * W, 0] : [0, sign * H];
        const [sx, sy] = horz ? [-sign * 100, 0] : [0, -sign * 100];
        return {
          target,
          keyframes: [
            { offset: 0, transform: move(tx, ty), clipPath: combBars(effect.params.orient, odd, sx, sy) },
            { offset: 1, transform: move(0, 0), clipPath: combBars(effect.params.orient, odd, 0, 0) },
            { offset: 1, transform: move(0, 0), clipPath: 'none' },
          ],
        };
      };
      return { parts: [half(scene.into, false), half(scene.clone(scene.into), true)] };
    }
    case 'zoom': {
      const turn = (deg: number) => (effect.params.rotate ? ` rotate(${deg}deg)` : '');
      if (effect.params.inOut === 'in') {
        return {
          parts: [
            {
              target: scene.into,
              keyframes: frames([
                [0, { transform: `scale(0.3)${turn(-720)}`, opacity: 0 }],
                [1, { transform: `scale(1)${turn(0)}`, opacity: 1 }],
              ]),
            },
          ],
        };
      }
      return {
        parts: outPart(
          scene,
          frames([
            [0, { transform: `scale(1)${turn(0)}`, opacity: 1 }],
            [1, { transform: `scale(1.5)${turn(720)}`, opacity: 0 }],
          ]),
        ),
        under: true,
      };
    }
    case 'flip':
    case 'rotate3d':
    case 'twist': {
      const [vx, vy] = dirVector(effect.params.dir);
      const axis = vx !== 0 ? 'Y' : 'X';
      const sign = vx !== 0 ? vx : -vy;
      const end = effect.effect === 'flip' ? 180 : 90;
      const small = effect.effect === 'twist' ? 0.8 : 1;
      const face = (deg: number, scale: number) => `${PERSPECTIVE} rotate${axis}(${deg}deg) scale(${scale})`;
      return {
        parts: [
          {
            target: scene.into,
            keyframes: frames([
              [0, { transform: face(-sign * end, small), opacity: 0 }],
              [0.5, { transform: face(-sign * end * 0.5, small), opacity: 0 }],
              [0.5, { transform: face(-sign * end * 0.5, small), opacity: 1 }],
              [1, { transform: face(0, 1), opacity: 1 }],
            ]),
          },
          ...outPart(
            scene,
            frames([
              [0, { transform: face(0, 1), opacity: 1 }],
              [0.5, { transform: face(sign * end * 0.5, small), opacity: 1 }],
              [0.5, { transform: face(sign * end * 0.5, small), opacity: 0 }],
              [1, { transform: face(sign * end, small), opacity: 0 }],
            ]),
          ),
        ],
      };
    }
    case 'cube': {
      const [vx, vy] = dirVector(effect.params.dir);
      const horizontal = vx !== 0;
      const depth = (horizontal ? W : H) / 2;
      const a = horizontal ? 90 * vx : -90 * vy;
      const face = (deg: number) =>
        horizontal
          ? `${PERSPECTIVE} translateZ(${px(-depth)}) rotateY(${deg}deg) translateZ(${px(depth)})`
          : `${PERSPECTIVE} translateZ(${px(-depth)}) rotateX(${deg}deg) translateZ(${px(depth)})`;
      // A face shows only while it faces the viewer (cos θ > d / (P + d)): the layers stack in a fixed order, so
      // a face turned away is hidden rather than culled.
      const turned = 1 - (Math.acos(depth / (DISTANCE + depth)) * 180) / Math.PI / 90;
      return {
        parts: [
          {
            target: scene.into,
            keyframes: [
              { offset: 0, transform: face(-a), opacity: 0 },
              { offset: turned, transform: face(-a * (1 - turned)), opacity: 0 },
              { offset: turned, transform: face(-a * (1 - turned)), opacity: 1 },
              { offset: 1, transform: face(0), opacity: 1 },
            ],
          },
          ...outPart(scene, [
            { offset: 0, transform: face(0), opacity: 1 },
            { offset: 1 - turned, transform: face(a * (1 - turned)), opacity: 1 },
            { offset: 1 - turned, transform: face(a * (1 - turned)), opacity: 0 },
            { offset: 1, transform: face(a), opacity: 0 },
          ]),
        ],
      };
    }
    case 'doors':
    case 'window': {
      if (scene.out === null) {
        return null;
      }
      const horz = effect.params.orient === 'horz';
      const turn = horz ? 'rotateX' : 'rotateY';
      const shift = horz ? 'translateY' : 'translateX';
      const half = horz ? H / 2 : W / 2;
      const door = (pivot: number, deg: number) =>
        `${PERSPECTIVE} ${shift}(${px(pivot)}) ${turn}(${deg}deg) ${shift}(${px(-pivot)})`;
      const first = horz ? 'inset(0 0 50% 0)' : 'inset(0 50% 0 0)';
      const second = horz ? 'inset(50% 0 0 0)' : 'inset(0 0 0 50%)';
      const open = effect.effect === 'doors' ? 90 : 75;
      const sign = horz ? 1 : -1;
      return {
        parts: [
          {
            target: scene.out,
            keyframes: [
              { offset: 0, transform: door(-half, 0), clipPath: first, opacity: 1 },
              { offset: 1, transform: door(-half, sign * open), clipPath: first, opacity: 0 },
            ],
          },
          {
            target: scene.clone(scene.out),
            keyframes: [
              { offset: 0, transform: door(half, 0), clipPath: second, opacity: 1 },
              { offset: 1, transform: door(half, -sign * open), clipPath: second, opacity: 0 },
            ],
          },
          {
            target: scene.into,
            keyframes: frames([
              [0, { transform: `scale(${effect.effect === 'doors' ? 0.8 : 0.5})` }],
              [1, { transform: 'scale(1)' }],
            ]),
          },
        ],
        under: true,
      };
    }
    case 'gallery':
    case 'switch': {
      const [vx, vy] = dirVector(effect.params.dir);
      const deg = effect.effect === 'gallery' ? 30 : 60;
      const turn = (s: number) => (vx !== 0 ? `rotateY(${s * deg * -vx}deg)` : `rotateX(${s * deg * vy}deg)`);
      const span = effect.effect === 'gallery' ? 1.4 : 0.6;
      return {
        parts: [
          {
            target: scene.into,
            keyframes: frames([
              [
                0,
                {
                  transform: `${PERSPECTIVE} ${move(-vx * W * span, -vy * H * span)} ${turn(-1)} scale(0.8)`,
                  opacity: effect.effect === 'switch' ? 0 : 1,
                },
              ],
              [1, { transform: `${PERSPECTIVE} ${move(0, 0)} ${turn(0)} scale(1)`, opacity: 1 }],
            ]),
          },
          ...outPart(
            scene,
            frames([
              [0, { transform: `${PERSPECTIVE} ${move(0, 0)} ${turn(0)} scale(1)`, opacity: 1 }],
              [
                1,
                {
                  transform: `${PERSPECTIVE} ${move(vx * W * span, vy * H * span)} ${turn(1)} scale(0.8)`,
                  opacity: effect.effect === 'switch' ? 0 : 1,
                },
              ],
            ]),
          ),
        ],
      };
    }
    case 'conveyor': {
      const [vx, vy] = dirVector(effect.params.dir);
      return {
        parts: [
          {
            target: scene.into,
            keyframes: frames([
              [0, { transform: `${move(-vx * (W + EDGE), -vy * (H + EDGE))} scale(0.8)` }],
              [0.3, { transform: `${move(-vx * W * 0.7, -vy * H * 0.7)} scale(0.8)` }],
              [0.7, { transform: `${move(0, 0)} scale(0.8)` }],
              [1, { transform: `${move(0, 0)} scale(1)` }],
            ]),
          },
          ...outPart(
            scene,
            frames([
              [0, { transform: `${move(0, 0)} scale(1)` }],
              [0.3, { transform: `${move(0, 0)} scale(0.8)` }],
              [0.7, { transform: `${move(vx * W * 0.7, vy * H * 0.7)} scale(0.8)` }],
              [1, { transform: `${move(vx * W, vy * H)} scale(0.8)` }],
            ]),
          ),
        ],
      };
    }
    case 'ferris': {
      // Both slides ride a wheel whose hub is far below (or beside) the stage: sampled along the arc, so each
      // keyframe is a plain translate and rotate.
      const [vx, vy] = dirVector(effect.params.dir);
      const radius = Math.max(W, H) * 2.5;
      const sign = vx !== 0 ? vx : vy;
      const spoke = (deg: number) => {
        const rad = (deg * Math.PI) / 180;
        const along = radius * Math.sin(rad);
        const drop = radius * (1 - Math.cos(rad));
        return vx !== 0 ? `${move(along, drop)} rotate(${deg}deg)` : `${move(drop, along)} rotate(${-deg}deg)`;
      };
      const arc = (from: number, to: number): Keyframe[] =>
        Array.from({ length: 9 }, (_, i) => ({ offset: i / 8, transform: spoke(from + ((to - from) * i) / 8) }));
      return {
        parts: [{ target: scene.into, keyframes: arc(-40 * sign, 0) }, ...outPart(scene, arc(0, 40 * sign))],
      };
    }
    case 'pan': {
      const [vx, vy] = dirVector(effect.params.dir);
      return {
        parts: [
          {
            target: scene.into,
            keyframes: frames([
              [0, { transform: move(-vx * (W + EDGE), -vy * (H + EDGE)) }],
              [1, { transform: move(0, 0) }],
            ]),
          },
          ...outPart(
            scene,
            frames([
              [0, { transform: move(0, 0) }],
              [1, { transform: move(vx * W * 0.3, vy * H * 0.3) }],
            ]),
          ),
        ],
      };
    }
    case 'reveal': {
      const [vx, vy] = dirVector(effect.params.dir);
      const plan: TransitionPlan = {
        parts: [
          {
            target: scene.into,
            keyframes: frames([
              [0, { opacity: 0 }],
              [effect.params.throughBlack ? 0.5 : 0, { opacity: 0 }],
              [1, { opacity: 1 }],
            ]),
          },
          ...outPart(
            scene,
            frames([
              [0, { transform: move(0, 0), opacity: 1 }],
              [effect.params.throughBlack ? 0.5 : 1, { transform: move(vx * W * 0.3, vy * H * 0.3), opacity: 0 }],
              [1, { transform: move(vx * W * 0.3, vy * H * 0.3), opacity: 0 }],
            ]),
          ),
        ],
        under: true,
      };
      return plan;
    }
    case 'flythrough': {
      const inward = effect.params.inOut === 'in';
      const from = inward ? 2 : 0.5;
      const list: [number, Keyframe][] = [[0, { transform: `scale(${from})`, opacity: 0 }]];
      if (effect.params.bounce) {
        list.push([0.8, { transform: `scale(${inward ? 0.95 : 1.05})`, opacity: 1 }]);
      }
      list.push([1, { transform: 'scale(1)', opacity: 1 }]);
      return {
        parts: [
          { target: scene.into, keyframes: frames(list) },
          ...outPart(
            scene,
            frames([
              [0, { transform: 'scale(1)', opacity: 1 }],
              [1, { transform: `scale(${inward ? 0.5 : 2})`, opacity: 0 }],
            ]),
          ),
        ],
      };
    }
    case 'swap': {
      const [vx, vy] = dirVector(effect.params.dir);
      return {
        parts: [
          {
            target: scene.into,
            keyframes: frames([
              [0, { transform: `${move(-vx * W * 0.4, -vy * H * 0.4)} scale(0.8)`, opacity: 0 }],
              [0.5, { transform: `${move(-vx * W * 0.4, -vy * H * 0.4)} scale(0.8)`, opacity: 1 }],
              [1, { transform: `${move(0, 0)} scale(1)`, opacity: 1 }],
            ]),
          },
          ...outPart(
            scene,
            frames([
              [0, { transform: `${move(0, 0)} scale(1)` }],
              [0.5, { transform: `${move(vx * W * 0.4, vy * H * 0.4)} scale(0.8)` }],
              [1, { transform: `${move(vx * W * 0.4, vy * H * 0.4)} scale(0.8)` }],
            ]),
          ),
        ],
      };
    }
    case 'pageFlip': {
      const [vx, vy] = dirVector(effect.params.dir);
      const horizontal = vx !== 0;
      // The page turns about its leading edge, the spine on the `dir` side: its free edge lifts towards the viewer
      // and travels along `dir`, uncovering the incoming slide from the opposite side.
      const pivot = horizontal ? (vx * W) / 2 : (vy * H) / 2;
      const hinge = (deg: number) =>
        horizontal
          ? `${PERSPECTIVE} translateX(${px(pivot)}) rotateY(${deg}deg) translateX(${px(-pivot)})`
          : `${PERSPECTIVE} translateY(${px(pivot)}) rotateX(${deg}deg) translateY(${px(-pivot)})`;
      const end = horizontal ? 90 * vx : -90 * vy;
      return {
        parts: outPart(
          scene,
          frames([
            [0, { transform: hinge(0), opacity: 1 }],
            [0.99, { transform: hinge(end * 0.99), opacity: 1 }],
            [1, { transform: hinge(end), opacity: 0 }],
          ]),
        ),
        under: true,
      };
    }
    case 'mosaic': {
      const [vx, vy] = dirVector(effect.params.dir);
      const { cols, rows } = dissolveGrid(aspect, Math.min(scene.cells, 144));
      const jitter = generator(scene.seed);
      const noise = Array.from({ length: cols * rows }, () => jitter() * 0.2);
      const ordered = (c: number, r: number) => {
        const along =
          vx !== 0
            ? vx > 0
              ? c / (cols - 1 || 1)
              : 1 - c / (cols - 1 || 1)
            : vy > 0
              ? r / (rows - 1 || 1)
              : 1 - r / (rows - 1 || 1);
        return Math.min(1, along * 0.8 + (noise[r * cols + c] ?? 0));
      };
      return clipIn(scene, dissolve(cols, rows, scene.seed, ordered));
    }
    case 'random':
      return transitionRecipe(randomTransition(scene.seed), scene);
    case 'morph':
    case 'magicMove':
      return null;
  }
}
