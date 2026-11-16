import { Button, Pill, cx, focusRing } from '@slidock/ui';
import { useMemo } from 'react';

import { ChipGroup, CopyCommand, SegmentedControl, Select } from '../components/Controls';
import { OutcomeBadge } from '../components/OutcomeBadge';
import { Empty } from '../components/States';
import { type Movement, movement } from '../model/compare';
import { DASH, formatDelta, formatNumber } from '../model/format';
import { ALL_OUTCOMES, outcomeLabel, outcomeVars } from '../model/outcomes';
import { FRONTENDS, frontendLabel } from '../model/overview';
import {
  type Row,
  facetCounts,
  filterRows,
  flagged,
  listCommand,
  parseRowId,
  rowsOf,
  runResultsPath,
  sortRows,
  stepLabel,
} from '../model/rows';
import type { Results, RowRecord } from '../schema';
import { NO_FILTERS, type SortKey, type StepMode } from '../state/url';
import type { ViewProps } from './context';

const MOVEMENT_GLYPH: Record<Movement, string> = { same: '', improved: '▲', regressed: '▼', undeclined: '◆' };

function RowName({ id }: { id: string }) {
  const parts = parseRowId(id);
  if (!parts) {
    return <span className="font-mono text-xs">{id}</span>;
  }
  const step = stepLabel(parts.step);
  return (
    <span className="flex flex-col">
      <span className="font-mono text-[13px] text-fg">
        {parts.slide}
        {step && <span className="text-muted"> · {step}</span>}
      </span>
      <span className="text-xs text-muted">
        {parts.presentation} · {frontendLabel(parts.frontend)}
      </span>
    </span>
  );
}

function Flags({ record }: { record: RowRecord }) {
  return (
    <span className="flex flex-wrap gap-1">
      {record.declined && (
        <Pill size="xs" mono tint="var(--oc-declined)" title={record.declined.feature}>
          {record.declined.code}
        </Pill>
      )}
      {record.reason && (
        <Pill size="xs" mono title={record.code}>
          {record.reason}
        </Pill>
      )}
      {record.knownLoss?.map((kind) => (
        <Pill key={kind} size="xs" title={`known ${kind} loss`}>
          known loss · {kind}
        </Pill>
      ))}
      {record.reference && <Pill size="xs">reference</Pill>}
    </span>
  );
}

interface Column {
  key: SortKey | null;
  label: string;
  title?: string | undefined;
  numeric?: boolean;
}

function SortHeader({
  column,
  sort,
  dir,
  onSort,
}: {
  column: Column;
  sort: SortKey;
  dir: 'asc' | 'desc';
  onSort(key: SortKey): void;
}) {
  const active = column.key !== null && column.key === sort;
  return (
    <th
      scope="col"
      aria-sort={active ? (dir === 'asc' ? 'ascending' : 'descending') : undefined}
      className={cx(
        'whitespace-nowrap px-3 py-2.5 text-xs font-semibold uppercase tracking-wide text-muted',
        column.numeric ? 'text-right' : 'text-left',
      )}
    >
      {column.key === null ? (
        column.label
      ) : (
        <button
          type="button"
          title={column.title}
          onClick={() => column.key !== null && onSort(column.key)}
          className={cx('inline-flex items-center gap-1 uppercase hover:text-fg', active && 'text-fg', focusRing)}
        >
          {column.label}
          <span aria-hidden className="w-2">
            {active ? (dir === 'asc' ? '▲' : '▼') : ''}
          </span>
        </button>
      )}
    </th>
  );
}

function scoreCell(value: number | null | undefined) {
  return <td className="px-3 py-2 text-right font-mono tabular-nums text-fg-secondary">{formatNumber(value)}</td>;
}

/** The explorer's rows in display order: the order the drawer's previous and next follow. */
export function explorerOrder(results: Results, state: ViewProps['state']): Row[] {
  return sortRows(filterRows(rowsOf(results), state), state.sort, state.dir);
}

/** `baseResults`: the results of the fragment's `base` run, for the delta column; null without one. */
export function Explorer({
  state,
  set,
  results,
  run,
  base,
  thresholds,
  baseResults,
}: ViewProps & { baseResults: Results | null }) {
  return results ? (
    <ExplorerTable
      state={state}
      set={set}
      results={results}
      run={run}
      base={base}
      thresholds={thresholds}
      baseResults={baseResults}
    />
  ) : (
    <Empty>The bench no longer holds this run's results (garbage-collected); its summary is on the overview.</Empty>
  );
}

function ExplorerTable({
  state,
  set,
  results,
  run,
  base,
  thresholds,
  baseResults,
}: Pick<ViewProps, 'state' | 'set' | 'run' | 'base' | 'thresholds'> & {
  results: Results;
  baseResults: Results | null;
}) {
  const all = useMemo(() => rowsOf(results), [results]);
  const rows = useMemo(() => sortRows(filterRows(all, state), state.sort, state.dir), [all, state]);
  const counts = useMemo(() => facetCounts(all, state), [all, state]);
  const shown = state.ref ? all.length : all.filter((row) => !flagged(row.record)).length;
  const hasSteps = all.some((row) => row.record.stage !== undefined || row.record.transitionPm !== undefined);
  const decks = [...new Set(all.map((row) => row.record.presentation))].sort();
  const effects = [
    ...new Set([...all.flatMap((row) => row.effects ?? []), ...(state.fx === null ? [] : [state.fx])]),
  ].sort();
  const codes = [...new Set([...counts.code.keys(), ...state.code])].sort();
  const features = [...new Set([...counts.feat.keys(), ...state.feat])].sort();
  const outcomes = ALL_OUTCOMES.filter((outcome) => counts.oc.has(outcome) || state.oc.includes(outcome));
  const extraOutcomes = state.oc.filter((outcome) => !(ALL_OUTCOMES as readonly string[]).includes(outcome));
  const frontends = [...FRONTENDS.filter((fe) => counts.fe.has(fe) || state.fe.includes(fe))];
  const command = listCommand(state, run.baseline ? null : runResultsPath(run.key));
  const m = thresholds?.match;
  const columns: Column[] = [
    { key: 'id', label: 'Row' },
    { key: 'outcome', label: 'Outcome' },
    { key: 'ssim', label: 'SSIM', title: m ? `SSIM mean; match ≥ ${m.ssimMean}` : 'SSIM mean', numeric: true },
    { key: 'p1', label: 'SSIM p1', title: m ? `match ≥ ${m.ssimP1}` : undefined, numeric: true },
    { key: 'de', label: 'ΔE', title: m ? `mean ΔE00; match ≤ ${m.colorMean}` : 'mean ΔE00', numeric: true },
    { key: 'recall', label: 'Recall', title: m ? `text recall; match ≥ ${m.textRecall}` : undefined, numeric: true },
    { key: 'p95', label: 'p95 ‰', title: m ? `words placed, p95; match ≤ ${m.p95Pm}‰` : undefined, numeric: true },
    { key: null, label: 'Flags' },
    ...(baseResults ? [{ key: null, label: 'Δ base', numeric: true }] : []),
  ];
  const onSort = (key: SortKey) =>
    set(
      key === state.sort
        ? { dir: state.dir === 'asc' ? 'desc' : 'asc' }
        : { sort: key, dir: key === 'id' ? 'asc' : 'desc' },
    );

  return (
    <div className="space-y-4">
      <section
        aria-label="Filters"
        className="space-y-3 rounded-xl bg-surface/40 p-4 ring-1 ring-panel-edge ring-inset"
      >
        <div className="flex flex-wrap items-end gap-3">
          <label className="flex min-w-[14rem] flex-1 flex-col gap-1 text-xs text-muted">
            Search row ids
            <input
              type="search"
              value={state.q}
              onChange={(event) => set({ q: event.currentTarget.value })}
              placeholder="deck, slide or frontend"
              className={cx('rounded-md border border-line bg-input px-2 py-1.5 text-sm text-fg', focusRing)}
            />
          </label>
          <Select label="Deck" value={state.deck ?? ''} onChange={(deck) => set({ deck: deck === '' ? null : deck })}>
            <option value="">All decks</option>
            {decks.map((deck) => (
              <option key={deck} value={deck}>
                {deck} ({counts.deck.get(deck) ?? 0})
              </option>
            ))}
          </Select>
          {effects.length > 0 && (
            <Select label="Effect" value={state.fx ?? ''} onChange={(fx) => set({ fx: fx === '' ? null : fx })}>
              <option value="">All effects</option>
              {effects.map((effect) => (
                <option key={effect} value={effect}>
                  {effect}
                </option>
              ))}
            </Select>
          )}
          {hasSteps && (
            <div className="flex flex-col gap-1">
              <span className="text-xs text-muted">Rows</span>
              <SegmentedControl<StepMode>
                label="Rows"
                value={state.step}
                onChange={(step) => set({ step })}
                options={[
                  { value: 'all', label: 'All' },
                  { value: 'slide', label: `Slides (${counts.step.get('slide') ?? 0})` },
                  { value: 'steps', label: `Steps (${counts.step.get('steps') ?? 0})` },
                ]}
              />
            </div>
          )}
          <label className="flex items-center gap-2 pb-1.5 text-xs text-fg-secondary">
            <input
              type="checkbox"
              checked={state.ref}
              onChange={(event) => set({ ref: event.currentTarget.checked })}
              className="accent-[var(--brand-400)]"
            />
            Include reference and known-loss rows
          </label>
          <Button variant="ghost" size="sm" onClick={() => set({ ...NO_FILTERS })}>
            Reset
          </Button>
        </div>
        <ChipGroup
          label="Frontend"
          selected={state.fe}
          onChange={(fe) => set({ fe })}
          options={frontends.map((fe) => ({ value: fe, label: frontendLabel(fe), count: counts.fe.get(fe) ?? 0 }))}
        />
        <ChipGroup
          label="Outcome"
          selected={state.oc}
          onChange={(oc) => set({ oc })}
          options={[...outcomes, ...extraOutcomes].map((outcome) => ({
            value: outcome,
            label: outcomeLabel(outcome),
            count: counts.oc.get(outcome) ?? 0,
            tint: outcomeVars(outcome).fill,
          }))}
        />
        {codes.length > 0 && (
          <ChipGroup
            label="Diagnostic codes (all of)"
            selected={state.code}
            onChange={(code) => set({ code })}
            options={codes.map((code) => ({
              value: code,
              label: <span className="font-mono">{code}</span>,
              count: counts.code.get(code) ?? 0,
            }))}
          />
        )}
        {features.length > 0 && (
          <ChipGroup
            label="Declined feature"
            selected={state.feat}
            onChange={(feat) => set({ feat })}
            options={features.map((feature) => ({
              value: feature,
              label: <span className="font-mono">{feature}</span>,
              count: counts.feat.get(feature) ?? 0,
            }))}
          />
        )}
      </section>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="text-sm text-muted" aria-live="polite">
          <span className="font-mono tabular-nums text-fg">{rows.length}</span> / {shown} rows
        </p>
        {command && (
          <div className="w-full max-w-2xl">
            <CopyCommand command={command} label="the equivalent slidock-bench list command" />
          </div>
        )}
      </div>
      <div className="relative overflow-x-auto rounded-xl border border-line">
        <table className="w-full min-w-[56rem] text-sm">
          <thead className="sticky top-0 bg-surface">
            <tr>
              {columns.map((column) => (
                <SortHeader key={column.label} column={column} sort={state.sort} dir={state.dir} onSort={onSort} />
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 && (
              <tr>
                <td colSpan={columns.length} className="px-3 py-10 text-center text-muted">
                  No row matches these filters.
                </td>
              </tr>
            )}
            {rows.map(({ id, record }) => {
              const before = baseResults?.rows[id];
              const moved = before ? movement(before.outcome, record.outcome) : null;
              const delta = before?.scores && record.scores ? record.scores.ssim.mean - before.scores.ssim.mean : null;
              return (
                <tr
                  key={id}
                  onClick={() => set({ row: id })}
                  aria-selected={state.row === id}
                  className={cx(
                    'cursor-pointer border-t border-line-subtle transition-colors hover:bg-elevated/40',
                    state.row === id && 'bg-brand-wash',
                  )}
                >
                  <td className="px-3 py-2">
                    <button
                      type="button"
                      onClick={(event) => {
                        event.stopPropagation();
                        set({ row: id });
                      }}
                      aria-label={`Open ${id}`}
                      className={cx('text-left', focusRing)}
                    >
                      <RowName id={id} />
                    </button>
                  </td>
                  <td className="px-3 py-2">
                    <OutcomeBadge outcome={record.outcome} />
                  </td>
                  {scoreCell(record.scores?.ssim.mean)}
                  {scoreCell(record.scores?.ssim.p1)}
                  {scoreCell(record.scores?.color.mean)}
                  {scoreCell(record.scores?.text.recall)}
                  {scoreCell(record.scores?.text.p95Pm)}
                  <td className="px-3 py-2">
                    <Flags record={record} />
                  </td>
                  {baseResults && (
                    <td className="px-3 py-2 text-right font-mono text-xs tabular-nums">
                      {before === undefined ? (
                        <span className="text-muted">new</span>
                      ) : (
                        <span
                          style={{
                            color:
                              moved === 'improved' || moved === 'undeclined'
                                ? 'var(--delta-improved)'
                                : moved === 'regressed'
                                  ? 'var(--delta-regressed)'
                                  : 'var(--text-muted)',
                          }}
                        >
                          {moved && MOVEMENT_GLYPH[moved]} {delta === null ? DASH : formatDelta(delta)}
                        </span>
                      )}
                    </td>
                  )}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {base && !baseResults && state.base && (
        <p className="text-xs text-muted">The base run's results are not available, so no delta column is shown.</p>
      )}
    </div>
  );
}
