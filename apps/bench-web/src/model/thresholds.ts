// Imported by the build's dev mock too, so its imports name their files.
import type { Stamp, Status, Thresholds } from '../schema/index.ts';

/** `classifier@2`'s thresholds, which its version names. */
export const CLASSIFIER_THRESHOLDS: Readonly<Record<string, Thresholds>> = {
  'classifier@2': {
    match: { ssimMean: 0.97, ssimP1: 0.6, colorMean: 3, textRecall: 0.98, p95Pm: 2.8 },
    close: { ssimMean: 0.9, colorMean: 8, textRecall: 0.9, p95Pm: 11 },
  },
};

/**
 * The thresholds a results file was classified by. The server's (`Status.thresholds`) are HEAD's, and the harness
 * hash covers the classifier, so they hold for a run of HEAD's harness. Any other run reads the thresholds its
 * classifier version names, or null for a classifier this UI does not know.
 */
export function thresholdsFor(stamp: Pick<Stamp, 'classifier' | 'harness'>, status: Status | null): Thresholds | null {
  if (status !== null && stamp.harness === status.head.harness) {
    return status.thresholds;
  }
  return CLASSIFIER_THRESHOLDS[stamp.classifier] ?? null;
}
