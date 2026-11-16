// A row's scores against the classifier's thresholds: one bullet chart per metric (the value's marker over the
// close and match bands), with the numbers printed beside it with their scale (`0.962 / ≥ 0.97`).
import { cx } from '@slidock/ui';

import { formatNumber } from '../model/format';
import type { Scores, Thresholds } from '../schema';

export interface Bullet {
  label: string;
  value: number | null;
  /** Higher is better (`ge`) or lower is better (`le`). */
  direction: 'ge' | 'le';
  match: number | null;
  close: number | null;
  /** The axis. */
  min: number;
  max: number;
}

/** The bullets of a scored row; thresholds missing for an unknown classifier leave the bands out. */
export function bullets(scores: Scores, thresholds: Thresholds | null): Bullet[] {
  const m = thresholds?.match;
  const c = thresholds?.close;
  const low = (value: number, floor: number) => Math.min(floor, Math.floor(value * 20) / 20);
  return [
    {
      label: 'SSIM mean',
      value: scores.ssim.mean,
      direction: 'ge',
      match: m?.ssimMean ?? null,
      close: c?.ssimMean ?? null,
      min: low(scores.ssim.mean, 0.8),
      max: 1,
    },
    {
      label: 'SSIM p1',
      value: scores.ssim.p1,
      direction: 'ge',
      match: m?.ssimP1 ?? null,
      close: null,
      min: low(scores.ssim.p1, 0.3),
      max: 1,
    },
    {
      label: 'ΔE00 mean',
      value: scores.color.mean,
      direction: 'le',
      match: m?.colorMean ?? null,
      close: c?.colorMean ?? null,
      min: 0,
      max: Math.max(12, Math.ceil(scores.color.mean)),
    },
    {
      label: 'Text recall',
      value: scores.text.recall,
      direction: 'ge',
      match: m?.textRecall ?? null,
      close: c?.textRecall ?? null,
      min: low(scores.text.recall, 0.7),
      max: 1,
    },
    {
      label: 'Words placed p95 ‰',
      value: scores.text.p95Pm,
      direction: 'le',
      match: m?.p95Pm ?? null,
      close: c?.p95Pm ?? null,
      min: 0,
      max: Math.max(16, Math.ceil(scores.text.p95Pm ?? 0)),
    },
  ];
}

/** Which band a value falls in. */
export function band(bullet: Bullet): 'match' | 'close' | 'below' | null {
  const { value, direction, match, close } = bullet;
  if (value === null) {
    return null;
  }
  const passes = (threshold: number | null) =>
    threshold !== null && (direction === 'ge' ? value >= threshold : value <= threshold);
  return passes(match) ? 'match' : passes(close) ? 'close' : 'below';
}

function position(bullet: Bullet, value: number): number {
  const span = bullet.max - bullet.min;
  return span <= 0 ? 0 : Math.min(100, Math.max(0, ((value - bullet.min) / span) * 100));
}

function Track({ bullet }: { bullet: Bullet }) {
  const { direction, match, close, value, max, min } = bullet;
  const edge = direction === 'ge' ? max : min;
  const zone = (from: number | null, to: number | null, color: string) => {
    if (from === null || to === null) {
      return null;
    }
    const a = position(bullet, from);
    const b = position(bullet, to);
    return (
      <div
        className="absolute inset-y-0"
        style={{ left: `${Math.min(a, b)}%`, width: `${Math.abs(b - a)}%`, backgroundColor: color }}
      />
    );
  };
  return (
    <div aria-hidden className="relative h-3 overflow-hidden rounded-sm bg-elevated/60">
      {zone(close, match ?? edge, 'color-mix(in srgb, var(--oc-close-fill) 30%, transparent)')}
      {zone(match, edge, 'color-mix(in srgb, var(--oc-match-fill) 35%, transparent)')}
      {value !== null && (
        <div
          className="absolute inset-y-0 w-1 -translate-x-1/2 rounded-sm bg-fg"
          style={{ left: `${position(bullet, value)}%` }}
        />
      )}
    </div>
  );
}

export function ScoreBullets({ scores, thresholds }: { scores: Scores; thresholds: Thresholds | null }) {
  return (
    <dl className="grid grid-cols-1 gap-x-6 gap-y-3 sm:grid-cols-2 lg:grid-cols-3">
      {bullets(scores, thresholds).map((bullet) => {
        const where = band(bullet);
        const sign = bullet.direction === 'ge' ? '≥' : '≤';
        const scale = bullet.match === null ? '' : ` / ${sign} ${formatNumber(bullet.match)}`;
        return (
          <div key={bullet.label} className="min-w-0">
            <dt className="flex items-baseline justify-between gap-2 text-xs">
              <span className="text-muted">{bullet.label}</span>
              <span className="font-mono tabular-nums">
                <span className={cx('font-semibold', where === 'below' ? 'text-fg' : 'text-fg-secondary')}>
                  {formatNumber(bullet.value)}
                </span>
                <span className="text-muted">{scale}</span>
              </span>
            </dt>
            <dd className="mt-1">
              <Track bullet={bullet} />
              <span className="sr-only">
                {where === null ? 'no value' : where === 'below' ? 'below the close threshold' : `in the ${where} band`}
              </span>
            </dd>
          </div>
        );
      })}
    </dl>
  );
}
