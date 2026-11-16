import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';

import { samplePayload } from '../mock/report';
import { App } from '../src/App';
import { reportSource } from '../src/data/report';
import type { BenchSource } from '../src/data/source';
import { fixture, mockSource } from './fixture';

function renderApp(hash = '', source: BenchSource = mockSource().source) {
  window.history.replaceState(null, '', `/${hash}`);
  const user = userEvent.setup();
  render(<App source={source} />);
  return { user };
}

const params = () => new URLSearchParams(location.hash.slice(1));

describe('overview', () => {
  it('shows the run, the tiles and the outcome mix, and a segment opens its rows in the explorer', async () => {
    const { user } = renderApp();
    expect(await screen.findByRole('group', { name: 'ODP rows by outcome' })).toBeTruthy();
    const run = screen.getByRole('region', { name: 'Run' });
    expect(within(run).getByText(/Chromium 148\.0\.7778\.0/)).toBeTruthy();
    expect(screen.getByText('Decks').nextElementSibling?.textContent).toBe('3');
    expect(screen.getByRole('tab', { name: 'Overview' }).getAttribute('aria-selected')).toBe('true');

    const before = history.length;
    await user.click(screen.getByRole('button', { name: '3 mismatch: open in the explorer' }));
    expect(Object.fromEntries(params())).toEqual({ tab: 'explorer', fe: 'odp', oc: 'mismatch', ref: '1' });
    expect(history.length).toBe(before + 1);
    expect(await screen.findByText(/\/ 28 rows/)).toBeTruthy();
    const table = screen.getByRole('table');
    expect(within(table).getAllByRole('row')).toHaveLength(1 + 3);
  });

  it('shows the lanes and the parity strip, and opens another lane run', async () => {
    const { user } = renderApp();
    expect(await screen.findByRole('group', { name: 'Rows by parity status' })).toBeTruthy();
    const motion = fixture.runs.runs.findLast((run) => run.lane === 'motion');
    const card = screen.getByRole('heading', { name: 'Motion' }).closest('article');
    expect(card && within(card).getByText('end states identical')).toBeTruthy();
    await user.click(within(card as HTMLElement).getByRole('button', { name: 'Open' }));
    expect(params().get('run')).toBe(motion?.key);
    expect(await screen.findByText('Motion lane')).toBeTruthy();
  });

  it('flags a run that is stale for HEAD with the command that refreshes it', async () => {
    renderApp('#run=baseline');
    expect(await screen.findByText('stale for HEAD')).toBeTruthy();
    expect(screen.getByText('slidock-bench baseline')).toBeTruthy();
  });

  it('says so when the fragment names a run the index lacks', async () => {
    renderApp('#run=0000');
    expect(await screen.findByText('The runs index has no run 0000.')).toBeTruthy();
  });
});

describe('explorer', () => {
  it('filters with replaced history entries, counts options, sorts and prints the list command', async () => {
    const { user } = renderApp('#tab=explorer');
    expect(await screen.findByText(/\/ 23 rows/)).toBeTruthy();
    const before = history.length;
    const filters = screen.getByRole('region', { name: 'Filters' });
    await user.click(within(filters).getByRole('button', { name: /^Keynote/ }));
    expect(params().get('fe')).toBe('keynote');
    expect(history.length).toBe(before);
    expect(screen.getByText(/\/ 23 rows/).textContent).toBe('8 / 23 rows');
    expect(
      screen.getByText(
        /slidock-bench list --results "\$\{SLIDOCK_BENCH_CACHE:-bench\/\.cache\}\/runs\/.+\/results\.json" --frontend keynote/,
      ),
    ).toBeTruthy();

    await user.click(screen.getByRole('button', { name: /^SSIM$/ }));
    expect(params().get('sort')).toBe('ssim');
    expect(params().get('dir')).toBe('desc');
    const header = screen.getByRole('columnheader', { name: 'SSIM' });
    expect(header.getAttribute('aria-sort')).toBe('descending');

    await user.type(within(filters).getByRole('searchbox', { name: 'Search row ids' }), 'kn-100');
    expect(params().get('q')).toBe('kn-100');
    await user.click(within(filters).getByRole('button', { name: 'Reset' }));
    expect(params().get('fe')).toBeNull();
    expect(params().get('q')).toBeNull();
    expect(params().get('sort')).toBe('ssim');
  });

  it('filters by the declined feature from the fragment', async () => {
    const { user } = renderApp('#tab=explorer&feat=element.chart');
    expect(await screen.findByText(/\/ 23 rows/)).toBeTruthy();
    expect(screen.getByText(/\/ 23 rows/).textContent).toBe('1 / 23 rows');
    const filters = screen.getByRole('region', { name: 'Filters' });
    await user.click(within(filters).getByRole('button', { name: /^element\.chart/ }));
    expect(params().get('feat')).toBeNull();
    expect(screen.getByText(/\/ 23 rows/).textContent).toBe('23 / 23 rows');
  });

  it('lists slides and motion steps apart on a motion run', async () => {
    const motion = fixture.runs.runs.findLast((run) => run.lane === 'motion');
    const { user } = renderApp(`#tab=explorer&run=${motion?.key}`);
    const steps = await screen.findByRole('radio', { name: /^Steps/ });
    await user.click(steps);
    expect(params().get('step')).toBe('steps');
    const table = screen.getByRole('table');
    expect(within(table).getAllByRole('button', { name: /^Open .*#/ }).length).toBeGreaterThan(0);
    expect(within(table).queryByRole('button', { name: /^Open [^#]*$/ })).toBeNull();
  });

  it('adds a delta column against an explicit base run', async () => {
    renderApp('#tab=explorer&base=baseline');
    expect(await screen.findByRole('columnheader', { name: 'Δ base' })).toBeTruthy();
  });
});

describe('row drawer', () => {
  it('opens a row with a pushed entry, steps with J and K, switches compare modes and closes with Escape', async () => {
    const { user } = renderApp('#tab=explorer');
    const before = history.length;
    await user.click(await screen.findByRole('button', { name: 'Open demo-basics:od-p902:odp' }));
    expect(params().get('row')).toBe('demo-basics:od-p902:odp');
    expect(history.length).toBe(before + 1);
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByRole('heading', { name: 'demo-basics:od-p902:odp' })).toBeTruthy();
    expect(within(dialog).getByText('SSIM mean')).toBeTruthy();
    expect(await within(dialog).findByRole('img', { name: 'Heatmap' })).toBeTruthy();
    expect(within(dialog).getByRole('img', { name: 'Oracle' }).getAttribute('src')).toBe(
      `api/png/${fixture.latest}/${encodeURIComponent('demo-basics:od-p902:odp')}/oracle`,
    );
    expect(within(dialog).getByText('MID5602')).toBeTruthy();

    await user.keyboard('j');
    expect(params().get('row')).toBe('demo-basics:od-p904:odp');
    await user.keyboard('k');
    expect(params().get('row')).toBe('demo-basics:od-p902:odp');

    const modes = within(screen.getByRole('dialog')).getByRole('radiogroup', { name: 'Compare mode' });
    for (const [label, mode] of [
      ['Slider', 'slider'],
      ['Onion skin', 'onion'],
      ['Difference', 'difference'],
      ['Heatmap', 'heatmap'],
    ] as const) {
      await user.click(within(modes).getByRole('radio', { name: label }));
      expect(params().get('img')).toBe(mode);
    }
    expect(screen.getByRole('img', { name: 'Render' })).toBeTruthy();
    const heatmap = screen.getByRole('img', { name: 'Heatmap' }) as HTMLImageElement;
    await user.click(within(modes).getByRole('radio', { name: 'Slider' }));
    const divider = screen.getByRole('slider');
    act(() => {
      divider.focus();
    });
    expect(heatmap).toBeTruthy();

    const depth = history.length;
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(params().get('row')).toBeNull();
    expect(history.length).toBe(depth);
  });

  it("shows an unscored row's reason and the command that produces its missing render", async () => {
    renderApp(`#tab=explorer&row=${encodeURIComponent('demo-basics:od-p904:odp')}`);
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText(/Not scored/)).toBeTruthy();
    expect(within(dialog).getByText('Reason: slide-failed (IR0901).')).toBeTruthy();
    expect(await within(dialog).findAllByText('slidock-bench run --only demo-basics:od-p904:odp')).toHaveLength(3);
  });

  it("shows the base run's render beside the head's from Compare", async () => {
    renderApp(`#tab=compare&row=${encodeURIComponent('demo-basics:od-p901:odp')}`);
    const dialog = await screen.findByRole('dialog');
    const base = await within(dialog).findByRole('img', { name: 'Base render' });
    expect(base.getAttribute('src')).toContain(`api/png/${fixture.baseline}/`);
  });
});

describe('compare', () => {
  it('compares the baseline with the latest run and lists a transition cell', async () => {
    const { user } = renderApp('#tab=compare');
    expect(await screen.findByText('Regression fails: 28 rows compared')).toBeTruthy();
    expect(
      screen.getByText(/exits 1 on 6 regressed, 7 eroded, 1 frontend SSIM guard\./, { selector: 'div' }),
    ).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'Regressed (6)' })).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'Improved (0)' })).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'Eroded (7)' })).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'Flagged (4)' })).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'Newly flagged (0)' })).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'Frontend SSIM guard (1)' })).toBeTruthy();
    expect(screen.getByText('odp ssim median 0.9407 → 0.931')).toBeTruthy();
    expect(screen.getByText('compiler.commit')).toBeTruthy();
    const cell = screen.getByRole('button', { name: '3 rows from close to mismatch' });
    await user.click(cell);
    expect(cell.getAttribute('aria-pressed')).toBe('true');
  });

  it('marks two runs of different lanes as not comparable', async () => {
    const motion = fixture.runs.runs.findLast((run) => run.lane === 'motion');
    renderApp(`#tab=compare&run=${motion?.key}`);
    expect(await screen.findByText('These results are not comparable')).toBeTruthy();
    expect(screen.getByText(/lane/, { selector: 'div' })).toBeTruthy();
  });
});

describe('parity, methodology, fonts and the placeholder', () => {
  it('lists the parity rows, filters them by status and shows allowlist entries inline', async () => {
    const { user } = renderApp('#tab=parity');
    expect(await screen.findByText('B · Compiler parity')).toBeTruthy();
    expect(screen.getAllByText('slidock reads the stored line spacing literally').length).toBe(2);
    const status = screen.getByRole('group', { name: 'Status' });
    await user.click(within(status).getByRole('button', { name: /^regressed/ }));
    expect(params().get('ps')).toBe('regressed');
    const table = screen.getAllByRole('table').find((node) => within(node).queryByText('First IR difference'));
    const rows = table ? within(table).getAllByRole('row').slice(1) : [];
    expect(rows.length).toBe(7);
    await user.click(within(rows[0] as HTMLElement).getAllByRole('button')[0] as HTMLElement);
    expect(await screen.findByRole('dialog')).toBeTruthy();
  });

  it('says so for a run without a parity report', async () => {
    renderApp('#tab=parity&run=baseline');
    expect(await screen.findByText(/This run has no parity report/)).toBeTruthy();
  });

  it("explains the method with the run's thresholds and stamp", async () => {
    renderApp('#tab=methodology');
    expect(await screen.findByText(/SSIM mean ≥ 0\.97, SSIM p1 ≥ 0\.6/)).toBeTruthy();
    expect(screen.getByText('148.0.7778.0')).toBeTruthy();
  });

  it("shows the fonts lane's attribution, decks, families and rows, and opens a row in its run", async () => {
    const { user } = renderApp('#tab=fonts');
    expect(await screen.findByText('Glyph attribution')).toBeTruthy();
    expect(screen.getByText(/The fonts lane's newest run/)).toBeTruthy();
    expect(screen.getByText('No-fonts container')).toBeTruthy();
    expect(screen.getByText(/Equivalence with the static lane: not computed/)).toBeTruthy();
    expect(screen.getByRole('group', { name: 'pptx glyphs by class' })).toBeTruthy();
    expect(screen.getByRole('group', { name: 'Calibri glyphs by source' })).toBeTruthy();
    const rows = screen.getByText('Rows the renderer drew in').closest('section') ?? document.body;
    const first = within(rows as HTMLElement).getAllByRole('button')[0] as HTMLElement;
    const id = first.textContent ?? '';
    await user.click(first);
    expect(params().get('row')).toBe(id);
    expect(params().get('run')).not.toBeNull();
    expect(await screen.findByRole('dialog')).toBeTruthy();
  });

  it("narrows to one deck's families, shipped faces and rows", async () => {
    const { user } = renderApp('#tab=fonts');
    await user.click(await screen.findByRole('button', { name: 'demo-basics:pptx' }));
    expect(screen.getByText('Shipped faces of demo-basics:pptx')).toBeTruthy();
    expect(screen.getByText('Carlito-400')).toBeTruthy();
    expect(screen.getByText(/No file ships for Inter 800 normal/)).toBeTruthy();
    expect(screen.getByText('Rows of demo-basics:pptx')).toBeTruthy();
  });

  it("shows the motion lane's verdicts, coverage and rows, filters them and opens a row's timeline", async () => {
    const { user } = renderApp('#tab=fonts');
    await screen.findByText('Glyph attribution');
    await user.click(screen.getByRole('tab', { name: 'Effects' }));
    expect(await screen.findByText('Transitions and builds')).toBeTruthy();
    expect(screen.getByText(/The motion lane's newest run/)).toBeTruthy();
    expect(screen.getByRole('group', { name: 'demo-keynote:keynote items by verdict' })).toBeTruthy();
    expect(screen.getByText('Fallback Pareto')).toBeTruthy();
    expect(screen.getByText('keynote:apple:sparkle')).toBeTruthy();
    const rows = screen.getByText('Rows by motion class').closest('section') as HTMLElement;
    expect(within(rows).getByText('9 rows')).toBeTruthy();
    await user.click(within(rows).getByRole('button', { name: 'motion mismatch' }));
    expect(within(rows).getByText('1 rows')).toBeTruthy();
    expect(within(rows).getByRole('img', { name: /#s2: end$/ })).toBeTruthy();
    await user.click(within(rows).getByRole('button', { name: 'Timeline' }));
    expect(within(rows).getByText(/step 0 · duration: expected 500, played 400/)).toBeTruthy();
    const items = screen.getByText('Items that did not play natively').closest('section') as HTMLElement;
    expect(within(items).getByText('duration 400 ms, the deck says 500 ms')).toBeTruthy();
    await user.click(within(rows).getByRole('button', { name: 'Reset' }));
    const first = within(rows).getAllByTitle('Open the row')[0] as HTMLElement;
    await user.click(first);
    expect(params().get('row')).toBe(first.textContent);
  });
});

describe('the single-file report', () => {
  it('reads everything from the inlined payload', async () => {
    const source = reportSource(samplePayload(fixture));
    renderApp(`#tab=explorer&row=${encodeURIComponent('demo-basics:od-p901:odp')}`, source);
    const dialog = await screen.findByRole('dialog');
    const oracle = await within(dialog).findByRole('img', { name: 'Oracle' });
    expect(oracle.getAttribute('src')?.startsWith('data:image/png;base64,')).toBe(true);
    expect(screen.queryByText('stale for HEAD')).toBeNull();
  });
});
