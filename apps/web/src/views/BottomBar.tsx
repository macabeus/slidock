import type { Deck, DeckDiagnostic, Severity } from '@slidock/types';
import { Button, Icon, Pill, TabPanel, Tabs, cx, focusRing } from '@slidock/ui';
import { useState } from 'react';

const SEVERITY_ORDER: readonly Severity[] = ['error', 'decline', 'lossy', 'warning', 'info'];
const SEVERITY_TINT: Record<Severity, string> = {
  error: 'var(--danger)',
  decline: 'var(--warning)',
  lossy: 'var(--warning)',
  warning: 'var(--text-muted)',
  info: 'var(--text-muted)',
};
const SEVERITY_GLOSS: Record<Severity, string> = {
  error: 'a slide or the presentation failed',
  decline: 'a feature is shown differently or left out',
  lossy: 'approximated',
  warning: 'unusual input, handled',
  info: 'for information',
};

interface Group {
  code: string;
  severity: Severity;
  message: string;
  count: number;
  slides: number[];
}

function groupDiagnostics(deck: Deck, diagnostics: readonly DeckDiagnostic[]): Group[] {
  const index = new Map(deck.slides.map((slide, i) => [slide.id, i]));
  const groups = new Map<string, Group>();
  for (const diagnostic of diagnostics) {
    let group = groups.get(diagnostic.code);
    if (!group) {
      group = {
        code: diagnostic.code,
        severity: diagnostic.severity,
        message: diagnostic.message,
        count: 0,
        slides: [],
      };
      groups.set(diagnostic.code, group);
    }
    group.count += 1;
    const slide = diagnostic.container === null ? undefined : index.get(diagnostic.container);
    if (slide !== undefined && !group.slides.includes(slide)) {
      group.slides.push(slide);
    }
  }
  return [...groups.values()].sort(
    (a, b) => SEVERITY_ORDER.indexOf(a.severity) - SEVERITY_ORDER.indexOf(b.severity) || (a.code < b.code ? -1 : 1),
  );
}

function Diagnostics({
  deck,
  diagnostics,
  onGoTo,
}: {
  deck: Deck;
  diagnostics: readonly DeckDiagnostic[];
  onGoTo(slide: number): void;
}) {
  if (diagnostics.length === 0) {
    return <p className="text-sm text-muted">Nothing to report: the presentation converted without diagnostics.</p>;
  }
  const groups = groupDiagnostics(deck, diagnostics);
  return (
    <div className="space-y-3">
      <ul className="divide-y divide-line-subtle">
        {groups.map((group) => (
          <li key={group.code} className="flex flex-wrap items-start gap-x-3 gap-y-1 py-2 text-sm">
            <Pill tint={SEVERITY_TINT[group.severity]} size="xs" title={SEVERITY_GLOSS[group.severity]}>
              {group.severity}
            </Pill>
            <code className="font-mono text-xs font-semibold text-fg">{group.code}</code>
            <span className="font-mono text-xs tabular-nums text-muted">×{group.count}</span>
            <span className="min-w-0 flex-1 basis-64 text-fg-secondary">{group.message}</span>
            {group.slides.length > 0 && (
              <span className="flex flex-wrap gap-1">
                {group.slides.slice(0, 12).map((slide) => (
                  <button
                    key={slide}
                    type="button"
                    onClick={() => onGoTo(slide)}
                    className={cx(
                      'rounded bg-elevated/60 px-1.5 font-mono text-[11px] tabular-nums text-fg-secondary hover:text-brand-300',
                      focusRing,
                    )}
                    aria-label={`Go to slide ${deck.slides[slide]?.number ?? slide + 1}`}
                  >
                    {deck.slides[slide]?.number ?? `#${slide + 1}`}
                  </button>
                ))}
                {group.slides.length > 12 && <span className="text-xs text-muted">+{group.slides.length - 12}</span>}
              </span>
            )}
          </li>
        ))}
      </ul>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => void navigator.clipboard?.writeText(JSON.stringify(diagnostics, null, 2)).catch(() => {})}
      >
        <Icon name="clipboard" className="size-4" />
        Copy report
      </Button>
    </div>
  );
}

type Tab = 'notes' | 'diagnostics';

export interface BottomBarProps {
  deck: Deck;
  diagnostics: readonly DeckDiagnostic[];
  current: number;
  onGoTo(slide: number): void;
}

/** Speaker notes of the current slide and the conversion report, collapsed to one bar by default. */
export function BottomBar({ deck, diagnostics, current, onGoTo }: BottomBarProps) {
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<Tab>('notes');
  const notes = deck.slides[current]?.notes ?? [];
  return (
    <section aria-label="Notes and diagnostics" className="rounded-lg border border-line bg-surface/50">
      <div className="flex items-center gap-2 px-2">
        <Tabs
          variant="underline"
          label="Notes and diagnostics"
          idPrefix="bottom"
          panels={open ? 'selected' : 'none'}
          value={tab}
          onChange={(next) => {
            setTab(next);
            setOpen(true);
          }}
          className="flex-1 border-b-0"
          items={[
            { id: 'notes', label: 'Notes' },
            { id: 'diagnostics', label: 'Diagnostics', count: diagnostics.length },
          ]}
        />
        <button
          type="button"
          onClick={() => setOpen(!open)}
          aria-expanded={open}
          aria-controls={open ? `bottom-panel-${tab}` : undefined}
          className={cx('rounded-md p-1.5 text-muted hover:bg-elevated hover:text-fg', focusRing)}
          aria-label={open ? 'Collapse notes and diagnostics' : 'Expand notes and diagnostics'}
        >
          <Icon name={open ? 'chevron-down' : 'chevron-up'} className="size-4" />
        </button>
      </div>
      {open && (
        <TabPanel idPrefix="bottom" id={tab} className="h-40 overflow-y-auto border-t border-line px-4 py-3">
          {tab === 'notes' ? (
            notes.length > 0 ? (
              <div className="space-y-2 text-sm leading-relaxed text-fg-secondary">
                {notes.map((paragraph, index) => (
                  <p key={index} className="whitespace-pre-wrap">
                    {paragraph}
                  </p>
                ))}
              </div>
            ) : (
              <p className="text-sm text-muted">This slide has no speaker notes.</p>
            )
          ) : (
            <Diagnostics deck={deck} diagnostics={diagnostics} onGoTo={onGoTo} />
          )}
        </TabPanel>
      )}
    </section>
  );
}
