// How a slide advances: on a click, and after `afterMs` from the end of its transition. The deck
// stores it on the slide's own transition. A value outside the contract plays the declared default: advance on a
// click, no timer.
import type { DeckTransition } from '@slidock/types';

import { LIMITS } from './effects/vocab';

export interface Advance {
  /** A click on the slide advances; when not, the slide's click groups play on their own, one after another. */
  readonly onClick: boolean;
  /** The delay from the end of the transition after which the slide advances once no group runs. */
  readonly afterMs: number | null;
  /** The field that was out of the contract, when one was. */
  readonly issue: 'onClick' | 'afterMs' | null;
}

export const DEFAULT_ADVANCE: Advance = { onClick: true, afterMs: null, issue: null };

/** Reads a slide's advance from its transition. */
export function readAdvance(transition: DeckTransition | null): Advance {
  const advance: unknown = transition?.advance ?? null;
  if (advance === null) {
    return DEFAULT_ADVANCE;
  }
  if (typeof advance !== 'object') {
    return { ...DEFAULT_ADVANCE, issue: 'onClick' };
  }
  const { onClick, afterMs } = advance as { onClick?: unknown; afterMs?: unknown };
  let issue: Advance['issue'] = null;
  let click = true;
  if (typeof onClick === 'boolean') {
    click = onClick;
  } else {
    issue = 'onClick';
  }
  let after: number | null = null;
  if (typeof afterMs === 'number' && Number.isFinite(afterMs) && afterMs >= 0 && afterMs <= LIMITS.ms) {
    after = afterMs;
  } else if (afterMs !== null) {
    issue = 'afterMs';
  }
  return { onClick: click, afterMs: after, issue };
}

/** What an idle presenting player does next: advance (play the next group, or go to the next slide), or wait. */
export function autoNext(advance: Advance, due: boolean, groupsLeft: boolean): boolean {
  return due || (groupsLeft && !advance.onClick);
}
