// Sample inputs shared by the vector generator and `tests/fdlibm.rs` (`samples` module there): the same seeds,
// draws and IEEE operations in both languages give the same doubles.

const MASK = (1n << 64n) - 1n;
const view = new DataView(new ArrayBuffer(8));

export function fromBits(bits) {
  view.setBigUint64(0, bits & MASK);
  return view.getFloat64(0);
}

export function toBits(x) {
  view.setFloat64(0, x);
  return view.getBigUint64(0);
}

/** NaN payloads differ between engines; every NaN is written as the canonical quiet NaN. */
export function canonicalBits(x) {
  return Number.isNaN(x) ? 0x7ff8000000000000n : toBits(x);
}

export const hex = (bits) => bits.toString(16).padStart(16, '0');

/** splitmix64. */
export class Rng {
  constructor(seed) {
    this.state = BigInt(seed) & MASK;
  }
  next() {
    this.state = (this.state + 0x9e3779b97f4a7c15n) & MASK;
    let z = this.state;
    z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & MASK;
    z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & MASK;
    return z ^ (z >> 31n);
  }
  /** [0, 1) on a 2^-53 grid. */
  unit() {
    return Number(this.next() >> 11n) / 9007199254740992;
  }
  /** An integer in [0, n). */
  below(n) {
    return Math.floor(this.unit() * n);
  }
}

/** 2^e for -1074 ≤ e ≤ 1023. */
export function pow2(e) {
  if (e >= -1022) return fromBits(BigInt(e + 1023) << 52n);
  return fromBits(1n << BigInt(e + 1074));
}

/** Steps `x` by `k` ulps through its bit pattern (k may be negative; sign changes are not handled). */
export function ulpStep(x, k) {
  return fromBits(toBits(x) + BigInt(k));
}

/** A double whose high word is uniform in [lo, lo + count), under a random low word. */
function highWordDouble(lo, count, rng) {
  const high = BigInt((lo + rng.below(count)) >>> 0);
  return fromBits((high << 32n) | (rng.next() & 0xffffffffn));
}

/**
 * The distributions. Each takes the generator and the argument's index and returns one input; two-argument
 * functions draw twice. The Rust test and Jdk.java mirror this table line by line.
 */
export const DISTRIBUTIONS = {
  /** a + (b - a)·u */
  uniform: (a, b) => (rng) => a + (b - a) * rng.unit(),
  /** (u - 0.5) · 2^(lo + ⌊u'·count⌋) */
  signedPow2: (lo, count) => (rng) => {
    const u = rng.unit();
    return (u - 0.5) * pow2(lo + rng.below(count));
  },
  /** u · 2^(lo + ⌊u'·count⌋) */
  positivePow2: (lo, count) => (rng) => {
    const u = rng.unit();
    return u * pow2(lo + rng.below(count));
  },
  /** Any bit pattern: every finite double, infinities, NaNs. */
  anyBits: () => (rng) => fromBits(rng.next()),
  /** Subnormals of either sign. */
  subnormal: () => (rng) => fromBits(rng.next() & 0x800fffffffffffffn),
  /** n·(π/2) for n < count, then up to ±8 ulps. */
  nearHalfPi: (count) => (rng) => {
    const n = rng.below(count);
    return ulpStep(n * (Math.PI / 2), rng.below(17) - 8);
  },
  /** ±(1 - 2^-(1 + ⌊u·52⌋)). */
  nearOne: () => (rng) => {
    const sign = rng.below(2) === 0 ? 1 : -1;
    return sign * (1 - pow2(-1 - rng.below(52)));
  },
  /** High words in [lo, lo + count), one hex (lo, count) per argument (the last one repeats). */
  highWord: (...words) => {
    const ranges = [];
    for (let i = 0; i < words.length; i += 2) ranges.push([parseInt(words[i], 16), parseInt(words[i + 1], 16)]);
    return (rng, arg) => highWordDouble(...ranges[Math.min(arg, ranges.length - 1)], rng);
  },
  /** n·(π/2) for n < count, plus an offset with a high word in [lo, lo + span); hex. */
  reducedHighWord: (count, lo, span) => (rng) => {
    const n = rng.below(parseInt(count, 16));
    return n * (Math.PI / 2) + highWordDouble(parseInt(lo, 16), parseInt(span, 16), rng);
  },
};

export const SPECIALS = [
  0,
  -0,
  Infinity,
  -Infinity,
  NaN,
  1,
  -1,
  0.5,
  -0.5,
  2,
  pow2(-1074),
  -pow2(-1074),
  pow2(-1022),
  Number.MAX_VALUE,
  -Number.MAX_VALUE,
  pow2(-27),
  pow2(-28),
  pow2(-29),
  pow2(-57),
  Math.PI / 4,
  Math.PI / 2,
  Math.PI,
  2 * Math.PI,
  709.782712893384,
  -745.1332191019411,
  1e300,
  -1e-300,
];

/** Seeds per (function, distribution) row. */
export function seedFor(row) {
  let h = 0xcbf29ce484222325n;
  for (const c of row) h = ((h ^ BigInt(c.charCodeAt(0))) * 0x100000001b3n) & MASK;
  return h;
}
