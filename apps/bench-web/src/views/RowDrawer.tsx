import { Banner, Button, Drawer, Icon, Pill } from '@slidock/ui';
import { type ReactNode, useEffect } from 'react';

import { CopyCommand } from '../components/Controls';
import { ImageCompare, type Pane } from '../components/ImageCompare';
import { OutcomeBadge } from '../components/OutcomeBadge';
import { ScoreBullets } from '../components/ScoreBullets';
import { Loading } from '../components/States';
import { useBenchData, useResource } from '../data/resource';
import { DASH, formatNumber, formatTime, short } from '../model/format';
import { isRowOutcome, outcomeGloss } from '../model/outcomes';
import { frontendLabel } from '../model/overview';
import { neighbours, parseRowId, runDirPath, stepLabel } from '../model/rows';
import type {
  ElementFinding,
  ImageFrom,
  ImageKind,
  ImageRef,
  RowDetail,
  RowId,
  RowRecord,
  RunEntry,
  Thresholds,
} from '../schema';
import type { BenchState } from '../state/url';

const FROM: Record<ImageFrom, string> = {
  run: 'from the run',
  explain: 'from slidock-bench explain',
  'oracle-cache': 'from the oracle cache',
  'golden-reference': 'from the golden reference',
};

const LABELS: Record<ImageKind, string> = {
  oracle: 'Oracle',
  render: 'Render',
  heatmap: 'Heatmap',
  golden: 'Golden render',
  'golden-diff': 'Render vs golden',
  start: 'First frame',
  mid: 'Middle frame',
  end: 'Last frame',
  still: 'Still render',
};

function paneOf(
  kind: ImageKind,
  served: ImageRef | undefined,
  fallback: string | null,
  published: boolean,
  label: string = LABELS[kind],
): Pane {
  if (served && 'url' in served) {
    const caption =
      served.from === 'explain' && served.run ? `${FROM.explain} of ${short(served.run, 8)}` : FROM[served.from];
    return { id: kind, label, src: served.url, caption };
  }
  if (served && 'missing' in served) {
    // A published report's readers have no bench: its text says why the image is not there.
    return published
      ? { id: kind, label, src: null, note: served.missing }
      : { id: kind, label, src: null, missing: served.missing };
  }
  return { id: kind, label, src: fallback };
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="space-y-2">
      <h3 className="text-[11px] font-semibold uppercase tracking-wider text-muted">{title}</h3>
      {children}
    </section>
  );
}

function Finding({ finding }: { finding: ElementFinding }) {
  const parts: string[] = [];
  if (finding.moved) {
    parts.push(`moved ${formatNumber(finding.moved.dxPm)}‰, ${formatNumber(finding.moved.dyPm)}‰`);
  }
  if (finding.resized) {
    parts.push(`height × ${formatNumber(finding.resized.heightRatio)}`);
  }
  if (finding.restyled) {
    parts.push(`${finding.restyled.words} words restyled (ΔE ${formatNumber(finding.restyled.maxDeltaE)})`);
  }
  if (finding.reflowed) {
    parts.push(`reflowed ${finding.reflowed.oracleLines} → ${finding.reflowed.renderLines} lines`);
  }
  if (finding.image) {
    parts.push(`image SSIM ${formatNumber(finding.image.ssim)}, ΔE ${formatNumber(finding.image.deltaE)}`);
  }
  if (finding.missing > 0) {
    parts.push(`${finding.missing} words missing`);
  }
  if (finding.extra > 0) {
    parts.push(`${finding.extra} extra words`);
  }
  for (const font of finding.fonts) {
    parts.push(`font ${font.family}: ${font.binding} (${font.kind})`);
  }
  return (
    <li className="rounded-md border border-line/70 px-3 py-2 text-sm">
      <p className="flex flex-wrap items-baseline gap-2">
        <code className="font-mono text-fg">{finding.el}</code>
        <span className="text-xs text-muted">
          {finding.kind}
          {finding.name ? ` · ${finding.name}` : ''} · {finding.origin}
        </span>
      </p>
      <p className="mt-0.5 text-fg-secondary">{parts.length > 0 ? parts.join(' · ') : 'no finding'}</p>
    </li>
  );
}

function Facts({ items }: { items: [string, ReactNode][] }) {
  return (
    <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1 text-sm">
      {items.map(([term, value]) => (
        <div key={term} className="contents">
          <dt className="text-muted">{term}</dt>
          <dd className="min-w-0 break-all font-mono text-xs leading-5 text-fg-secondary">{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function RowChecks({ record }: { record: RowRecord }) {
  const items: [string, ReactNode][] = [];
  if (record.shadow) {
    const shadow = record.shadow;
    items.push([
      'Shadow rings',
      `${shadow.elements}: worst RMSE ${formatNumber(shadow.worstRmse)} (${shadow.worstEl}), mean ${formatNumber(shadow.meanRmse)}`,
    ]);
  }
  if (record.gradient) {
    const gradient = record.gradient;
    items.push([
      'Gradient regions',
      `${gradient.elements}: worst RMSE ${formatNumber(gradient.worstRmse)} (${gradient.worstEl}), mean ${formatNumber(gradient.meanRmse)}`,
    ]);
  }
  if (record.host) {
    items.push(['Host', `${record.host.status}${record.host.ssim === undefined ? '' : ` (SSIM ${record.host.ssim})`}`]);
  }
  if (record.fonts) {
    const fonts = record.fonts;
    items.push([
      'Glyphs',
      `${fonts.total.glyphs}: ${fonts.total.exact} exact, ${fonts.total.substitute} substitute, ${fonts.total.fallback} renderer fallback`,
    ]);
    items.push(['Exact share', formatNumber(fonts.exactShare)]);
    if (fonts.total.coverageGaps > 0) {
      items.push(['Coverage gaps', String(fonts.total.coverageGaps)]);
    }
    if (fonts.fallbackFonts.length > 0) {
      items.push(['Fallback fonts', fonts.fallbackFonts.join(', ')]);
    }
    if (record.fonts.equivalence) {
      items.push(['Against static', record.fonts.equivalence.status]);
    }
  }
  if (record.motion) {
    const motion = record.motion;
    if (motion.effect) {
      items.push(['Effect', `${motion.effect.id} (${motion.effect.played}; fallback ${motion.effect.fallback})`]);
    }
    if (motion.ssim !== undefined) {
      items.push(['Model SSIM', formatNumber(motion.ssim)]);
    }
    if (motion.endState) {
      items.push(['End state', motion.endState.status]);
    }
    if (motion.startState !== undefined) {
      items.push(['Start state', motion.startState ? 'as the stage before left it' : 'differs from the stage before']);
    }
    for (const failure of motion.failures ?? []) {
      items.push(['Failed', failure]);
    }
    if (motion.monotonic !== undefined) {
      items.push(['Reveal', motion.monotonic ? 'monotonic' : 'not monotonic']);
    }
    if (motion.stageCount) {
      items.push(['Stages', `${motion.stageCount.expected} expected, ${motion.stageCount.oracle} in the oracle`]);
    }
  }
  if (record.shell) {
    items.push(['Shell frame', record.shell.status]);
  }
  return items.length > 0 ? (
    <Section title="Checks">
      <Facts items={items} />
    </Section>
  ) : null;
}

function Unscored({ record, thresholds }: { record: RowRecord; thresholds: Thresholds | null }) {
  const gloss = isRowOutcome(record.outcome)
    ? outcomeGloss(record.outcome, thresholds)
    : 'a class this UI does not know';
  const detail = record.declined
    ? `Declined by ${record.declined.code}${record.declined.feature ? ` (${record.declined.feature})` : ''}${record.declined.via ? `, through ${record.declined.via}` : ''}.`
    : record.reason
      ? `Reason: ${record.reason}${record.code ? ` (${record.code})` : ''}.`
      : null;
  return (
    <Banner tone="info" title={`Not scored: ${gloss}`}>
      {detail}
    </Banner>
  );
}

export interface RowDrawerProps {
  state: BenchState;
  set(change: Partial<BenchState>): void;
  run: RunEntry;
  record: RowRecord;
  id: RowId;
  /** The rows in the explorer's order, for previous and next. */
  order: readonly RowId[];
  thresholds: Thresholds | null;
  /** In base-vs-head mode (from Compare), the base run whose render is shown beside the head's. */
  base: RunEntry | null;
}

export function RowDrawer({ state, set, run, record, id, order, thresholds, base }: RowDrawerProps) {
  const { source } = useBenchData();
  const published = Boolean(source.scope?.public);
  const detail = useResource<RowDetail | null>(`row:${run.key}:${id}`, (s) => s.row(run.key, id));
  const { previous, next } = neighbours(order, id);
  const parts = parseRowId(id);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target;
      if (target instanceof HTMLElement && target.closest('input, textarea, select, [contenteditable]')) {
        return;
      }
      if (event.key === 'j' && next) {
        set({ row: next });
      } else if (event.key === 'k' && previous) {
        set({ row: previous });
      }
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [next, previous, set]);

  const loaded = detail.state === 'ready' ? detail.value : null;
  const images = loaded?.images ?? {};
  const fallback = (kind: ImageKind) => (loaded ? null : source.imageUrl(run.key, id, kind));
  const oracle = paneOf('oracle', images.oracle, fallback('oracle'), published);
  const render = paneOf('render', images.render, fallback('render'), published);
  const heatmap = paneOf('heatmap', images.heatmap, fallback('heatmap'), published);
  const extra: Pane[] = [];
  if (images.golden) {
    extra.push(paneOf('golden', images.golden, null, published));
  }
  if (base && base.key !== run.key) {
    const baseRender = source.imageUrl(base.key, id, 'render');
    extra.push({ id: 'base-render', label: 'Base render', src: baseRender, caption: formatTime(base.recordedAt) });
  }

  const subtitle = parts
    ? [parts.presentation, parts.slide, stepLabel(parts.step), frontendLabel(parts.frontend)]
        .filter(Boolean)
        .join(' · ')
    : undefined;

  return (
    <Drawer
      open
      onClose={() => set({ row: null })}
      title={<span className="font-mono">{id}</span>}
      subtitle={subtitle}
      className="max-w-6xl!"
    >
      <div className="sticky -top-6 z-10 -mx-6 -mt-6 flex flex-wrap items-center gap-2 border-b border-line bg-root px-6 py-3">
        <OutcomeBadge outcome={record.outcome} />
        {record.knownLoss?.map((kind) => (
          <Pill key={kind} size="sm">
            known loss · {kind}
          </Pill>
        ))}
        {record.reference && <Pill size="sm">reference</Pill>}
        <span className="flex-1" />
        <Button
          variant="ghost"
          size="sm"
          disabled={!previous}
          onClick={() => previous && set({ row: previous })}
          title="Previous row (K)"
        >
          <Icon name="chevron-left" className="size-4" />
          Previous
        </Button>
        <Button
          variant="ghost"
          size="sm"
          disabled={!next}
          onClick={() => next && set({ row: next })}
          title="Next row (J)"
        >
          Next
          <Icon name="chevron-right" className="size-4" />
        </Button>
      </div>

      {record.scores ? (
        <Section title="Scores against the thresholds">
          <ScoreBullets scores={record.scores} thresholds={thresholds} />
          {!thresholds && (
            <p className="text-xs text-muted">This UI does not know the thresholds of this run's classifier.</p>
          )}
        </Section>
      ) : (
        <Unscored record={record} thresholds={thresholds} />
      )}

      {detail.state === 'loading' ? (
        <Loading label="Loading the row's images…" />
      ) : (
        <ImageCompare
          oracle={oracle}
          render={render}
          heatmap={heatmap}
          extra={extra}
          mode={state.img}
          onMode={(img) => set({ img })}
        />
      )}

      <RowChecks record={record} />

      {loaded?.explain && (
        <Section title="Findings">
          {!loaded.explain.matchesRow && (
            <Banner tone="warning" title="These findings come from another record of this row">
              {published
                ? 'They were measured on an earlier render of the slide.'
                : 'Re-run the explain command below for this record.'}
            </Banner>
          )}
          <ul className="space-y-2">
            {loaded.explain.elements.map((finding) => (
              <Finding key={finding.el} finding={finding} />
            ))}
          </ul>
          {loaded.explain.missing.length > 0 && (
            <ul className="space-y-1 text-sm text-fg-secondary">
              {loaded.explain.missing.map((line) => (
                <li key={`${line.xPm}:${line.yPm}`}>
                  Missing “{line.text}” at {formatNumber(line.xPm)}‰, {formatNumber(line.yPm)}‰
                  {line.el ? ` (near ${line.el})` : ''}
                </li>
              ))}
            </ul>
          )}
        </Section>
      )}

      {loaded && loaded.knownLoss.length > 0 && (
        <Section title="Known loss">
          <ul className="space-y-1 text-sm">
            {loaded.knownLoss.map((loss) => (
              <li key={`${loss.kind}:${loss.feature}`}>
                <code className="font-mono text-fg-secondary">{loss.feature}</code>{' '}
                <span className="text-muted">
                  ({loss.kind}) {loss.note}
                </span>
              </li>
            ))}
          </ul>
        </Section>
      )}

      <Section title="Diagnostics">
        {loaded?.diagnostics ? (
          loaded.diagnostics.length === 0 ? (
            <p className="text-sm text-muted">No diagnostic applies to this row.</p>
          ) : (
            <div className="relative overflow-x-auto">
              <table className="w-full text-sm">
                <thead className="text-xs uppercase tracking-wide text-muted">
                  <tr>
                    <th scope="col" className="py-1 pr-3 text-left font-semibold">
                      Code
                    </th>
                    <th scope="col" className="py-1 pr-3 text-left font-semibold">
                      Severity
                    </th>
                    <th scope="col" className="py-1 pr-3 text-left font-semibold">
                      Message
                    </th>
                    <th scope="col" className="py-1 pr-3 text-left font-semibold">
                      Via
                    </th>
                    <th scope="col" className="py-1 text-right font-semibold">
                      Count
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {loaded.diagnostics.map((diagnostic) => (
                    <tr key={`${diagnostic.code}:${diagnostic.via ?? ''}`} className="border-t border-line-subtle">
                      <td className="py-1.5 pr-3 font-mono text-fg">{diagnostic.code}</td>
                      <td className="py-1.5 pr-3 text-fg-secondary">{diagnostic.severity}</td>
                      <td className="py-1.5 pr-3 text-fg-secondary">{diagnostic.message}</td>
                      <td className="py-1.5 pr-3 text-muted">{diagnostic.via ?? 'slide'}</td>
                      <td className="py-1.5 text-right font-mono tabular-nums">{diagnostic.count ?? 1}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )
        ) : (
          <ul className="flex flex-wrap gap-1.5">
            {Object.entries(record.diagnostics).map(([code, count]) => (
              <li key={code}>
                <Pill mono size="sm">
                  {code} × {count}
                </Pill>
              </li>
            ))}
            {Object.keys(record.diagnostics).length === 0 && <li className="text-sm text-muted">None.</li>}
          </ul>
        )}
      </Section>

      {!published && (
        <Section title="Reproduce">
          <CopyCommand
            command={loaded?.commands.explain ?? `slidock-bench explain ${id} --from ${runDirPath(run.key)}`}
            label="explain command"
          />
          <CopyCommand command={loaded?.commands.run ?? `slidock-bench run --only ${id}`} label="run command" />
          {loaded?.viewer.available && (
            <p className="text-xs text-muted">
              The corpus holds the source file, <span className="font-mono">{loaded.viewer.file}</span>.
            </p>
          )}
        </Section>
      )}

      <Section title="Provenance">
        <Facts
          items={
            // A published report's records hold no run key or hash.
            published
              ? [['Oracle page', String(record.oracle.page)]]
              : [
                  ['Run', `${run.key}${loaded ? ` (${loaded.run.state})` : ''}`],
                  ['Compiler content', record.content],
                  ['Composed document', record.html ?? DASH],
                  ['Render', record.render ?? DASH],
                  ['Oracle page', String(record.oracle.page)],
                ]
          }
        />
        <details className="rounded-md border border-line/70">
          <summary className="cursor-pointer px-3 py-2 text-sm text-fg-secondary">The record as JSON</summary>
          <pre className="max-h-[46vh] overflow-auto bg-sunken/70 p-3 font-mono text-[12px] leading-relaxed text-fg-secondary">
            {JSON.stringify(record, null, 2)}
          </pre>
        </details>
      </Section>
    </Drawer>
  );
}
