import type { ButtonHTMLAttributes } from 'react';

import { cx } from '../cx';

export type ButtonVariant = 'primary' | 'wash' | 'ghost' | 'danger';

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: 'sm' | 'md';
}

const VARIANTS: Record<ButtonVariant, string> = {
  primary: 'brand-fill shadow-lg shadow-brand-500/20 hover:brightness-95',
  wash: 'border border-brand-500/40 bg-brand-wash text-brand-300 hover:border-brand-400',
  ghost: 'text-fg-secondary hover:bg-elevated hover:text-fg',
  danger: 'border border-danger/40 bg-danger/10 text-danger hover:bg-danger/20',
};

/**
 * The keyboard focus indicator: an outline, which every browser draws on native-appearance controls (WebKit draws no
 * box shadow on a `<select>`) and which forced-colors mode keeps.
 */
export const focusRing = 'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand-400';

export function Button({ variant = 'ghost', size = 'md', className, type = 'button', ...props }: ButtonProps) {
  return (
    <button
      type={type}
      className={cx(
        'inline-flex items-center justify-center gap-2 rounded-md font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50',
        size === 'sm' ? 'px-2.5 py-1 text-xs' : 'px-3.5 py-2 text-sm',
        VARIANTS[variant],
        focusRing,
        className,
      )}
      {...props}
    />
  );
}
