// Motion paths: the IR's path (slide pixels from the element's static top-left) flattened and resampled at even
// arc lengths, so a path plays as translate (and, auto-rotated, rotate) keyframes rather than `offset-path`.
import type { PathSeg } from '@slidock/types';

export interface PathSample {
  readonly x: number;
  readonly y: number;
  /** The tangent's angle in degrees, clockwise from the x axis. */
  readonly angle: number;
}

type Point = { x: number; y: number };

/** Curve subdivisions per cubic segment. */
const CURVE_STEPS = 16;
/** The most points a flattened path keeps (paths are bounded to 4,096 segments). */
const MAX_POINTS = 8_192;

/** A cubic bezier's coordinate at `t`, from `a` with control points `b` and `c` to `d`. */
export function cubic(a: number, b: number, c: number, d: number, t: number): number {
  const u = 1 - t;
  return u * u * u * a + 3 * u * u * t * b + 3 * u * t * t * c + t * t * t * d;
}

/** The path as a polyline. */
export function flatten(d: readonly PathSeg[]): Point[] {
  const out: Point[] = [];
  let start: Point = { x: 0, y: 0 };
  let at: Point = { x: 0, y: 0 };
  const steps = Math.max(1, Math.min(CURVE_STEPS, Math.floor(MAX_POINTS / Math.max(1, d.length))));
  for (const seg of d) {
    if (out.length >= MAX_POINTS) {
      break;
    }
    switch (seg[0]) {
      case 'M':
        at = { x: seg[1], y: seg[2] };
        start = at;
        out.push(at);
        break;
      case 'L':
        at = { x: seg[1], y: seg[2] };
        out.push(at);
        break;
      case 'C': {
        const [, x1, y1, x2, y2, x, y] = seg;
        for (let i = 1; i <= steps; i++) {
          const t = i / steps;
          out.push({ x: cubic(at.x, x1, x2, x, t), y: cubic(at.y, y1, y2, y, t) });
        }
        at = { x, y };
        break;
      }
      case 'Z':
        at = start;
        out.push(at);
        break;
    }
  }
  return out;
}

/** `count + 1` points at even arc lengths along the path, each with its tangent. */
export function samplePath(d: readonly PathSeg[], count: number): PathSample[] {
  const points = flatten(d);
  const first = points[0] ?? { x: 0, y: 0 };
  if (points.length < 2) {
    return Array.from({ length: count + 1 }, () => ({ x: first.x, y: first.y, angle: 0 }));
  }
  const lengths = [0];
  for (let i = 1; i < points.length; i++) {
    const a = points[i - 1] as Point;
    const b = points[i] as Point;
    lengths.push((lengths[i - 1] as number) + Math.hypot(b.x - a.x, b.y - a.y));
  }
  const total = lengths[lengths.length - 1] as number;
  const samples: PathSample[] = [];
  let segment = 1;
  let lastAngle = 0;
  for (let i = 0; i <= count; i++) {
    const target = total === 0 ? 0 : (i / count) * total;
    while (segment < points.length - 1 && (lengths[segment] as number) < target) {
      segment++;
    }
    const a = points[segment - 1] as Point;
    const b = points[segment] as Point;
    const l0 = lengths[segment - 1] as number;
    const l1 = lengths[segment] as number;
    const t = l1 > l0 ? (target - l0) / (l1 - l0) : 0;
    if (b.x !== a.x || b.y !== a.y) {
      lastAngle = (Math.atan2(b.y - a.y, b.x - a.x) * 180) / Math.PI;
    }
    samples.push({ x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t, angle: lastAngle });
  }
  return samples;
}

/** The path's point at arc-length fraction `progress`. */
export function pointAt(d: readonly PathSeg[], progress: number): PathSample {
  const samples = samplePath(d, 256);
  const index = Math.round(Math.min(1, Math.max(0, progress)) * 256);
  return samples[index] ?? { x: 0, y: 0, angle: 0 };
}
