import { AppHeader, Banner, Pill, TabPanel, Tabs, ThemeProvider, ThemeToggle } from '@slidock/ui';
import { type ReactNode, useMemo, useState } from 'react';

import { CommitLink, CopyCommand, SegmentedControl, Select } from './components/Controls';
import { Logo } from './components/Logo';
import { Await, Empty, LoadError, Loading } from './components/States';
import { BenchDataContext, ResourceCache, useBenchData, useResource } from './data/resource';
import type { BenchSource, ReportScope } from './data/source';
import { formatTime, short } from './model/format';
import { compareIds, filterRows, rowsOf, sortRows } from './model/rows';
import { LANE_LABELS, resolveRun, runLabel } from './model/runs';
import { thresholdsFor } from './model/thresholds';
import type { Results, RunEntry, RunsIndex, Status } from './schema';
import { type BenchState, type Tab } from './state/url';
import { useBenchState } from './state/useBenchState';
import { themeCss } from './theme';
import { Compare } from './views/Compare';
import { Effects } from './views/Effects';
import { Explorer } from './views/Explorer';
import { Fonts } from './views/Fonts';
import { Methodology } from './views/Methodology';
import { Overview } from './views/Overview';
import { Parity } from './views/Parity';
import { RowDrawer } from './views/RowDrawer';
import type { ViewProps } from './views/context';

const TAB_ITEMS: { id: Tab; label: string }[] = [
  { id: 'overview', label: 'Overview' },
  { id: 'explorer', label: 'Explorer' },
  { id: 'compare', label: 'Compare' },
  { id: 'parity', label: 'Parity' },
  { id: 'fonts', label: 'Fonts' },
  { id: 'effects', label: 'Effects' },
  { id: 'methodology', label: 'Methodology' },
];

/** The tabs with something to show: every tab over the API; a report leaves out the views whose data it lacks. */
export function tabsFor(index: RunsIndex, scope: ReportScope | undefined): { id: Tab; label: string }[] {
  if (!scope) {
    return TAB_ITEMS;
  }
  const holds = (lane: RunEntry['lane']) => index.runs.some((run) => run.lane === lane);
  return TAB_ITEMS.filter(({ id }) => {
    switch (id) {
      case 'compare':
        return index.runs.length > 1;
      case 'parity':
        return scope.parity;
      case 'fonts':
        return holds('fonts');
      case 'effects':
        return holds('motion');
      default:
        return true;
    }
  });
}

/** Whether the run's stamp is not HEAD's (local server only): the command that refreshes it. */
function staleFor(run: RunEntry, status: Status | null): string | null {
  if (!status) {
    return null;
  }
  const stale = run.compiler.content !== status.head.content || run.harness !== status.head.harness;
  return stale ? (run.baseline ? 'slidock-bench baseline' : 'slidock-bench run') : null;
}

function ContextRow({
  state,
  set,
  index,
  run,
  results,
  status,
}: {
  state: BenchState;
  set(change: Partial<BenchState>): void;
  index: RunsIndex;
  run: RunEntry;
  results: Results | null;
  status: Status | null;
}) {
  // A published report names its run by the commit alone: its payload holds no run key or hash, and no base run.
  const published = Boolean(useBenchData().source.scope?.public);
  const stale = staleFor(run, status);
  const stamp = results?.stamp;
  const pairs = !results
    ? null
    : published
      ? new Set(Object.values(results.rows).map((record) => `${record.presentation}:${record.frontend}`)).size
      : Object.keys(results.stamp.corpus).length;
  const alias = state.run === 'latest' || state.run === 'baseline' ? state.run : 'run';
  return (
    <section aria-label="Run" className="space-y-3">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <p className="flex flex-wrap items-center gap-2 text-sm">
            <span className="text-muted">slidock</span>
            {published ? (
              <CommitLink commit={run.compiler.commit} />
            ) : (
              <code className="font-mono text-fg">{short(run.compiler.commit)}</code>
            )}
            {run.compiler.dirty && <Pill size="xs">+dirty</Pill>}
            <Pill size="xs">{LANE_LABELS[run.lane]} lane</Pill>
            {run.baseline && !published && <Pill size="xs">baseline</Pill>}
            {run.partial && <Pill size="xs">partial</Pill>}
            {stale && (
              <Pill size="xs" tint="var(--warning)" title={`Refresh with ${stale}`}>
                stale for HEAD
              </Pill>
            )}
          </p>
          {published ? (
            <p className="text-xs text-muted">
              {formatTime(run.recordedAt)}
              {stamp && (
                <>
                  {' '}
                  · Chromium {stamp.renderer.chromium} · poppler {stamp.renderer.poppler} · {pairs} pairs ·{' '}
                  {stamp.classifier}
                </>
              )}
            </p>
          ) : (
            <p className="text-xs text-muted">
              {formatTime(run.recordedAt)} · run <span className="font-mono">{short(run.key, 8)}</span>
              {stamp && (
                <>
                  {' '}
                  · Chromium {stamp.renderer.chromium} · poppler {stamp.renderer.poppler} · font set{' '}
                  <span className="font-mono">{short(stamp.fonts.fontSetId, 8)}</span> · {pairs} pairs ·{' '}
                  {stamp.classifier}
                </>
              )}
            </p>
          )}
        </div>
        {index.runs.length > 1 && (
          <div className="flex min-w-0 max-w-full flex-wrap items-end gap-3">
            <div className="flex flex-col gap-1">
              <span className="text-xs text-muted">Data</span>
              <SegmentedControl<'latest' | 'baseline' | 'run'>
                label="Data"
                value={alias}
                onChange={(value) => {
                  if (value !== 'run') {
                    set({ run: value, row: null });
                  }
                }}
                options={[
                  { value: 'latest', label: 'Latest' },
                  { value: 'baseline', label: 'Baseline' },
                  { value: 'run', label: 'A run', title: 'Pick a run from the list' },
                ]}
              />
            </div>
            <Select
              label="Run"
              value={run.key}
              onChange={(key) => set({ run: key, row: null })}
              className="max-w-[22rem]"
            >
              {[...index.runs].reverse().map((entry) => (
                <option key={entry.key} value={entry.key}>
                  {runLabel(entry)}
                  {entry.baseline ? ' · baseline' : ''}
                </option>
              ))}
            </Select>
          </div>
        )}
      </div>
      {stale && (
        <Banner tone="warning" title="These results are stale for HEAD">
          <p className="mb-2">The compiler or the bench changed since this run. Refresh it with:</p>
          <CopyCommand command={stale} label="refresh command" />
        </Banner>
      )}
    </section>
  );
}

function View({ tab, props, baseResults }: { tab: Tab; props: ViewProps; baseResults: Results | null }) {
  switch (tab) {
    case 'overview':
      return <Overview {...props} />;
    case 'explorer':
      return <Explorer {...props} baseResults={baseResults} />;
    case 'compare':
      return <Compare {...props} />;
    case 'parity':
      return <Parity {...props} />;
    case 'fonts':
      return <Fonts {...props} />;
    case 'effects':
      return <Effects {...props} />;
    case 'methodology':
      return <Methodology {...props} />;
  }
}

function Bench({ index, status }: { index: RunsIndex; status: Status | null }) {
  const [state, set] = useBenchState();
  const { scope } = useBenchData().source;
  const tabs = tabsFor(index, scope);
  const tab = tabs.some(({ id }) => id === state.tab) ? state.tab : 'overview';
  const run = resolveRun(index, state.run);
  const results = useResource<Results | null>(run ? `results:${run.key}` : null, (source) =>
    run ? source.results(run.key) : Promise.resolve(null),
  );
  // A published report shows its one run against no other.
  const baseRef = scope?.public ? null : (state.base ?? (run?.baseline ? null : 'baseline'));
  const base = baseRef === null ? null : resolveRun(index, baseRef);
  const explicitBase = state.base === null || baseRef === null ? null : base;
  const baseResults = useResource<Results | null>(explicitBase ? `results:${explicitBase.key}` : null, (source) =>
    explicitBase ? source.results(explicitBase.key) : Promise.resolve(null),
  );
  const loaded = results.state === 'ready' ? results.value : null;
  const order = useMemo(() => {
    if (!loaded) {
      return [];
    }
    const listed = sortRows(filterRows(rowsOf(loaded), state), state.sort, state.dir).map((row) => row.id);
    return state.row !== null && !listed.includes(state.row) ? Object.keys(loaded.rows).sort(compareIds) : listed;
  }, [loaded, state]);

  if (!run) {
    return (
      <Empty>
        {index.runs.length === 0
          ? 'The bench holds no run yet. slidock-bench run writes one.'
          : `The runs index has no ${state.run === 'latest' || state.run === 'baseline' ? `${state.run} run` : `run ${state.run}`}.`}
      </Empty>
    );
  }
  if (results.state === 'loading') {
    return <Loading label="Loading the run's results…" />;
  }
  if (results.state === 'error') {
    return <LoadError what="the run's results" error={results.error} />;
  }
  const thresholds = loaded ? thresholdsFor(loaded.stamp, status) : null;
  const props: ViewProps = {
    state,
    set,
    index,
    status,
    run,
    results: loaded,
    base: base?.key === run.key ? null : base,
    thresholds,
  };
  const record = state.row !== null ? loaded?.rows[state.row] : undefined;
  return (
    <>
      <ContextRow state={state} set={set} index={index} run={run} results={loaded} status={status} />
      <Tabs
        variant="pills"
        label="Bench views"
        idPrefix="bench"
        panels="selected"
        items={tabs}
        value={tab}
        onChange={(tab) => set({ tab })}
        className="flex-wrap"
      />
      <TabPanel idPrefix="bench" id={tab} className="min-w-0">
        {!loaded && (
          <div className="mb-4">
            <Banner tone="warning" title="This run's results are no longer in the bench">
              Its summary comes from the runs index; the rows were garbage-collected.
            </Banner>
          </div>
        )}
        <View tab={tab} props={props} baseResults={baseResults.state === 'ready' ? baseResults.value : null} />
      </TabPanel>
      {state.row !== null && record && (
        <RowDrawer
          state={state}
          set={set}
          run={run}
          id={state.row}
          record={record}
          order={order}
          thresholds={thresholds}
          base={tab === 'compare' ? base : null}
        />
      )}
      {state.row !== null && loaded && !record && (
        <Banner tone="warning" title={`Row ${state.row} is not in this run`}>
          <button type="button" className="underline" onClick={() => set({ row: null })}>
            Close it
          </button>
        </Banner>
      )}
    </>
  );
}

function Shell({ published = null, children }: { published?: ReportScope['public']; children: ReactNode }) {
  return (
    <div className="mx-auto flex min-h-dvh max-w-[1400px] flex-col gap-6 px-4 py-6">
      <AppHeader
        logo={<Logo />}
        title="slidock bench"
        subtitle={
          published
            ? 'Published slides, rendered and scored against their oracle'
            : 'Every corpus slide, rendered and scored against its oracle'
        }
        right={
          <div className="flex items-center gap-3">
            {published && (
              <a className="text-sm text-muted underline hover:text-fg" href="../">
                Open slidock
              </a>
            )}
            <ThemeToggle />
          </div>
        }
      />
      <main className="flex min-w-0 flex-col gap-6">
        {published && (
          <Banner tone="info" title="A published snapshot of slidock's fidelity bench">
            Each slide of the decks cleared for publication is converted by slidock, rendered in a pinned Chromium and
            scored against the authoring app's own PDF export. The rest of the corpus stays private, and every figure
            here counts the published slides only.
            {published.images.length > 0 && ` Images are included for ${published.images.join(', ')} rows.`}
          </Banner>
        )}
        {children}
      </main>
    </div>
  );
}

function Root() {
  const published = useBenchData().source.scope?.public ?? null;
  const runs = useResource<RunsIndex>('runs', (source) => source.runs());
  const status = useResource<Status | null>('status', (source) => source.status());
  return (
    <Shell published={published}>
      <Await resource={runs} what="the runs index">
        {(index) => <Bench index={index} status={status.state === 'ready' ? status.value : null} />}
      </Await>
    </Shell>
  );
}

/** A page that cannot load any bench data (an unfilled or broken report). */
export function NoData({ title, children }: { title: string; children: ReactNode }) {
  return (
    <ThemeProvider>
      <style>{themeCss()}</style>
      <Shell>
        <Banner tone="danger" title={title}>
          {children}
        </Banner>
      </Shell>
    </ThemeProvider>
  );
}

export function App({ source }: { source: BenchSource }) {
  const [data] = useState(() => ({ source, cache: new ResourceCache() }));
  return (
    <ThemeProvider>
      <style>{themeCss()}</style>
      <BenchDataContext.Provider value={data}>
        <Root />
      </BenchDataContext.Provider>
    </ThemeProvider>
  );
}
