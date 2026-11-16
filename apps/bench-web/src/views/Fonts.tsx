import { Panel, cx, focusRing } from '@slidock/ui';
import { useState } from 'react';

import { CopyCommand, Select, Stat } from '../components/Controls';
import { OutcomeBadge } from '../components/OutcomeBadge';
import { OutcomeBar } from '../components/OutcomeBar';
import { Await, Empty } from '../components/States';
import { useResource } from '../data/resource';
import {
  FONTS_RENDERER_NOTES,
  GLYPH_SOURCES,
  GLYPH_SOURCE_LABELS,
  type PairFigures,
  equivalenceCount,
  familyFigures,
  fontRows,
  fontsRenderer,
  fontsRun,
  formatBytes,
  laneTotals,
  pairFigures,
  share,
  sourceSegments,
} from '../model/fonts';
import { DASH, formatPercent, formatTime, short } from '../model/format';
import type { GlyphCounts, Results, RunEntry } from '../schema';
import { FONT_SOURCE_COLORS, OUTCOME_COLORS } from '../theme';
import type { ViewProps } from './context';

const CLASS_COLORS = {
  exact: OUTCOME_COLORS['fonts-complete'].fill,
  substitute: FONT_SOURCE_COLORS.substitute.fill,
  fallback: FONT_SOURCE_COLORS.fallback.fill,
};

/** Glyphs by class: exact, substitute, the renderer's fallback. */
function ClassBar({ counts, label, size = 'md' }: { counts: GlyphCounts; label: string; size?: 'md' | 'sm' }) {
  return (
    <OutcomeBar
      label={label}
      size={size}
      segments={[
        { key: 'exact', count: counts.exact, color: CLASS_COLORS.exact, label: 'exact' },
        { key: 'substitute', count: counts.substitute, color: CLASS_COLORS.substitute, label: 'substitute' },
        { key: 'fallback', count: counts.fallback, color: CLASS_COLORS.fallback, label: 'renderer fallback' },
      ]}
    />
  );
}

/** Glyphs by the source of the face that drew them. */
function SourceBar({ counts, label, size = 'sm' }: { counts: GlyphCounts; label: string; size?: 'md' | 'sm' }) {
  return (
    <OutcomeBar
      label={label}
      size={size}
      segments={sourceSegments(counts).map(({ source, count }) => ({
        key: source,
        count,
        color: FONT_SOURCE_COLORS[source].fill,
        label: GLYPH_SOURCE_LABELS[source],
      }))}
    />
  );
}

function Swatch({ color, label }: { color: string; label: string }) {
  return (
    <li className="flex items-center gap-1.5">
      <span aria-hidden className="size-2.5 shrink-0 rounded-sm" style={{ backgroundColor: color }} />
      <span className="text-fg-secondary">{label}</span>
    </li>
  );
}

function ClassLegend() {
  return (
    <ul className="flex flex-wrap gap-x-4 gap-y-1 text-xs" aria-label="Glyph classes">
      <Swatch color={CLASS_COLORS.exact} label="exact: a shipped face of the requested family" />
      <Swatch color={CLASS_COLORS.substitute} label="substitute: a bundled face standing in" />
      <Swatch color={CLASS_COLORS.fallback} label="renderer fallback: a font the deck does not ship" />
    </ul>
  );
}

function SourceLegend() {
  return (
    <ul className="flex flex-wrap gap-x-4 gap-y-1 text-xs" aria-label="Face sources">
      {GLYPH_SOURCES.map((source) => (
        <Swatch key={source} color={FONT_SOURCE_COLORS[source].fill} label={GLYPH_SOURCE_LABELS[source]} />
      ))}
    </ul>
  );
}

const th = 'px-3 py-2.5 font-semibold';
const td = 'px-3 py-2';
const num = 'px-3 py-2 text-right font-mono tabular-nums';

function Attribution({ pairs, results }: { pairs: PairFigures[]; results: Results }) {
  const totals = laneTotals(pairs);
  const { counts } = totals;
  const frontends = Object.entries(results.summary.lanes?.fonts ?? {}).filter(
    (entry): entry is [string, NonNullable<(typeof entry)[1]>] => entry[1] !== undefined,
  );
  const renderer = fontsRenderer(results);
  const compared = equivalenceCount(results);
  return (
    <Panel
      title="Glyph attribution"
      subtitle={`Every glyph of every row, by the face that drew it: ${FONTS_RENDERER_NOTES[renderer]}`}
      headerRight={
        <span
          className={cx(
            'whitespace-nowrap rounded-full border px-2.5 py-1 text-xs font-medium',
            renderer === 'native' ? 'border-warning/40 text-warning' : 'border-line text-fg-secondary',
          )}
          data-renderer={renderer}
        >
          {renderer === 'native' ? 'Native renderer (host fonts)' : 'No-fonts container'}
        </span>
      }
    >
      <div className="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
        <Stat label="Glyphs" value={counts.glyphs.toLocaleString('en-US')} hint={`${totals.rows} rows`} />
        <Stat
          label="Exact"
          value={formatPercent(share(counts.exact, counts.glyphs))}
          hint={`${counts.exact.toLocaleString('en-US')} glyphs`}
          accent
        />
        <Stat
          label="Substitute"
          value={formatPercent(share(counts.substitute, counts.glyphs))}
          hint={`${counts.substitute.toLocaleString('en-US')} glyphs`}
        />
        <Stat
          label="Renderer fallback"
          value={counts.fallback.toLocaleString('en-US')}
          hint={counts.fallback === 0 ? 'none: every glyph shipped' : `${totals.rows - totals.complete} rows`}
        />
        <Stat
          label="Coverage gaps"
          value={counts.coverageGaps.toLocaleString('en-US')}
          hint="glyphs the bound face lacks"
        />
        <Stat label="Font bytes" value={formatBytes(totals.fontBytes)} hint={`${totals.faces} faces shipped`} />
      </div>
      <div className="mt-5 space-y-3">
        {frontends.map(([frontend, summary]) => (
          <div key={frontend} className="grid grid-cols-[6rem_1fr] items-center gap-3 sm:grid-cols-[7rem_1fr_9rem]">
            <span className="text-sm font-medium text-fg-secondary">{frontend}</span>
            <ClassBar counts={summary.total} label={`${frontend} glyphs by class`} />
            <span className="hidden text-right font-mono text-xs tabular-nums text-muted sm:block">
              {summary.complete}/{summary.rows} rows complete
            </span>
          </div>
        ))}
        <ClassLegend />
        <p className="text-xs text-muted">
          {compared === null
            ? 'Equivalence with the static lane: not computed for this run; it needs both renders from the container renderer.'
            : `Equivalence with the static lane: ${compared} rows compared.`}
        </p>
      </div>
    </Panel>
  );
}

function Decks({
  pairs,
  selected,
  onSelect,
}: {
  pairs: PairFigures[];
  selected: string | null;
  onSelect(pair: string | null): void;
}) {
  return (
    <Panel title="Decks" subtitle="Per corpus pair and probe deck: its rows, their glyphs, and the faces it shipped.">
      <div className="relative overflow-x-auto rounded-xl border border-line">
        <table className="w-full min-w-[56rem] text-sm">
          <thead className="bg-surface text-xs uppercase tracking-wide text-muted">
            <tr>
              <th scope="col" className={cx(th, 'text-left')}>
                Deck
              </th>
              <th scope="col" className={cx(th, 'text-right')}>
                Rows complete
              </th>
              <th scope="col" className={cx(th, 'text-right')}>
                Glyphs
              </th>
              <th scope="col" className={cx(th, 'text-right')}>
                Exact
              </th>
              <th scope="col" className={cx(th, 'text-right')}>
                Fallback
              </th>
              <th scope="col" className={cx(th, 'w-[14rem] text-left')}>
                Source mix
              </th>
              <th scope="col" className={cx(th, 'text-right')}>
                Faces
              </th>
              <th scope="col" className={cx(th, 'text-right')}>
                Font bytes
              </th>
            </tr>
          </thead>
          <tbody>
            {pairs.map((figures) => {
              const active = figures.pair === selected;
              return (
                <tr
                  key={figures.pair}
                  className={cx('border-t border-line-subtle', active && 'bg-elevated/60')}
                  aria-selected={active}
                >
                  <td className={td}>
                    <button
                      type="button"
                      onClick={() => onSelect(active ? null : figures.pair)}
                      className={cx('font-mono text-[13px] hover:text-brand-300', focusRing)}
                    >
                      {figures.pair}
                    </button>
                    {figures.failed > 0 && (
                      <span className="ml-2 text-[11px] text-danger">{figures.failed} failed</span>
                    )}
                  </td>
                  <td className={num}>
                    {figures.complete}/{figures.rows}
                  </td>
                  <td className={num}>{figures.counts.glyphs.toLocaleString('en-US')}</td>
                  <td className={num}>{formatPercent(share(figures.counts.exact, figures.counts.glyphs))}</td>
                  <td className={cx(num, figures.counts.fallback > 0 && 'text-danger')}>{figures.counts.fallback}</td>
                  <td className={td}>
                    <SourceBar counts={figures.counts} label={`${figures.pair} glyphs by source`} />
                  </td>
                  <td className={num}>{figures.report?.faces.length ?? DASH}</td>
                  <td className={num}>{figures.report ? formatBytes(figures.report.fontBytes) : DASH}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <div className="mt-3">
        <SourceLegend />
      </div>
    </Panel>
  );
}

function Families({
  pairs,
  selected,
  onSelect,
}: {
  pairs: PairFigures[];
  selected: string | null;
  onSelect(pair: string | null): void;
}) {
  const families = familyFigures(pairs, selected);
  return (
    <Panel
      title="Families"
      subtitle={`The families the slides name${selected ? ` in ${selected}` : ''}: where the faces that drew their glyphs came from.`}
      headerRight={
        <Select label="Deck" value={selected ?? ''} onChange={(value) => onSelect(value === '' ? null : value)}>
          <option value="">Every deck</option>
          {pairs.map((figures) => (
            <option key={figures.pair} value={figures.pair}>
              {figures.pair}
            </option>
          ))}
        </Select>
      }
    >
      {families.length === 0 ? (
        <Empty>No glyph was attributed.</Empty>
      ) : (
        <ul className="space-y-2.5">
          {families.map(({ family, counts }) => (
            <li key={family} className="grid grid-cols-[9rem_1fr] items-center gap-3 sm:grid-cols-[12rem_1fr_12rem]">
              <span className="truncate text-sm text-fg" title={family}>
                {family}
              </span>
              <SourceBar counts={counts} label={`${family} glyphs by source`} size="md" />
              <span className="hidden text-right font-mono text-xs tabular-nums text-muted sm:block">
                {counts.glyphs.toLocaleString('en-US')} glyphs · {formatPercent(share(counts.exact, counts.glyphs))}{' '}
                exact
                {counts.coverageGaps > 0 ? ` · ${counts.coverageGaps} gaps` : ''}
              </span>
            </li>
          ))}
        </ul>
      )}
    </Panel>
  );
}

function Faces({ pairs, selected }: { pairs: PairFigures[]; selected: string }) {
  const report = pairs.find((figures) => figures.pair === selected)?.report;
  if (!report) {
    return null;
  }
  const notices = Object.entries(report.notices);
  return (
    <Panel
      title={`Shipped faces of ${selected}`}
      subtitle={`${report.faces.length} faces, ${formatBytes(report.fontBytes)} of WOFF2 (${formatBytes(report.cssBytes)} as the deck's <style id="sd-fonts">).`}
    >
      <div className="relative overflow-x-auto rounded-xl border border-line">
        <table className="w-full min-w-[44rem] text-sm">
          <thead className="bg-surface text-xs uppercase tracking-wide text-muted">
            <tr>
              <th scope="col" className={cx(th, 'text-left')}>
                Family
              </th>
              <th scope="col" className={cx(th, 'text-left')}>
                Weight, style
              </th>
              <th scope="col" className={cx(th, 'text-left')}>
                Source
              </th>
              <th scope="col" className={cx(th, 'text-left')}>
                PostScript name
              </th>
              <th scope="col" className={cx(th, 'text-right')}>
                Bytes
              </th>
              <th scope="col" className={cx(th, 'text-left')}>
                WOFF2
              </th>
            </tr>
          </thead>
          <tbody>
            {report.faces.map((face) => (
              <tr key={face.woff2Sha256 + face.family + face.weight} className="border-t border-line-subtle">
                <td className={td}>{face.family}</td>
                <td className={cx(td, 'font-mono text-xs')}>
                  {face.weight} {face.style}
                </td>
                <td className={td}>
                  <span className="inline-flex items-center gap-1.5 text-xs text-fg-secondary">
                    <span
                      aria-hidden
                      className="size-2.5 rounded-sm"
                      style={{ backgroundColor: FONT_SOURCE_COLORS[face.source].fill }}
                    />
                    {GLYPH_SOURCE_LABELS[face.source]}
                  </span>
                </td>
                <td className={cx(td, 'font-mono text-xs text-fg-secondary')}>{face.postscriptName}</td>
                <td className={num}>{formatBytes(face.bytes)}</td>
                <td className={cx(td, 'font-mono text-xs text-muted')} title={face.woff2Sha256}>
                  {short(face.woff2Sha256, 12)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {report.requested.some((face) => face.source === null) && (
        <p className="mt-3 text-xs text-muted">
          No file ships for{' '}
          {report.requested
            .filter((face) => face.source === null)
            .map((face) => `${face.family} ${face.weight} ${face.style}`)
            .join(', ')}
          : another face of the family draws them, synthesized.
        </p>
      )}
      {notices.length > 0 && (
        <p className="mt-2 text-xs text-muted">
          Font diagnostics: {notices.map(([code, count]) => `${code} ×${count}`).join(', ')}
        </p>
      )}
    </Panel>
  );
}

function Rows({
  results,
  run,
  selected,
  set,
}: {
  results: Results;
  run: RunEntry;
  selected: string | null;
  set: ViewProps['set'];
}) {
  const rows = fontRows(results, selected);
  return (
    <Panel
      title={selected ? `Rows of ${selected}` : 'Rows the renderer drew in'}
      subtitle={
        selected
          ? 'Every row of the deck, worst first. Open one for its render.'
          : 'Rows with renderer-fallback glyphs or no attribution, worst first. Pick a deck above for all its rows.'
      }
    >
      {rows.length === 0 ? (
        <Empty>Every row is fonts-complete: no glyph was drawn by a font the deck does not ship.</Empty>
      ) : (
        <div className="relative max-h-[32rem] overflow-auto rounded-xl border border-line">
          <table className="w-full min-w-[52rem] text-sm">
            <thead className="sticky top-0 bg-surface text-xs uppercase tracking-wide text-muted">
              <tr>
                <th scope="col" className={cx(th, 'text-left')}>
                  Row
                </th>
                <th scope="col" className={cx(th, 'text-left')}>
                  Class
                </th>
                <th scope="col" className={cx(th, 'text-right')}>
                  Glyphs
                </th>
                <th scope="col" className={cx(th, 'text-right')}>
                  Exact
                </th>
                <th scope="col" className={cx(th, 'text-right')}>
                  Substitute
                </th>
                <th scope="col" className={cx(th, 'text-right')}>
                  Fallback
                </th>
                <th scope="col" className={cx(th, 'w-[10rem] text-left')}>
                  Classes
                </th>
                <th scope="col" className={cx(th, 'text-left')}>
                  {fontsRenderer(results) === 'native' ? 'Host fallback fonts' : 'Fallback fonts'}
                </th>
              </tr>
            </thead>
            <tbody>
              {rows.map(([id, row]) => (
                <tr key={id} className="border-t border-line-subtle">
                  <td className={td}>
                    <button
                      type="button"
                      onClick={() => set({ run: run.key, row: id })}
                      className={cx('whitespace-nowrap font-mono text-[13px] hover:text-brand-300', focusRing)}
                    >
                      {id}
                    </button>
                  </td>
                  <td className={td}>
                    <OutcomeBadge outcome={row.outcome} size="xs" />
                  </td>
                  <td className={num}>{row.fonts?.total.glyphs ?? DASH}</td>
                  <td className={num}>{formatPercent(row.fonts?.exactShare)}</td>
                  <td className={num}>{formatPercent(row.fonts?.substituteShare)}</td>
                  <td className={cx(num, (row.fonts?.total.fallback ?? 0) > 0 && 'text-danger')}>
                    {row.fonts?.total.fallback ?? DASH}
                  </td>
                  <td className={td}>
                    {row.fonts ? <ClassBar counts={row.fonts.total} label={`${id} glyphs by class`} size="sm" /> : DASH}
                  </td>
                  <td className={cx(td, 'text-xs text-fg-secondary')}>
                    {row.fonts?.fallbackFonts.join(', ') || (row.reason ?? DASH)}
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

function FontsRun({ results, run, set }: { results: Results; run: RunEntry; set: ViewProps['set'] }) {
  const pairs = pairFigures(results);
  const [selected, setSelected] = useState<string | null>(null);
  const known = selected !== null && pairs.some((figures) => figures.pair === selected) ? selected : null;
  return (
    <div className="space-y-6">
      <Attribution pairs={pairs} results={results} />
      <Decks pairs={pairs} selected={known} onSelect={setSelected} />
      <div className={cx('grid grid-cols-1 gap-6', known && '2xl:grid-cols-2')}>
        <Families pairs={pairs} selected={known} onSelect={setSelected} />
        {known && <Faces pairs={pairs} selected={known} />}
      </div>
      <Rows results={results} run={run} selected={known} set={set} />
    </div>
  );
}

/** The fonts lane: the selected run when it is a fonts run, else the newest one. */
export function Fonts({ index, run, results, set }: ViewProps) {
  const shown = fontsRun(index, run);
  const own = shown?.key === run.key;
  const loaded = useResource<Results | null>(shown && !own ? `results:${shown.key}` : null, (source) =>
    shown && !own ? source.results(shown.key) : Promise.resolve(null),
  );
  if (!shown) {
    return (
      <Panel
        title="No fonts run yet"
        subtitle="The fonts lane bundles each deck with its fonts and attributes every glyph."
      >
        <CopyCommand command="slidock-bench run --lane fonts" label="fonts lane command" />
      </Panel>
    );
  }
  const header = !own && (
    <p className="mb-4 text-sm text-muted">
      The fonts lane's newest run, {formatTime(shown.recordedAt)} (run{' '}
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
      <FontsRun results={results} run={shown} set={set} />
    ) : (
      <Empty>The run's rows were collected.</Empty>
    );
  }
  return (
    <>
      {header}
      <Await resource={loaded} what="the fonts run's results">
        {(value) =>
          value ? <FontsRun results={value} run={shown} set={set} /> : <Empty>The run's rows were collected.</Empty>
        }
      </Await>
    </>
  );
}
