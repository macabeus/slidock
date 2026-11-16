// The decks of the font packaging matrix, compiled with shipped fonts, and
// what the matrix reads from them: each `@font-face` rule of `<style id="sd-fonts">` with the names of its file.
//
// The committed decks compile hermetically (`--offline --no-substitutes`, an empty cache):
//
// - the font probe ODP ships its embedded faces; the font probe PPTX ships the same faces as dropped files (taken from
//   the ODP's `Fonts/`);
// - the synthetic deck (fixture/fontsPptx.ts `MATRIX_DECK`) ships two embedded EOT faces and four dropped files; its
//   copy with a build has one paragraph appear on a click, written into the compiled deck's timeline (the frontends
//   lower no builds yet: PPTX animations report PX2401 and show fully built);
// - the stack deck (`STACK_DECK`) requests families that bind to a differently named shipped family.
//
// `SLIDOCK_E2E_FONTS_CORPUS=1` adds the corpus deck `pptx-talk` (a PPTX that embeds fonts; `corpusSource` finds its
// file) compiled as a user would (embedded fonts, Google Fonts, bundled substitutes), with its downloads cached under
// `<cargo target>/e2e/font-matrix/corpus-cache/`.
//
// Each deck is compiled by the first worker that needs it, into a folder named by a hash of its inputs, so parallel
// workers and later runs reuse it.
import type { Deck } from '@slidock/types';
import { execFileSync, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { cpSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { type Server, createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { extname, join, normalize, relative, sep } from 'node:path';
import { pathToFileURL } from 'node:url';

import { FIREFOX_IN_DOCKER, WEBKIT_IN_DOCKER } from '../../../../packages/player/e2e/support/browserServer';
import { FONT_FIXTURES, type FontDeckSpec, MATRIX_DECK, STACK_DECK, fontsPptx } from '../fixture/fontsPptx';
import { CLI, CORPUS_MANIFEST, OUT, REPO, corpusSource } from './decks';
import { type FaceNames, woff2Names } from './woff2';

/** Everything the font matrix writes. */
export const MATRIX_OUT = join(OUT, 'font-matrix');

export const WITH_CORPUS = process.env.SLIDOCK_E2E_FONTS_CORPUS === '1';

export interface FontDeckCase {
  /** A file-system-safe name, unique in the run. */
  key: string;
  /** What the test titles call it. */
  name: string;
  /** Writes the input and returns its path, the CLI's font arguments, and whether the compile downloads nothing. */
  prepare(work: string): { input: string; args: string[]; hermetic: boolean };
  /** The text of a box that appears on a click, written into the compiled deck's timeline. */
  clickBuild?: string;
}

const PROBES = join(REPO, 'probes/out');

/** Copies the font probe ODP's embedded files (OFL, licences in probes/out/licenses) into `dir`. */
function probeFonts(dir: string): void {
  mkdirSync(dir, { recursive: true });
  execFileSync('unzip', ['-q', '-o', '-j', join(PROBES, 'fonts-probe.odp'), 'Fonts/*', '-d', dir], { stdio: 'pipe' });
}

const HERMETIC = ['--offline', '--no-substitutes'];

/** A deck of fixture/fontsPptx.ts with its dropped files. */
function fixtureDeck(spec: FontDeckSpec, work: string): { input: string; args: string[]; hermetic: boolean } {
  const input = join(work, 'font-matrix.pptx');
  writeFileSync(input, fontsPptx(REPO, spec));
  const fonts = join(work, 'fonts');
  mkdirSync(fonts, { recursive: true });
  for (const { file } of spec.dropped) {
    cpSync(join(REPO, FONT_FIXTURES, file), join(fonts, file));
  }
  return { input, args: [...HERMETIC, '--fonts', fonts], hermetic: true };
}

const SYNTHETIC = 'font matrix deck (embedded EOT, TrueType, variable, CFF, collection)';

export function fontDeckCases(): FontDeckCase[] {
  const cases: FontDeckCase[] = [
    {
      key: 'probe-odp',
      name: 'fonts-probe.odp (embedded)',
      prepare: () => ({ input: join(PROBES, 'fonts-probe.odp'), args: HERMETIC, hermetic: true }),
    },
    {
      key: 'probe-pptx',
      name: 'fonts-probe.pptx (dropped files)',
      prepare: (work) => {
        const fonts = join(work, 'fonts');
        probeFonts(fonts);
        return { input: join(PROBES, 'fonts-probe.pptx'), args: [...HERMETIC, '--fonts', fonts], hermetic: true };
      },
    },
    { key: 'synthetic', name: SYNTHETIC, prepare: (work) => fixtureDeck(MATRIX_DECK, work) },
    {
      key: 'synthetic-build',
      name: `${SYNTHETIC} with a click build`,
      prepare: (work) => fixtureDeck(MATRIX_DECK, work),
      clickBuild: 'Orcas: bocas!',
    },
    {
      key: 'stack',
      name: 'font stack deck (requested family bound to another shipped family)',
      prepare: (work) => fixtureDeck(STACK_DECK, work),
    },
  ];
  if (WITH_CORPUS) {
    cases.push({
      key: 'corpus',
      name: 'pptx-talk .pptx (corpus: embedded, Google, substitutes)',
      prepare: () => {
        const input = corpusSource('pptx-talk', 'pptx');
        if (input === undefined) {
          throw new Error(`SLIDOCK_E2E_FONTS_CORPUS=1 needs the pptx-talk deck of ${CORPUS_MANIFEST}`);
        }
        return { input, args: [], hermetic: false };
      },
    });
  }
  return cases;
}

/** One `@font-face` rule of `<style id="sd-fonts">`, with the names of its file. */
export interface ShippedRule extends FaceNames {
  /** The `font-family` the slides name. */
  cssFamily: string;
  weight: string;
  style: string;
}

export interface BuiltDeck {
  /** The unpacked zip. */
  site: string;
  deck: Deck;
  rules: ShippedRule[];
}

const DECK_JSON = /<script type="application\/json" id="sd-deck">([\s\S]*?)<\/script>/;
const FONTS_STYLE = /<style id="sd-fonts">([\s\S]*?)<\/style>/;
const RULE = /@font-face\{([^}]*)\}/g;

/** A CSS string's value (`"…"` with backslash escapes). */
export function cssString(quoted: string): string {
  return quoted
    .slice(1, -1)
    .replace(/\\([0-9a-fA-F]{1,6}) ?|\\([\s\S])/g, (_, hex: string | undefined, char: string | undefined) =>
      hex === undefined ? (char ?? '') : String.fromCodePoint(parseInt(hex, 16)),
    );
}

function descriptor(rule: string, name: string): string {
  return new RegExp(`(?:^|;)${name}:((?:"(?:[^"\\\\]|\\\\.)*"|[^;])*)`).exec(rule)?.[1] ?? '';
}

function readSite(site: string): BuiltDeck {
  const html = readFileSync(join(site, 'index.html'), 'utf8');
  const json = DECK_JSON.exec(html)?.[1];
  if (json === undefined) {
    throw new Error(`${site}: index.html has no #sd-deck`);
  }
  const css = FONTS_STYLE.exec(html)?.[1] ?? '';
  const rules = [...css.matchAll(RULE)].map(([, body = '']) => {
    const data = /(?:^|;)src:url\(data:font\/woff2;base64,([A-Za-z0-9+/=]+)\)/.exec(body)?.[1];
    if (data === undefined) {
      throw new Error(`a rule ships no data: WOFF2: ${body.slice(0, 200)}`);
    }
    return {
      cssFamily: cssString(descriptor(body, 'font-family')),
      weight: descriptor(body, 'font-weight'),
      style: descriptor(body, 'font-style'),
      ...woff2Names(Buffer.from(data, 'base64')),
    };
  });
  return {
    site,
    deck: JSON.parse(json) as Deck,
    rules,
  };
}

function sha256(...parts: (string | Buffer)[]): string {
  const hash = createHash('sha256');
  for (const part of parts) {
    hash.update(part);
    hash.update('\0');
  }
  return hash.digest('hex');
}

/**
 * Compiles `deck` (or reuses an earlier compile of the same inputs) and reads what it shipped; a deck with a
 * `clickBuild` is then copied with that build.
 */
export function buildFontDeck(deck: FontDeckCase): BuiltDeck {
  const work = join(MATRIX_OUT, 'inputs', `${deck.key}.${process.pid}`);
  rmSync(work, { recursive: true, force: true });
  mkdirSync(work, { recursive: true });
  try {
    const built = compileOnce(deck, work);
    return deck.clickBuild === undefined ? built : withClickBuild(built, deck.clickBuild);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

/**
 * A copy of `built` with `html` as its `index.html`, beside the compiled decks and named by its folder. Workers
 * that derive the same copy write the same bytes, each through a file of its own renamed into place, so a page
 * another worker has open never reads a half-written file.
 */
function copySite(built: BuiltDeck, folder: string, suffix: string, html: string): BuiltDeck {
  const site = join(MATRIX_OUT, folder, `${relative(join(MATRIX_OUT, 'decks'), built.site).split(sep)[0]}-${suffix}`);
  mkdirSync(site, { recursive: true });
  const part = join(site, `index.html.${process.pid}`);
  writeFileSync(part, html);
  renameSync(part, join(site, 'index.html'));
  return readSite(site);
}

/**
 * A copy of `built` whose box with the text `text` appears on a click: an entrance step on that element is appended to
 * its slide's timeline in `#sd-deck`, which the player reads.
 */
function withClickBuild(built: BuiltDeck, text: string): BuiltDeck {
  const html = readFileSync(join(built.site, 'index.html'), 'utf8');
  // In a slide template: the page's text view, which comes first, holds the text too.
  const at = html.indexOf(`>${text}<`, html.indexOf('<template data-sd-slide='));
  /** The value of the last `name="…"` attribute before the text. */
  const before = (name: string) => {
    const start = html.lastIndexOf(`${name}="`, at);
    return at < 0 || start < 0 ? undefined : /^[^"]*/.exec(html.slice(start + name.length + 2))?.[0];
  };
  const element = before('data-el');
  const slide = before('data-sd-slide');
  if (element === undefined || slide === undefined) {
    throw new Error(`${built.site}: no element draws "${text}"`);
  }
  const deck = structuredClone(built.deck);
  const target = deck.slides.find((candidate) => candidate.id === slide);
  if (target === undefined) {
    throw new Error(`${built.site}: no slide ${slide} in #sd-deck`);
  }
  target.timeline.push({
    target: { element, paragraphs: null },
    trigger: 'click',
    kind: 'entrance',
    delivery: 'all',
    delayMs: 0,
    durationMs: 0,
    native: 'pptx:appear',
    fallback: 'appear',
  });
  const json = JSON.stringify(deck).replace(/</g, '\\u003c');
  return copySite(
    built,
    'derived',
    'build',
    html.replace(DECK_JSON, (whole, old: string) => whole.replace(old, () => json)),
  );
}

function compileOnce(deck: FontDeckCase, work: string): BuiltDeck {
  const { input, args, hermetic } = deck.prepare(work);
  const cli = statSync(CLI);
  const fontArgs = args.map((arg) => (arg.startsWith(work) ? relative(work, arg) : arg));
  const fontFiles = args
    .filter((arg) => arg.startsWith(work))
    .flatMap((dir) => execFileSync('find', [dir, '-type', 'f'], { encoding: 'utf8' }).split('\n').filter(Boolean))
    .sort()
    .flatMap((file) => [relative(work, file), readFileSync(file)]);
  const inputs = [readFileSync(input), JSON.stringify(fontArgs), `${cli.size}:${cli.mtimeMs}`, ...fontFiles];
  const stamp = sha256(...inputs).slice(0, 16);
  const dir = join(MATRIX_OUT, 'decks', `${deck.key}-${stamp}`);
  const site = join(dir, 'site');
  if (existsSync(join(dir, 'done'))) {
    return readSite(site);
  }
  const temp = `${dir}.${process.pid}`;
  rmSync(temp, { recursive: true, force: true });
  mkdirSync(temp, { recursive: true });
  const zip = join(temp, 'deck.zip');
  // The CLI's cache: empty for a hermetic compile, else the deck's own, so nothing lands in this computer's cache.
  const cache = join(MATRIX_OUT, hermetic ? 'empty-cache' : `${deck.key}-cache`);
  mkdirSync(cache, { recursive: true });
  const env = { ...process.env, SLIDOCK_CACHE_DIR: cache };
  const run = spawnSync(CLI, ['compile', input, '-o', zip, ...args], {
    encoding: 'utf8',
    env,
    maxBuffer: 64 * 1024 * 1024,
  });
  // Exit 1 reports diagnostics in a deck that still printed; 2 is a refused input.
  if (run.status !== 0 && run.status !== 1) {
    throw new Error(`slidock compile ${deck.name} exited ${run.status}: ${run.stderr.slice(-2000)}`);
  }
  writeFileSync(join(temp, 'stderr.txt'), run.stderr);
  execFileSync('unzip', ['-q', zip, '-d', join(temp, 'site')], { stdio: 'pipe' });
  writeFileSync(join(temp, 'done'), '');
  try {
    renameSync(temp, dir);
  } catch {
    // Another worker compiled the same inputs first; its output is the same.
    rmSync(temp, { recursive: true, force: true });
  }
  return readSite(site);
}

/** Ways to break a deck's font injection, for the matrix to show its checks catch them. */
export type Tamper =
  /** The CSP blocks every font: `font-src 'none'`. */
  | 'no-font-src'
  /** One family's rule covers no character (`unicode-range:U+0`), so the browser falls back. */
  | 'uncovered'
  /** One family's rule is renamed, so the slides name a family that ships no face. */
  | 'renamed'
  /** A later rule for the same family and face, from a font of this computer, wins over the shipped file. */
  | 'shadowed'
  /** One code point is taken out of the unicode-range of the family's first rule, so that glyph falls back. */
  | { gap: number };

/** A copy of `built`'s `index.html` with `family`'s injection broken as `tamper` says, read as `buildFontDeck` does. */
export function tamperedDeck(built: BuiltDeck, family: string, tamper: Tamper): BuiltDeck {
  let html = readFileSync(join(built.site, 'index.html'), 'utf8');
  const quoted = `font-family:"${family}"`;
  const rule = new RegExp(`@font-face\\{${quoted}[^}]*\\}`);
  const replace = (from: string | RegExp, to: (match: string) => string) => {
    const next = html.replace(from, to);
    if (next === html) {
      throw new Error(`tampering ${tamper}: nothing matched ${String(from)}`);
    }
    html = next;
  };
  if (tamper === 'no-font-src') {
    replace('font-src data: blob:', () => "font-src 'none'");
  } else if (tamper === 'uncovered') {
    replace(rule, (match) => match.replace(/unicode-range:[^;}]*/, 'unicode-range:U+0'));
  } else if (tamper === 'renamed') {
    replace(rule, (match) => match.replace(quoted, `font-family:"${family} renamed"`));
  } else if (tamper === 'shadowed') {
    replace(
      /(<style id="sd-fonts">[\s\S]*?<\/style>)/,
      (fonts) => `${fonts}<style>@font-face{${quoted};src:local("Helvetica"),local("DejaVu Sans")}</style>`,
    );
  } else {
    replace(rule, (match) =>
      match.replace(
        /unicode-range:([^;}]*)/,
        (_, range: string) => `unicode-range:${withoutCodePoint(range, tamper.gap)}`,
      ),
    );
  }
  const name = typeof tamper === 'string' ? tamper : `gap-${tamper.gap.toString(16)}`;
  return copySite(built, 'tampered', name, html);
}

/** A `unicode-range` value (`U+20,U+41-5A`) without `point`; it must contain it. */
export function withoutCodePoint(range: string, point: number): string {
  let found = false;
  const hex = (value: number) => `U+${value.toString(16).toUpperCase()}`;
  const parts = range.split(',').flatMap((part) => {
    const [from = NaN, to = from] = part
      .trim()
      .replace(/^U\+/i, '')
      .split('-')
      .map((value) => parseInt(value, 16));
    if (point < from || point > to) {
      return [part.trim()];
    }
    found = true;
    return [
      ...(from < point
        ? [from === point - 1 ? hex(from) : `${hex(from)}-${(point - 1).toString(16).toUpperCase()}`]
        : []),
      ...(point < to ? [point + 1 === to ? hex(to) : `${hex(point + 1)}-${to.toString(16).toUpperCase()}`] : []),
    ];
  });
  if (!found) {
    throw new Error(`unicode-range ${range} does not contain U+${point.toString(16)}`);
  }
  return parts.join(',');
}

const TYPES: Record<string, string> = {
  '.html': 'text/html; charset=utf-8',
  '.txt': 'text/plain; charset=utf-8',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.gif': 'image/gif',
  '.svg': 'image/svg+xml',
  '.webp': 'image/webp',
  '.mp4': 'video/mp4',
  '.webm': 'video/webm',
  '.mov': 'video/quicktime',
};

/**
 * The `file://` URL of `built`'s `index.html`. A browser in Docker reads it from the repository or the suite's output,
 * mounted read-only at the same paths, so the matrix's output must be inside one of them.
 */
export function fileUrl(built: BuiltDeck, browserName: string): string {
  const inDocker = (browserName === 'firefox' && FIREFOX_IN_DOCKER) || (browserName === 'webkit' && WEBKIT_IN_DOCKER);
  const mounted = [REPO, OUT].some((root) => !relative(root, built.site).startsWith('..'));
  if (inDocker && !mounted) {
    throw new Error(`a browser in Docker opens file:// pages from ${REPO} and ${OUT} only, not ${built.site}`);
  }
  return pathToFileURL(join(built.site, 'index.html')).href;
}

/** A static server for the compiled decks on this machine's loopback; `url(site)` is a site's `index.html`. */
export async function serveDecks(): Promise<{ url: (site: string) => string; close: () => Promise<void> }> {
  const root = normalize(MATRIX_OUT);
  const server: Server = createServer((request, response) => {
    const path = normalize(join(root, decodeURIComponent(new URL(request.url ?? '/', 'http://x').pathname)));
    if (!path.startsWith(root + sep) || !existsSync(path) || !statSync(path).isFile()) {
      response.writeHead(404).end();
      return;
    }
    response.writeHead(200, { 'content-type': TYPES[extname(path).toLowerCase()] ?? 'application/octet-stream' });
    response.end(readFileSync(path));
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address() as AddressInfo;
  return {
    url: (site) =>
      `http://127.0.0.1:${port}/${relative(root, join(site, 'index.html')).split(sep).map(encodeURIComponent).join('/')}`,
    close: () => new Promise((resolve) => server.close(() => resolve())),
  };
}
