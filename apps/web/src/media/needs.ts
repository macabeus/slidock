// The HEIC photos of an open deck: each one the compiler planned is decoded by the browser (`photos.ts`) and handed
// to the compiler before `finish`; each other one stays omitted, or drawn from the preview its deck keeps, and its
// diagnostic (MID5601, MID5609) says what to do.
import type { DeckDiagnostic, HeicPlan, MediaNeed } from '@slidock/types';

import type { DecodePath } from './heic';
import { type PhotoDecoder, PhotoUndecodable } from './photos';

/** What became of one photo. */
export type PhotoOutcome =
  | { id: string; state: 'decoded'; path: DecodePath; elapsed: number }
  /** This browser cannot decode it. */
  | { id: string; state: 'undecodable'; reasons: readonly string[] }
  /** The compiler plans no decode of it. */
  | { id: string; state: 'unplanned'; reason: string }
  /** The compiler refused the browser's file. */
  | { id: string; state: 'refused'; reason: string };

export interface DecodedPhotos {
  outcomes: PhotoOutcome[];
  /** The decoded files by need, to add again to a reopened deck. */
  files: Map<string, ArrayBuffer>;
}

function planned(need: MediaNeed): need is MediaNeed & { plan: HeicPlan } {
  return need.plan !== null;
}

/** How many of `needs` the browser will be asked to decode. */
export function decodable(needs: readonly MediaNeed[]): number {
  return needs.filter(planned).length;
}

/**
 * Decodes each planned need in order and adds its file with `add` (which moves the bytes it is given); `progress`
 * hears how many are done. `current` false stops it between photos, and a photo decoded once the deck is gone is
 * never added.
 */
export async function decodePhotos(
  needs: readonly MediaNeed[],
  decoder: PhotoDecoder,
  add: (id: string, bytes: ArrayBuffer) => Promise<void>,
  progress: (done: number) => void = () => {},
  current: () => boolean = () => true,
): Promise<DecodedPhotos> {
  const outcomes: PhotoOutcome[] = [];
  const files = new Map<string, ArrayBuffer>();
  let done = 0;
  for (const need of needs) {
    if (!current()) {
      break;
    }
    if (!planned(need)) {
      outcomes.push({ id: need.id, state: 'unplanned', reason: need.refusal ?? 'no plan' });
      continue;
    }
    try {
      const decoded = await decoder.decode(need);
      if (!current()) {
        break;
      }
      try {
        await add(need.id, decoded.bytes.slice(0));
        files.set(need.id, decoded.bytes);
        outcomes.push({ id: need.id, state: 'decoded', path: decoded.path, elapsed: decoded.elapsed });
      } catch (error) {
        outcomes.push({
          id: need.id,
          state: 'refused',
          reason: error instanceof Error ? error.message : String(error),
        });
      }
    } catch (error) {
      if (!current()) {
        break;
      }
      const reasons = error instanceof PhotoUndecodable ? error.reasons : [String(error)];
      outcomes.push({ id: need.id, state: 'undecodable', reasons });
    }
    done += 1;
    progress(done);
  }
  return { outcomes, files };
}

const OPEN_ELSEWHERE = 'open the deck in Chrome or Safari, or export it to PPTX from Keynote';
const EXPORT = 'export the deck to PPTX from Keynote';

/** What the app says of a photo it could not show. */
function explanation(outcome: Exclude<PhotoOutcome, { state: 'decoded' }>): string {
  switch (outcome.state) {
    case 'undecodable':
      return `Media ${outcome.id} is a HEIC photo, which this browser cannot decode: ${OPEN_ELSEWHERE}, which converts its photos to JPEG`;
    case 'unplanned':
      return `Media ${outcome.id} is a HEIC photo slidock cannot decode (${outcome.reason}): ${EXPORT}, which converts its photos to JPEG`;
    case 'refused':
      return `Media ${outcome.id} is a HEIC photo this browser did not decode as its file describes (${outcome.reason}): ${OPEN_ELSEWHERE}, which converts its photos to JPEG`;
  }
}

/** What the app says after the compiler's own message of a photo drawn from its preview. */
function previewAdvice(outcome: Exclude<PhotoOutcome, { state: 'decoded' }>): string {
  switch (outcome.state) {
    case 'undecodable':
      return `This browser cannot decode it: ${OPEN_ELSEWHERE}, which converts its photos to JPEG`;
    case 'unplanned':
      return `slidock cannot decode it (${outcome.reason}): ${EXPORT}, which converts its photos to JPEG`;
    case 'refused':
      return `This browser did not decode it as its file describes (${outcome.reason}): ${OPEN_ELSEWHERE}, which converts its photos to JPEG`;
  }
}

/**
 * `diagnostics` with each photo's `MID5601` (left out) saying why and what the user can do, and each `MID5609` (drawn
 * from its preview) followed by the same.
 */
export function explainPhotos(
  diagnostics: readonly DeckDiagnostic[],
  outcomes: readonly PhotoOutcome[],
): DeckDiagnostic[] {
  const missing = outcomes.filter((outcome) => outcome.state !== 'decoded');
  if (missing.length === 0) {
    return [...diagnostics];
  }
  return diagnostics.map((diagnostic) => {
    const outcome =
      diagnostic.code === 'MID5601' || diagnostic.code === 'MID5609'
        ? missing.find((candidate) => diagnostic.message.includes(candidate.id))
        : undefined;
    if (!outcome) {
      return diagnostic;
    }
    const message =
      diagnostic.code === 'MID5601' ? explanation(outcome) : `${diagnostic.message}. ${previewAdvice(outcome)}`;
    return { ...diagnostic, message };
  });
}
