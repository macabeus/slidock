import type { FontsOutcome, MotionOutcome, Outcome, RowOutcome, Thresholds } from '../schema';
import { OUTCOME_COLORS, UNKNOWN_COLOR } from '../theme';
import { formatNumber } from './format';

/** `classifier@2`'s classes, best first: the order of legends, bars and the outcome filter. */
export const OUTCOMES = [
  'match',
  'close',
  'mismatch',
  'declined',
  'render-error',
  'failed',
] as const satisfies readonly Outcome[];

export const MOTION_OUTCOMES = [
  'motion-match',
  'motion-close',
  'motion-fallback',
  'motion-mismatch',
] as const satisfies readonly MotionOutcome[];

/** The fonts lane's classes of rendered rows, best first. */
export const FONTS_OUTCOMES = ['fonts-complete', 'fonts-fallback'] as const satisfies readonly FontsOutcome[];

export const ALL_OUTCOMES: readonly RowOutcome[] = [...OUTCOMES, ...MOTION_OUTCOMES, ...FONTS_OUTCOMES];

export function isOutcome(value: string): value is Outcome {
  return (OUTCOMES as readonly string[]).includes(value);
}

export function isRowOutcome(value: string): value is RowOutcome {
  return (ALL_OUTCOMES as readonly string[]).includes(value);
}

export const OUTCOME_LABELS: Record<RowOutcome, string> = {
  match: 'match',
  close: 'close',
  mismatch: 'mismatch',
  declined: 'declined',
  'render-error': 'render error',
  failed: 'failed',
  'motion-match': 'motion match',
  'motion-close': 'motion close',
  'motion-mismatch': 'motion mismatch',
  'motion-fallback': 'fallback played',
  'fonts-complete': 'fonts complete',
  'fonts-fallback': 'fallback fonts',
};

/** A label for any class, known or not. */
export function outcomeLabel(outcome: string): string {
  return isRowOutcome(outcome) ? OUTCOME_LABELS[outcome] : outcome;
}

/**
 * The movement rank (`classifier@2`): failed 0 < render-error 1 < declined 2 < mismatch 3 < close 4 < match 5. Motion
 * classes sort beside their static counterparts, a played fallback between mismatch and close. Unknown classes rank
 * below everything.
 */
export function outcomeRank(outcome: string): number {
  switch (outcome) {
    case 'failed':
      return 0;
    case 'render-error':
      return 1;
    case 'declined':
      return 2;
    case 'mismatch':
    case 'motion-mismatch':
    case 'fonts-fallback':
      return 3;
    case 'motion-fallback':
      return 3.5;
    case 'close':
    case 'motion-close':
      return 4;
    case 'match':
    case 'motion-match':
    case 'fonts-complete':
      return 5;
    default:
      return -1;
  }
}

/** The CSS variables a class is drawn with: text and fill. */
export function outcomeVars(outcome: string): { text: string; fill: string } {
  const name = isRowOutcome(outcome) ? outcome : 'unknown';
  return { text: `var(--oc-${name})`, fill: `var(--oc-${name}-fill)` };
}

export function outcomeFill(outcome: string): string {
  return isRowOutcome(outcome) ? OUTCOME_COLORS[outcome].fill : UNKNOWN_COLOR.fill;
}

/** What each class means, quoting the thresholds when they are known. */
export function outcomeGloss(outcome: RowOutcome, thresholds: Thresholds | null): string {
  const n = formatNumber;
  switch (outcome) {
    case 'match':
      return thresholds
        ? `SSIM mean ≥ ${n(thresholds.match.ssimMean)}, SSIM p1 ≥ ${n(thresholds.match.ssimP1)}, ΔE ≤ ${n(thresholds.match.colorMean)}, text recall ≥ ${n(thresholds.match.textRecall)}, words placed within ${n(thresholds.match.p95Pm)}‰ (p95)`
        : 'the best class of the classifier';
    case 'close':
      return thresholds
        ? `SSIM mean ≥ ${n(thresholds.close.ssimMean)}, ΔE ≤ ${n(thresholds.close.colorMean)}, text recall ≥ ${n(thresholds.close.textRecall)}, words placed within ${n(thresholds.close.p95Pm)}‰ (p95)`
        : 'the second class of the classifier';
    case 'mismatch':
      return 'rendered and scored, below close';
    case 'declined':
      return 'a decline applies; not scored';
    case 'render-error':
      return 'the renderer or the oracle failed, not the compiler';
    case 'failed':
      return 'the compile crashed, timed out or refused, the slide is missing or failed, or its documents hold code';
    case 'motion-match':
      return 'every check holds: a stage ends on its still render, a transition frame is between its two slides';
    case 'motion-close':
      return 'the checks hold, but the transition frame shows one of its two slides unchanged';
    case 'motion-mismatch':
      return "a check failed: an end state, what shows, the timeline, or a transition's ends";
    case 'motion-fallback':
      return "the IR's fallback played (the deck names no effect the player plays) and the checks hold";
    case 'fonts-complete':
      return 'every glyph was drawn by a face the deck ships';
    case 'fonts-fallback':
      return "the renderer's own fonts drew some glyphs";
  }
}
