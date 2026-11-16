import { useHashParams } from '@slidock/ui';
import { useCallback, useMemo } from 'react';

import { type BenchState, navigate, readState } from './url';

/** The fragment's state and a setter that applies a change with the history rules of `historyFor`. */
export function useBenchState(): [BenchState, (change: Partial<BenchState>) => void] {
  const params = useHashParams();
  const state = useMemo(() => readState(params), [params]);
  const set = useCallback((change: Partial<BenchState>) => navigate(state, change), [state]);
  return [state, set];
}
