import { describe, expect, it } from 'vitest';

import { ALL_OUTCOMES } from '../src/model/outcomes';
import { PARITY_STATUSES } from '../src/model/parity';
import {
  DELTA_COLORS,
  FONT_SOURCE_COLORS,
  ON_FILL,
  OUTCOME_COLORS,
  PARITY_COLORS,
  UNKNOWN_COLOR,
  themeCss,
} from '../src/theme';

function luminance(hex: string): number {
  const value = Number.parseInt(hex.slice(1), 16);
  const channel = (shift: number) => {
    const c = ((value >> shift) & 0xff) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0);
}

const contrast = (a: string, b: string) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
};

describe('data colors', () => {
  it('give every class and parity status a swatch', () => {
    expect(Object.keys(OUTCOME_COLORS).sort()).toEqual([...ALL_OUTCOMES].sort());
    expect(Object.keys(PARITY_COLORS).sort()).toEqual([...PARITY_STATUSES].sort());
  });

  it('keep text readable on its own pill wash: dark text over the dark surface, light text over the light chip', () => {
    // A pill is its text color on 13% of itself (packages/ui Pill).
    const wash = (tint: string, background: string) => {
      const mix = (shift: number) =>
        Math.round(
          ((parseInt(tint.slice(1), 16) >> shift) & 0xff) * 0.13 +
            ((parseInt(background.slice(1), 16) >> shift) & 0xff) * 0.87,
        );
      return `#${[16, 8, 0].map((shift) => mix(shift).toString(16).padStart(2, '0')).join('')}`;
    };
    const scales = {
      ...OUTCOME_COLORS,
      ...PARITY_COLORS,
      ...DELTA_COLORS,
      ...FONT_SOURCE_COLORS,
      unknown: UNKNOWN_COLOR,
    };
    for (const [name, swatch] of Object.entries(scales)) {
      expect(contrast(swatch.light, wash(swatch.light, '#f1f5f9')), `${name} light`).toBeGreaterThanOrEqual(4.5);
      expect(contrast(swatch.dark, wash(swatch.dark, '#1e293b')), `${name} dark`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it('keep a label inside any fill readable', () => {
    const scales = {
      ...OUTCOME_COLORS,
      ...PARITY_COLORS,
      ...DELTA_COLORS,
      ...FONT_SOURCE_COLORS,
      unknown: UNKNOWN_COLOR,
    };
    for (const [name, swatch] of Object.entries(scales)) {
      expect(contrast(ON_FILL, swatch.fill), name).toBeGreaterThanOrEqual(4.5);
    }
  });

  it('never use the brand pink', () => {
    const brand = ['#f472b6', '#ec4899', '#db2777', '#be185d', '#fb7185', '#f9a8d4', '#9d174d', '#be123c'];
    const scales = {
      ...OUTCOME_COLORS,
      ...PARITY_COLORS,
      ...DELTA_COLORS,
      ...FONT_SOURCE_COLORS,
      unknown: UNKNOWN_COLOR,
    };
    for (const [name, swatch] of Object.entries(scales)) {
      for (const color of [swatch.fill, swatch.dark, swatch.light]) {
        expect(brand, name).not.toContain(color);
      }
    }
  });

  it('publish both themes under the token selectors', () => {
    const css = themeCss();
    expect(css).toContain(':root { --oc-match: #34d399; --oc-match-fill: #34d399;');
    expect(css).toContain(":root[data-theme='light'] { --oc-match: #065f46; --oc-match-fill: #34d399;");
    expect(css).toContain('@media (prefers-color-scheme: light) { :root:not([data-theme]) {');
    expect(css).toContain('--parity-drift-same-class: #92400e;');
    expect(css).toContain('--on-fill: #0f172a;');
  });
});
