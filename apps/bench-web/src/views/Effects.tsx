import { Panel, cx, focusRing } from '@slidock/ui';
import { useMemo, useState } from 'react';

import { CopyCommand, SegmentedControl, Select, Stat } from '../components/Controls';
import { OutcomeBadge } from '../components/OutcomeBadge';
import { OutcomeBar } from '../components/OutcomeBar';
import { Await, Empty } from '../components/States';
import { useBenchData, useResource } from '../data/resource';
import {
  type CoverageRow,
  type DeckFigures,
  MOTION_CLASSES,
  type MotionFilters,
  NO_MOTION_FILTERS,
  VERDICTS,
  VERDICT_LABELS,
  coverage,
  deckFigures,
  failedChecks,
  fallbackPareto,
  motionRenderer,
  motionRows,
  motionRun,
  nonNativeItems,
  paramsText,
  rowKind,
  slideOf,
  stageOf,
  stageSteps,
  timing,
  verdictTotals,
} from '../model/effects';
import { DASH, formatTime, short } from '../model/format';
import { outcomeLabel } from '../model/outcomes';
import type { Frontend, ImageKind, MotionVerdict, Results, RowRecord, RunEntry, TimelineFidelity } from '../schema';
import { OUTCOME_COLORS } from '../theme';
import type { ViewProps } from './context';

const th = 'px-3 py-2.5 font-semibold';
const td = 'px-3 py-2';
const num = 'px-3 py-2 text-right font-mono tabular-nums';

// Adjacent verdicts stay apart for colour-blind readers; every bar carries a legend and per-segment labels.
const VERDICT_COLORS: Record<MotionVerdict, string> = {
  native: OUTCOME_COLORS['motion-match'].fill,
  declared: OUTCOME_COLORS['motion-close'].fill,
  allowed: OUTCOME_COLORS.declined.fill,
  accepted: OUTCOME_COLORS.failed.fill,
  dropped: OUTCOME_COLORS['render-error'].fill,
  mismatched: OUTCOME_COLORS['motion-mismatch'].fill,
};

/** The page size of the rows list. */
const PAGE = 60;

function VerdictBar({ deck, label, size = 'sm' }: { deck: DeckFigures['report']; label: string; size?: 'md' | 'sm' }) {
  const declared = deck.items.filter((item) => item.verdict === 'declared').length;
  const allowed = deck.items.filter((item) => item.verdict === 'allowed').length;
  return (
    <OutcomeBar
      label={label}
      size={size}
      segments={[
        { key: 'native', count: deck.native, color: VERDICT_COLORS.native, label: VERDICT_LABELS.native },
        { key: 'declared', count: declared, color: VERDICT_COLORS.declared, label: VERDICT_LABELS.declared },
        { key: 'allowed', count: allowed, color: VERDICT_COLORS.allowed, label: VERDICT_LABELS.allowed },
        { key: 'accepted', count: deck.accepted, color: VERDICT_COLORS.accepted, label: VERDICT_LABELS.accepted },
        { key: 'dropped', count: deck.dropped, color: VERDICT_COLORS.dropped, label: VERDICT_LABELS.dropped },
        {
          key: 'mismatched',
          count: deck.mismatched,
          color: VERDICT_COLORS.mismatched,
          label: VERDICT_LABELS.mismatched,
        },
      ]}
    />
  );
}

function Legend() {
  return (
    <ul className="flex flex-wrap gap-x-4 gap-y-1 text-xs" aria-label="Verdicts">
      {VERDICTS.map((verdict) => (
        <li key={verdict} className="flex items-center gap-1.5">
          <span
            aria-hidden
            className="size-2.5 shrink-0 rounded-sm"
            style={{ backgroundColor: VERDICT_COLORS[verdict] }}
          />
          <span className="text-fg-secondary">{VERDICT_LABELS[verdict]}</span>
        </li>
      ))}
    </ul>
  );
}

function Verdicts({ results, decks, onDeck }: { results: Results; decks: DeckFigures[]; onDeck(deck: string): void }) {
  const totals = verdictTotals(decks);
  const lanes = Object.values(results.summary.lanes?.motion ?? {}).filter((s) => s !== undefined);
  const sum = (pick: (s: (typeof lanes)[number]) => number) => lanes.reduce((n, s) => n + pick(s), 0);
  const renderer = motionRenderer(results);
  const items = totals.native + totals.fallbackAllowed + totals.accepted + totals.dropped + totals.mismatched;
  return (
    <Panel
      title="Transitions and builds"
      subtitle="Every transition and step of every deck, played by the deck's own player with its animations paused and seeked: native, its declared or an accepted fallback, an accepted defect (a failed check a review tracks), dropped by the IR, or mismatched. Probe decks are held to their expectations, params included."
      headerRight={
        <span
          className="whitespace-nowrap rounded-full border border-line px-2.5 py-1 text-xs font-medium text-fg-secondary"
          data-renderer={renderer}
        >
          {renderer === 'native' ? 'Native renderer' : 'Renderer container'}
        </span>
      }
    >
      <div className="grid grid-cols-2 gap-3 md:grid-cols-4 xl:grid-cols-7">
        <Stat label="Items" value={items.toLocaleString('en-US')} hint={`${decks.length} decks`} />
        <Stat label="Native" value={totals.native.toLocaleString('en-US')} accent />
        <Stat
          label="Fallback allowed"
          value={totals.fallbackAllowed.toLocaleString('en-US')}
          hint="declared or accepted fallbacks"
        />
        <Stat
          label="Accepted defects"
          value={totals.accepted.toLocaleString('en-US')}
          hint="failed checks a review tracks"
        />
        <Stat
          label="Mismatched"
          value={totals.mismatched.toLocaleString('en-US')}
          hint={totals.mismatched === 0 ? 'none' : 'the lane fails'}
        />
        <Stat
          label="End states"
          value={`${sum((s) => s.endState.identical + (s.endState.antialiased ?? 0))}/${sum((s) => s.endState.rows)}`}
          hint="equal to the still render"
        />
        <Stat
          label="Timelines exact"
          value={`${sum((s) => s.timelines.exact)}/${sum((s) => s.timelines.slides)}`}
          hint="slides as the deck says"
        />
      </div>
      <div className="mt-5 relative overflow-x-auto rounded-xl border border-line">
        <table className="w-full min-w-[52rem] text-sm">
          <thead className="bg-surface text-xs uppercase tracking-wide text-muted">
            <tr>
              <th scope="col" className={cx(th, 'text-left')}>
                Deck
              </th>
              <th scope="col" className={cx(th, 'text-right')}>
                Slides
              </th>
              <th scope="col" className={cx(th, 'text-right')}>
                Transitions
              </th>
              <th scope="col" className={cx(th, 'text-right')}>
                Steps
              </th>
              <th scope="col" className={cx(th, 'text-right')}>
                Native
              </th>
              <th scope="col" className={cx(th, 'text-right')}>
                Fallback
              </th>
              <th scope="col" className={cx(th, 'text-right')}>
                Accepted
              </th>
              <th scope="col" className={cx(th, 'text-right')}>
                Mismatched
              </th>
              <th scope="col" className={cx(th, 'w-[14rem] text-left')}>
                Verdicts
              </th>
            </tr>
          </thead>
          <tbody>
            {decks.map(({ pair, report }) => (
              <tr key={pair} className="border-t border-line-subtle">
                <td className={td}>
                  <button
                    type="button"
                    onClick={() => onDeck(pair)}
                    className={cx('font-mono text-[13px] hover:text-brand-300', focusRing)}
                    title="List its rows below"
                  >
                    {pair}
                  </button>
                  {report.probe && <span className="ml-2 text-xs text-muted">probe</span>}
                </td>
                <td className={num}>{report.slides}</td>
                <td className={num}>{report.transitions}</td>
                <td className={num}>{report.steps}</td>
                <td className={num}>{report.native}</td>
                <td className={num}>{report.fallbackAllowed}</td>
                <td className={num}>{report.accepted}</td>
                <td className={cx(num, report.mismatched > 0 && 'text-danger')}>{report.mismatched}</td>
                <td className={td}>
                  <VerdictBar deck={report} label={`${pair} items by verdict`} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="mt-3">
        <Legend />
      </div>
    </Panel>
  );
}

const FRONTENDS: Frontend[] = ['pptx', 'odp', 'keynote', 'ir'];

function Coverage({
  rows,
  onEffect,
  onExplore,
}: {
  rows: CoverageRow[];
  onEffect(effect: string): void;
  onExplore(effect: string, frontend: Frontend): void;
}) {
  const shown = FRONTENDS.filter((frontend) => rows.some((row) => row.byFrontend[frontend] !== undefined));
  return (
    <Panel
      title="Coverage"
      subtitle="Each effect the decks name, by frontend: how often it played natively against its fallback. — is a step or transition the deck names no effect for. A cell opens its rows in the Explorer; an effect lists them below."
    >
      <div className="relative max-h-[28rem] overflow-auto rounded-xl border border-line">
        <table className="w-full min-w-[40rem] text-sm">
          <thead className="sticky top-0 bg-surface text-xs uppercase tracking-wide text-muted">
            <tr>
              <th scope="col" className={cx(th, 'text-left')}>
                Effect
              </th>
              <th scope="col" className={cx(th, 'text-left')}>
                Kind
              </th>
              {shown.map((frontend) => (
                <th key={frontend} scope="col" className={cx(th, 'text-left')}>
                  {frontend}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={`${row.kind}:${row.effect}`} className="border-t border-line-subtle">
                <td className={td}>
                  <button
                    type="button"
                    onClick={() => onEffect(row.effect)}
                    className={cx('font-mono text-[13px] hover:text-brand-300', focusRing)}
                    title="List its rows below"
                  >
                    {row.effect}
                  </button>
                </td>
                <td className={cx(td, 'text-xs text-muted')}>{row.kind}</td>
                {shown.map((frontend) => {
                  const cell = row.byFrontend[frontend];
                  return (
                    <td key={frontend} className={td}>
                      {cell ? (
                        <button
                          type="button"
                          onClick={() => onExplore(row.effect, frontend)}
                          title={`Open the ${row.effect} rows of ${frontend} in the Explorer`}
                          className={cx('flex items-center gap-2 rounded-md hover:bg-elevated/60', focusRing)}
                        >
                          <span className="w-16 font-mono text-xs tabular-nums text-fg-secondary">
                            {cell.native}/{cell.native + cell.fallback + cell.unsupported}
                          </span>
                          <OutcomeBar
                            size="sm"
                            className="w-24"
                            label={`${row.effect} on ${frontend}: native against fallback`}
                            segments={[
                              { key: 'native', count: cell.native, color: VERDICT_COLORS.native, label: 'native' },
                              {
                                key: 'fallback',
                                count: cell.fallback,
                                color: VERDICT_COLORS.declared,
                                label: 'fallback',
                              },
                              {
                                key: 'unsupported',
                                count: cell.unsupported,
                                color: VERDICT_COLORS.mismatched,
                                label: 'nothing played',
                              },
                            ]}
                          />
                        </button>
                      ) : (
                        <span className="text-muted">{DASH}</span>
                      )}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Panel>
  );
}

function Pareto({ results }: { results: Results }) {
  const entries = fallbackPareto(results);
  return (
    <Panel
      title="Fallback Pareto"
      subtitle="Native effects that played their canonical fallback, most frequent first: the effects to implement next."
    >
      {entries.length === 0 ? (
        <Empty>Every effect the decks name played natively.</Empty>
      ) : (
        <div className="relative max-h-[28rem] overflow-auto rounded-xl border border-line">
          <table className="w-full text-sm">
            <thead className="sticky top-0 bg-surface text-xs uppercase tracking-wide text-muted">
              <tr>
                <th scope="col" className={cx(th, 'text-left')}>
                  Native id
                </th>
                <th scope="col" className={cx(th, 'text-left')}>
                  Kind
                </th>
                <th scope="col" className={cx(th, 'text-left')}>
                  Plays
                </th>
                <th scope="col" className={cx(th, 'text-right')}>
                  Times
                </th>
                <th scope="col" className={cx(th, 'text-left')}>
                  Frontends
                </th>
              </tr>
            </thead>
            <tbody>
              {entries.map((entry) => (
                <tr key={`${entry.kind}:${entry.native}`} className="border-t border-line-subtle">
                  <td className={cx(td, 'font-mono text-[13px]')}>{entry.native}</td>
                  <td className={cx(td, 'text-xs text-muted')}>{entry.kind}</td>
                  <td className={cx(td, 'font-mono text-xs')}>{entry.fallback}</td>
                  <td className={num}>{entry.count}</td>
                  <td className={cx(td, 'text-xs text-fg-secondary')}>{entry.frontends.join(', ')}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Panel>
  );
}

/** A frame of a row, or the reason it is not shown. */
function Frame({ run, id, kind, label }: { run: string; id: string; kind: ImageKind; label: string }) {
  const { source } = useBenchData();
  const url = source.imageUrl(run, id, kind);
  return (
    <figure className="flex flex-col items-center gap-1">
      {url ? (
        <img
          src={url}
          alt={`${id}: ${label}`}
          loading="lazy"
          className="h-[68px] w-[120px] rounded border border-line bg-elevated object-contain"
        />
      ) : (
        <div className="flex h-[68px] w-[120px] items-center justify-center rounded border border-dashed border-line text-[10px] text-muted">
          not in this report
        </div>
      )}
      <figcaption className="text-[10px] uppercase tracking-wide text-muted">{label}</figcaption>
    </figure>
  );
}

function Timeline({
  fidelity,
  stage,
  kind,
}: {
  fidelity: TimelineFidelity;
  stage: number | null;
  kind: 'stage' | 'transition';
}) {
  const ofTransition: string[] = ['transition', 'moves', 'direction'];
  const mismatches = (fidelity.mismatches ?? []).filter((m) =>
    kind === 'transition'
      ? m.step === null && ofTransition.includes(m.field)
      : m.step === null
        ? !ofTransition.includes(m.field) && m.field !== 'ir'
        : stageSteps(fidelity, stage).some((p) => p.index === m.step),
  );
  if (kind === 'transition') {
    const transition = fidelity.transition;
    return (
      <div className="space-y-2 text-xs">
        {transition ? (
          <p className="text-fg-secondary">
            <span className="font-mono">{transition.effect.id}</span> plays{' '}
            <span className="font-mono">{transition.name ?? `fallback ${transition.effect.fallback}`}</span>
            {transition.params !== undefined && (
              <span className="font-mono text-muted"> {`{${paramsText(transition.params) ?? ''}}`}</span>
            )}{' '}
            ({transition.effect.played}); {transition.expectedMs} ms expected, {transition.actualMs ?? DASH} ms played;
            first frame {transition.from?.status ?? DASH}, last frame {transition.to?.status ?? DASH}.
          </p>
        ) : (
          <p className="text-muted">No transition.</p>
        )}
        <Mismatches mismatches={mismatches} />
      </div>
    );
  }
  const steps = stageSteps(fidelity, stage);
  const failing = new Set((fidelity.mismatches ?? []).map((m) => m.step));
  return (
    <div className="space-y-2">
      {steps.length === 0 ? (
        <p className="text-xs text-muted">This stage plays no step.</p>
      ) : (
        <div className="overflow-x-auto rounded-lg border border-line">
          <table className="w-full min-w-[48rem] text-xs">
            <thead className="bg-surface uppercase tracking-wide text-muted">
              <tr>
                <th scope="col" className={cx(th, 'text-left')}>
                  Step
                </th>
                <th scope="col" className={cx(th, 'text-left')}>
                  Trigger
                </th>
                <th scope="col" className={cx(th, 'text-left')}>
                  Target
                </th>
                <th scope="col" className={cx(th, 'text-left')}>
                  Effect
                </th>
                <th scope="col" className={cx(th, 'text-left')}>
                  Params
                </th>
                <th scope="col" className={cx(th, 'text-left')}>
                  Played
                </th>
                <th scope="col" className={cx(th, 'text-right')}>
                  Expected (ms)
                </th>
                <th scope="col" className={cx(th, 'text-right')}>
                  Played (ms)
                </th>
              </tr>
            </thead>
            <tbody>
              {steps.map((play) => (
                <tr
                  key={play.index}
                  className={cx('border-t border-line-subtle', failing.has(play.index) && 'bg-warning/10')}
                  data-exact={!failing.has(play.index)}
                >
                  <td className={cx(td, 'font-mono')}>{play.index}</td>
                  <td className={td}>
                    {play.trigger} · {play.kind}
                  </td>
                  <td className={cx(td, 'font-mono')}>{play.target}</td>
                  <td className={cx(td, 'font-mono')}>
                    {play.effect ?? DASH}
                    <span className="block text-[10px] text-muted">{play.native}</span>
                  </td>
                  <td className={cx(td, 'font-mono')}>
                    {paramsText(play.params) ?? DASH}
                    {play.delivery !== undefined && (
                      <span className="block text-[10px] text-muted">{play.delivery}</span>
                    )}
                  </td>
                  <td className={td}>{play.played === 'fallback' ? `fallback ${play.fallback}` : play.played}</td>
                  <td className={num}>{timing(play.expected)}</td>
                  <td className={num}>{timing(play.actual)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <Mismatches mismatches={mismatches} />
    </div>
  );
}

function Mismatches({ mismatches }: { mismatches: NonNullable<TimelineFidelity['mismatches']> }) {
  if (mismatches.length === 0) {
    return null;
  }
  return (
    <ul className="space-y-1 text-xs text-warning" aria-label="Timeline differences">
      {mismatches.map((m, i) => (
        <li key={i}>
          {m.step === null ? 'slide' : `step ${m.step}`} · {m.field}: expected {m.expected || DASH}, played {m.actual}
        </li>
      ))}
    </ul>
  );
}

function MotionRow({
  id,
  row,
  run,
  fidelity,
  open,
  onToggle,
  onOpen,
}: {
  id: string;
  row: RowRecord;
  run: RunEntry;
  fidelity: TimelineFidelity | undefined;
  open: boolean;
  onToggle(): void;
  onOpen(): void;
}) {
  const kind = rowKind(id) ?? 'stage';
  const checks = failedChecks(row);
  const moving = row.motion?.moving;
  return (
    <li className="rounded-xl border border-line-subtle bg-surface/40 p-3" data-class={row.outcome}>
      <div className="flex flex-wrap items-center gap-3">
        <div className="min-w-[16rem] flex-1 space-y-1">
          <button
            type="button"
            onClick={onOpen}
            className={cx('font-mono text-[13px] hover:text-brand-300', focusRing)}
            title="Open the row"
          >
            {id}
          </button>
          <div className="flex flex-wrap items-center gap-2 text-xs">
            <OutcomeBadge outcome={row.outcome} size="xs" />
            {kind === 'transition' && row.motion?.effect && (
              <span className="font-mono text-fg-secondary">{row.motion.effect.id}</span>
            )}
            {kind === 'transition' && moving === false && <span className="text-muted">no motion at this frame</span>}
            {checks.length > 0 && (
              <span className="text-warning">failed: {[...new Set(checks.map((c) => c.check))].join(', ')}</span>
            )}
            {row.motion?.accepted !== undefined && (
              <span className="text-muted" title={row.motion.accepted}>
                accepted by review
              </span>
            )}
            {row.motion?.ssim !== undefined && (
              <span className="font-mono text-muted">
                SSIM {row.motion.ssim} · ΔE {row.motion.deltaE ?? DASH}
              </span>
            )}
          </div>
        </div>
        <div className="flex gap-2" aria-label={`${id} frames`}>
          <Frame run={run.key} id={id} kind="start" label={kind === 'transition' ? '0%' : 'start'} />
          <Frame
            run={run.key}
            id={id}
            kind="mid"
            label={kind === 'transition' ? `${(row.transitionPm ?? 0) / 10}%` : 'middle'}
          />
          <Frame run={run.key} id={id} kind="end" label={kind === 'transition' ? '100%' : 'end'} />
          {kind === 'stage' && <Frame run={run.key} id={id} kind="still" label="still" />}
        </div>
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={open}
          className={cx('rounded-md px-2 py-1 text-xs text-brand-400 hover:text-brand-300', focusRing)}
        >
          {open ? 'Hide timeline' : 'Timeline'}
        </button>
      </div>
      {open && (
        <div className="mt-3">
          {checks.length > 0 && (
            <ul className="mb-2 space-y-1 text-xs text-warning" aria-label="Failed checks">
              {checks.map((c, i) => (
                <li key={i}>
                  <span className="font-mono">{c.check}</span>: {c.detail}
                </li>
              ))}
            </ul>
          )}
          {fidelity ? (
            <Timeline fidelity={fidelity} stage={stageOf(id)} kind={kind} />
          ) : (
            <p className="text-xs text-muted">The run holds no timeline for this slide.</p>
          )}
        </div>
      )}
    </li>
  );
}

function Rows({
  results,
  run,
  filters,
  setFilters,
  set,
  effects,
}: {
  results: Results;
  run: RunEntry;
  filters: MotionFilters;
  setFilters(change: Partial<MotionFilters>): void;
  set: ViewProps['set'];
  effects: string[];
}) {
  const rows = useMemo(() => motionRows(results, filters), [results, filters]);
  const [limit, setLimit] = useState(PAGE);
  const [open, setOpen] = useState<string | null>(null);
  const decks = Object.keys(results.motionPairs ?? {}).sort();
  const toggleClass = (name: string) =>
    setFilters({
      classes: filters.classes.includes(name) ? filters.classes.filter((c) => c !== name) : [...filters.classes, name],
    });
  return (
    <Panel
      title="Rows by motion class"
      subtitle="Stage rows (each click group: its first, middle and last frame against the still render) and transition rows (frames at 25, 50 and 75% between their ends), worst first."
    >
      <div className="mb-4 flex flex-wrap items-end gap-3">
        <div className="flex flex-col gap-1">
          <span className="text-xs text-muted">Class</span>
          <div className="flex flex-wrap gap-1" role="group" aria-label="Motion classes">
            {MOTION_CLASSES.map((name) => (
              <button
                key={name}
                type="button"
                aria-pressed={filters.classes.includes(name)}
                onClick={() => toggleClass(name)}
                className={cx(
                  'rounded-md border px-2 py-1 text-xs transition-colors',
                  filters.classes.includes(name)
                    ? 'border-brand-400 text-fg'
                    : 'border-line text-muted hover:text-fg-secondary',
                  focusRing,
                )}
              >
                {outcomeLabel(name)}
              </button>
            ))}
          </div>
        </div>
        <div className="flex flex-col gap-1">
          <span className="text-xs text-muted">Rows</span>
          <SegmentedControl<MotionFilters['kind']>
            label="Row kind"
            value={filters.kind}
            onChange={(kind) => setFilters({ kind })}
            options={[
              { value: 'all', label: 'All' },
              { value: 'stage', label: 'Stages' },
              { value: 'transition', label: 'Transitions' },
            ]}
          />
        </div>
        <Select
          label="Deck"
          value={filters.deck ?? ''}
          onChange={(deck) => setFilters({ deck: deck === '' ? null : deck })}
        >
          <option value="">Every deck</option>
          {decks.map((deck) => (
            <option key={deck} value={deck}>
              {deck}
            </option>
          ))}
        </Select>
        <Select
          label="Effect"
          value={filters.effect ?? ''}
          onChange={(effect) => setFilters({ effect: effect === '' ? null : effect })}
        >
          <option value="">Every effect</option>
          {effects.map((effect) => (
            <option key={effect} value={effect}>
              {effect}
            </option>
          ))}
        </Select>
        <button
          type="button"
          onClick={() => setFilters(NO_MOTION_FILTERS)}
          className={cx('pb-1 text-xs text-brand-400 hover:text-brand-300', focusRing)}
        >
          Reset
        </button>
      </div>
      <p className="mb-3 text-xs text-muted" aria-live="polite">
        {rows.length.toLocaleString('en-US')} rows
      </p>
      {rows.length === 0 ? (
        <Empty>No row matches these filters.</Empty>
      ) : (
        <>
          <ul className="space-y-2">
            {rows.slice(0, limit).map(([id, row]) => (
              <MotionRow
                key={id}
                id={id}
                row={row}
                run={run}
                fidelity={results.timelines?.[slideOf(id)]}
                open={open === id}
                onToggle={() => setOpen(open === id ? null : id)}
                onOpen={() => set({ run: run.key, row: id })}
              />
            ))}
          </ul>
          {rows.length > limit && (
            <button
              type="button"
              onClick={() => setLimit(limit + PAGE)}
              className={cx('mt-3 text-sm text-brand-400 hover:text-brand-300', focusRing)}
            >
              Show {Math.min(PAGE, rows.length - limit)} more of {rows.length - limit}
            </button>
          )}
        </>
      )}
    </Panel>
  );
}

function Items({ results }: { results: Results }) {
  const [verdict, setVerdict] = useState<MotionVerdict | null>('mismatched');
  const items = nonNativeItems(results, verdict);
  return (
    <Panel
      title="Items that did not play natively"
      subtitle="Each transition or step with another verdict than native, and why: a probe's expectation, the fallback allowlist's reason, or the check it failed."
      headerRight={
        <SegmentedControl<MotionVerdict | 'all'>
          label="Verdict"
          value={verdict ?? 'all'}
          onChange={(value) => setVerdict(value === 'all' ? null : value)}
          options={[
            { value: 'mismatched', label: 'Mismatched' },
            { value: 'allowed', label: 'Accepted fallback' },
            { value: 'accepted', label: 'Accepted defect' },
            { value: 'declared', label: 'Declared' },
            { value: 'dropped', label: 'Dropped' },
            { value: 'all', label: 'All' },
          ]}
        />
      }
    >
      {items.length === 0 ? (
        <Empty>{verdict === 'mismatched' ? 'Nothing is mismatched.' : 'No item has this verdict.'}</Empty>
      ) : (
        <div className="relative max-h-[28rem] overflow-auto rounded-xl border border-line">
          <table className="w-full min-w-[56rem] text-sm">
            <thead className="sticky top-0 bg-surface text-xs uppercase tracking-wide text-muted">
              <tr>
                <th scope="col" className={cx(th, 'text-left')}>
                  Deck
                </th>
                <th scope="col" className={cx(th, 'text-left')}>
                  Slide
                </th>
                <th scope="col" className={cx(th, 'text-left')}>
                  Item
                </th>
                <th scope="col" className={cx(th, 'text-left')}>
                  Expected
                </th>
                <th scope="col" className={cx(th, 'text-left')}>
                  Played
                </th>
                <th scope="col" className={cx(th, 'text-left')}>
                  Verdict
                </th>
                <th scope="col" className={cx(th, 'text-left')}>
                  Why
                </th>
              </tr>
            </thead>
            <tbody>
              {items.slice(0, 400).map((item) => (
                <tr key={`${item.pair} ${item.slide} ${item.item}`} className="border-t border-line-subtle">
                  <td className={cx(td, 'font-mono text-xs')}>{item.pair}</td>
                  <td className={cx(td, 'font-mono text-xs')}>
                    {item.slide}
                    {item.key && <span className="block text-[10px] text-muted">{item.key}</span>}
                  </td>
                  <td className={cx(td, 'text-xs')}>
                    {item.item}
                    <span className="block font-mono text-[10px] text-muted">{item.native}</span>
                  </td>
                  <td className={cx(td, 'font-mono text-xs')}>{item.expected ?? DASH}</td>
                  <td className={cx(td, 'font-mono text-xs')}>{item.played}</td>
                  <td className={td}>
                    <span className="flex items-center gap-1.5 text-xs">
                      <span
                        aria-hidden
                        className="size-2 rounded-full"
                        style={{ backgroundColor: VERDICT_COLORS[item.verdict] }}
                      />
                      {VERDICT_LABELS[item.verdict]}
                    </span>
                  </td>
                  <td className={cx(td, 'text-xs text-fg-secondary')}>{item.why ?? DASH}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Panel>
  );
}

function EffectsRun({ results, run, set }: { results: Results; run: RunEntry; set: ViewProps['set'] }) {
  const decks = deckFigures(results);
  const rows = coverage(results);
  const [filters, setFilterState] = useState<MotionFilters>(NO_MOTION_FILTERS);
  const setFilters = (change: Partial<MotionFilters>) => setFilterState((current) => ({ ...current, ...change }));
  const effects = [...new Set(rows.map((row) => row.effect))].sort();
  return (
    <div className="space-y-6">
      <Verdicts results={results} decks={decks} onDeck={(deck) => setFilters({ deck })} />
      <div className="grid grid-cols-1 gap-6 2xl:grid-cols-2">
        <Coverage
          rows={rows}
          onEffect={(effect) => setFilters({ effect })}
          onExplore={(fx, frontend) => set({ tab: 'explorer', run: run.key, fx, fe: [frontend], row: null })}
        />
        <Pareto results={results} />
      </div>
      <Rows results={results} run={run} filters={filters} setFilters={setFilters} set={set} effects={effects} />
      <Items results={results} />
    </div>
  );
}

/** The motion lane: the selected run when it is a motion run, else the newest one. */
export function Effects({ index, run, results, set }: ViewProps) {
  const shown = motionRun(index, run);
  const own = shown?.key === run.key;
  const loaded = useResource<Results | null>(shown && !own ? `results:${shown.key}` : null, (source) =>
    shown && !own ? source.results(shown.key) : Promise.resolve(null),
  );
  if (!shown) {
    return (
      <Panel
        title="No motion run yet"
        subtitle="The motion lane plays every deck with its own player and checks each transition and build."
      >
        <CopyCommand command="slidock-bench run --lane motion" label="motion lane command" />
      </Panel>
    );
  }
  const header = !own && (
    <p className="mb-4 text-sm text-muted">
      The motion lane's newest run, {formatTime(shown.recordedAt)} (run{' '}
      <span className="font-mono">{short(shown.key, 8)}</span>).{' '}
      <button
        type="button"
        onClick={() => set({ run: shown.key, row: null })}
        className={cx('text-brand-400 hover:text-brand-300', focusRing)}
      >
        Select it
      </button>
    </p>
  );
  if (own) {
    return results ? (
      <EffectsRun results={results} run={shown} set={set} />
    ) : (
      <Empty>The run's rows were collected.</Empty>
    );
  }
  return (
    <>
      {header}
      <Await resource={loaded} what="the motion run's results">
        {(value) =>
          value ? <EffectsRun results={value} run={shown} set={set} /> : <Empty>The run's rows were collected.</Empty>
        }
      </Await>
    </>
  );
}
