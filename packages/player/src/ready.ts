// The readiness gate: a slide shows, and a transition starts, once its faces have
// loaded and its images are decoded, or after a cap (1.5 s).
import type { DeckFont } from '@slidock/types';

export const READY_TIMEOUT_MS = 1500;

export interface ReadyResult {
  /** Whether the cap ran out before everything loaded. */
  readonly timedOut: boolean;
}

export interface ReadyOptions {
  readonly fonts?: Pick<FontFaceSet, 'load'> | undefined;
  readonly timeoutMs?: number;
  /** Resolves the wait at once (a click while waiting). */
  readonly signal?: AbortSignal;
}

/** The CSS font shorthand `document.fonts.load` takes for a face. */
export function fontQuery(face: Pick<DeckFont, 'family' | 'weight' | 'style'>): string {
  const family = face.family.replace(/["\\]/g, (c) => `\\${c}`);
  return `${face.style} ${face.weight} 16px "${family}"`;
}

/** Waits for `faces` and the images under `root`. Never rejects: a failed load counts as done. */
export function ready(
  root: ParentNode,
  faces: readonly Pick<DeckFont, 'family' | 'weight' | 'style'>[],
  options: ReadyOptions = {},
): Promise<ReadyResult> {
  const timeoutMs = options.timeoutMs ?? READY_TIMEOUT_MS;
  const loads: Promise<unknown>[] = [];
  const fonts = options.fonts;
  if (fonts !== undefined) {
    for (const face of faces) {
      loads.push(settle(() => fonts.load(fontQuery(face))));
    }
  }
  for (const image of Array.from(root.querySelectorAll('img'))) {
    if (typeof image.decode === 'function') {
      loads.push(settle(() => image.decode()));
    }
  }
  if (loads.length === 0) {
    return Promise.resolve({ timedOut: false });
  }
  return new Promise((resolve) => {
    let done = false;
    const finish = (timedOut: boolean): void => {
      if (!done) {
        done = true;
        clearTimeout(timer);
        options.signal?.removeEventListener('abort', abort);
        resolve({ timedOut });
      }
    };
    const abort = (): void => finish(false);
    const timer = setTimeout(() => finish(true), timeoutMs);
    if (options.signal?.aborted === true) {
      finish(false);
      return;
    }
    options.signal?.addEventListener('abort', abort);
    void Promise.all(loads).then(() => finish(false));
  });
}

function settle(load: () => Promise<unknown>): Promise<unknown> {
  try {
    return load().catch(() => undefined);
  } catch {
    return Promise.resolve();
  }
}
