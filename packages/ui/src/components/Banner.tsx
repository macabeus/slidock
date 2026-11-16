import type { ReactNode } from 'react';

import { cx } from '../cx';
import { Icon, type IconName } from './Icon';

export type BannerTone = 'success' | 'warning' | 'danger' | 'info';

const TONES: Record<BannerTone, { box: string; icon: IconName }> = {
  success: { box: 'border-success/40 bg-success/10 text-success', icon: 'check-circle' },
  warning: { box: 'border-warning/30 bg-warning/5 text-warning', icon: 'exclamation' },
  danger: { box: 'border-danger/40 bg-danger/10 text-danger', icon: 'exclamation' },
  info: { box: 'border-line bg-surface/40 text-fg-secondary', icon: 'information' },
};

export interface BannerProps {
  tone: BannerTone;
  title: ReactNode;
  children?: ReactNode;
  actions?: ReactNode;
  className?: string | undefined;
}

/** A status banner. `danger` and `warning` banners are alerts for assistive technology. */
export function Banner({ tone, title, children, actions, className }: BannerProps) {
  const { box, icon } = TONES[tone];
  return (
    <div
      role={tone === 'danger' || tone === 'warning' ? 'alert' : 'status'}
      className={cx('flex gap-3 rounded-md border px-4 py-3 text-sm', box, className)}
    >
      <Icon name={icon} className="mt-0.5 size-5 shrink-0" />
      <div className="min-w-0 flex-1">
        <p className="font-semibold">{title}</p>
        {children && <div className="mt-1 text-fg-secondary">{children}</div>}
        {actions && <div className="mt-3 flex flex-wrap gap-2">{actions}</div>}
      </div>
    </div>
  );
}
