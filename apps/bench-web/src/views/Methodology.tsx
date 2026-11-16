import { Panel } from '@slidock/ui';
import type { ReactNode } from 'react';

import { CopyCommand } from '../components/Controls';
import { OutcomeBadge } from '../components/OutcomeBadge';
import { useBenchData } from '../data/resource';
import { EROSION, FRONTEND_SSIM_DROP, GRADIENT_APPROXIMATED, SHADOW_NOT_DRAWN } from '../model/compare';
import { DASH, formatNumber, short } from '../model/format';
import { OUTCOMES, outcomeGloss } from '../model/outcomes';
import { LANES, LANE_GLOSS, LANE_LABELS } from '../model/runs';
import type { ViewProps } from './context';

function Prose({ children }: { children: ReactNode }) {
  return <div className="max-w-3xl space-y-3 text-sm leading-relaxed text-fg-secondary">{children}</div>;
}

/** The bench's documentation, for the readers of a published report. */
const BENCH_README = 'https://github.com/macabeus/slidock/blob/main/bench/README.md';

export function Methodology({ results, run, thresholds }: ViewProps) {
  const published = Boolean(useBenchData().source.scope?.public);
  const stamp = results?.stamp;
  const n = formatNumber;
  return (
    <div className="space-y-6">
      <Panel title="What a row is" subtitle="The bench's unit of measurement.">
        <Prose>
          <p>
            A row is one shown slide of one corpus presentation through one frontend:{' '}
            <code className="font-mono text-fg">&lt;presentation&gt;:&lt;slide&gt;:&lt;frontend&gt;</code>. Motion rows
            add a build stage (<code className="font-mono">#s2</code>) or a transition frame (
            <code className="font-mono">#t@500</code>). The rows are frozen in the corpus manifest, and each one is
            scored against a fixed oracle page: Google Slides' PDF export, or Keynote's own PDF export.
          </p>
          <p>
            slidock compiles the presentation, the bench renders each slide's composed document in a pinned Chromium (a
            pinned Playwright container image), and the metrics compare that render with the oracle raster at 960 px on
            the long side. Outcome classes come before numbers: a row is first failed, declined or a render error, and
            only a scored row gets a match, close or mismatch class.
          </p>
        </Prose>
      </Panel>

      <Panel
        title="Classes"
        subtitle={`${stamp?.classifier ?? 'The classifier'}: the order a row is classed in, best first.`}
      >
        <ul className="space-y-2">
          {OUTCOMES.map((outcome) => (
            <li key={outcome} className="flex flex-wrap items-baseline gap-2 text-sm">
              <OutcomeBadge outcome={outcome} />
              <span className="text-fg-secondary">{outcomeGloss(outcome, thresholds)}</span>
            </li>
          ))}
        </ul>
        {!thresholds && (
          <p className="mt-3 text-xs text-muted">
            This UI does not know the thresholds of {stamp?.classifier ?? 'this classifier'}.
          </p>
        )}
      </Panel>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <Panel
          title="Metrics"
          subtitle="Deterministic to the bit, held by slidock's scoring goldens; their versions are stamped in every results file."
        >
          <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-2 text-sm">
            <dt className="font-mono text-xs text-fg">{stamp?.metrics.ssim ?? 'ssim'}</dt>
            <dd className="text-fg-secondary">
              Luma SSIM over 8 × 8 windows at stride 4: the mean, and the 1st percentile (p1) of the windows.
            </dd>
            <dt className="font-mono text-xs text-fg">{stamp?.metrics.color ?? 'color'}</dt>
            <dd className="text-fg-secondary">
              CIEDE2000 between the mean Lab of each 8 × 8 tile: the mean and the 99th percentile.
            </dd>
            <dt className="font-mono text-xs text-fg">{stamp?.metrics.text ?? 'text'}</dt>
            <dd className="text-fg-secondary">
              Oracle words (from the PDF's text layer) aligned with the render's words; recall counts the matched words
              that show ink, and the p95 distance (‰ of the slide's long side) says how well they are placed.
            </dd>
            <dt className="font-mono text-xs text-fg">{stamp?.metrics.pixel ?? 'pixel'}</dt>
            <dd className="text-fg-secondary">pixelmatch's share of equal pixels, reported only.</dd>
          </dl>
        </Panel>
        <Panel
          title="Regression policy"
          subtitle="What slidock-bench regression fails on, and the tolerances the overview's deltas use."
        >
          <Prose>
            <p>
              A row regresses when its class moves down the rank (failed &lt; render error &lt; declined &lt; mismatch
              &lt; close &lt; match), when more of its shadows go undrawn ({SHADOW_NOT_DRAWN.join(', ')}) or when more
              of its gradients are approximated ({GRADIENT_APPROXIMATED.join(', ')}), and erodes when it keeps its class
              but its SSIM mean drops by more than {n(-EROSION.ssimMean)}, its text recall by more than{' '}
              {n(-EROSION.textRecall)}, its ΔE rises by more than {n(EROSION.colorMean)}, its p95 placement by more than{' '}
              {n(EROSION.textP95Pm)}‰, the RMSE of its worst shadow ring by more than {n(EROSION.shadowWorstRmse)}, the
              RMSE of its worst gradient region by more than {n(EROSION.gradientWorstRmse)}, or a gradient region goes.
            </p>
            <p>
              Per frontend, the SSIM median or p10 over the rows both runs hold, reference rows apart, may drop by at
              most {n(FRONTEND_SSIM_DROP)}. Improved and undeclined rows fail too, so the baseline is refreshed on
              purpose. A change to a row the baseline flags reference or known loss is reported but stays out of the
              decision; a row the run newly flags fails. Two runs compare only when their stamps agree on everything but
              the compiler. The overview's SSIM deltas read as unchanged within the same bound; its text recall and
              match share deltas show any move.
            </p>
          </Prose>
        </Panel>
      </div>

      <Panel
        title="Lanes"
        subtitle="The static lane scores each slide against its oracle; the others hold the player, the fonts and the download."
      >
        <dl className="grid grid-cols-1 gap-x-6 gap-y-2 text-sm sm:grid-cols-[max-content_1fr]">
          {LANES.map((lane) => (
            <div key={lane} className="contents">
              <dt className="font-semibold text-fg">{LANE_LABELS[lane]}</dt>
              <dd className="text-fg-secondary">{LANE_GLOSS[lane]}</dd>
            </div>
          ))}
        </dl>
      </Panel>

      <Panel
        title="Parity with the goldens"
        subtitle="Every compile is held to slidock's goldens: its own outputs, frozen and pinned by sha256."
      >
        <Prose>
          <p>
            <code className="font-mono">slidock-bench parity reference</code> renders and scores the goldens'
            static-lane output with this bench. <strong className="text-fg">B</strong> compares each row of a run with
            that reference: identical, pixel-identical, drifting better or in the same class, regressed, missing or new.{' '}
            <strong className="text-fg">C</strong> names the first IR path where the run's IR departs from the golden
            IR. An intended change records new goldens; a reviewed divergence is listed in the allowlist with a reason
            and a review date.
          </p>
        </Prose>
      </Panel>

      <Panel title="This run" subtitle="The stamp: every input that can move a score.">
        {stamp ? (
          <dl className="grid grid-cols-1 gap-x-6 gap-y-1 text-sm sm:grid-cols-[max-content_1fr]">
            {(
              [
                ['Compiler', `${stamp.compiler.commit}${stamp.compiler.dirty ? ' (+dirty)' : ''}`],
                ['Content id', stamp.compiler.content],
                ['Harness', stamp.harness],
                ['Chromium', stamp.renderer.chromium],
                ['Poppler', stamp.renderer.poppler],
                ['Renderer recipe', stamp.renderer.recipe],
                ['Platform', `${stamp.renderer.platformDigest} (${stamp.renderer.arch})`],
                ['Raster / metric size', `${stamp.renderer.rasterLongSidePx} / ${stamp.renderer.metricLongSidePx} px`],
                ['Font set', stamp.fonts.fontSetId],
                ['Selection', stamp.selection.tiers.join(', ')],
                ['Determinism report', stamp.determinism ?? DASH],
                ['Run key', run.key],
              ] as const
            ).map(([term, value]) => (
              <div key={term} className="contents">
                <dt className="text-muted">{term}</dt>
                <dd className="break-all font-mono text-xs leading-5 text-fg-secondary">{value}</dd>
              </div>
            ))}
          </dl>
        ) : (
          <p className="text-sm text-muted">
            The bench no longer holds this run's results; its key is {short(run.key, 12)}.
          </p>
        )}
      </Panel>

      {published ? (
        <Panel title="Reproduce" subtitle="The bench runs on any decks you have the right to use.">
          <Prose>
            <p>
              The corpus behind this report is private.{' '}
              <a className="underline hover:text-fg" href={BENCH_README}>
                The bench's README
              </a>{' '}
              says how to score your own decks against their oracles.
            </p>
          </Prose>
        </Panel>
      ) : (
        <Panel title="Reproduce" subtitle="The commands behind these numbers.">
          <div className="space-y-2">
            <CopyCommand command="slidock-bench run" label="run command" />
            <CopyCommand command="slidock-bench regression --base HEAD" label="regression command" />
            <CopyCommand command="slidock-bench parity check" label="parity command" />
            <CopyCommand command="slidock-bench report" label="report command" />
          </div>
        </Panel>
      )}
    </div>
  );
}
