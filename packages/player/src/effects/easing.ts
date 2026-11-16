// Timing curves: the IR's named curves and a bezier map to `cubic-bezier()`, and PPTX's
// accelerate/decelerate to a `linear()` curve sampled from its three segments. `evaluate` computes a curve in script,
// for the held state of a step that stops part-way (a fractional repeat). A curve plays on the keyframes
// (`easeKeyframes`), not as the effect's own easing, which WebKit's compositor draws as linear.
import { cubic } from './path';

export type Curve =
  | { readonly kind: 'bezier'; readonly x1: number; readonly y1: number; readonly x2: number; readonly y2: number }
  | { readonly kind: 'accelDecel'; readonly accel: number; readonly decel: number };

const NAMED: Readonly<Record<'linear' | 'in' | 'out' | 'in-out', Curve>> = {
  linear: { kind: 'bezier', x1: 0, y1: 0, x2: 1, y2: 1 },
  in: { kind: 'bezier', x1: 0.42, y1: 0, x2: 1, y2: 1 },
  out: { kind: 'bezier', x1: 0, y1: 0, x2: 0.58, y2: 1 },
  'in-out': { kind: 'bezier', x1: 0.42, y1: 0, x2: 0.58, y2: 1 },
};

export const LINEAR: Curve = NAMED.linear;
export const EASE_IN_OUT: Curve = NAMED['in-out'];

/** The largest bezier ordinate either way (the IR's `BEZIER_Y`). */
const BEZIER_Y = 10;

function finite(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

/** The curve an IR easing names, or `null` for a value this player does not read. */
export function readEasing(easing: unknown): Curve | null {
  if (typeof easing === 'string') {
    return easing in NAMED ? NAMED[easing as keyof typeof NAMED] : null;
  }
  if (typeof easing !== 'object' || easing === null) {
    return null;
  }
  const keys = Object.keys(easing);
  if (keys.length !== 1) {
    return null;
  }
  const bezier = (easing as { bezier?: unknown }).bezier;
  if (Array.isArray(bezier)) {
    const [x1, y1, x2, y2] = bezier as unknown[];
    if (
      bezier.length === 4 &&
      finite(x1) &&
      finite(y1) &&
      finite(x2) &&
      finite(y2) &&
      x1 >= 0 &&
      x1 <= 1 &&
      x2 >= 0 &&
      x2 <= 1 &&
      Math.abs(y1) <= BEZIER_Y &&
      Math.abs(y2) <= BEZIER_Y
    ) {
      return { kind: 'bezier', x1, y1, x2, y2 };
    }
    return null;
  }
  const accelDecel = (easing as { accelDecel?: unknown }).accelDecel;
  if (typeof accelDecel === 'object' && accelDecel !== null) {
    const { accel, decel } = accelDecel as { accel?: unknown; decel?: unknown };
    if (
      Object.keys(accelDecel).length === 2 &&
      finite(accel) &&
      finite(decel) &&
      accel >= 0 &&
      decel >= 0 &&
      accel + decel <= 1 + 1e-9
    ) {
      return { kind: 'accelDecel', accel, decel };
    }
  }
  return null;
}

function round(value: number): number {
  return Math.round(value * 1e5) / 1e5;
}

let linearSupport: boolean | null = null;

/** Whether the browser reads CSS `linear()` easing. */
function supportsLinear(): boolean {
  linearSupport ??= typeof CSS !== 'undefined' && CSS.supports('animation-timing-function', 'linear(0, 1)');
  return linearSupport;
}

/** The CSS easing of a curve. */
export function cssEasing(curve: Curve): string {
  if (curve.kind === 'bezier') {
    const { x1, y1, x2, y2 } = curve;
    if (x1 === 0 && y1 === 0 && x2 === 1 && y2 === 1) {
      return 'linear';
    }
    return `cubic-bezier(${round(x1)}, ${round(y1)}, ${round(x2)}, ${round(y2)})`;
  }
  const { accel, decel } = curve;
  if (accel === 0 && decel === 0) {
    return 'linear';
  }
  if (supportsLinear()) {
    return linearPart(curve, 0, 1, 0, 1);
  }
  return accel > 0 && decel > 0 ? 'ease-in-out' : accel > 0 ? 'ease-in' : 'ease-out';
}

/** Samples of a curve per unit of time in a `linear()`. */
const SAMPLES = 64;

/** A `linear()` of `curve` from time `t0` to `t1` (progress `p0` to `p1`), both scaled to 0–1, through its samples. */
function linearPart(curve: Curve, t0: number, t1: number, p0: number, p1: number): string {
  const point = (t: number, p: number) => `${round((p - p0) / (p1 - p0))} ${round(((t - t0) / (t1 - t0)) * 100)}%`;
  const points = [point(t0, p0)];
  for (let j = Math.floor(t0 * SAMPLES) + 1; j < t1 * SAMPLES; j++) {
    points.push(point(j / SAMPLES, evaluate(curve, j / SAMPLES)));
  }
  points.push(point(t1, p1));
  return `linear(${points.join(', ')})`;
}

/** The progress a curve reaches at time fraction `t` (0 to 1). */
export function evaluate(curve: Curve, t: number): number {
  const time = Math.min(1, Math.max(0, t));
  if (time === 0 || time === 1) {
    return time;
  }
  if (curve.kind === 'accelDecel') {
    const { accel, decel } = curve;
    const peak = 1 / (1 - accel / 2 - decel / 2);
    if (time < accel) {
      return (peak * time * time) / (2 * accel);
    }
    if (time > 1 - decel) {
      const left = 1 - time;
      return 1 - (peak * left * left) / (2 * decel);
    }
    return peak * (time - accel / 2);
  }
  // Solve x(s) = time by bisection (x is monotonic for x1, x2 in [0, 1]), then read y(s).
  let lo = 0;
  let hi = 1;
  for (let i = 0; i < 40; i++) {
    const mid = (lo + hi) / 2;
    if (cubic(0, curve.x1, curve.x2, 1, mid) < time) {
      lo = mid;
    } else {
      hi = mid;
    }
  }
  return cubic(0, curve.y1, curve.y2, 1, (lo + hi) / 2);
}

/**
 * Keyframes that play `curve` on their own, for an effect whose own easing is linear: WebKit's compositor draws an
 * effect's own easing as linear but keeps each keyframe's. Two keyframes take the curve whole; more are re-timed,
 * each offset moved to the time the curve reaches it, and each segment takes its part of the curve as a `linear()`.
 * Null where the keyframes cannot carry the curve (an implicit end, given offsets off the even grid of missing ones,
 * an easing of their own, a curve that overshoots, no `linear()`): the effect keeps it.
 */
export function easeKeyframes(keyframes: Keyframe[], curve: Curve): Keyframe[] | null {
  const css = cssEasing(curve);
  const last = keyframes.length - 1;
  // Missing offsets spread evenly, which is how WAAPI spaces them while every offset given sits on that grid.
  const spread = keyframes.some((frame) => typeof frame.offset !== 'number');
  const offsets = keyframes.map((frame, i) => (spread ? i / last : (frame.offset ?? 0)));
  if (css === 'linear') {
    return keyframes;
  }
  if (
    offsets[0] !== 0 ||
    offsets[last] !== 1 ||
    keyframes.some((frame, i) => frame.easing !== undefined || (frame.offset ?? offsets[i]) !== offsets[i])
  ) {
    return null;
  }
  if (last < 2) {
    return keyframes.map((frame, i) => (i === 0 ? { ...frame, easing: css } : frame));
  }
  if (!supportsLinear() || (curve.kind === 'bezier' && Math.min(curve.y1, curve.y2, 1 - curve.y1, 1 - curve.y2) < 0)) {
    return null;
  }
  // Each offset's time: when the curve reaches it.
  const times = offsets.map((p) => {
    let [lo, hi] = [0, 1];
    for (let i = 0; i < 24; i++) {
      const mid = (lo + hi) / 2;
      [lo, hi] = evaluate(curve, mid) < p ? [mid, hi] : [lo, mid];
    }
    return round(hi);
  });
  return keyframes.map((frame, i) => {
    const [t0, t1, p0, p1] = [times[i] ?? 1, times[i + 1] ?? 1, offsets[i] ?? 1, offsets[i + 1] ?? 1];
    return { ...frame, offset: t0, ...(t1 > t0 && p1 > p0 ? { easing: linearPart(curve, t0, t1, p0, p1) } : {}) };
  });
}
