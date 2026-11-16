import { cx } from '../cx';

export interface ProgressBarProps {
  /** Omit for an indeterminate bar (a travelling stripe; no `aria-valuenow`). */
  value?: number;
  max?: number;
  label: string;
  className?: string | undefined;
}

export function ProgressBar({ value, max = 1, label, className }: ProgressBarProps) {
  const determinate = value !== undefined;
  const ratio = determinate && max > 0 ? Math.min(1, Math.max(0, value / max)) : 0;
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={determinate ? 0 : undefined}
      aria-valuemax={determinate ? max : undefined}
      aria-valuenow={determinate ? Math.min(max, Math.max(0, value)) : undefined}
      className={cx('relative h-1.5 w-full overflow-hidden rounded-full bg-elevated', className)}
    >
      {determinate ? (
        <div
          className="brand-gradient h-full rounded-full transition-[width] motion-reduce:transition-none"
          style={{ width: `${ratio * 100}%` }}
        />
      ) : (
        <div className="brand-gradient absolute inset-y-0 left-0 w-2/5 rounded-full motion-safe:animate-indeterminate" />
      )}
    </div>
  );
}
