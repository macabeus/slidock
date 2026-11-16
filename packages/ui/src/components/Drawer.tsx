import { type ReactNode, useId, useRef } from 'react';

import { cx } from '../cx';
import { useOverlay } from '../overlay';
import { focusRing } from './Button';
import { Icon } from './Icon';

export interface OverlayProps {
  open: boolean;
  onClose(): void;
  title: ReactNode;
  /** Shown under the title. */
  subtitle?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  className?: string | undefined;
}

function ModalOverlay({
  open,
  onClose,
  title,
  subtitle,
  children,
  footer,
  className,
  placement,
}: OverlayProps & {
  placement: 'right' | 'center';
}) {
  const ref = useRef<HTMLDivElement>(null);
  const titleId = useId();
  useOverlay(open, onClose, ref);
  if (!open) {
    return null;
  }
  return (
    <div
      className={cx(
        'fixed inset-0 z-50 flex bg-overlay backdrop-blur-sm',
        placement === 'right' ? 'justify-end' : 'items-center justify-center p-4',
      )}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className={cx(
          'flex max-h-full flex-col bg-root text-fg shadow-2xl outline-none',
          placement === 'right'
            ? 'h-full w-full max-w-md border-l border-line'
            : 'w-full max-w-lg rounded-xl border border-line',
          className,
        )}
      >
        <header className="sticky top-0 flex items-start justify-between gap-4 border-b border-line px-6 py-4">
          <div className="min-w-0">
            <h2 id={titleId} className="text-lg font-bold">
              {title}
            </h2>
            {subtitle && <p className="mt-0.5 text-sm text-muted">{subtitle}</p>}
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className={cx('rounded-md p-1 text-muted hover:bg-elevated hover:text-fg', focusRing)}
          >
            <Icon name="x" className="size-5" />
          </button>
        </header>
        <div className="min-h-0 flex-1 space-y-5 overflow-y-auto p-6">{children}</div>
        {footer && <footer className="flex justify-end gap-2 border-t border-line px-6 py-4">{footer}</footer>}
      </div>
    </div>
  );
}

/** A modal panel on the right edge, for detail. */
export function Drawer(props: OverlayProps) {
  return <ModalOverlay {...props} placement="right" />;
}

/** A centred modal dialog. */
export function Dialog(props: OverlayProps) {
  return <ModalOverlay {...props} placement="center" />;
}
