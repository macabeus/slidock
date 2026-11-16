import { Icon, cx, focusRing } from '@slidock/ui';
import { type ReactNode, useId, useState } from 'react';

import { short } from '../model/format';
import type { Trend } from '../model/overview';
import { commitUrl } from '../model/runs';

export interface SegmentOption<T extends string> {
  value: T;
  label: ReactNode;
  title?: string;
}

/** One choice out of a few, as a radio group: a segmented control. */
export function SegmentedControl<T extends string>({
  options,
  value,
  onChange,
  label,
  className,
}: {
  options: readonly SegmentOption<T>[];
  value: T;
  onChange(value: T): void;
  label: string;
  className?: string;
}) {
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={cx('inline-flex flex-wrap rounded-lg border border-line/60 bg-surface/60 p-0.5', className)}
    >
      {options.map((option) => {
        const selected = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={selected}
            title={option.title}
            onClick={() => onChange(option.value)}
            className={cx(
              'rounded-md px-2.5 py-1 text-xs font-medium transition-colors',
              selected ? 'bg-elevated text-fg shadow-sm' : 'text-muted hover:text-fg-secondary',
              focusRing,
            )}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

/** A commit, short, linking to its page in the repository. */
export function CommitLink({ commit }: { commit: string }) {
  return (
    <a className="font-mono text-fg underline hover:text-brand-300" href={commitUrl(commit)} title={commit}>
      {short(commit)}
    </a>
  );
}

/** A labelled native select. */
export function Select({
  label,
  value,
  onChange,
  children,
  className,
}: {
  label: string;
  value: string;
  onChange(value: string): void;
  children: ReactNode;
  className?: string;
}) {
  const id = useId();
  return (
    <div className={cx('flex min-w-0 max-w-full flex-col gap-1', className)}>
      <label htmlFor={id} className="text-xs text-muted">
        {label}
      </label>
      <select
        id={id}
        value={value}
        onChange={(event) => onChange(event.currentTarget.value)}
        className={cx('w-full min-w-0 rounded-md border border-line bg-input px-2 py-1.5 text-sm text-fg', focusRing)}
      >
        {children}
      </select>
    </div>
  );
}

export interface ToggleOption {
  value: string;
  label: ReactNode;
  count?: number | undefined;
  /** A CSS color for the option's dot. */
  tint?: string | undefined;
}

/** A set of toggle chips (any of); each shows the rows choosing it would leave. */
export function ChipGroup({
  label,
  options,
  selected,
  onChange,
}: {
  label: string;
  options: readonly ToggleOption[];
  selected: readonly string[];
  onChange(next: string[]): void;
}) {
  const labelId = useId();
  return (
    <div className="flex flex-col gap-1">
      <span id={labelId} className="text-xs text-muted">
        {label}
      </span>
      <div role="group" aria-labelledby={labelId} className="flex flex-wrap gap-1.5">
        {options.map((option) => {
          const on = selected.includes(option.value);
          return (
            <button
              key={option.value}
              type="button"
              aria-pressed={on}
              onClick={() =>
                onChange(on ? selected.filter((value) => value !== option.value) : [...selected, option.value])
              }
              className={cx(
                'inline-flex items-center gap-1.5 rounded-md border px-2.5 py-1 text-xs transition-colors',
                on
                  ? 'border-brand-500/50 bg-brand-wash text-fg'
                  : 'border-line text-fg-secondary hover:border-muted hover:text-fg',
                focusRing,
              )}
            >
              {option.tint && (
                <span aria-hidden className="size-2 rounded-full" style={{ backgroundColor: option.tint }} />
              )}
              {option.label}
              {option.count !== undefined && <span className="font-mono tabular-nums text-muted">{option.count}</span>}
            </button>
          );
        })}
      </div>
    </div>
  );
}

/** A mono command with a copy button. */
export function CopyCommand({ command, label }: { command: string; label?: string }) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    const clipboard = typeof navigator === 'undefined' ? undefined : navigator.clipboard;
    if (!clipboard) {
      return;
    }
    clipboard.writeText(command).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      },
      () => setCopied(false),
    );
  };
  return (
    <div className="flex items-start gap-2 rounded-md bg-sunken/70 px-3 py-2">
      <code className="min-w-0 flex-1 break-all font-mono text-[12px] leading-relaxed text-fg-secondary">
        {label && <span className="sr-only">{label}: </span>}
        {command}
      </code>
      <button
        type="button"
        onClick={copy}
        aria-label={copied ? 'Copied' : `Copy ${label ?? 'command'}`}
        title={copied ? 'Copied' : 'Copy'}
        className={cx('shrink-0 rounded p-1 text-muted hover:bg-elevated hover:text-fg', focusRing)}
      >
        <Icon name={copied ? 'check' : 'clipboard'} className="size-4" />
      </button>
    </div>
  );
}

const TREND_GLYPH: Record<Trend, string> = { improved: '▲', regressed: '▼', unchanged: '' };

/** A delta with its direction as a glyph and a color, so color is never the only signal. */
export function DeltaBadge({ delta, trend, text }: { delta: number | null; trend: Trend; text: string }) {
  if (delta === null) {
    return null;
  }
  const color = trend === 'unchanged' ? 'var(--text-muted)' : `var(--delta-${trend})`;
  return (
    <span className="inline-flex items-center gap-0.5 font-mono text-xs tabular-nums" style={{ color }}>
      {TREND_GLYPH[trend] && <span aria-hidden>{TREND_GLYPH[trend]}</span>}
      {text}
      {trend !== 'unchanged' && <span className="sr-only"> ({trend})</span>}
    </span>
  );
}

/** A headline number: an eyebrow label, a mono value and a hint. */
export function Stat({
  label,
  value,
  hint,
  accent,
  extra,
}: {
  label: string;
  value: ReactNode;
  hint?: ReactNode;
  accent?: boolean;
  extra?: ReactNode;
}) {
  return (
    <div className="rounded-xl bg-surface/60 p-4 ring-1 ring-panel-edge ring-inset">
      <p className="text-[11px] font-semibold uppercase tracking-wider text-muted">{label}</p>
      <p className={cx('mt-1 font-mono text-2xl font-bold tabular-nums sm:text-3xl', accent && 'brand-text w-fit')}>
        {value}
      </p>
      {(hint ?? extra) && (
        <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted">
          {hint}
          {extra}
        </div>
      )}
    </div>
  );
}
