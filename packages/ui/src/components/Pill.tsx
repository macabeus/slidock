import type { ReactNode } from 'react';

import { cx } from '../cx';

export interface PillProps {
  /** A CSS color (`var(--font-google)`, `#34d399`); the pill is that color on a ~13% wash of it. Omit for neutral. */
  tint?: string | undefined;
  dot?: boolean | undefined;
  mono?: boolean | undefined;
  size?: 'sm' | 'xs';
  className?: string | undefined;
  title?: string | undefined;
  children: ReactNode;
}

export function Pill({ tint, dot, mono, size = 'sm', className, title, children }: PillProps) {
  return (
    <span
      title={title}
      className={cx(
        'inline-flex items-center gap-1.5 whitespace-nowrap font-medium',
        size === 'sm' ? 'rounded-md px-2 py-0.5 text-xs' : 'rounded px-1.5 text-[11px]',
        mono && 'font-mono tabular-nums',
        !tint && 'bg-elevated/60 text-fg-secondary',
        className,
      )}
      style={tint ? { color: tint, backgroundColor: `color-mix(in srgb, ${tint} 13%, transparent)` } : undefined}
    >
      {dot && <span aria-hidden className="size-1.5 rounded-full bg-current" />}
      {children}
    </span>
  );
}
