// Properties of the sanitizer's URL and CSS checks over generated input, against what the generator knows it put in:
// a resource `isSafeResource` accepts stays inside the deck's folder whatever the folder is called, however deep it
// sits and whether it is served or opened from a file; a CSS value `isSafeCss` accepts loads nothing but such
// resources, whatever escapes, comments, quoting and functions wrap them. The generator is seeded, so a failure
// names the case that reproduces it.
import { describe, expect, it } from 'vitest';

import { isSafeCss, isSafeResource } from '../src/sanitize';

/** mulberry32: a small seeded generator. */
function random(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

type Rng = () => number;
const pick = <T>(rng: Rng, items: readonly T[]): T => items[Math.floor(rng() * items.length)] as T;
const repeat = (rng: Rng, most: number, part: () => string): string =>
  Array.from({ length: Math.floor(rng() * (most + 1)) }, part).join('');

/** Pieces a hostile URL is made of: schemes, hosts, traversals, separators browsers fold, encodings. */
const SCHEMES = ['', '', '', 'javascript:', 'JaVaScRiPt:', 'http:', 'https:', 'file:', 'data:', 'blob:', 'vbscript:'];
const AUTHORITIES = ['', '', '//evil.example/', '\\\\evil.example\\', '/\\evil.example/', '//', '///'];
const SEGMENTS = ['media', 'x.png', '..', '.', '%2e%2e', '%2E.', '.%2e', '', 'a b', 'slides', '~', '%00', 'é'];
const SEPARATORS = ['/', '/', '\\', '/\t', '\n/', '%2f', '%5c'];
const DATA = ['image/png;base64,iVBOR', 'video/mp4,x', 'audio/mpeg,x', 'text/html,<script>', 'image/svg+xml,<svg>'];
const PREFIXES = ['', '', ' ', '\t', '\u0000', '\n'];

function hostileUrl(rng: Rng): string {
  const scheme = pick(rng, SCHEMES);
  if (scheme === 'data:') {
    return pick(rng, PREFIXES) + pick(rng, ['data:', 'DATA:', 'data :']) + pick(rng, DATA);
  }
  const path = repeat(rng, 5, () => pick(rng, SEGMENTS) + pick(rng, SEPARATORS));
  const tail = pick(rng, ['', 'x.png', '#frag', '?q=1', '..']);
  return pick(rng, PREFIXES) + scheme + pick(rng, AUTHORITIES) + path + tail;
}

/** Deck folders of many names and depths, served and opened from a file. */
const FOLDERS = [
  'https://decks.example/',
  'https://decks.example/talks/2026/my%20deck/',
  'http://127.0.0.1:5761/d/',
  'file:///Users/someone/Desktop/My%20Deck/',
  'file:///C:/decks/talk/',
  'file:///srv/a/b/c/d/e/',
];

/** Where a resource of a slide document (one level below the deck's folder) lands, or `null` when it is no URL. */
function landing(value: string, folder: string): string | null {
  try {
    return new URL(value, `${folder}slides/`).href;
  } catch {
    return null;
  }
}

const isInlineMedia = (href: string): boolean => /^data:(image|video|audio)\//i.test(href);

describe('sanitizer properties', () => {
  it('a resource it accepts stays inside any deck folder, or is inlined media', () => {
    const rng = random(0x5eed);
    let accepted = 0;
    for (let n = 0; n < 5000; n++) {
      const value = hostileUrl(rng);
      if (!isSafeResource(value)) {
        continue;
      }
      accepted++;
      for (const folder of FOLDERS) {
        const href = landing(value, folder);
        expect(href, `case ${n}: ${JSON.stringify(value)} from ${folder}`).not.toBeNull();
        if (href !== null) {
          expect(href.startsWith(folder) || isInlineMedia(href), `case ${n}: ${JSON.stringify(value)} → ${href}`).toBe(
            true,
          );
        }
      }
    }
    // The generator reaches both outcomes.
    expect(accepted).toBeGreaterThan(200);
  });

  /** A CSS value around `payload` in one of the ways CSS can spell a URL, and whether that spelling loads it. */
  function wrap(rng: Rng, payload: string): { css: string; loads: boolean } {
    const quote = pick(rng, ['"', "'", '']);
    const property = pick(rng, ['fill', 'background', 'background-image', 'mask', 'clip-path', 'list-style']);
    const comment = pick(rng, ['', '/**/', '/* url(x) */']);
    const escaped = payload.replace(/[()'" \\\n]/g, (c) => `\\${c.charCodeAt(0).toString(16)} `);
    const inQuotes = quote === '' ? escaped : payload.replace(/[\\"'\n]/g, (c) => `\\${c}`);
    const forms = [
      { css: `${property}:${comment}url(${quote}${inQuotes}${quote})`, loads: true },
      { css: `${property}: URL( ${quote}${inQuotes}${quote} )`, loads: true },
      { css: `${property}:u\\72l(${quote}${inQuotes}${quote})`, loads: true },
      { css: `${property}:image-set(${quote || '"'}${payload.replace(/["\\]/g, '')}${quote || '"'} 1x)`, loads: true },
      { css: `${property}:-webkit-image-set(url(${quote}${inQuotes}${quote}) 1x)`, loads: true },
      { css: `${property}:cross-fade(url(${quote}${inQuotes}${quote}),url(#a),50%)`, loads: true },
      { css: `@import ${quote || '"'}${payload.replace(/["\\]/g, '')}${quote || '"'}`, loads: true },
      { css: `${property}:linear-gradient(red,blue);${comment}fill:url(${quote}${inQuotes}${quote})`, loads: true },
    ];
    return pick(rng, forms);
  }

  it('a CSS value it accepts loads only resources it accepts', () => {
    const rng = random(0xc55);
    let accepted = 0;
    let refusedUnsafe = 0;
    for (let n = 0; n < 5000; n++) {
      const payload = pick(rng, [hostileUrl(rng), '#g', 'media/a.png', '../media/b.png', 'data:image/png;base64,AA']);
      const { css, loads } = wrap(rng, payload);
      const safe = isSafeCss(css);
      if (safe) {
        accepted++;
        // Whatever the spelling, what it loads is a resource the deck may load.
        if (loads) {
          expect(isSafeResource(payload), `case ${n}: ${JSON.stringify(css)}`).toBe(true);
        }
      } else if (!isSafeResource(payload)) {
        refusedUnsafe++;
      }
    }
    expect(accepted).toBeGreaterThan(200);
    expect(refusedUnsafe).toBeGreaterThan(200);
  });

  it('never accepts a script scheme, an import or an expression, however spelled', () => {
    const rng = random(0xbad);
    const dangers = ['javascript:alert(1)', 'expression(alert(1))', '@import "x"', 'behavior: url(x.htc)'];
    for (let n = 0; n < 2000; n++) {
      const danger = pick(rng, dangers);
      const spelled = danger.replace(/[a-z]/g, (c) => (rng() < 0.3 ? c.toUpperCase() : c));
      const css = `${pick(rng, ['fill:', 'x:', ''])}${pick(rng, ['', ' ', '/**/'])}${spelled}`;
      expect(isSafeCss(css), `case ${n}: ${JSON.stringify(css)}`).toBe(false);
    }
  });
});
