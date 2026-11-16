import { useSyncExternalStore } from 'react';

import type { FontsController, FontsState } from './controller';

const noSubscription = () => () => {};
const noState = () => null;

/** The fonts flow's state, re-rendering on each change; `null` without a controller. */
export function useFontsState(fonts: FontsController | null): FontsState | null {
  return useSyncExternalStore(fonts?.subscribe ?? noSubscription, fonts?.getSnapshot ?? noState);
}
