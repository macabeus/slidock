// Keys and pointer zones (docs/player-api.md), with PowerPoint's conventions: arrows, PageUp/PageDown, Space and Enter,
// Home/End, a number then Enter to go to that slide, B/W (and `.`/`,`) to blank, F for full screen, `?` for help,
// P for the presenter view (when the player offers one), Escape to stop. The mapping is a pure function so it is tested without a DOM.

export type Command =
  | { readonly type: 'next' }
  | { readonly type: 'previous' }
  | { readonly type: 'first' }
  | { readonly type: 'last' }
  | { readonly type: 'goToNumber'; readonly n: number }
  | { readonly type: 'blank'; readonly kind: 'black' | 'white' }
  | { readonly type: 'fullscreen' }
  | { readonly type: 'help' }
  | { readonly type: 'presenter' }
  | { readonly type: 'present' }
  | { readonly type: 'stop' };

export interface KeyLike {
  readonly key: string;
  readonly ctrlKey: boolean;
  readonly metaKey: boolean;
  readonly altKey: boolean;
  readonly defaultPrevented: boolean;
}

export interface KeyContext {
  readonly presenting: boolean;
  /** The digits typed so far for a go-to. */
  readonly digits: string;
  readonly helpOpen: boolean;
  /** Whether P opens a presenter view: otherwise the key is not the player's. */
  readonly presenter: boolean;
}

export interface KeyResult {
  readonly command: Command | null;
  /** The go-to digits after this key. */
  readonly digits: string;
  /** Whether the key was the player's (its default is then prevented). */
  readonly handled: boolean;
}

const IGNORED: KeyResult = { command: null, digits: '', handled: false };

export function keyCommand(event: KeyLike, context: KeyContext): KeyResult {
  if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey) {
    return { ...IGNORED, digits: context.digits };
  }
  const { key } = event;
  const command = (c: Command): KeyResult => ({ command: c, digits: '', handled: true });
  if (context.helpOpen) {
    // The help overlay is modal: its own keys close it, and the deck behind it does not move.
    return key === 'Escape' || key === 'Esc' || key === '?' ? command({ type: 'help' }) : IGNORED;
  }
  if (/^[0-9]$/.test(key)) {
    return { command: null, digits: (context.digits + key).slice(-4), handled: true };
  }
  switch (key) {
    case 'Enter':
      if (context.digits !== '') {
        return command({ type: 'goToNumber', n: Number(context.digits) });
      }
      return context.presenting ? command({ type: 'next' }) : IGNORED;
    case ' ':
    case 'Spacebar':
      return context.presenting ? command({ type: 'next' }) : IGNORED;
    case 'ArrowRight':
    case 'ArrowDown':
    case 'PageDown':
      return command({ type: 'next' });
    case 'ArrowLeft':
    case 'ArrowUp':
    case 'PageUp':
    case 'Backspace':
      return command({ type: 'previous' });
    case 'Home':
      return command({ type: 'first' });
    case 'End':
      return command({ type: 'last' });
    case 'b':
    case 'B':
    case '.':
      return command({ type: 'blank', kind: 'black' });
    case 'w':
    case 'W':
    case ',':
      return command({ type: 'blank', kind: 'white' });
    case 'f':
    case 'F':
      return command({ type: 'fullscreen' });
    case '?':
      return command({ type: 'help' });
    case 'p':
    case 'P':
      return context.presenter ? command({ type: 'presenter' }) : IGNORED;
    case 'F5':
      return command({ type: 'present' });
    case 'Escape':
    case 'Esc':
      if (context.digits !== '') {
        return { command: null, digits: '', handled: true };
      }
      return context.presenting ? command({ type: 'stop' }) : IGNORED;
    default:
      return IGNORED;
  }
}

/** Whether a key event comes from a field the user types in. */
export function isEditable(target: EventTarget | null): boolean {
  if (target === null || typeof (target as Partial<Element>).closest !== 'function') {
    return false;
  }
  const element = target as HTMLElement;
  return (
    element.isContentEditable === true ||
    element.closest('input, textarea, select, [contenteditable=""], [contenteditable="true"]') !== null
  );
}

/** Whether `key` activates a focused link or button (Enter, Space). */
export function activates(key: string): boolean {
  return key === 'Enter' || key === ' ' || key === 'Spacebar';
}

/**
 * Whether a key event comes from an element that Enter or Space activates (a link, a button, a disclosure's summary),
 * whose own action those keys then keep.
 */
export function isActivatable(target: EventTarget | null): boolean {
  if (target === null || typeof (target as Partial<Element>).closest !== 'function') {
    return false;
  }
  return (target as Element).closest('a[href], button, summary, [role="button"], [role="link"]') !== null;
}

/** The pointer zone of a click at `x` on a box `width` wide: the left third goes back. */
export function clickZone(x: number, width: number): 'previous' | 'next' {
  return width > 0 && x < width / 3 ? 'previous' : 'next';
}

/** What a link in a slide asks for: a slide of the deck, or an external page. */
export type LinkTarget =
  | { readonly type: 'slide'; readonly id: string }
  | { readonly type: 'number'; readonly n: number }
  | { readonly type: 'external'; readonly url: string }
  | null;

/** The moves of a slide's `data-link="action:<name>"`. */
export const LINK_ACTIONS: ReadonlyMap<string, 'next' | 'previous' | 'first' | 'last'> = new Map([
  ['action:next', 'next'],
  ['action:prev', 'previous'],
  ['action:first', 'first'],
  ['action:last', 'last'],
]);

export function linkTarget(href: string): LinkTarget {
  const value = href.trim();
  const shown = /^#slide-(\d+)$/.exec(value);
  if (shown?.[1] !== undefined) {
    return { type: 'number', n: Number(shown[1]) };
  }
  const slide = /^#slide=(.+)$/.exec(value);
  if (slide?.[1] !== undefined) {
    return { type: 'slide', id: decodeURIComponent(slide[1]) };
  }
  const number = /^#(\d+)$/.exec(value);
  if (number?.[1] !== undefined) {
    return { type: 'number', n: Number(number[1]) };
  }
  if (/^(https?|mailto):/i.test(value)) {
    return { type: 'external', url: value };
  }
  return null;
}
