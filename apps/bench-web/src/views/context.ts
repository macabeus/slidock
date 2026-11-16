import type { Results, RunEntry, RunsIndex, Status, Thresholds } from '../schema';
import type { BenchState } from '../state/url';

/** What every view reads: the fragment's state, the runs, the selected run and its results. */
export interface ViewProps {
  state: BenchState;
  set(change: Partial<BenchState>): void;
  index: RunsIndex;
  status: Status | null;
  /** The run the fragment selects. */
  run: RunEntry;
  /** Its results; null when the bench no longer holds them. */
  results: Results | null;
  /** The run compared against: the fragment's `base`, or the baseline when another run is selected. */
  base: RunEntry | null;
  thresholds: Thresholds | null;
}
