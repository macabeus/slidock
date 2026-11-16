import type { Phase } from '@slidock/types';
import { Button, Icon, Panel, ProgressBar, cx } from '@slidock/ui';
import { Fragment } from 'react';

import { COMPILE_PHASES } from '../compile/session';

const PHASE_LABELS: Record<(typeof COMPILE_PHASES)[number], string> = {
  open: 'Reading and lowering the slides',
  finish: 'Fonts, text metrics and layout',
  print: 'Writing the slides',
};

export interface ConvertingProps {
  name: string;
  /** `null` while the input is being read (a Keynote package folder is packed first). */
  phase: Phase | null;
  done: readonly Phase[];
  warnings: number;
  errors: number;
  preparing: boolean;
  /** The HEIC photos the browser decodes after reading the slides; `null` for none. */
  photos?: { total: number; done: number; active: boolean } | null;
  onCancel(): void;
}

type StepState = 'done' | 'active' | 'pending';

function Step({ state, children }: { state: StepState; children: React.ReactNode }) {
  return (
    <li
      aria-current={state === 'active' ? 'step' : undefined}
      className={cx(
        'flex items-center gap-2',
        state === 'pending' ? 'text-muted' : state === 'done' ? 'text-fg-secondary' : 'text-fg',
      )}
    >
      {state === 'done' ? (
        <Icon name="check" className="size-4 text-success" label="done" />
      ) : state === 'active' ? (
        <span className="size-4 animate-spin rounded-full border-2 border-elevated border-t-brand-400 motion-reduce:animate-none" />
      ) : (
        <span aria-hidden className="size-4 rounded-full border-2 border-elevated" />
      )}
      {children}
    </li>
  );
}

export function Converting({ name, phase, done, warnings, errors, preparing, photos, onCancel }: ConvertingProps) {
  return (
    <div className="flex flex-1 items-center justify-center py-8">
      <Panel variant="card" className="w-full max-w-lg" aria-label="Conversion">
        <p className="text-[11px] font-semibold uppercase tracking-wider text-muted">Converting</p>
        <p className="mt-1 truncate text-lg font-bold text-fg" title={name}>
          {name}
        </p>
        <ProgressBar label={`Converting ${name}`} className="mt-4" />
        <ol className="mt-4 space-y-2 text-sm">
          {preparing && (
            <li className="flex items-center gap-2 text-fg" aria-current="step">
              <span className="size-4 animate-spin rounded-full border-2 border-elevated border-t-brand-400 motion-reduce:animate-none" />
              Reading the input
            </li>
          )}
          {COMPILE_PHASES.map((step) => {
            // The slides are read once the photos are being decoded.
            const read = done.includes(step) || (step === 'open' && photos !== null && photos !== undefined);
            const state: StepState = read ? 'done' : phase === step ? 'active' : 'pending';
            return (
              <Fragment key={step}>
                <Step state={state}>{PHASE_LABELS[step]}</Step>
                {step === 'open' && photos && (
                  <Step state={photos.active ? 'active' : 'done'}>
                    Decoding {photos.total} HEIC {photos.total === 1 ? 'photo' : 'photos'}
                    {photos.active && photos.done > 0 && (
                      <span className="font-mono text-xs tabular-nums text-muted">
                        {` (${photos.done} of ${photos.total})`}
                      </span>
                    )}
                  </Step>
                )}
              </Fragment>
            );
          })}
        </ol>
        <div className="mt-5 flex items-center justify-between gap-4">
          <p className="font-mono text-xs tabular-nums text-muted" aria-live="polite">
            {errors} {errors === 1 ? 'error' : 'errors'} · {warnings} {warnings === 1 ? 'warning' : 'warnings'}
          </p>
          <Button variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
        </div>
      </Panel>
    </div>
  );
}
