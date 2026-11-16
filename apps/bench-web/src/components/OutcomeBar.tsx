import { cx } from '@slidock/ui';

import { outcomeLabel, outcomeVars } from '../model/outcomes';

export interface BarSegment {
  key: string;
  count: number;
  /** A class name (its color) or a CSS color. */
  outcome?: string;
  color?: string;
  label?: string;
}

export interface OutcomeBarProps {
  segments: readonly BarSegment[];
  /** The bar's name for assistive technology (`PPTX rows by outcome`). */
  label: string;
  onSelect?: ((key: string) => void) | undefined;
  /** What a click on a segment does, for its accessible name (`open in the explorer`). */
  action?: string;
  className?: string;
  size?: 'md' | 'sm';
}

/** A stacked horizontal bar; each segment is a button when the bar is clickable, labelled with its count. */
export function OutcomeBar({ segments, label, onSelect, action, className, size = 'md' }: OutcomeBarProps) {
  const total = segments.reduce((sum, segment) => sum + segment.count, 0);
  return (
    <div
      role="group"
      aria-label={label}
      className={cx('flex w-full overflow-hidden rounded-md bg-elevated/50', size === 'md' ? 'h-7' : 'h-2', className)}
    >
      {total > 0 &&
        segments
          .filter((segment) => segment.count > 0)
          .map((segment) => {
            const name = segment.label ?? (segment.outcome ? outcomeLabel(segment.outcome) : segment.key);
            const color = segment.color ?? (segment.outcome ? outcomeVars(segment.outcome).fill : 'var(--text-dim)');
            const style = { width: `${(segment.count / total) * 100}%`, backgroundColor: color };
            const text = `${segment.count} ${name}`;
            const content = size === 'md' && segment.count / total >= 0.06 ? segment.count : null;
            const classes =
              'flex min-w-0 items-center justify-center font-mono text-xs font-semibold tabular-nums text-(--on-fill)';
            return onSelect ? (
              <button
                key={segment.key}
                type="button"
                style={style}
                title={text}
                aria-label={action ? `${text}: ${action}` : text}
                onClick={() => onSelect(segment.key)}
                className={cx(
                  classes,
                  'transition-[filter] hover:brightness-110 focus-visible:relative focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-(--on-fill)',
                )}
              >
                {content}
              </button>
            ) : (
              <span key={segment.key} style={style} title={text} className={classes}>
                <span className="sr-only">{text}</span>
                <span aria-hidden>{content}</span>
              </span>
            );
          })}
    </div>
  );
}
