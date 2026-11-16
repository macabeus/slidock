import { type RefObject, useEffect, useRef } from 'react';

// Open overlays, innermost last. Escape closes only the innermost; the page scroll stays locked while any is open.
const stack: object[] = [];
let savedOverflow = '';

function lockScroll() {
  if (stack.length === 1) {
    savedOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
  }
}

function unlockScroll() {
  if (stack.length === 0) {
    document.body.style.overflow = savedOverflow;
  }
}

/** The number of open overlays (for tests and for keyboard handlers that must stay quiet under an overlay). */
export function openOverlayCount(): number {
  return stack.length;
}

const FOCUSABLE =
  'a[href],button:not([disabled]),input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex="-1"])';

function focusables(container: HTMLElement): HTMLElement[] {
  return Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE));
}

/**
 * Registers an open modal overlay: Escape closes it when it is the innermost, Tab cycles inside `container`, the
 * page does not scroll, and focus moves in on open and back to where it was on close.
 */
export function useOverlay(open: boolean, onClose: () => void, container: RefObject<HTMLElement | null>) {
  const onCloseRef = useRef(onClose);
  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  useEffect(() => {
    if (!open) {
      return;
    }
    const token = {};
    stack.push(token);
    lockScroll();
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const node = container.current;
    if (node) {
      const autofocus = node.querySelector<HTMLElement>('[data-autofocus]');
      (autofocus ?? focusables(node)[0] ?? node).focus();
    }

    const onKeyDown = (event: KeyboardEvent) => {
      if (stack[stack.length - 1] !== token || event.defaultPrevented) {
        return;
      }
      if (event.key === 'Escape') {
        event.preventDefault();
        onCloseRef.current();
        return;
      }
      const root = container.current;
      if (event.key === 'Tab' && root) {
        const items = focusables(root);
        const first = items[0];
        const last = items[items.length - 1];
        // Focus the Tab key moves shows as keyboard focus, even after a pointer interaction (WebKit and Firefox
        // otherwise leave a programmatic focus without :focus-visible).
        if (!first || !last) {
          event.preventDefault();
          root.focus({ focusVisible: true });
        } else if (event.shiftKey && (document.activeElement === first || !root.contains(document.activeElement))) {
          event.preventDefault();
          last.focus({ focusVisible: true });
        } else if (!event.shiftKey && (document.activeElement === last || !root.contains(document.activeElement))) {
          event.preventDefault();
          first.focus({ focusVisible: true });
        }
      }
    };
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('keydown', onKeyDown);
      const index = stack.indexOf(token);
      if (index >= 0) {
        stack.splice(index, 1);
      }
      unlockScroll();
      if (previous?.isConnected) {
        previous.focus();
      }
    };
  }, [open, container]);
}
