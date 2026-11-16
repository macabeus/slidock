// The bench's data colors, each scale defined once. Fills (bars, dots, chart series)
// are the same in both themes, and a label inside a fill is `ON_FILL`; text on the page uses the theme's text value,
// which stays readable on a pill's wash of itself. The brand pink never appears in data. `themeCss` turns the scales
// into CSS variables for both themes.
import type { FontSource, FontsOutcome, MotionOutcome, Outcome, ParityStatus } from './schema';

export interface Swatch {
  /** Bars, dots and chart series, in both themes. */
  fill: string;
  /** Text on the dark theme's surfaces, on its own pill wash too (at least 4.5:1). */
  dark: string;
  /** Text on the light theme's surfaces, on its own pill wash too (at least 4.5:1). */
  light: string;
}

/** The label color inside a fill: every fill is light, in both themes. */
export const ON_FILL = '#0f172a';

const MATCH: Swatch = { fill: '#34d399', dark: '#34d399', light: '#065f46' };
const CLOSE: Swatch = { fill: '#38bdf8', dark: '#38bdf8', light: '#075985' };
const AMBER: Swatch = { fill: '#fbbf24', dark: '#fbbf24', light: '#92400e' };
const VIOLET: Swatch = { fill: '#a78bfa', dark: '#c4b5fd', light: '#6d28d9' };
const SLATE: Swatch = { fill: '#94a3b8', dark: '#cbd5e1', light: '#475569' };
const RED: Swatch = { fill: '#f87171', dark: '#fca5a5', light: '#991b1b' };
const TEAL: Swatch = { fill: '#2dd4bf', dark: '#2dd4bf', light: '#115e59' };
const ORANGE: Swatch = { fill: '#fb923c', dark: '#fb923c', light: '#9a3412' };

export const OUTCOME_COLORS: Record<Outcome | MotionOutcome | FontsOutcome, Swatch> = {
  match: MATCH,
  close: CLOSE,
  mismatch: AMBER,
  declined: VIOLET,
  'render-error': SLATE,
  failed: RED,
  'motion-match': MATCH,
  'motion-close': CLOSE,
  'motion-mismatch': AMBER,
  'motion-fallback': TEAL,
  'fonts-complete': MATCH,
  'fonts-fallback': RED,
};

/**
 * Where a glyph's face came from (fonts lane), in the order bars stack them: the font-source identity the consumer app
 * uses too, with the renderer's fallback in the failure colour. Every bar carries a legend and
 * per-segment labels.
 */
export const FONT_SOURCE_COLORS: Record<FontSource | 'fallback', Swatch> = {
  embedded: VIOLET,
  google: MATCH,
  local: CLOSE,
  dropped: AMBER,
  substitute: ORANGE,
  fallback: RED,
};

/** A class the UI does not know: shown, never coerced into a known one. */
export const UNKNOWN_COLOR: Swatch = { fill: '#cbd5e1', dark: '#cbd5e1', light: '#475569' };

export const PARITY_COLORS: Record<ParityStatus, Swatch> = {
  identical: MATCH,
  'pixel-identical': CLOSE,
  'drift-better': TEAL,
  'drift-same-class': AMBER,
  regressed: RED,
  missing: SLATE,
  new: VIOLET,
};

export const DELTA_COLORS = {
  improved: MATCH,
  regressed: RED,
} satisfies Record<string, Swatch>;

/** The CSS variable a swatch's text color is published under. */
export const textVar = (scale: 'oc' | 'parity' | 'delta', name: string) => `--${scale}-${name}`;
export const fillVar = (scale: 'oc' | 'parity' | 'delta', name: string) => `--${scale}-${name}-fill`;

function declarations(theme: 'dark' | 'light'): string {
  const lines: string[] = [];
  const scales: [scale: 'oc' | 'parity' | 'delta', swatches: Record<string, Swatch>][] = [
    ['oc', OUTCOME_COLORS],
    ['parity', PARITY_COLORS],
    ['delta', DELTA_COLORS],
  ];
  for (const [scale, swatches] of scales) {
    for (const [name, swatch] of Object.entries(swatches)) {
      lines.push(`${textVar(scale, name)}: ${swatch[theme]};`);
      lines.push(`${fillVar(scale, name)}: ${swatch.fill};`);
    }
  }
  lines.push(`--oc-unknown: ${UNKNOWN_COLOR[theme]};`);
  lines.push(`--on-fill: ${ON_FILL};`);
  lines.push(`--oc-unknown-fill: ${UNKNOWN_COLOR.fill};`);
  return lines.join(' ');
}

/** The scales as CSS variables, under the same selectors as `@slidock/ui/tokens.css`. */
export function themeCss(): string {
  const dark = declarations('dark');
  const light = declarations('light');
  return [
    `:root { ${dark} }`,
    `:root[data-theme='light'] { ${light} }`,
    `@media (prefers-color-scheme: light) { :root:not([data-theme]) { ${light} } }`,
  ].join('\n');
}
