import type { ReactNode } from 'react';

import { cx } from '../cx';

export interface PanelProps {
  title?: ReactNode;
  /** Explains how to read the panel. */
  subtitle?: ReactNode;
  headerRight?: ReactNode;
  /** `panel`: translucent; `card`: solid; `tool`: bordered, with an eyebrow header (dense tool layouts). */
  variant?: 'panel' | 'card' | 'tool';
  className?: string | undefined;
  children?: ReactNode;
  as?: 'section' | 'div' | 'aside';
  'aria-label'?: string;
}

export function Panel({
  title,
  subtitle,
  headerRight,
  variant = 'panel',
  className,
  children,
  as: Tag = 'section',
  ...rest
}: PanelProps) {
  if (variant === 'tool') {
    return (
      <Tag className={cx('rounded-lg border border-line bg-surface/50', className)} {...rest}>
        {(title ?? headerRight) && (
          <header className="flex items-center justify-between gap-2 border-b border-line px-3 py-2">
            <h2 className="text-[11px] font-semibold uppercase tracking-wider text-muted">{title}</h2>
            {headerRight}
          </header>
        )}
        {children}
      </Tag>
    );
  }
  return (
    <Tag
      className={cx(
        'ring-1 ring-panel-edge ring-inset',
        variant === 'card' ? 'rounded-xl bg-surface/60 p-5' : 'rounded-lg bg-surface/30 p-4',
        className,
      )}
      {...rest}
    >
      {(title ?? headerRight) && (
        <header className="mb-3 flex items-start justify-between gap-3">
          <div>
            <h2 className="text-sm font-semibold text-fg">{title}</h2>
            {subtitle && <p className="mt-0.5 text-xs text-muted">{subtitle}</p>}
          </div>
          {headerRight}
        </header>
      )}
      {children}
    </Tag>
  );
}
