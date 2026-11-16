import type { ReactNode } from 'react';

import { cx } from '../cx';

export interface AppHeaderProps {
  logo?: ReactNode;
  title: string;
  subtitle?: ReactNode;
  right?: ReactNode;
  /** A 56 px bar, for when content needs the space. */
  compact?: boolean | undefined;
  className?: string | undefined;
}

/** The family header card: a glowing logo, the gradient title, a subtitle, right-side content and the brand strip. */
export function AppHeader({ logo, title, subtitle, right, compact, className }: AppHeaderProps) {
  return (
    <header
      className={cx(
        'overflow-hidden bg-gradient-to-r from-surface via-elevated/40 to-surface shadow-xl',
        compact ? 'rounded-xl' : 'rounded-2xl',
        className,
      )}
    >
      <div
        className={cx(
          'flex items-center gap-4',
          // The right side stays on the title's row and wraps under it only when it does not fit.
          compact ? 'h-14 px-4' : 'flex-wrap px-4 py-5 sm:px-8 sm:py-6',
        )}
      >
        <div className={cx('flex flex-1 items-center gap-4', compact ? 'min-w-0' : 'min-w-48')}>
          {logo && (
            <div className={cx('relative shrink-0', compact ? 'size-8' : 'size-14')}>
              <div aria-hidden className="absolute inset-0 rounded-full bg-accent/25 blur-xl" />
              <div className="relative size-full">{logo}</div>
            </div>
          )}
          <div className="min-w-0">
            <p
              className={cx(
                'brand-text w-fit font-bold tracking-tight',
                compact ? 'text-lg leading-tight' : 'text-2xl sm:text-3xl',
              )}
            >
              {title}
            </p>
            {subtitle && (
              <div className={cx('truncate font-medium text-muted', compact ? 'text-xs' : 'text-sm')}>{subtitle}</div>
            )}
          </div>
        </div>
        {right && <div className="ml-auto flex shrink-0 flex-wrap items-center gap-2">{right}</div>}
      </div>
      <div aria-hidden className="brand-gradient h-1" />
    </header>
  );
}
