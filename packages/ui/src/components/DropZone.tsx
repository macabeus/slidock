import { type ReactNode, useEffect, useRef, useState } from 'react';

import { cx } from '../cx';
import { focusRing } from './Button';
import { Icon } from './Icon';

function carriesFiles(event: DragEvent): boolean {
  return Array.from(event.dataTransfer?.types ?? []).includes('Files');
}

/**
 * Makes the whole window a drop target for files and folders while `enabled`. Returns whether a drag carrying files
 * is over the window, for a drop overlay. `onDrop` receives the drop's `DataTransfer`, whose items can be read as
 * entries (folders included) during the call.
 */
export function useWindowDrop(onDrop: (data: DataTransfer) => void, enabled = true): boolean {
  const [dragging, setDragging] = useState(false);
  const onDropRef = useRef(onDrop);
  useEffect(() => {
    onDropRef.current = onDrop;
  }, [onDrop]);

  useEffect(() => {
    if (!enabled) {
      return;
    }
    let depth = 0;
    const enter = (event: DragEvent) => {
      if (!carriesFiles(event)) {
        return;
      }
      event.preventDefault();
      depth += 1;
      setDragging(true);
    };
    const over = (event: DragEvent) => {
      if (!carriesFiles(event)) {
        return;
      }
      event.preventDefault();
      if (event.dataTransfer) {
        event.dataTransfer.dropEffect = 'copy';
      }
    };
    const leave = (event: DragEvent) => {
      if (!carriesFiles(event)) {
        return;
      }
      depth = Math.max(0, depth - 1);
      if (depth === 0) {
        setDragging(false);
      }
    };
    const drop = (event: DragEvent) => {
      if (!carriesFiles(event) || !event.dataTransfer) {
        return;
      }
      event.preventDefault();
      depth = 0;
      setDragging(false);
      onDropRef.current(event.dataTransfer);
    };
    window.addEventListener('dragenter', enter);
    window.addEventListener('dragover', over);
    window.addEventListener('dragleave', leave);
    window.addEventListener('drop', drop);
    return () => {
      window.removeEventListener('dragenter', enter);
      window.removeEventListener('dragover', over);
      window.removeEventListener('dragleave', leave);
      window.removeEventListener('drop', drop);
    };
  }, [enabled]);

  return dragging && enabled;
}

/** The full-window highlight shown while a drag with files is over the window. */
export function DropOverlay({ visible, label }: { visible: boolean; label: string }) {
  if (!visible) {
    return null;
  }
  return (
    <div
      aria-hidden
      className="pointer-events-none fixed inset-0 z-[60] flex items-center justify-center bg-overlay p-4 backdrop-blur-sm"
    >
      <div className="max-w-lg rounded-2xl border-2 border-dashed border-brand-400 bg-root/80 px-6 py-8 text-center text-lg font-semibold text-brand-300 sm:px-10">
        {label}
      </div>
    </div>
  );
}

export interface DropZoneProps {
  /** The file input's `accept`. */
  accept: string;
  onPick(files: File[]): void;
  title: ReactNode;
  hint?: ReactNode;
  buttonLabel: string;
  children?: ReactNode;
  className?: string | undefined;
}

/** The click-to-browse card of the load screen; drops are taken window-wide by `useWindowDrop`. */
export function DropZone({ accept, onPick, title, hint, buttonLabel, children, className }: DropZoneProps) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <div
      className={cx(
        'flex w-full max-w-lg flex-col items-center gap-4 rounded-2xl border-2 border-dashed border-dim bg-surface/50 p-12 text-center transition-colors hover:border-muted',
        className,
      )}
    >
      <Icon name="upload" className="size-12 text-muted" />
      <div>
        <p className="text-lg font-semibold text-fg">{title}</p>
        {hint && <p className="mt-1 text-sm text-muted">{hint}</p>}
      </div>
      <button
        type="button"
        onClick={() => input.current?.click()}
        className={cx(
          'brand-fill rounded-lg px-5 py-2.5 text-sm font-semibold shadow-lg shadow-brand-500/20 hover:brightness-95',
          focusRing,
        )}
      >
        {buttonLabel}
      </button>
      <input
        ref={input}
        type="file"
        accept={accept}
        className="hidden"
        tabIndex={-1}
        aria-hidden
        onChange={(event) => {
          const files = Array.from(event.currentTarget.files ?? []);
          event.currentTarget.value = '';
          if (files.length > 0) {
            onPick(files);
          }
        }}
      />
      {children}
    </div>
  );
}
