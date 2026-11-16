import { Banner, Panel, cx, focusRing } from '@slidock/ui';
import { useMemo, useState } from 'react';

import { Select } from '../components/Controls';
import { OutcomeBadge } from '../components/OutcomeBadge';
import { Await, Empty } from '../components/States';
import { useResource } from '../data/resource';
import {
  type Comparison,
  FRONTEND_SSIM_DROP,
  type RowChange,
  type Transition,
  blocks,
  compareResults,
  regressionExit,
} from '../model/compare';
import { DASH, formatDelta } from '../model/format';
import { outcomeLabel, outcomeRank } from '../model/outcomes';
import { frontendLabel } from '../model/overview';
import { resolveRun, runLabel } from '../model/runs';
import type { Results, RowId, RunEntry } from '../schema';
import type { ViewProps } from './context';

function RunPicker({
  label,
  value,
  runs,
  onChange,
}: {
  label: string;
  value: string;
  runs: readonly RunEntry[];
  onChange(value: string): void;
}) {
  return (
    <Select label={label} value={value} onChange={onChange} className="min-w-[18rem]">
      <option value="latest">Latest static run</option>
      <option value="baseline">Baseline</option>
      {[...runs].reverse().map((run) => (
        <option key={run.key} value={run.key}>
          {runLabel(run)} · {run.key.slice(0, 8)}
        </option>
      ))}
    </Select>
  );
}

function RowButton({ id, onOpen }: { id: RowId; onOpen(id: RowId): void }) {
  return (
    <button
      type="button"
      onClick={() => onOpen(id)}
      className={cx('whitespace-nowrap text-left font-mono text-[13px] text-fg hover:text-brand-300', focusRing)}
    >
      {id}
    </button>
  );
}

function ChangeTable({
  title,
  subtitle,
  changes,
  onOpen,
}: {
  title: string;
  subtitle: string;
  changes: readonly RowChange[];
  onOpen(id: RowId): void;
}) {
  return (
    <Panel title={`${title} (${changes.length})`} subtitle={subtitle}>
      {changes.length === 0 ? (
        <Empty>None.</Empty>
      ) : (
        <div className="relative overflow-x-auto">
          <table className="w-full min-w-[40rem] text-sm">
            <thead className="text-xs uppercase tracking-wide text-muted">
              <tr>
                <th scope="col" className="px-2 py-1.5 text-left font-semibold">
                  Row
                </th>
                <th scope="col" className="px-2 py-1.5 text-left font-semibold">
                  Base → head
                </th>
                <th scope="col" className="px-2 py-1.5 text-right font-semibold">
                  Δ SSIM
                </th>
                <th scope="col" className="px-2 py-1.5 text-left font-semibold">
                  Guards broken
                </th>
              </tr>
            </thead>
            <tbody>
              {changes.map((change) => (
                <tr key={change.id} className="border-t border-line-subtle">
                  <td className="px-2 py-1.5">
                    <RowButton id={change.id} onOpen={onOpen} />
                  </td>
                  <td className="px-2 py-1.5">
                    <span className="inline-flex items-center gap-1.5">
                      <OutcomeBadge outcome={change.base.outcome} size="xs" />
                      <span aria-label="to">→</span>
                      <OutcomeBadge outcome={change.head.outcome} size="xs" />
                    </span>
                  </td>
                  <td className="px-2 py-1.5 text-right font-mono tabular-nums">
                    {change.ssimDelta === null ? DASH : formatDelta(change.ssimDelta)}
                  </td>
                  <td className="px-2 py-1.5 font-mono text-xs text-fg-secondary">
                    {[...change.losses, ...change.guards].join('; ') || DASH}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Panel>
  );
}

function Matrix({
  transitions,
  selected,
  onSelect,
}: {
  transitions: readonly Transition[];
  selected: Transition | null;
  onSelect(transition: Transition | null): void;
}) {
  const byRank = (a: string, b: string) => outcomeRank(b) - outcomeRank(a);
  const froms = [...new Set(transitions.map((t) => t.from))].sort(byRank);
  const tos = [...new Set(transitions.map((t) => t.to))].sort(byRank);
  const cell = (from: string, to: string) => transitions.find((t) => t.from === from && t.to === to);
  return (
    <div className="relative overflow-x-auto">
      <table className="text-sm">
        <caption className="mb-2 text-left text-xs text-muted">
          Rows by base class (rows) and head class (columns).
        </caption>
        <thead>
          <tr>
            <th scope="col" className="px-2 py-1 text-left text-xs font-semibold text-muted">
              base ↓ · head →
            </th>
            {tos.map((to) => (
              <th key={to} scope="col" className="px-2 py-1 text-xs font-semibold text-fg-secondary">
                {outcomeLabel(to)}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {froms.map((from) => (
            <tr key={from}>
              <th scope="row" className="px-2 py-1 text-left text-xs font-semibold text-fg-secondary">
                {outcomeLabel(from)}
              </th>
              {tos.map((to) => {
                const transition = cell(from, to);
                const count = transition?.rows.length ?? 0;
                const rank = outcomeRank(to) - outcomeRank(from);
                const tint =
                  rank > 0 ? 'var(--delta-improved)' : rank < 0 ? 'var(--delta-regressed)' : 'var(--text-secondary)';
                const active = transition !== undefined && transition === selected;
                return (
                  <td key={to} className="px-1 py-1 text-center">
                    {transition ? (
                      <button
                        type="button"
                        aria-pressed={active}
                        aria-label={`${count} rows from ${outcomeLabel(from)} to ${outcomeLabel(to)}`}
                        onClick={() => onSelect(active ? null : transition)}
                        className={cx(
                          'min-w-12 rounded-md border px-2 py-1 font-mono tabular-nums',
                          active ? 'border-brand-400 bg-brand-wash' : 'border-line/60 hover:border-muted',
                          focusRing,
                        )}
                        style={{ color: tint }}
                      >
                        {count}
                      </button>
                    ) : (
                      <span className="text-dim">·</span>
                    )}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** The causes `slidock-bench regression` would exit 1 for, as `<count> <bucket>`. */
function failures(comparison: Comparison): string[] {
  const causes: [number, string][] = [
    [comparison.regressed.length, 'regressed'],
    [comparison.improved.length, 'improved'],
    [comparison.undeclined.length, 'undeclined'],
    [comparison.eroded.length, 'eroded'],
    [comparison.vanished.length, 'vanished'],
    [comparison.newlyFlagged.length, 'newly flagged'],
    [comparison.frontendGuards.length, `frontend SSIM guard${comparison.frontendGuards.length === 1 ? '' : 's'}`],
  ];
  return causes.filter(([count]) => count > 0).map(([count, name]) => `${count} ${name}`);
}

function Verdict({ base, head, comparison }: { base: RunEntry; head: RunEntry; comparison: Comparison }) {
  const runs = base.key === head.key ? 'Both sides are the same run.' : `${runLabel(base)} → ${runLabel(head)}.`;
  switch (regressionExit(comparison)) {
    case 2:
      return (
        <Banner tone="danger" title="These results are not comparable">
          Their stamps differ beyond the compiler in {comparison.incomparable.join(', ')}; slidock-bench regression
          refuses them (exit 2). The tables below are shown for inspection only.
        </Banner>
      );
    case 3:
      return (
        <Banner tone="warning" title="No row compared">
          The runs share no row; slidock-bench regression exits 3. {runs}
        </Banner>
      );
    case 1:
      return (
        <Banner tone="danger" title={`Regression fails: ${comparison.compared} rows compared`}>
          slidock-bench regression exits 1 on {failures(comparison).join(', ')}. {runs}
        </Banner>
      );
    case 0:
      return (
        <Banner tone="success" title={`Regression passes: ${comparison.compared} rows compared`}>
          {runs}
        </Banner>
      );
  }
}

function Comparing({
  base,
  head,
  comparison,
  set,
}: {
  base: RunEntry;
  head: RunEntry;
  comparison: Comparison;
  set: ViewProps['set'];
}) {
  const [selected, setSelected] = useState<Transition | null>(null);
  const open = (row: RowId) => set({ row });
  return (
    <div className="space-y-6">
      <Verdict base={base} head={head} comparison={comparison} />
      <Panel title="Stamp" subtitle="Every stamp field that differs between the two runs.">
        {comparison.stamp.length === 0 ? (
          <Empty>The stamps are equal.</Empty>
        ) : (
          <div className="relative overflow-x-auto">
            <table className="w-full min-w-[40rem] text-sm">
              <thead className="text-xs uppercase tracking-wide text-muted">
                <tr>
                  <th scope="col" className="px-2 py-1.5 text-left font-semibold">
                    Field
                  </th>
                  <th scope="col" className="px-2 py-1.5 text-left font-semibold">
                    Base
                  </th>
                  <th scope="col" className="px-2 py-1.5 text-left font-semibold">
                    Head
                  </th>
                </tr>
              </thead>
              <tbody>
                {comparison.stamp.map((field) => {
                  const blocking = blocks(field.field, comparison.incomparable);
                  return (
                    <tr key={field.field} className="border-t border-line-subtle">
                      <th scope="row" className="px-2 py-1.5 text-left font-mono text-xs font-normal">
                        {field.field}
                        {blocking && <span className="ml-2 text-[11px] text-danger">not comparable</span>}
                      </th>
                      <td
                        className="max-w-[18rem] truncate px-2 py-1.5 font-mono text-xs text-fg-secondary"
                        title={field.base ?? ''}
                      >
                        {field.base ?? DASH}
                      </td>
                      <td
                        className="max-w-[18rem] truncate px-2 py-1.5 font-mono text-xs text-fg-secondary"
                        title={field.head ?? ''}
                      >
                        {field.head ?? DASH}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </Panel>
      <Panel title="Class transitions" subtitle="Click a cell to list its rows.">
        <Matrix transitions={comparison.transitions} selected={selected} onSelect={setSelected} />
        {selected && (
          <ul className="mt-3 flex flex-wrap gap-x-4 gap-y-1">
            {selected.rows.map((row) => (
              <li key={row}>
                <RowButton id={row} onOpen={open} />
              </li>
            ))}
          </ul>
        )}
      </Panel>
      <div className="grid grid-cols-1 gap-6 xl:grid-cols-2">
        <ChangeTable
          title="Regressed"
          subtitle="The class moved down, or more shadows went undrawn or gradients approximated; largest SSIM change first."
          changes={comparison.regressed}
          onOpen={open}
        />
        <ChangeTable
          title="Improved"
          subtitle="The class moved up: a re-baseline is due."
          changes={comparison.improved}
          onOpen={open}
        />
        <ChangeTable
          title="Undeclined"
          subtitle="Declined in the base, scored in the head: review each."
          changes={comparison.undeclined}
          onOpen={open}
        />
        <ChangeTable
          title="Eroded"
          subtitle="Same class, but an erosion guard broke."
          changes={comparison.eroded}
          onOpen={open}
        />
      </div>
      <Panel
        title={`Frontend SSIM guard (${comparison.frontendGuards.length})`}
        subtitle={`Per frontend, the SSIM median and p10 over the rows both runs hold, reference rows apart: a drop of more than ${FRONTEND_SSIM_DROP} fails.`}
      >
        {comparison.frontendGuards.length === 0 ? (
          <Empty>No frontend dropped.</Empty>
        ) : (
          <ul className="space-y-1">
            {comparison.frontendGuards.map((guard) => (
              <li key={guard.text} className="flex flex-wrap items-baseline gap-x-3 text-sm">
                <span className="font-medium text-fg-secondary">{frontendLabel(guard.frontend)}</span>
                <span className="font-mono text-xs text-fg">{guard.text}</span>
                <span className="font-mono text-xs tabular-nums text-danger">
                  {formatDelta(guard.head - guard.base)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Panel>
      <div className="grid grid-cols-1 gap-6 xl:grid-cols-2">
        <ChangeTable
          title="Flagged"
          subtitle="Changed rows the base flags reference or known loss: reported, not counted."
          changes={comparison.flagged}
          onOpen={open}
        />
        <ChangeTable
          title="Newly flagged"
          subtitle="Rows the head flags and the base does not: review each."
          changes={comparison.newlyFlagged}
          onOpen={open}
        />
      </div>
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <Panel
          title={`Vanished (${comparison.vanished.length})`}
          subtitle="Base rows missing from a pair the head covered."
        >
          {comparison.vanished.length === 0 ? (
            <Empty>None.</Empty>
          ) : (
            <ul className="space-y-1">
              {comparison.vanished.map((id) => (
                <li key={id} className="font-mono text-sm">
                  {id}
                </li>
              ))}
            </ul>
          )}
        </Panel>
        <Panel
          title={`New (${comparison.added.length})`}
          subtitle={`Rows only the head holds. ${comparison.notRun} base rows are in pairs the head did not run.`}
        >
          {comparison.added.length === 0 ? (
            <Empty>None.</Empty>
          ) : (
            <ul className="space-y-1">
              {comparison.added.map((id) => (
                <li key={id}>
                  <RowButton id={id} onOpen={open} />
                </li>
              ))}
            </ul>
          )}
        </Panel>
      </div>
    </div>
  );
}

function Pair({
  base,
  head,
  baseResults,
  headResults,
  set,
}: {
  base: RunEntry;
  head: RunEntry;
  baseResults: Results;
  headResults: Results;
  set: ViewProps['set'];
}) {
  const comparison = useMemo(() => compareResults(baseResults, headResults), [baseResults, headResults]);
  return <Comparing base={base} head={head} comparison={comparison} set={set} />;
}

export function Compare({ state, set, index, run, results }: ViewProps) {
  const baseRef = state.base ?? 'baseline';
  const base = resolveRun(index, baseRef);
  const baseResults = useResource<Results | null>(base ? `results:${base.key}` : null, (source) =>
    base ? source.results(base.key) : Promise.resolve(null),
  );
  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end gap-4 rounded-xl bg-surface/40 p-4 ring-1 ring-panel-edge ring-inset">
        <RunPicker
          label="Base"
          value={baseRef}
          runs={index.runs}
          onChange={(value) => set({ base: value === 'baseline' ? null : value })}
        />
        <span aria-hidden className="pb-2 text-muted">
          →
        </span>
        <RunPicker
          label="Head"
          value={state.run}
          runs={index.runs}
          onChange={(value) => set({ run: value, row: null })}
        />
      </div>
      {!base ? (
        <Empty>
          The runs index has no {baseRef === 'baseline' ? 'baseline run' : `run ${baseRef}`} to compare against.
        </Empty>
      ) : !results ? (
        <Empty>The bench no longer holds the head run's results.</Empty>
      ) : (
        <Await resource={baseResults} what="the base run's results">
          {(value) =>
            value ? (
              <Pair base={base} head={run} baseResults={value} headResults={results} set={set} />
            ) : (
              <Empty>The bench no longer holds the base run's results.</Empty>
            )
          }
        </Await>
      )}
    </div>
  );
}
