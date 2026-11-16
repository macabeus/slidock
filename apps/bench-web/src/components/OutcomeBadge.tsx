import { Pill } from '@slidock/ui';

import { outcomeLabel, outcomeVars } from '../model/outcomes';
import { PARITY_GLOSS } from '../model/parity';
import type { ParityStatus } from '../schema';

/** A row's class as a tinted pill; a class the UI does not know shows its raw name in neutral grey. */
export function OutcomeBadge({ outcome, size = 'sm' }: { outcome: string; size?: 'sm' | 'xs' }) {
  return (
    <Pill tint={outcomeVars(outcome).text} dot size={size}>
      {outcomeLabel(outcome)}
    </Pill>
  );
}

export function ParityBadge({ status, size = 'sm' }: { status: string; size?: 'sm' | 'xs' }) {
  const known = status in PARITY_GLOSS;
  return (
    <Pill
      tint={known ? `var(--parity-${status})` : 'var(--oc-unknown)'}
      dot
      size={size}
      title={known ? PARITY_GLOSS[status as ParityStatus] : undefined}
    >
      {status}
    </Pill>
  );
}
