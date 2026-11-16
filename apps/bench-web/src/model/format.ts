// Number and identity formatting. Scores live on a 1e-4 grid, so they print as JavaScript prints them (`0.9707`,
// `1`), never padded or rounded further; missing values print as an em dash.

export const DASH = '—';

export function formatNumber(value: number | null | undefined): string {
  return value === null || value === undefined || Number.isNaN(value) ? DASH : String(value);
}

/** A share in [0, 1] as a percentage with one decimal (`87.5%`). */
export function formatPercent(value: number | null | undefined): string {
  if (value === null || value === undefined || Number.isNaN(value)) {
    return DASH;
  }
  return `${(Math.round(value * 1000) / 10).toFixed(1)}%`;
}

/** A signed delta (`+0.0021`, `−0.0105`), with a true minus sign; deltas print on the 1e-4 grid. */
export function formatDelta(value: number | null | undefined): string {
  if (value === null || value === undefined || Number.isNaN(value)) {
    return DASH;
  }
  const rounded = Math.round(value * 10_000) / 10_000;
  if (rounded === 0) {
    return '±0';
  }
  return rounded > 0 ? `+${rounded}` : `−${Math.abs(rounded)}`;
}

/** A delta of two shares in percentage points (`+12.5 pp`, `−3.0 pp`). */
export function formatPointsDelta(value: number | null | undefined): string {
  if (value === null || value === undefined || Number.isNaN(value)) {
    return DASH;
  }
  const points = Math.round(value * 1000) / 10;
  if (points === 0) {
    return '±0 pp';
  }
  return `${points > 0 ? '+' : '−'}${Math.abs(points).toFixed(1)} pp`;
}

/** The first 7 characters of a commit or key. */
export const short = (id: string, length = 7) => id.slice(0, length);

/** An RFC 3339 UTC time as `2026-10-30 09:30 UTC`. */
export function formatTime(rfc3339: string): string {
  const match = /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2})/.exec(rfc3339);
  return match ? `${match[1]} ${match[2]} UTC` : rfc3339;
}
