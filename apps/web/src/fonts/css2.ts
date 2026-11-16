// Reads a Google Fonts `css2` response: the `@font-face` rules a browser gets, one per face and script chunk, each
// with its WOFF2 URL and `unicode-range`. Same rules as `slidock-fonts::google::parse_css2`: a response over 1 MiB,
// an unreadable rule, or any font file outside fonts.gstatic.com refuses the whole response.
import type { FaceStyle } from '@slidock/types';

/** The `css2` endpoint: the only stylesheet the app fetches. */
export const CSS2_ENDPOINT = 'https://fonts.googleapis.com/css2';
/** Where a `css2` response's font files may be. */
const GSTATIC = ['https://fonts.gstatic.com/s/', 'https://fonts.gstatic.com/l/font?kit='];
export const MAX_CSS2_BYTES = 1 << 20;
const MAX_FACES = 4096;
const MAX_RANGES = 4096;
const MAX_CODE_POINT = 0x10ffff;

export class Css2Error extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'Css2Error';
  }
}

/** Code point ranges, inclusive, sorted and merged. */
export class UnicodeRange {
  readonly ranges: readonly (readonly [number, number])[];

  constructor(ranges: Iterable<readonly [number, number]>) {
    const sorted = [...ranges].filter(([lo, hi]) => lo <= hi).sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    const merged: [number, number][] = [];
    for (const [lo, hi] of sorted) {
      const last = merged.at(-1);
      if (last && lo <= last[1] + 1) {
        last[1] = Math.max(last[1], hi);
      } else {
        merged.push([lo, hi]);
      }
    }
    this.ranges = merged;
  }

  /** `U+X`, `U+X-Y` and `U+X??` items, comma-separated; `null` when one does not parse. */
  static parse(value: string): UnicodeRange | null {
    const ranges: [number, number][] = [];
    for (const raw of value.split(',')) {
      const item = raw.trim();
      if (item === '') {
        continue;
      }
      if (ranges.length === MAX_RANGES || !/^[Uu]\+/.test(item)) {
        return null;
      }
      const hex = item.slice(2);
      let range: [number, number] | null;
      const dash = hex.indexOf('-');
      if (dash >= 0) {
        const lo = codePoint(hex.slice(0, dash));
        const hi = codePoint(hex.slice(dash + 1));
        range = lo === null || hi === null ? null : [lo, hi];
      } else if (hex.includes('?')) {
        const digits = hex.replace(/\?+$/, '');
        const wild = hex.length - digits.length;
        if (hex.length > 6 || digits.includes('?') || (digits !== '' && codePoint(digits) === null)) {
          return null;
        }
        const base = digits === '' ? 0 : Number.parseInt(digits, 16);
        const span = 16 ** wild;
        range = [base * span, Math.min(base * span + span - 1, MAX_CODE_POINT)];
      } else {
        const point = codePoint(hex);
        range = point === null ? null : [point, point];
      }
      if (range === null || range[0] > range[1] || range[0] > MAX_CODE_POINT) {
        return null;
      }
      ranges.push(range);
    }
    return new UnicodeRange(ranges);
  }

  /** Whether any of `points` (sorted ascending) is in the range. */
  intersects(points: readonly number[]): boolean {
    let at = 0;
    for (const [lo, hi] of this.ranges) {
      while (at < points.length && (points[at] ?? Infinity) < lo) {
        at += 1;
      }
      if (at < points.length && (points[at] ?? Infinity) <= hi) {
        return true;
      }
    }
    return false;
  }

  toString(): string {
    const hex = (point: number) => point.toString(16).toUpperCase().padStart(4, '0');
    return this.ranges.map(([lo, hi]) => (lo === hi ? `U+${hex(lo)}` : `U+${hex(lo)}-${hex(hi)}`)).join(', ');
  }
}

function codePoint(hex: string): number | null {
  if (!/^[0-9A-Fa-f]{1,6}$/.test(hex)) {
    return null;
  }
  const point = Number.parseInt(hex, 16);
  return point <= MAX_CODE_POINT ? point : null;
}

/** One rule of a `css2` response: a face, or one script chunk of it. */
export interface Css2Face {
  family: string;
  style: FaceStyle;
  /** `[lo, hi]`: one weight, or a variable file's range. */
  weight: [number, number];
  url: string;
  /** The `format()` hint: `woff2`, `truetype`, … */
  format: string | null;
  /** Browser responses split a face into per-script chunks; `null` for a whole face. */
  unicodeRange: UnicodeRange | null;
  /** The subset named by the comment before the rule (`latin`, `cyrillic-ext`). */
  subset: string | null;
}

/** Whether `url` is a font file of a `css2` response. */
export function isGstaticUrl(url: string): boolean {
  return GSTATIC.some((prefix) => url.startsWith(prefix));
}

/** The offset of the `}` that closes the block starting at `start` (quotes and parentheses respected), or -1. */
function blockEnd(text: string, start: number): number {
  let depth = 0;
  let quote: string | null = null;
  for (let at = start; at < text.length; at++) {
    const c = text[at];
    if (quote !== null) {
      if (c === quote) {
        quote = null;
      }
    } else if (c === '"' || c === "'") {
      quote = c;
    } else if (c === '(') {
      depth += 1;
    } else if (c === ')') {
      depth = Math.max(0, depth - 1);
    } else if (c === '}' && depth === 0) {
      return at;
    }
  }
  return -1;
}

/** Declarations split at `;` outside quotes and parentheses, as `[property, value]`. */
function declarations(body: string): [string, string][] {
  const out: [string, string][] = [];
  let depth = 0;
  let quote: string | null = null;
  let start = 0;
  const push = (part: string) => {
    const colon = part.indexOf(':');
    if (colon >= 0) {
      out.push([part.slice(0, colon).trim().toLowerCase(), part.slice(colon + 1).trim()]);
    }
  };
  for (let at = 0; at < body.length; at++) {
    const c = body[at];
    if (quote !== null) {
      if (c === quote) {
        quote = null;
      }
    } else if (c === '"' || c === "'") {
      quote = c;
    } else if (c === '(') {
      depth += 1;
    } else if (c === ')') {
      depth = Math.max(0, depth - 1);
    } else if (c === ';' && depth === 0) {
      push(body.slice(start, at));
      start = at + 1;
    }
  }
  push(body.slice(start));
  return out;
}

function unquote(value: string): string {
  const trimmed = value.trim();
  for (const q of ['"', "'"]) {
    if (trimmed.length >= 2 && trimmed.startsWith(q) && trimmed.endsWith(q)) {
      return trimmed.slice(1, -1);
    }
  }
  return trimmed;
}

function source(value: string): [string, string | null] | null {
  const trimmed = value.trim();
  if (!trimmed.startsWith('url(')) {
    return null;
  }
  const rest = trimmed.slice(4);
  const close = rest.indexOf(')');
  if (close < 0) {
    return null;
  }
  const url = unquote(rest.slice(0, close));
  const after = rest.slice(close + 1).trimStart();
  const format = /^format\(([^)]*)\)/.exec(after);
  return [url, format?.[1] === undefined ? null : unquote(format[1])];
}

function weight(value: string): [number, number] | null {
  const parse = (word: string): number | null => {
    if (word === 'normal') {
      return 400;
    }
    if (word === 'bold') {
      return 700;
    }
    if (!/^\d{1,4}$/.test(word)) {
      return null;
    }
    const number = Number(word);
    return number >= 1 && number <= 1000 ? number : null;
  };
  const words = value.trim().split(/\s+/);
  if (words.length > 2) {
    return null;
  }
  const first = parse(words[0] ?? '');
  const second = words[1] === undefined ? first : parse(words[1]);
  if (first === null || second === null) {
    return null;
  }
  return [Math.min(first, second), Math.max(first, second)];
}

function face(body: string, subset: string | null): Css2Face {
  let family: string | null = null;
  let style: FaceStyle = 'normal';
  let weights: [number, number] | null = null;
  let src: [string, string | null] | null = null;
  let range: UnicodeRange | null = null;
  for (const [property, value] of declarations(body)) {
    if (property === 'font-family') {
      family = unquote(value);
    } else if (property === 'font-style') {
      if (value === 'normal') {
        style = 'normal';
      } else if (value === 'italic' || value.startsWith('oblique')) {
        style = 'italic';
      } else {
        throw new Css2Error(`A rule cannot be read: font-style ${value}`);
      }
    } else if (property === 'font-weight') {
      weights = weight(value);
      if (weights === null) {
        throw new Css2Error(`A rule cannot be read: font-weight ${value}`);
      }
    } else if (property === 'src') {
      src = source(value);
      if (src === null) {
        throw new Css2Error(`A rule cannot be read: src ${value}`);
      }
    } else if (property === 'unicode-range') {
      range = UnicodeRange.parse(value);
      if (range === null) {
        throw new Css2Error(`A rule cannot be read: unicode-range ${value}`);
      }
    }
  }
  if (family === null || src === null) {
    throw new Css2Error('A rule cannot be read: no font-family or src');
  }
  const [url, format] = src;
  if (!isGstaticUrl(url)) {
    throw new Css2Error(`A rule's font file is not on fonts.gstatic.com: ${url}`);
  }
  return { family, style, weight: weights ?? [400, 400], url, format, unicodeRange: range, subset };
}

/** The faces of a `css2` response, in order, read in one pass. */
export function parseCss2(css: string): Css2Face[] {
  if (css.length > MAX_CSS2_BYTES) {
    throw new Css2Error(`The response is larger than ${MAX_CSS2_BYTES} bytes`);
  }
  const faces: Css2Face[] = [];
  let subset: string | null = null;
  let at: number;
  // The next comment and rule at or after `at`, each searched again only once `at` passes it.
  let comment = css.indexOf('/*');
  let rule = css.indexOf('@font-face');
  while (comment >= 0 || rule >= 0) {
    if (comment >= 0 && (rule < 0 || comment < rule)) {
      const end = css.indexOf('*/', comment + 2);
      if (end < 0) {
        throw new Css2Error('A rule cannot be read: an unclosed comment');
      }
      const text = css.slice(comment + 2, end).trim();
      subset = text === '' ? null : text;
      at = end + 2;
    } else {
      let open = rule + '@font-face'.length;
      while (open < css.length && /\s/.test(css[open] ?? '')) {
        open += 1;
      }
      if (css[open] !== '{') {
        throw new Css2Error('A rule cannot be read: @font-face without a block');
      }
      const close = blockEnd(css, open + 1);
      if (close < 0) {
        throw new Css2Error('A rule cannot be read: an unclosed @font-face block');
      }
      if (faces.length === MAX_FACES) {
        throw new Css2Error(`The response has more than ${MAX_FACES} rules`);
      }
      faces.push(face(css.slice(open + 1, close), subset));
      subset = null;
      at = close + 1;
    }
    if (comment >= 0 && comment < at) {
      comment = css.indexOf('/*', at);
    }
    if (rule >= 0 && rule < at) {
      rule = css.indexOf('@font-face', at);
    }
  }
  if (faces.length === 0) {
    throw new Css2Error('The response has no @font-face rule');
  }
  return faces;
}
