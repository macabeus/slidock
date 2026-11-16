// Writes the JavaScript reference vectors for slidock-canon from the engine running it (Node 24):
//   node crates/slidock-canon/tests/data/generate.mjs
// Doubles are written as 16 hex digits of their bits; strings as JSON.
//   number-string.tsv  x  String(x)
//   math-round.tsv     x  Math.round(x)
//   to-fixed.tsv       x  digits  x.toFixed(digits)
//   grid.tsv           x  formatNumber(quantise(x, 'pt'))  formatNumber(quantise(x, 'ratio'))
//   whitespace.txt     the code points /\s/ matches
//   strings.jsonl      {s, length, trim, trimStart, trimEnd, slices: [[start, end, s.slice(start, end) | null]]}
//                      (null where the slice splits a surrogate pair)
//   sorted.json        random strings in Array.prototype.sort order
//   json-strings.tsv   code points (hex, comma-separated)  JSON.stringify(string)
//   canonical.jsonl    {input: JSON text, compact, indented} (both null when canonical JSON refuses the value)
//   sha256.tsv         JSON string  sha256 hex of its UTF-8 bytes
import { createHash } from 'node:crypto';
import { writeFileSync } from 'node:fs';

const dir = new URL('.', import.meta.url);
const MASK = (1n << 64n) - 1n;
const view = new DataView(new ArrayBuffer(8));
const fromBits = (b) => (view.setBigUint64(0, b & MASK), view.getFloat64(0));
const toBits = (x) => (view.setFloat64(0, x), view.getBigUint64(0));
const hex = (x) => toBits(x).toString(16).padStart(16, '0');
const write = (name, lines) => writeFileSync(new URL(name, dir), lines.join('\n') + '\n');

/** splitmix64 */
class Rng {
  constructor(seed) {
    this.state = BigInt(seed);
  }
  next() {
    this.state = (this.state + 0x9e3779b97f4a7c15n) & MASK;
    let z = this.state;
    z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & MASK;
    z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & MASK;
    return z ^ (z >> 31n);
  }
  unit() {
    return Number(this.next() >> 11n) / 2 ** 53;
  }
  below(n) {
    return Math.floor(this.unit() * n);
  }
  pick(list) {
    return list[this.below(list.length)];
  }
  bits() {
    return fromBits(this.next());
  }
  /** A decimal literal with up to `digits` significant digits and exponent in [lo, hi). */
  decimal(digits, lo, hi) {
    const mantissa = String(1 + this.below(10 ** this.below(digits + 1)));
    const sign = this.below(2) ? '-' : '';
    return Number(`${sign}${mantissa}e${lo + this.below(hi - lo)}`);
  }
}

const rng = new Rng(0x51d0c);
const nextUp = (x) => fromBits(toBits(x) + 1n);
const nextDown = (x) => fromBits(toBits(x) - 1n);
const SPECIAL = [0, -0, NaN, Infinity, -Infinity, Number.MAX_VALUE, -Number.MAX_VALUE, Number.MIN_VALUE, 2 ** -1022];

// String(n)
{
  const xs = [...SPECIAL, 0.1 + 0.2, 1e21, nextDown(1e21), 1e-6, nextDown(1e-6), 1e-7, 123e-20, 2 ** 53, 2 ** 53 + 2];
  for (let e = -324; e <= 308; e++) {
    const p = Number(`1e${e}`);
    xs.push(p, -p, nextUp(p), nextDown(p));
  }
  for (let i = 0; i < 6000; i++) xs.push(rng.bits());
  for (let i = 0; i < 4000; i++) xs.push(rng.decimal(17, -30, 30));
  for (let i = 0; i < 2000; i++) xs.push(rng.decimal(6, -8, 8));
  for (let i = 0; i < 1000; i++) xs.push(Math.floor(rng.unit() * 2 ** (rng.below(70))) * (rng.below(2) ? 1 : -1));
  write('number-string.tsv', xs.map((x) => `${hex(x)}\t${String(x)}`));
}

// Math.round
{
  const xs = [...SPECIAL, 0.5, -0.5, 1.5, -1.5, 2.5, -2.5, 0.49999999999999994, -0.49999999999999994, 2 ** 52, 2 ** 52 - 0.5];
  xs.push(-(2 ** 52) + 0.5, 2 ** 51 + 0.5, -(2 ** 51) - 0.5, 1e-300, -1e-300, 4503599627370495.5, -4503599627370495.5);
  for (let i = 0; i < 3000; i++) xs.push(rng.bits());
  for (let i = 0; i < 3000; i++) {
    const tie = rng.below(2 ** (1 + rng.below(52))) * (rng.below(2) ? 1 : -1) + 0.5;
    xs.push(tie, nextUp(tie), nextDown(tie));
  }
  for (let i = 0; i < 2000; i++) xs.push((rng.unit() - 0.5) * 2 ** rng.below(60));
  write('math-round.tsv', xs.map((x) => `${hex(x)}\t${hex(Math.round(x))}`));
}

// toFixed
{
  const cases = [];
  const add = (x, d) => cases.push(`${hex(x)}\t${d}\t${x.toFixed(d)}`);
  for (const x of [...SPECIAL, 1.005, 2.675, 1.45, -1.45, 0.5, -0.5, 1.5, 2.5, -0.0001, -1e-10, 1e21, nextDown(1e21), -1e21]) {
    for (const d of [0, 1, 2, 3, 5, 10, 20, 100]) add(x, d);
  }
  for (let i = 0; i < 4000; i++) add(rng.decimal(10, -10, 6), rng.below(21));
  for (let i = 0; i < 2000; i++) {
    // Decimal ties at the rounding digit: their doubles sit just above or below the tie.
    const d = rng.below(10);
    const k = rng.below(10 ** (1 + rng.below(8)));
    add(Number(`${rng.below(2) ? '-' : ''}${k}5e-${d + 1}`), d);
  }
  for (let i = 0; i < 2000; i++) add(rng.bits(), rng.below(101));
  for (let i = 0; i < 2000; i++) add((rng.unit() - 0.5) * 2 ** (rng.below(150) - 80), rng.below(101));
  write('to-fixed.tsv', cases);
}

// formatNumber(quantise(x, unit))
{
  const QUANTUM = { pt: [0.001, 3], ratio: [0.00001, 5] };
  const quantise = (x, [q, d]) => {
    const r = Number((Math.round(x / q) * q).toFixed(d));
    return r === 0 ? 0 : r;
  };
  const formatNumber = (n) => {
    const text = String(n === 0 ? 0 : n);
    if (!Number.isFinite(n) || text.includes('e')) throw new RangeError(text);
    return text;
  };
  const xs = [7.1994, 7.1995, 7.1996, 2.675, 1.0005, 0.0005, -0.0005, -0.0015, 0.1 + 0.2, 1e-7, 33.333333, -0];
  for (let i = 0; i < 4000; i++) xs.push(-1e5 + 2e5 * rng.unit());
  for (let i = 0; i < 3000; i++) xs.push((rng.unit() - 0.5) * 2 ** (rng.below(70) - 40));
  for (let i = 0; i < 3000; i++) {
    const [q] = i % 2 ? QUANTUM.pt : QUANTUM.ratio;
    const tie = (rng.below(2e8) - 1e8 + 0.5) * q;
    xs.push(tie, nextUp(tie), nextDown(tie));
  }
  for (let i = 0; i < 1000; i++) xs.push(rng.decimal(8, -6, 9));
  const lines = xs
    .filter((x) => Math.abs(x) <= 1e9)
    .map((x) => `${hex(x)}\t${formatNumber(quantise(x, QUANTUM.pt))}\t${formatNumber(quantise(x, QUANTUM.ratio))}`);
  write('grid.tsv', lines);
}

// Strings
const WHITESPACE = [];
for (let c = 0; c <= 0x10ffff; c++) {
  if (c >= 0xd800 && c <= 0xdfff) continue;
  if (/\s/u.test(String.fromCodePoint(c))) WHITESPACE.push(c.toString(16));
}
write('whitespace.txt', WHITESPACE);

const POOLS = [
  () => 0x20 + rng.below(0x5f),
  () => rng.below(0x20),
  () => 0x80 + rng.below(0x780),
  () => rng.pick([...WHITESPACE.map((h) => parseInt(h, 16)), 0x85, 0x180e, 0x200b, 0x2060]),
  () => 0xe000 + rng.below(0x2000),
  () => 0x10000 + rng.below(0x100000),
];
function randomString(maxLength) {
  let s = '';
  const n = rng.below(maxLength + 1);
  for (let i = 0; i < n; i++) s += String.fromCodePoint(rng.pick(POOLS)());
  return s;
}
const lone = /[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/;

{
  const lines = [];
  for (let i = 0; i < 3000; i++) {
    const s = randomString(12);
    const slices = [];
    for (let j = 0; j < 4; j++) {
      const a = rng.below(s.length + 3);
      const b = rng.below(s.length + 3);
      const sliced = s.slice(a, b);
      slices.push([a, b, lone.test(sliced) ? null : sliced]);
    }
    lines.push(JSON.stringify({ s, length: s.length, trim: s.trim(), trimStart: s.trimStart(), trimEnd: s.trimEnd(), slices }));
  }
  write('strings.jsonl', lines);
}

{
  const list = [];
  for (let i = 0; i < 3000; i++) list.push(randomString(6));
  list.sort();
  writeFileSync(new URL('sorted.json', dir), JSON.stringify(list, null, 0) + '\n');
}

{
  const lines = [];
  for (let c = 0; c < 0x3000; c++) {
    if (c >= 0xd800 && c <= 0xdfff) continue;
    lines.push(`${c.toString(16)}\t${JSON.stringify(String.fromCodePoint(c))}`);
  }
  for (let i = 0; i < 2000; i++) {
    const s = randomString(10);
    lines.push(`${[...s].map((ch) => ch.codePointAt(0).toString(16)).join(',')}\t${JSON.stringify(s)}`);
  }
  write('json-strings.tsv', lines);
}

// Canonical JSON: keys sorted by code unit, compact or two-space indent with a trailing newline.
{
  const formatNumber = (n) => {
    const text = String(n === 0 ? 0 : n);
    if (text.includes('e')) throw new RangeError(text);
    return text;
  };
  const print = (value, indent, depth) => {
    if (value === null) return 'null';
    if (typeof value === 'boolean') return String(value);
    if (typeof value === 'number') return formatNumber(value);
    if (typeof value === 'string') return JSON.stringify(value);
    const open = indent ? '\n' + indent.repeat(depth + 1) : '';
    const close = indent ? '\n' + indent.repeat(depth) : '';
    const sep = ',' + open;
    if (Array.isArray(value)) {
      if (!value.length) return '[]';
      return '[' + open + value.map((v) => print(v, indent, depth + 1)).join(sep) + close + ']';
    }
    const keys = Object.keys(value).sort();
    if (!keys.length) return '{}';
    const colon = indent ? ': ' : ':';
    return '{' + open + keys.map((k) => JSON.stringify(k) + colon + print(value[k], indent, depth + 1)).join(sep) + close + '}';
  };
  const NUMBERS = [() => rng.decimal(8, -6, 15), () => rng.below(1e6), () => rng.bits(), () => rng.decimal(17, -25, 25)];
  const randomValue = (depth) => {
    const kind = rng.below(depth > 3 ? 4 : 6);
    if (kind === 0) return null;
    if (kind === 1) return rng.below(2) === 1;
    if (kind === 2) {
      const n = rng.pick(NUMBERS)();
      return Number.isFinite(n) ? n : 0;
    }
    if (kind === 3) return randomString(5);
    if (kind === 4) return Array.from({ length: rng.below(4) }, () => randomValue(depth + 1));
    const object = {};
    // Integer-like keys would be reordered by the engine's own object model; ids never are.
    for (let i = rng.below(5); i > 0; i--) object['k' + randomString(3)] = randomValue(depth + 1);
    return object;
  };
  const lines = [];
  for (let i = 0; i < 1500; i++) {
    const value = randomValue(0);
    let compact = null;
    let indented = null;
    try {
      compact = print(value, '', 0);
      indented = print(value, '  ', 0) + '\n';
    } catch {
      // e-notation: refused
    }
    lines.push(JSON.stringify({ input: JSON.stringify(value), compact, indented }));
  }
  write('canonical.jsonl', lines);
}

{
  const lines = [];
  for (let i = 0; i < 500; i++) {
    const s = randomString(40);
    lines.push(`${JSON.stringify(s)}\t${createHash('sha256').update(s, 'utf8').digest('hex')}`);
  }
  write('sha256.tsv', lines);
}
