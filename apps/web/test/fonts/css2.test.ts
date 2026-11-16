import { describe, expect, it } from 'vitest';

import { Css2Error, MAX_CSS2_BYTES, UnicodeRange, isGstaticUrl, parseCss2 } from '../../src/fonts/css2';
import { points, recorded } from './recorded';

describe('parseCss2 on the recorded responses', () => {
  it('reads every rule of a browser response: per-script WOFF2 chunks with their unicode-range', () => {
    const faces = parseCss2(recorded('roboto-browser'));
    expect(faces).toHaveLength(27);
    expect(faces.every((face) => face.family === 'Roboto' && face.format === 'woff2')).toBe(true);
    expect(faces.every((face) => face.url.startsWith('https://fonts.gstatic.com/s/roboto/'))).toBe(true);
    expect(faces.every((face) => face.unicodeRange !== null)).toBe(true);
    expect(faces.filter((face) => face.style === 'italic')).toHaveLength(9);
    expect(faces.filter((face) => face.weight[0] === 700)).toHaveLength(9);
    expect(faces[0]?.subset).toBe('cyrillic-ext');
    const latin = faces.filter((face) => face.subset === 'latin');
    expect(latin).toHaveLength(3);
    expect(latin.every((face) => face.unicodeRange?.intersects(points('Hello')))).toBe(true);
  });

  it('reads a variable family whose weights share one file', () => {
    const faces = parseCss2(recorded('oswald-browser'));
    expect(faces).toHaveLength(10);
    const latin = faces.filter((face) => face.subset === 'latin');
    expect(latin.map((face) => face.weight)).toEqual([
      [400, 400],
      [700, 700],
    ]);
    expect(new Set(latin.map((face) => face.url)).size).toBe(1);
  });

  it('reads the static TTF rules a non-browser gets, with no unicode-range', () => {
    const faces = parseCss2(recorded('roboto-cli'));
    expect(faces.map((face) => [face.weight[0], face.style, face.format, face.unicodeRange])).toEqual([
      [400, 'italic', 'truetype', null],
      [400, 'normal', 'truetype', null],
      [700, 'normal', 'truetype', null],
    ]);
    expect(parseCss2(recorded('two-families-cli')).map((face) => face.family)).toEqual([
      'Barlow Condensed',
      'Barlow Condensed',
      'Roboto Mono',
    ]);
  });
});

describe('parseCss2 refusals', () => {
  const rule = (src: string, extra = '') =>
    `@font-face { font-family: 'X'; font-style: normal; font-weight: 400; src: ${src}; ${extra} }`;
  const gstatic = "url(https://fonts.gstatic.com/s/x/v1/a.woff2) format('woff2')";

  it('refuses a font file anywhere but fonts.gstatic.com', () => {
    expect(() => parseCss2(rule("url(https://evil.example/x.woff2) format('woff2')"))).toThrow(Css2Error);
    expect(() => parseCss2(rule('url(http://fonts.gstatic.com/s/x.woff2)'))).toThrow(Css2Error);
    expect(() => parseCss2(rule('url(data:font/woff2;base64,AAAA)'))).toThrow(Css2Error);
    expect(parseCss2(rule(gstatic))[0]?.url).toBe('https://fonts.gstatic.com/s/x/v1/a.woff2');
    expect(isGstaticUrl('https://fonts.gstatic.com/l/font?kit=abc')).toBe(true);
    expect(isGstaticUrl('https://fonts.gstatic.com.evil.example/s/x')).toBe(false);
  });

  it('refuses a response over 1 MiB, an unclosed rule or comment, and a response with no rule', () => {
    expect(() => parseCss2(`${rule(gstatic)}/*${' '.repeat(MAX_CSS2_BYTES)}*/`)).toThrow(/larger than/);
    expect(() => parseCss2(`@font-face { font-family: 'X'; src: ${gstatic};`)).toThrow(/unclosed/);
    expect(() => parseCss2(`/* latin ${rule(gstatic)}`)).toThrow(/unclosed comment/);
    expect(() => parseCss2('<html>400</html>')).toThrow(/no @font-face/);
    expect(() => parseCss2(rule(gstatic, 'unicode-range: U+zz;'))).toThrow(/unicode-range/);
    expect(() => parseCss2(rule(gstatic, 'font-weight: heavy;'))).toThrow(/font-weight/);
  });

  it('reads a response at the size limit in one pass', () => {
    const comments = '/**/'.repeat(Math.floor((MAX_CSS2_BYTES - 200) / 4));
    const started = performance.now();
    expect(parseCss2(`${comments}${rule(gstatic)}`)).toHaveLength(1);
    expect(performance.now() - started).toBeLessThan(2_000);
  });
});

describe('UnicodeRange', () => {
  it('parses single points, ranges and wildcards, merged and sorted', () => {
    expect(UnicodeRange.parse('U+0041, U+0030-0039, u+4??')?.ranges).toEqual([
      [0x30, 0x39],
      [0x41, 0x41],
      [0x400, 0x4ff],
    ]);
    expect(UnicodeRange.parse('U+0000-00FF, U+0100-017F')?.toString()).toBe('U+0000-017F');
    expect(UnicodeRange.parse('U+0041-0030')).toBeNull();
    expect(UnicodeRange.parse('U+110000')).toBeNull();
    expect(UnicodeRange.parse('0041')).toBeNull();
  });

  it('says whether a sorted list of code points falls in it', () => {
    const range = UnicodeRange.parse('U+0400-045F, U+2116');
    expect(range?.intersects(points('Hello'))).toBe(false);
    expect(range?.intersects(points('Привет'))).toBe(true);
    expect(range?.intersects([0x2116])).toBe(true);
    expect(range?.intersects([])).toBe(false);
  });
});
