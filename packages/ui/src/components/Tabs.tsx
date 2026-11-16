import { type KeyboardEvent, type ReactNode, useRef } from 'react';

import { cx } from '../cx';
import { focusRing } from './Button';

export interface TabItem<T extends string> {
  id: T;
  label: ReactNode;
  count?: number;
}

export interface TabsProps<T extends string> {
  items: readonly TabItem<T>[];
  value: T;
  onChange(id: T): void;
  /** `pills`: primary navigation; `sub`: secondary tabs; `underline`: tabs inside a panel. */
  variant?: 'pills' | 'sub' | 'underline';
  label: string;
  /** Prefix of the tab and panel ids: tab `${idPrefix}-tab-${id}` controls panel `${idPrefix}-panel-${id}`. */
  idPrefix: string;
  /**
   * Which `TabPanel`s are in the document, so a tab names its panel only when there is one: `each` (hidden ones
   * hidden), only the `selected` one, or `none` (the panel area is collapsed).
   */
  panels?: 'each' | 'selected' | 'none';
  className?: string | undefined;
}

const BASE = {
  pills: 'rounded-xl border px-3 py-2 text-sm font-medium sm:px-4',
  sub: 'rounded-lg px-3.5 py-1.5 text-sm font-medium',
  underline: '-mb-px border-b-2 px-3 py-2 text-sm font-medium',
};
const ACTIVE = {
  pills: 'border-brand-500/30 bg-brand-wash text-brand-300 shadow-lg shadow-brand-500/10',
  sub: 'bg-brand-wash text-brand-300',
  underline: 'border-brand-400 text-fg',
};
const INACTIVE = {
  pills: 'border-transparent text-muted hover:bg-surface/50 hover:text-fg-secondary',
  sub: 'text-muted hover:bg-surface/50 hover:text-fg-secondary',
  underline: 'border-transparent text-muted hover:text-fg-secondary',
};

/** A WAI-ARIA tab list: arrow keys, Home and End move between tabs and select them. */
export function Tabs<T extends string>({
  items,
  value,
  onChange,
  variant = 'sub',
  label,
  idPrefix,
  panels = 'each',
  className,
}: TabsProps<T>) {
  const refs = useRef(new Map<T, HTMLButtonElement>());

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const index = items.findIndex((item) => item.id === value);
    const last = items.length - 1;
    const target =
      event.key === 'ArrowRight'
        ? index === last
          ? 0
          : index + 1
        : event.key === 'ArrowLeft'
          ? index <= 0
            ? last
            : index - 1
          : event.key === 'Home'
            ? 0
            : event.key === 'End'
              ? last
              : null;
    const item = target === null ? undefined : items[target];
    if (!item) {
      return;
    }
    event.preventDefault();
    onChange(item.id);
    refs.current.get(item.id)?.focus();
  };

  return (
    <div
      role="tablist"
      aria-label={label}
      onKeyDown={onKeyDown}
      className={cx('flex gap-1', variant === 'underline' && 'border-b border-line', className)}
    >
      {items.map((item) => {
        const selected = item.id === value;
        return (
          <button
            key={item.id}
            ref={(node) => {
              if (node) {
                refs.current.set(item.id, node);
              } else {
                refs.current.delete(item.id);
              }
            }}
            type="button"
            role="tab"
            id={`${idPrefix}-tab-${item.id}`}
            aria-selected={selected}
            aria-controls={
              panels === 'each' || (panels === 'selected' && selected) ? `${idPrefix}-panel-${item.id}` : undefined
            }
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange(item.id)}
            className={cx(
              'inline-flex items-center gap-2 transition-colors',
              BASE[variant],
              selected ? ACTIVE[variant] : INACTIVE[variant],
              focusRing,
            )}
          >
            {item.label}
            {item.count !== undefined && (
              <span className="rounded bg-elevated/70 px-1.5 font-mono text-[11px] tabular-nums text-fg-secondary">
                {item.count}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}

/** The panel a tab controls. */
export function TabPanel({
  idPrefix,
  id,
  children,
  className,
}: {
  idPrefix: string;
  id: string;
  children: ReactNode;
  className?: string | undefined;
}) {
  return (
    <div
      role="tabpanel"
      id={`${idPrefix}-panel-${id}`}
      aria-labelledby={`${idPrefix}-tab-${id}`}
      tabIndex={0}
      className={cx('rounded-lg', focusRing, className)}
    >
      {children}
    </div>
  );
}
