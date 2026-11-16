// @vitest-environment node
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const css = readFileSync(new URL('../src/tokens.css', import.meta.url), 'utf8');

/** The custom properties declared in the first block that opens with `selector {`. */
function block(selector: string): Map<string, string> {
  const start = css.indexOf(`${selector} {`);
  expect(start, `no block for ${selector}`).toBeGreaterThanOrEqual(0);
  const end = css.indexOf('}', start);
  const vars = new Map<string, string>();
  for (const match of css.slice(start, end).matchAll(/(--[\w-]+):\s*([^;]+);/g)) {
    vars.set(match[1] ?? '', (match[2] ?? '').trim());
  }
  return vars;
}

const dark = block(':root');
const light = block(":root[data-theme='light']");
const systemLight = block(':root:not([data-theme])');

function luminance(hex: string): number {
  const match = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!match?.[1]) {
    throw new Error(`not a #rrggbb color: ${hex}`);
  }
  const value = parseInt(match[1], 16);
  const channel = (shift: number) => {
    const c = ((value >> shift) & 0xff) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0);
}

function rgb(hex: string): [number, number, number] {
  const value = parseInt(hex.slice(1), 16);
  return [(value >> 16) & 0xff, (value >> 8) & 0xff, value & 0xff];
}

/** `tint` at `alpha` over `background`, as a `#rrggbb` color (a pill's or a banner's wash). */
function wash(tint: string, alpha: number, background: string): string {
  const top = rgb(tint);
  const below = rgb(background);
  return `#${top
    .map((channel, index) => Math.round(channel * alpha + (below[index] ?? 0) * (1 - alpha)))
    .map((channel) => channel.toString(16).padStart(2, '0'))
    .join('')}`;
}

/** The colors of a `linear-gradient(…)` value. */
function stops(gradient: string): string[] {
  return [...gradient.matchAll(/#[0-9a-f]{6}/gi)].map((match) => match[0]);
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

// Text a reader needs, on the page and on panels. `--text-dim` is for glyph separators only and is not listed.
const TEXT = [
  '--text',
  '--text-secondary',
  '--text-muted',
  '--brand-300',
  '--brand-400',
  '--accent',
  '--success',
  '--warning',
  '--danger',
  '--font-embedded',
  '--font-local',
  '--font-google',
  '--font-dropped',
  '--font-missing',
  '--font-substituted',
];

// The colors pills, badges and banners tint themselves with.
const TINTS = [
  '--text-muted',
  '--success',
  '--warning',
  '--danger',
  '--font-embedded',
  '--font-local',
  '--font-google',
  '--font-dropped',
  '--font-missing',
  '--font-substituted',
];

describe('tokens', () => {
  it('light defines exactly the dark tokens', () => {
    expect([...light.keys()].sort()).toEqual([...dark.keys()].sort());
  });

  it('the system-light block repeats the light theme', () => {
    expect(systemLight).toEqual(light);
  });

  for (const [name, theme] of [
    ['dark', dark],
    ['light', light],
  ] as const) {
    for (const background of ['--bg-root', '--bg-surface']) {
      it(`${name}: every reader-facing text color is at least 4.5:1 on ${background}`, () => {
        const bg = theme.get(background) ?? '';
        const failures = TEXT.filter((token) => contrast(theme.get(token) ?? '', bg) < 4.5).map(
          (token) => `${token} ${theme.get(token)} = ${contrast(theme.get(token) ?? '', bg).toFixed(2)}:1`,
        );
        expect(failures).toEqual([]);
      });
    }
  }

  for (const [name, theme] of [
    ['dark', dark],
    ['light', light],
  ] as const) {
    it(`${name}: every reader-facing text color is at least 4.5:1 on a neutral chip (--bg-elevated at 60%)`, () => {
      const chip = wash(theme.get('--bg-elevated') ?? '', 0.6, theme.get('--bg-surface') ?? '');
      const failures = TEXT.filter((token) => contrast(theme.get(token) ?? '', chip) < 4.5);
      expect(failures).toEqual([]);
    });

    it(`${name}: a tinted pill (its text on 13% of itself, components/Pill.tsx) is at least 4.5:1`, () => {
      const failures = TINTS.flatMap((token) =>
        ['--bg-root', '--bg-surface'].flatMap((background) => {
          const tint = theme.get(token) ?? '';
          const ratio = contrast(tint, wash(tint, 0.13, theme.get(background) ?? ''));
          return ratio < 4.5 ? [`${token} on ${background}: ${ratio.toFixed(2)}:1`] : [];
        }),
      );
      expect(failures).toEqual([]);
    });

    it(`${name}: --on-brand is at least 4.5:1 on every stop of --brand-fill`, () => {
      const fill = stops(theme.get('--brand-fill') ?? '');
      expect(fill.length).toBeGreaterThanOrEqual(2);
      for (const stop of fill) {
        expect(contrast(theme.get('--on-brand') ?? '', stop), stop).toBeGreaterThanOrEqual(4.5);
      }
    });

    it(`${name}: every stop of the gradient title is at least 4.5:1 on --bg-root and --bg-surface`, () => {
      const title = stops(theme.get('--brand-gradient') ?? '');
      expect(title.length).toBeGreaterThanOrEqual(2);
      for (const stop of title) {
        for (const background of ['--bg-root', '--bg-surface']) {
          expect(contrast(stop, theme.get(background) ?? ''), `${stop} on ${background}`).toBeGreaterThanOrEqual(4.5);
        }
      }
    });
  }

  it("a field's placeholder is --text-muted at full opacity, over Tailwind's half-faded default", () => {
    const rule = /::placeholder \{([^}]*)\}/.exec(css)?.[1] ?? '';
    expect(rule).toMatch(/color: var\(--text-muted\);/);
    expect(rule).toMatch(/opacity: 1;/);
  });

  for (const [name, theme] of [
    ['dark', dark],
    ['light', light],
  ] as const) {
    it(`${name}: a field's text and placeholder are at least 4.5:1 on --bg-input`, () => {
      for (const token of ['--text', '--text-muted']) {
        expect(contrast(theme.get(token) ?? '', theme.get('--bg-input') ?? ''), token).toBeGreaterThanOrEqual(4.5);
      }
    });
  }

  it('maps every semantic color for Tailwind', () => {
    for (const token of ['--bg-surface', '--text-muted', '--brand-400', '--font-google']) {
      expect(css).toMatch(new RegExp(`--color-[\\w-]+: var\\(${token}\\);`));
    }
  });
});
