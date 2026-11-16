// Clip generators: pure functions of the reveal progress (0 hidden, 1 shown) that return a
// `clip-path`, sampled into keyframes. Coordinates are percentages of the clipped box, so one generator serves an
// element and a slide layer. Bar sets (blinds, random bars, strips, checkerboard, dissolve) are one `polygon()` of
// many rectangles joined by zero-width bridges; the vertex count stays constant, unrevealed bars collapse to a point,
// so WAAPI interpolates the keyframes. Shapes that grow inward are the box minus the shape (`evenodd`).
import type { Diagonal, Dir, Dir4, IrisShape, Orient } from '@slidock/types';

import { permutation } from './random';

/** Width over height of the clipped box, for shapes that must stay round or square on screen. */
export type Aspect = number;

export interface ClipFrames {
  /** Progress values to sample (0 to 1, rising): the generator is linear between them. */
  readonly samples: readonly number[];
  readonly at: (progress: number) => string;
}

type Rect = readonly [x0: number, y0: number, x1: number, y1: number];
type Point = readonly [x: number, y: number];

function clamp01(value: number): number {
  return Math.min(1, Math.max(0, value));
}

function fmt(value: number): string {
  const rounded = Math.round(value * 1000) / 1000;
  return `${Object.is(rounded, -0) ? 0 : rounded}%`;
}

function points(list: readonly Point[]): string {
  return list.map(([x, y]) => `${fmt(x)} ${fmt(y)}`).join(', ');
}

/** One polygon covering every rectangle (percent coordinates). */
export function barSet(rects: readonly Rect[]): string {
  if (rects.length === 0) {
    return 'polygon(0% 0%, 0% 0%, 0% 0%)';
  }
  const out: Point[] = [];
  for (const [x0, y0, x1, y1] of rects) {
    out.push([x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]);
  }
  for (let i = rects.length - 2; i >= 0; i--) {
    const rect = rects[i] as Rect;
    out.push([rect[0], rect[1]]);
  }
  return `polygon(${points(out)})`;
}

/** Evenly spaced samples, `count` intervals. */
export function evenly(count: number): number[] {
  return Array.from({ length: count + 1 }, (_, i) => i / count);
}

/** Pixels past an edge an off-stage or off-box start keeps, so no antialiased sliver shows. */
export const EDGE = 2;

/** A CSS length in px, rounded to a thousandth (never `-0`). */
export function px(value: number): string {
  const rounded = Math.round(value * 1000) / 1000;
  return `${Object.is(rounded, -0) ? 0 : rounded}px`;
}

/** The unit vector of a direction (screen y grows down). */
export function dirVector(dir: Dir): Point {
  switch (dir) {
    case 'l':
      return [-1, 0];
    case 'r':
      return [1, 0];
    case 'u':
      return [0, -1];
    case 'd':
      return [0, 1];
    case 'lu':
      return [-1, -1];
    case 'ru':
      return [1, -1];
    case 'ld':
      return [-1, 1];
    case 'rd':
      return [1, 1];
  }
}

export function oppositeDir<D extends Dir>(dir: D): D {
  const map: Record<Dir, Dir> = { l: 'r', r: 'l', u: 'd', d: 'u', lu: 'rd', rd: 'lu', ru: 'ld', ld: 'ru' };
  return map[dir] as D;
}

/** A wipe whose leading edge moves along `dir`, revealing from the trailing side. */
export function wipe(dir: Dir): ClipFrames {
  const [vx, vy] = dirVector(dir);
  if (vx === 0 || vy === 0) {
    return {
      samples: [0, 1],
      at: (p) => {
        const hidden = (1 - clamp01(p)) * 100;
        // inset(top right bottom left): a rightward wipe hides the right part, a downward one the bottom.
        const [t, r, b, l] = [vy < 0 ? hidden : 0, vx > 0 ? hidden : 0, vy > 0 ? hidden : 0, vx < 0 ? hidden : 0];
        return `inset(${fmt(t)} ${fmt(r)} ${fmt(b)} ${fmt(l)})`;
      },
    };
  }
  // Diagonal: the region u + v <= t of the box, mirrored for the direction, t from 0 to 2.
  return {
    samples: [0, 0.5, 1],
    at: (p) => {
      const t = clamp01(p) * 2;
      const raw: Point[] = [
        [0, 0],
        [Math.min(t, 1), 0],
        [Math.min(t, 1), Math.max(0, t - 1)],
        [Math.max(0, t - 1), Math.min(t, 1)],
        [0, Math.min(t, 1)],
      ];
      return `polygon(${points(raw.map(([u, v]) => [(vx > 0 ? u : 1 - u) * 100, (vy > 0 ? v : 1 - v) * 100]))})`;
    },
  };
}

/** A split along a line of `orient`: outward from the line, or inward from the edges. */
export function split(orient: Orient, inOut: 'in' | 'out'): ClipFrames {
  return {
    samples: [0, 1],
    at: (p) => {
      const q = clamp01(p) * 50;
      if (inOut === 'out') {
        return orient === 'horz' ? barSet([[0, 50 - q, 100, 50 + q]]) : barSet([[50 - q, 0, 50 + q, 100]]);
      }
      return orient === 'horz'
        ? barSet([
            [0, 0, 100, q],
            [0, 100 - q, 100, 100],
          ])
        : barSet([
            [0, 0, q, 100],
            [100 - q, 0, 100, 100],
          ]);
    },
  };
}

const CIRCLE_POINTS = 48;

/** The outline of an iris shape at scale `s` (1 covers the box), in percent coordinates. */
function shapeOutline(shape: IrisShape, s: number, aspect: Aspect): Point[] {
  switch (shape) {
    case 'circle': {
      // A circle on screen: its radius is s times the half diagonal, an ellipse in percent coordinates.
      const half = Math.hypot(aspect, 1) / 2;
      const rx = ((s * half) / aspect) * 100;
      const ry = s * half * 100;
      return Array.from({ length: CIRCLE_POINTS }, (_, i) => {
        const a = (i / CIRCLE_POINTS) * 2 * Math.PI;
        return [50 + rx * Math.sin(a), 50 - ry * Math.cos(a)] as Point;
      });
    }
    case 'diamond':
      return [
        [50, 50 - s * 100],
        [50 + s * 100, 50],
        [50, 50 + s * 100],
        [50 - s * 100, 50],
      ];
    case 'box':
      return [
        [50 - s * 50, 50 - s * 50],
        [50 + s * 50, 50 - s * 50],
        [50 + s * 50, 50 + s * 50],
        [50 - s * 50, 50 + s * 50],
      ];
    case 'plus': {
      const long = s * 100;
      const half = s * 50;
      return [
        [50 - half, 50 - long],
        [50 + half, 50 - long],
        [50 + half, 50 - half],
        [50 + long, 50 - half],
        [50 + long, 50 + half],
        [50 + half, 50 + half],
        [50 + half, 50 + long],
        [50 - half, 50 + long],
        [50 - half, 50 + half],
        [50 - long, 50 + half],
        [50 - long, 50 - half],
        [50 - half, 50 - half],
      ];
    }
  }
}

/** An iris: the shape grows from the centre (`out`), or the box shows from the edges inward (`in`). */
export function iris(shape: IrisShape, inOut: 'in' | 'out', aspect: Aspect): ClipFrames {
  if (shape === 'circle' && inOut === 'out') {
    return { samples: [0, 1], at: (p) => `circle(${fmt(clamp01(p) * 70.72)} at 50% 50%)` };
  }
  if (inOut === 'out') {
    return { samples: [0, 1], at: (p) => `polygon(${points(shapeOutline(shape, clamp01(p), aspect))})` };
  }
  return {
    samples: shape === 'circle' ? evenly(4) : [0, 1],
    at: (p) => {
      // Slightly larger than the box at the start, so no corner shows before the reveal begins.
      const outline = shapeOutline(shape, (1 - clamp01(p)) * 1.05, aspect);
      const first = outline[0] as Point;
      // The outer ring passes outside the box, so the box's own edges never meet the even-odd boundary.
      const box: Point[] = [
        [-10, -10],
        [110, -10],
        [110, 110],
        [-10, 110],
        [-10, -10],
      ];
      return `polygon(evenodd, ${points([...box, ...outline, first, [-10, -10]])})`;
    },
  };
}

/** Blinds: bars of `orient` growing together. */
export function blinds(orient: Orient, count = 8): ClipFrames {
  return {
    samples: [0, 1],
    at: (p) => {
      const size = 100 / count;
      const grown = clamp01(p) * size;
      return barSet(
        Array.from({ length: count }, (_, i): Rect =>
          orient === 'horz' ? [0, i * size, 100, i * size + grown] : [i * size, 0, i * size + grown, 100],
        ),
      );
    },
  };
}

/** Each of `count` items shows over a window starting at `starts[i]`; how far along item `i` is at `p`. */
function staggered(starts: readonly number[], window: number): (i: number, p: number) => number {
  return (i, p) => clamp01((p - (starts[i] ?? 0)) / window);
}

/** Random bars: thin bars of `orient` appearing one by one in a seeded order. */
export function randomBars(orient: Orient, seed: number, count = 48): ClipFrames {
  const order = permutation(count, seed);
  const window = 1 / count;
  const starts = order.map((rank) => rank * window);
  const along = staggered(starts, window);
  return {
    samples: evenly(16),
    at: (p) => {
      const size = 100 / count;
      return barSet(
        Array.from({ length: count }, (_, i): Rect => {
          const half = (along(i, p) * size) / 2;
          const mid = i * size + size / 2;
          return orient === 'horz' ? [0, mid - half, 100, mid + half] : [mid - half, 0, mid + half, 100];
        }),
      );
    },
  };
}

/** A grid of `cols` by `rows` cells; `cell(c, r, p)` is a cell's rectangle at progress `p`. */
function grid(
  cols: number,
  rows: number,
  cell: (c: number, r: number, x0: number, y0: number, w: number, h: number, p: number) => Rect,
): (p: number) => string {
  const w = 100 / cols;
  const h = 100 / rows;
  return (p) => {
    const rects: Rect[] = [];
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        rects.push(cell(c, r, c * w, r * h, w, h, p));
      }
    }
    return barSet(rects);
  };
}

/** A cell wiped from its trailing side along `dir` by fraction `f`. */
function wipeCell(dir: Dir4, x0: number, y0: number, w: number, h: number, f: number): Rect {
  switch (dir) {
    case 'r':
      return [x0, y0, x0 + w * f, y0 + h];
    case 'l':
      return [x0 + w * (1 - f), y0, x0 + w, y0 + h];
    case 'd':
      return [x0, y0, x0 + w, y0 + h * f];
    case 'u':
      return [x0, y0 + h * (1 - f), x0 + w, y0 + h];
  }
}

/** A checkerboard: alternate cells wipe along `dir` in the first half, the others in the second. */
export function checker(dir: Dir4, cols = 8, rows = 6): ClipFrames {
  return {
    samples: [0, 0.5, 1],
    at: grid(cols, rows, (c, r, x0, y0, w, h, p) => {
      const f = (c + r) % 2 === 0 ? clamp01(p * 2) : clamp01(p * 2 - 1);
      return wipeCell(dir, x0, y0, w, h, f);
    }),
  };
}

/** Strips: diagonal bands of cells moving toward `corner`, each cell growing from the far corner. */
export function strips(corner: Diagonal, cols = 16, rows = 9): ClipFrames {
  const window = 0.3;
  const span = cols + rows - 2 || 1;
  const [vx, vy] = dirVector(corner);
  return {
    samples: evenly(10),
    at: grid(cols, rows, (c, r, x0, y0, w, h, p) => {
      const cc = vx > 0 ? c : cols - 1 - c;
      const rr = vy > 0 ? r : rows - 1 - r;
      const start = ((cc + rr) / span) * (1 - window);
      const f = clamp01((p - start) / window);
      const x = vx > 0 ? x0 : x0 + w * (1 - f);
      const y = vy > 0 ? y0 : y0 + h * (1 - f);
      return [x, y, x + w * f, y + h * f];
    }),
  };
}

/** A dissolve: a grid of cells appearing in a seeded order, each growing from its centre. */
export function dissolve(
  cols: number,
  rows: number,
  seed: number,
  ordered?: (c: number, r: number) => number,
): ClipFrames {
  const count = cols * rows;
  const order = permutation(count, seed);
  const window = Math.max(1 / 12, 1 / count);
  const starts = Array.from({ length: count }, (_, i) => {
    const rank = ordered === undefined ? (order[i] as number) / count : ordered(i % cols, Math.floor(i / cols));
    return rank * (1 - window);
  });
  const along = staggered(starts, window);
  return {
    samples: evenly(12),
    at: grid(cols, rows, (c, r, x0, y0, w, h, p) => {
      const f = along(r * cols + c, p);
      const cx = x0 + w / 2;
      const cy = y0 + h / 2;
      return [cx - (w * f) / 2, cy - (h * f) / 2, cx + (w * f) / 2, cy + (h * f) / 2];
    }),
  };
}

/** The dissolve grid's cell budget for this device: fewer cores, fewer polygon vertices (slow on low-end hardware). */
export function cellBudget(cores: number | undefined): number {
  const n = typeof cores === 'number' && Number.isFinite(cores) ? cores : 4;
  return n >= 8 ? 576 : n >= 4 ? 288 : 128;
}

/** The grid of a dissolve over a box of `aspect`, within `budget` cells (32 x 18 at most). */
export function dissolveGrid(aspect: Aspect, budget: number): { cols: number; rows: number } {
  const safe = Number.isFinite(aspect) && aspect > 0 ? Math.min(16, Math.max(1 / 16, aspect)) : 16 / 9;
  const cells = Math.max(1, Math.min(576, Math.floor(budget)));
  let rows = Math.max(1, Math.round(Math.sqrt(cells / safe)));
  let cols = Math.max(1, Math.floor(cells / rows));
  cols = Math.min(cols, 32);
  rows = Math.min(rows, 18);
  return { cols, rows };
}

/** Samples a clip into keyframe values at its sample points, offsets in progress terms. */
export function sampleClip(clip: ClipFrames): { offset: number; clipPath: string }[] {
  return clip.samples.map((offset) => ({ offset, clipPath: clip.at(offset) }));
}
