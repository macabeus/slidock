// Writes the fdlibm reference vectors from the JavaScript engine running it (Node 24, V8 13.6):
//   node crates/slidock-math/tests/data/generate.mjs
// - fdlibm.tsv: `fn  dist  x  [y]  result` with every double as 16 hex digits of its bits; the engine's Math
//   result, or for cbrt and hypot the correctly rounded result (V8's cbrt is FreeBSD's later algorithm and its
//   hypot is its own, so neither is fdlibm's).
// - fdlibm-sweep.tsv: `fn  dist  params  count  sha256` over the big-endian result bits of `count` samples.
// - quantise.tsv: `x  pt  ratio`, the IR's quantise formula evaluated by the engine.
import { createHash } from 'node:crypto';
import { writeFileSync } from 'node:fs';
import { DISTRIBUTIONS, Rng, SPECIALS, canonicalBits, hex, pow2, seedFor, toBits } from './samples.mjs';

const dir = new URL('.', import.meta.url);

// fn, distribution, parameters, explicit vectors, sweep samples.
const ROWS = [
  ['sin', 'uniform', [-1e6, 1e6], 600, 100_000],
  ['sin', 'signedPow2', [1, 1020], 600, 100_000],
  ['sin', 'nearHalfPi', [1_000_000], 600, 100_000],
  ['sin', 'anyBits', [], 300, 50_000],
  ['cos', 'uniform', [-1e6, 1e6], 600, 100_000],
  ['cos', 'signedPow2', [1, 1020], 600, 100_000],
  ['cos', 'nearHalfPi', [1_000_000], 600, 100_000],
  ['cos', 'anyBits', [], 300, 50_000],
  ['tan', 'uniform', [-1e3, 1e3], 600, 100_000],
  ['tan', 'signedPow2', [-40, 1060], 600, 100_000],
  ['tan', 'nearHalfPi', [1_000_000], 600, 100_000],
  ['tan', 'anyBits', [], 300, 50_000],
  ['asin', 'uniform', [-1, 1], 600, 100_000],
  ['asin', 'nearOne', [], 300, 50_000],
  ['asin', 'signedPow2', [-70, 70], 300, 100_000],
  ['acos', 'uniform', [-1, 1], 600, 100_000],
  ['acos', 'nearOne', [], 300, 50_000],
  ['acos', 'signedPow2', [-70, 70], 300, 100_000],
  ['atan', 'signedPow2', [-40, 80], 600, 100_000],
  ['atan', 'anyBits', [], 300, 50_000],
  ['exp', 'uniform', [-745, 1455], 600, 100_000],
  ['exp', 'signedPow2', [-60, 70], 600, 100_000],
  ['exp', 'anyBits', [], 300, 50_000],
  ['log', 'positivePow2', [-1000, 2000], 600, 100_000],
  ['log', 'uniform', [0.5, 1.5], 600, 100_000],
  ['log', 'subnormal', [], 300, 50_000],
  ['log', 'anyBits', [], 300, 50_000],
  ['cbrt', 'signedPow2', [-1000, 2000], 600, 0],
  ['cbrt', 'subnormal', [], 300, 0],
  ['cbrt', 'anyBits', [], 300, 0],
  ['atan2', 'signedPow2', [-20, 40], 600, 100_000],
  ['atan2', 'signedPow2', [-1000, 2000], 600, 100_000],
  ['atan2', 'anyBits', [], 300, 50_000],
  ['hypot', 'signedPow2', [-1000, 2000], 600, 0],
  ['hypot', 'signedPow2', [-20, 40], 600, 0],
  ['hypot', 'subnormal', [], 200, 0],
  // The subnormal cut, where StrictMath lacks FreeBSD's fix.
  ['hypot', 'highWord', ['000ffffe', '3'], 600, 0],
  ['hypot', 'anyBits', [], 300, 0],
];

const ARITY = { atan2: 2, hypot: 2 };

const EXACT = { cbrt: exactCbrt, hypot: exactHypot };
const reference = (fn) => EXACT[fn] ?? Math[fn];

const ENGINE = `# engine\tnode=${process.version}\tv8=${process.versions.v8}\tarch=${process.arch}`;
const vectors = [ENGINE, '# fn\tdist\tx\t[y]\tresult'];
const sweeps = [ENGINE, '# fn\tdist\tparams\tcount\tsha256'];

for (const [fn, dist, params, explicit, sweep] of ROWS) {
  const draw = DISTRIBUTIONS[dist](...params);
  const key = `${fn}\t${dist}\t${params.join(',')}`;
  const rng = new Rng(seedFor(key));
  const f = reference(fn);
  const hash = createHash('sha256');
  const word = Buffer.alloc(8);
  for (let i = 0; i < Math.max(explicit, sweep); i++) {
    const args = ARITY[fn] === 2 ? [draw(rng, 0), draw(rng, 1)] : [draw(rng, 0)];
    const result = f(...args);
    if (i < explicit) vectors.push([fn, dist, ...args.map((a) => hex(toBits(a))), hex(canonicalBits(result))].join('\t'));
    if (i < sweep) {
      word.writeBigUInt64BE(canonicalBits(result));
      hash.update(word);
    }
  }
  if (sweep > 0) sweeps.push(`${key}\t${sweep}\t${hash.digest('hex')}`);
}

for (const fn of ['sin', 'cos', 'tan', 'asin', 'acos', 'atan', 'exp', 'log', 'cbrt']) {
  for (const x of SPECIALS) vectors.push([fn, 'special', hex(toBits(x)), hex(canonicalBits(reference(fn)(x)))].join('\t'));
}
for (const fn of ['atan2', 'hypot']) {
  for (const y of SPECIALS) {
    for (const x of SPECIALS) {
      const result = reference(fn)(y, x);
      vectors.push([fn, 'special', hex(toBits(y)), hex(toBits(x)), hex(canonicalBits(result))].join('\t'));
    }
  }
}

writeFileSync(new URL('fdlibm.tsv', dir), vectors.join('\n') + '\n');
writeFileSync(new URL('fdlibm-sweep.tsv', dir), sweeps.join('\n') + '\n');

// quantise: the IR formula, Number((Math.round(x / q) * q).toFixed(d)) with -0 as 0.
const QUANTUM = { pt: [0.001, 3], ratio: [0.00001, 5] };
function quantise(x, [q, d]) {
  const r = Number((Math.round(x / q) * q).toFixed(d));
  return r === 0 ? 0 : r;
}
const quantised = ['# x\tpt\tratio'];
const qrng = new Rng(seedFor('quantise'));
const inputs = [...SPECIALS, 7.1994, 7.1995, 7.1996, 2.675, 1.0005, 0.0005, -0.0005, -0.0015, 0.1 + 0.2, 1e-7, 33.333333];
for (let i = 0; i < 4000; i++) inputs.push(DISTRIBUTIONS.uniform(-1e5, 2e5)(qrng));
for (let i = 0; i < 2000; i++) inputs.push(DISTRIBUTIONS.signedPow2(-30, 100)(qrng));
for (let i = 0; i < 1000; i++) inputs.push(DISTRIBUTIONS.anyBits()(qrng));
// Exact and near ties of both grids, and their neighbours.
for (let i = 0; i < 2000; i++) {
  const [q] = i % 2 ? QUANTUM.pt : QUANTUM.ratio;
  const k = qrng.below(2e8) - 1e8;
  const tie = (k + 0.5) * q;
  inputs.push(tie, tie + Number.EPSILON * Math.abs(tie), tie - Number.EPSILON * Math.abs(tie));
}
// Large step counts, where toFixed is the only path.
for (let i = 0; i < 1000; i++) inputs.push(DISTRIBUTIONS.signedPow2(40, 40)(qrng));
for (const x of inputs) {
  quantised.push([x, quantise(x, QUANTUM.pt), quantise(x, QUANTUM.ratio)].map((v) => hex(canonicalBits(v))).join('\t'));
}
writeFileSync(new URL('quantise.tsv', dir), quantised.join('\n') + '\n');

/** √(x² + y²) correctly rounded (round half to even), with exact BigInt arithmetic. */
function exactHypot(x, y) {
  if (x === Infinity || x === -Infinity || y === Infinity || y === -Infinity) return Infinity;
  if (Number.isNaN(x) || Number.isNaN(y)) return NaN;
  const a = dyadic(x);
  const b = dyadic(y);
  if (a.m === 0n && b.m === 0n) return 0;
  // x² + y² = s · 2^(2e).
  const e = Math.min(a.e, b.e);
  const s = (a.m << BigInt(a.e - e)) ** 2n + (b.m << BigInt(b.e - e)) ** 2n;
  // root = ⌊√(s · 4^k)⌋ with at least 64 bits, so the result is root · 2^(e - k) plus a sticky remainder.
  const k = Math.max(0, Math.ceil((128 - s.toString(2).length) / 2));
  const scaled = s << BigInt(2 * k);
  const root = isqrt(scaled);
  const inexact = root * root !== scaled;
  return roundDyadic(root, inexact, e - k);
}

/** ∛x correctly rounded (round half to even), with exact BigInt arithmetic. */
function exactCbrt(x) {
  if (!Number.isFinite(x) || x === 0) return x;
  const { m, e } = dyadic(x);
  // m · 2^e = M · 2^(3q) with M ≥ 2^192, so ⌊∛M⌋ has at least 64 bits.
  let s = 192;
  while ((((e - s) % 3) + 3) % 3 !== 0) s++;
  const big = m << BigInt(s);
  const root = icbrt(big);
  const result = roundDyadic(root, root ** 3n !== big, (e - s) / 3);
  return x < 0 ? -result : result;
}

function icbrt(n) {
  let x = 1n << BigInt(Math.ceil(n.toString(2).length / 3) + 1);
  for (;;) {
    const next = (2n * x + n / (x * x)) / 3n;
    if (next >= x) break;
    x = next;
  }
  while (x ** 3n > n) x -= 1n;
  while ((x + 1n) ** 3n <= n) x += 1n;
  return x;
}

/** |x| = m · 2^e with an integer m. */
function dyadic(x) {
  const raw = toBits(Math.abs(x));
  const biased = Number(raw >> 52n);
  const fraction = raw & ((1n << 52n) - 1n);
  return biased === 0 ? { m: fraction, e: -1074 } : { m: fraction | (1n << 52n), e: biased - 1075 };
}

function isqrt(n) {
  if (n < 2n) return n;
  let x = 1n << BigInt(Math.ceil(n.toString(2).length / 2));
  for (;;) {
    const next = (x + n / x) >> 1n;
    if (next >= x) return x;
    x = next;
  }
}

/** (m + sticky) · 2^e rounded to the nearest double, ties to even. */
function roundDyadic(m, sticky, e) {
  const bits = m.toString(2).length;
  // Keep 53 bits, or fewer when the result is subnormal (its last bit is 2^-1074).
  let keep = 53;
  const top = e + bits - 1; // exponent of the leading bit
  if (top < -1022) keep = 53 - (-1022 - top);
  if (keep <= 0) keep = 0;
  const drop = bits - keep;
  if (drop <= 0) return Number(m) * pow2Wide(e);
  const kept = m >> BigInt(drop);
  const rest = m & ((1n << BigInt(drop)) - 1n);
  const half = 1n << BigInt(drop - 1);
  let rounded = kept;
  if (rest > half || (rest === half && (sticky || (kept & 1n) === 1n))) rounded += 1n;
  return Number(rounded) * pow2Wide(e + drop);
}

/** 2^e for any e, by splitting it into two in-range factors (exact while the product is representable). */
function pow2Wide(e) {
  if (e > 1023) return pow2(1023) * pow2Wide(e - 1023);
  if (e < -1074) return pow2(-1074) * pow2Wide(e + 1074);
  return pow2(e);
}
