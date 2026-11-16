import { Banner, Panel, cx, focusRing } from '@slidock/ui';

import { ChipGroup } from '../components/Controls';
import { OutcomeBadge, ParityBadge } from '../components/OutcomeBadge';
import { Await, Empty } from '../components/States';
import { useResource } from '../data/resource';
import { DASH, formatNumber, short } from '../model/format';
import { PARITY_GLOSS, PARITY_STATUSES, parityCounts, parityRows } from '../model/parity';
import type { ParityView } from '../schema';
import { PARITY_COLORS } from '../theme';
import type { ViewProps } from './context';

function Levels({ view, state, set }: { view: ParityView; state: ViewProps['state']; set: ViewProps['set'] }) {
  const { report, allowlist } = view;
  const counts = parityCounts(report);
  const rows = parityRows(report, state.ps);
  const containers = Object.entries(report.ir?.containers ?? {});
  const failing = report.compiler?.failing.length ?? 0;
  return (
    <div className="space-y-6">
      <Panel
        title="Reference"
        subtitle="The golden output this run is held to, and the renderer its render hashes are keyed under."
      >
        <dl className="grid grid-cols-1 gap-x-6 gap-y-1 text-sm sm:grid-cols-[max-content_1fr]">
          <dt className="text-muted">Golden commit</dt>
          <dd className="font-mono text-xs leading-5">{report.reference.golden.commit}</dd>
          <dt className="text-muted">Renderer recipe</dt>
          <dd className="font-mono text-xs leading-5">{short(report.reference.rendererRecipe, 12)}</dd>
          <dt className="text-muted">Font set</dt>
          <dd className="font-mono text-xs leading-5">{short(report.reference.fontSetId, 12)}</dd>
          <dt className="text-muted">Reference file</dt>
          <dd className="font-mono text-xs leading-5">{short(report.reference.sha256, 12)}</dd>
        </dl>
      </Panel>

      <Panel
        title="B · Compiler parity"
        subtitle={`Each row of this run against the goldens' reference. parity check fails on ${failing} row${failing === 1 ? '' : 's'}: regressed, missing or drifting rows the allowlist does not list.`}
      >
        <div className="space-y-4">
          <ChipGroup
            label="Status"
            selected={state.ps}
            onChange={(ps) => set({ ps })}
            options={counts.map(({ status, rows: n }) => ({
              value: status,
              label: status,
              count: n,
              tint: PARITY_COLORS[status].fill,
            }))}
          />
          {report.compiler?.strict && <Banner tone="info" title="Strict: every row must be identical." />}
          <div className="relative overflow-x-auto rounded-xl border border-line">
            <table className="w-full min-w-[60rem] text-sm">
              <thead className="bg-surface text-xs uppercase tracking-wide text-muted">
                <tr>
                  <th scope="col" className="px-3 py-2.5 text-left font-semibold">
                    Row
                  </th>
                  <th scope="col" className="px-3 py-2.5 text-left font-semibold">
                    Status
                  </th>
                  <th scope="col" className="px-3 py-2.5 text-left font-semibold">
                    Golden → run
                  </th>
                  <th scope="col" className="px-3 py-2.5 text-right font-semibold">
                    Δ class
                  </th>
                  <th scope="col" className="px-3 py-2.5 text-right font-semibold">
                    Render vs render
                  </th>
                  <th scope="col" className="px-3 py-2.5 text-left font-semibold">
                    First IR difference
                  </th>
                  <th scope="col" className="px-3 py-2.5 text-left font-semibold">
                    Allowlist
                  </th>
                </tr>
              </thead>
              <tbody>
                {rows.length === 0 && (
                  <tr>
                    <td colSpan={7} className="px-3 py-8 text-center text-muted">
                      No row has these statuses.
                    </td>
                  </tr>
                )}
                {rows.map(({ id, row, classDelta, failing: fails, allowed }) => (
                  <tr
                    key={id}
                    className={cx(
                      'border-t border-line-subtle align-top',
                      fails && 'bg-[color-mix(in_srgb,var(--oc-failed-fill)_7%,transparent)]',
                    )}
                  >
                    <td className="px-3 py-2">
                      <button
                        type="button"
                        onClick={() => set({ row: id })}
                        className={cx(
                          'whitespace-nowrap text-left font-mono text-[13px] hover:text-brand-300',
                          focusRing,
                        )}
                      >
                        {id}
                      </button>
                      {fails && <span className="ml-2 text-[11px] font-semibold text-danger">fails check</span>}
                      {row.guards.length > 0 && (
                        <p className="mt-0.5 font-mono text-[11px] text-muted">{row.guards.join('; ')}</p>
                      )}
                    </td>
                    <td className="px-3 py-2">
                      <ParityBadge status={row.status} size="xs" />
                    </td>
                    <td className="px-3 py-2">
                      <span className="inline-flex items-center gap-1.5">
                        <OutcomeBadge outcome={row.reference} size="xs" />
                        <span aria-label="to">→</span>
                        {row.outcome ? (
                          <OutcomeBadge outcome={row.outcome} size="xs" />
                        ) : (
                          <span className="text-muted">{DASH}</span>
                        )}
                      </span>
                    </td>
                    <td className="px-3 py-2 text-right font-mono tabular-nums">
                      {classDelta === null
                        ? DASH
                        : classDelta > 0
                          ? `▲ ${classDelta}`
                          : classDelta < 0
                            ? `▼ ${-classDelta}`
                            : '0'}
                    </td>
                    <td className="whitespace-nowrap px-3 py-2 text-right font-mono text-xs tabular-nums text-fg-secondary">
                      {row.renderVsRender
                        ? `SSIM ${formatNumber(row.renderVsRender.ssim)} · ΔE ${formatNumber(row.renderVsRender.deltaE)}`
                        : DASH}
                    </td>
                    <td className="max-w-[16rem] break-all px-3 py-2 font-mono text-xs text-fg-secondary">
                      {row.firstIrDiff ?? DASH}
                    </td>
                    <td className="max-w-[18rem] px-3 py-2 text-xs">
                      {allowed ? (
                        <span>
                          <span className="text-fg-secondary">{allowed.reason}</span>
                          <span className="block text-muted">
                            review by {allowed.reviewBy}
                            {allowed.status ? ` · for ${allowed.status}` : ''}
                          </span>
                        </span>
                      ) : (
                        <span className="text-muted">{DASH}</span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {(report.compiler?.unreferenced.length ?? 0) > 0 && (
            <p className="text-xs text-muted">
              Rows the golden reference does not have:{' '}
              <span className="font-mono">{report.compiler?.unreferenced.join(', ')}</span>
            </p>
          )}
          <ul className="grid grid-cols-1 gap-x-6 gap-y-1 text-xs sm:grid-cols-2">
            {PARITY_STATUSES.map((status) => (
              <li key={status}>
                <span className="font-semibold" style={{ color: `var(--parity-${status})` }}>
                  {status}
                </span>{' '}
                <span className="text-muted">{PARITY_GLOSS[status]}</span>
              </li>
            ))}
          </ul>
        </div>
      </Panel>

      <div className="grid grid-cols-1 gap-6 xl:grid-cols-2">
        <Panel
          title="C · IR parity"
          subtitle="Per container, where the run's IR first departs from the golden IR (diagnostic only)."
        >
          {containers.length === 0 ? (
            <Empty>No IR parity in this report.</Empty>
          ) : (
            <table className="w-full text-sm">
              <thead className="text-xs uppercase tracking-wide text-muted">
                <tr>
                  <th scope="col" className="py-1.5 text-left font-semibold">
                    Container
                  </th>
                  <th scope="col" className="py-1.5 text-left font-semibold">
                    Status
                  </th>
                  <th scope="col" className="py-1.5 text-left font-semibold">
                    First difference
                  </th>
                </tr>
              </thead>
              <tbody>
                {containers.map(([id, entry]) => (
                  <tr key={id} className="border-t border-line-subtle">
                    <td className="py-1.5 pr-3 font-mono text-xs">{id}</td>
                    <td className="py-1.5 pr-3 text-fg-secondary">{entry.status}</td>
                    <td className="break-all py-1.5 font-mono text-xs text-fg-secondary">{entry.path ?? DASH}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Panel>
        <Panel title="Allowlist" subtitle="Reviewed, intentional divergences (bench/parity/allow.json).">
          {allowlist.length === 0 ? (
            <Empty>The allowlist is empty.</Empty>
          ) : (
            <ul className="space-y-2 text-sm">
              {allowlist.map((entry) => (
                <li key={entry.row} className="rounded-md border border-line/70 px-3 py-2">
                  <p className="font-mono text-[13px]">{entry.row}</p>
                  <p className="text-fg-secondary">{entry.reason}</p>
                  <p className="text-xs text-muted">
                    {entry.path} · review by {entry.reviewBy}
                    {entry.status ? ` · for ${entry.status}` : ''}
                  </p>
                </li>
              ))}
            </ul>
          )}
        </Panel>
      </div>
    </div>
  );
}

export function Parity({ run, state, set }: ViewProps) {
  const view = useResource<ParityView | null>(`parity:${run.key}`, (source) => source.parity(run.key));
  return (
    <Await resource={view} what="the parity report">
      {(value) =>
        value ? (
          <Levels view={value} state={state} set={set} />
        ) : (
          <Empty>
            This run has no parity report. <span className="font-mono">slidock-bench parity check</span> writes one for
            a static run under the reference's renderer and font set.
          </Empty>
        )
      }
    </Await>
  );
}
