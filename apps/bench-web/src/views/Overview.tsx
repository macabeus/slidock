import { Button, Panel } from '@slidock/ui';

import { DeltaBadge, Stat } from '../components/Controls';
import { ParityBadge } from '../components/OutcomeBadge';
import { OutcomeBar } from '../components/OutcomeBar';
import { Empty } from '../components/States';
import { useBenchData, useResource } from '../data/resource';
import { FRONTEND_SSIM_DROP } from '../model/compare';
import { formatDelta, formatNumber, formatPercent, formatPointsDelta, formatTime } from '../model/format';
import { laneFigures } from '../model/lanes';
import { OUTCOMES, outcomeGloss, outcomeLabel, outcomeVars } from '../model/outcomes';
import {
  type Figure,
  declineRanking,
  fidelity,
  frontendLabel,
  outcomeSegments,
  summaryFrontends,
  tiles,
} from '../model/overview';
import { parityCounts } from '../model/parity';
import { LANE_GLOSS, LANE_LABELS, latestByLane, runLabel } from '../model/runs';
import type { ParityView, Summary } from '../schema';
import { explore } from '../state/url';
import { PARITY_COLORS } from '../theme';
import type { ViewProps } from './context';

function FigureCell({
  figure,
  format,
  delta = formatDelta,
}: {
  figure: Figure;
  format(value: number | null): string;
  delta?: (value: number | null) => string;
}) {
  return (
    <td className="px-3 py-2 text-right">
      <span className="font-mono tabular-nums text-fg">{format(figure.value)}</span>{' '}
      <DeltaBadge delta={figure.delta} trend={figure.trend} text={delta(figure.delta)} />
    </td>
  );
}

function Distribution({ summary, set }: { summary: Summary; set: ViewProps['set'] }) {
  const frontends = summaryFrontends(summary.byFrontend);
  const reference = summaryFrontends(summary.reference);
  return (
    <Panel
      title="Outcome distribution"
      subtitle="Rows by class per frontend, best class first. Click a segment to list those rows in the explorer."
    >
      {frontends.length === 0 ? (
        <Empty>This run scored no row.</Empty>
      ) : (
        <div className="space-y-3">
          {frontends.map(([frontend, value]) => (
            <div key={frontend} className="grid grid-cols-[6rem_1fr] items-center gap-3 sm:grid-cols-[7rem_1fr_4rem]">
              <span className="text-sm font-medium text-fg-secondary">{frontendLabel(frontend)}</span>
              <OutcomeBar
                label={`${frontendLabel(frontend)} rows by outcome`}
                action="open in the explorer"
                segments={outcomeSegments(value).map((segment) => ({
                  key: segment.outcome,
                  count: segment.count,
                  outcome: segment.outcome,
                }))}
                onSelect={(outcome) => set(explore({ fe: [frontend], oc: [outcome], ref: true }))}
              />
              <span className="hidden text-right font-mono text-xs tabular-nums text-muted sm:block">
                {value.rows} rows
              </span>
            </div>
          ))}
          {reference.map(([frontend, value]) => (
            <div
              key={`ref-${frontend}`}
              className="grid grid-cols-[6rem_1fr] items-center gap-3 opacity-60 sm:grid-cols-[7rem_1fr_4rem]"
            >
              <span className="text-sm text-muted">{frontendLabel(frontend)} (reference)</span>
              <OutcomeBar
                label={`${frontendLabel(frontend)} reference rows by outcome`}
                segments={outcomeSegments(value).map((segment) => ({
                  key: segment.outcome,
                  count: segment.count,
                  outcome: segment.outcome,
                }))}
              />
              <span className="hidden text-right font-mono text-xs tabular-nums text-muted sm:block">
                {value.rows} rows
              </span>
            </div>
          ))}
        </div>
      )}
    </Panel>
  );
}

function Legend({ thresholds }: { thresholds: ViewProps['thresholds'] }) {
  return (
    <ul
      className="grid grid-cols-1 gap-x-6 gap-y-1.5 text-xs sm:grid-cols-2 lg:grid-cols-3"
      aria-label="Outcome classes"
    >
      {OUTCOMES.map((outcome) => (
        <li key={outcome} className="flex gap-2">
          <span
            aria-hidden
            className="mt-1 size-2.5 shrink-0 rounded-sm"
            style={{ backgroundColor: outcomeVars(outcome).fill }}
          />
          <span>
            <span className="font-semibold" style={{ color: outcomeVars(outcome).text }}>
              {outcomeLabel(outcome)}
            </span>{' '}
            <span className="text-muted">{outcomeGloss(outcome, thresholds)}</span>
          </span>
        </li>
      ))}
    </ul>
  );
}

function ParityStrip({ view, set }: { view: ParityView; set: ViewProps['set'] }) {
  const counts = parityCounts(view.report);
  const failing = view.report.compiler?.failing.length ?? 0;
  return (
    <Panel
      title="Parity with the goldens"
      subtitle={`Level B: this run's rows against slidock's goldens recorded at ${view.report.reference.golden.commit.slice(0, 7)}. ${failing} row${failing === 1 ? '' : 's'} fail parity check.`}
      headerRight={
        <Button variant="ghost" size="sm" onClick={() => set({ tab: 'parity', ps: [] })}>
          Open parity
        </Button>
      }
    >
      <OutcomeBar
        label="Rows by parity status"
        action="open in the parity table"
        segments={counts.map(({ status, rows }) => ({
          key: status,
          count: rows,
          color: PARITY_COLORS[status].fill,
          label: status,
        }))}
        onSelect={(status) => set({ tab: 'parity', ps: [status] })}
      />
      <ul className="mt-3 flex flex-wrap gap-2">
        {counts
          .filter(({ rows }) => rows > 0)
          .map(({ status, rows }) => (
            <li key={status} className="flex items-center gap-1.5 text-xs">
              <ParityBadge status={status} size="xs" />
              <span className="font-mono tabular-nums text-fg-secondary">{rows}</span>
            </li>
          ))}
      </ul>
    </Panel>
  );
}

function Lanes({ index, run, set }: Pick<ViewProps, 'index' | 'run' | 'set'>) {
  const runs = latestByLane(index);
  return (
    <Panel title="Lanes" subtitle="The newest run of each lane and what its checks report.">
      <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
        {runs.map((entry) => {
          const figures = laneFigures(entry.lane, entry.summary);
          const selected = entry.key === run.key;
          return (
            <article key={entry.key} className="flex flex-col gap-2 rounded-lg border border-line/70 bg-surface/40 p-3">
              <header className="flex items-start justify-between gap-2">
                <div>
                  <h3 className="text-sm font-semibold">{LANE_LABELS[entry.lane]}</h3>
                  <p className="text-xs text-muted">{formatTime(entry.recordedAt)}</p>
                </div>
                {selected ? (
                  <span className="text-xs text-muted">shown</span>
                ) : (
                  <Button variant="ghost" size="sm" onClick={() => set({ run: entry.key, row: null })}>
                    Open
                  </Button>
                )}
              </header>
              <p className="text-xs text-muted">{LANE_GLOSS[entry.lane]}</p>
              <dl className="mt-auto space-y-1.5">
                {figures.map((figure) => (
                  <div key={figure.label}>
                    <dt className="flex justify-between text-xs">
                      <span className="text-fg-secondary">{figure.label}</span>
                      <span className="font-mono tabular-nums">
                        {figure.pass} / {figure.rows}
                      </span>
                    </dt>
                    <dd className="mt-1 h-1.5 overflow-hidden rounded-full bg-elevated/60">
                      <div
                        className="h-full rounded-full"
                        style={{
                          width: `${figure.rows > 0 ? (figure.pass / figure.rows) * 100 : 0}%`,
                          backgroundColor: 'var(--oc-match-fill)',
                        }}
                      />
                    </dd>
                  </div>
                ))}
                {figures.length === 0 && <dd className="text-xs text-muted">No lane summary in this run.</dd>}
              </dl>
            </article>
          );
        })}
      </div>
    </Panel>
  );
}

export function Overview({ index, run, results, base, thresholds, set }: ViewProps) {
  // A report without parity data leaves the panel out; over the API it says how to make the report.
  const showParity = useBenchData().source.scope?.parity ?? true;
  const summary = results?.summary ?? run.summary;
  const parity = useResource<ParityView | null>(results ? `parity:${run.key}` : null, (source) =>
    source.parity(run.key),
  );
  const counts = results ? tiles(results) : null;
  const frontends = summaryFrontends(summary.byFrontend);
  const declines = declineRanking(summary);
  return (
    <div className="space-y-6">
      {counts && (
        <div className="grid grid-cols-2 gap-4 md:grid-cols-3 lg:grid-cols-6">
          <Stat label="Decks" value={counts.decks} />
          <Stat label="Slides" value={counts.slides} hint="static rows" />
          <Stat label="Scored" value={counts.scored} hint="match, close or mismatch" />
          <Stat label="Step rows" value={counts.steps} hint="build stages and transition frames" />
          <Stat label="Frontends" value={counts.frontends} />
          <Stat label="Match" value={formatPercent(counts.matchShare)} accent hint="of every row" />
        </div>
      )}
      <Distribution summary={summary} set={set} />
      <Legend thresholds={thresholds} />
      {frontends.length > 0 && (
        <Panel
          title="Fidelity by frontend"
          subtitle={
            base
              ? `Against ${runLabel(base)}'s summary: ▲ better, ▼ worse. An SSIM move within ${FRONTEND_SSIM_DROP} (the regression guard's bound) reads as unchanged; text recall and match share show any move. Compare applies the guard over the rows both runs hold.`
              : 'No base run to compare against.'
          }
        >
          <div className="relative overflow-x-auto">
            <table className="w-full min-w-[34rem] text-sm">
              <thead>
                <tr className="text-xs uppercase tracking-wide text-muted">
                  <th scope="col" className="px-3 py-2 text-left font-semibold">
                    Frontend
                  </th>
                  <th scope="col" className="px-3 py-2 text-right font-semibold">
                    SSIM median
                  </th>
                  <th scope="col" className="px-3 py-2 text-right font-semibold">
                    SSIM p10
                  </th>
                  <th scope="col" className="px-3 py-2 text-right font-semibold">
                    Text recall
                  </th>
                  <th scope="col" className="px-3 py-2 text-right font-semibold">
                    Match share
                  </th>
                </tr>
              </thead>
              <tbody>
                {frontends.map(([frontend, value]) => {
                  const strip = fidelity(value, base?.summary.byFrontend[frontend]);
                  return (
                    <tr key={frontend} className="border-t border-line-subtle">
                      <th scope="row" className="px-3 py-2 text-left font-medium text-fg-secondary">
                        {frontendLabel(frontend)}
                      </th>
                      <FigureCell figure={strip.ssimMedian} format={formatNumber} />
                      <FigureCell figure={strip.ssimP10} format={formatNumber} />
                      <FigureCell figure={strip.textRecall} format={formatNumber} />
                      <FigureCell figure={strip.matchShare} format={formatPercent} delta={formatPointsDelta} />
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </Panel>
      )}
      <div className={showParity ? 'grid grid-cols-1 gap-6 lg:grid-cols-2' : 'grid grid-cols-1 gap-6'}>
        {!showParity ? null : parity.state === 'ready' && parity.value ? (
          <ParityStrip view={parity.value} set={set} />
        ) : (
          <Panel title="Parity with the goldens" subtitle="Level B, row by row against slidock's goldens.">
            <Empty>
              {parity.state === 'loading' && results
                ? 'Loading the parity report…'
                : 'This run has no parity report. slidock-bench parity check writes one.'}
            </Empty>
          </Panel>
        )}
        <Panel title="Declines" subtitle="The deciding decline codes, most rows first: the compiler's roadmap.">
          {declines.length === 0 ? (
            <Empty>No row is declined.</Empty>
          ) : (
            <ul className="space-y-1.5">
              {declines.map(({ code, rows }) => (
                <li key={code}>
                  <button
                    type="button"
                    onClick={() => set(explore({ oc: ['declined'], code: [code], ref: true }))}
                    className="flex w-full items-center justify-between rounded-md px-2 py-1 text-sm hover:bg-elevated/60"
                    aria-label={`${rows} declined row${rows === 1 ? '' : 's'} with ${code}: open in the explorer`}
                  >
                    <code className="font-mono text-fg-secondary">{code}</code>
                    <span className="font-mono tabular-nums text-muted">{rows}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </Panel>
      </div>
      <Lanes index={index} run={run} set={set} />
    </div>
  );
}
