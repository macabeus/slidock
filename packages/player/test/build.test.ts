import * as esbuild from 'esbuild';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { gzipSync } from 'node:zlib';
import { describe, expect, it } from 'vitest';

import { builds } from '../esbuild.config.mjs';
import { version } from '../src/index';

const packageJson = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8')) as {
  version: string;
  dependencies?: Record<string, string>;
};

/**
 * The player's size budget: its IIFE's gzip -9 size, 39,605 B with the text view (page frame, deep links,
 * `data-link` clicks, the key gate and the Text link), Magic Move's pair mixing and the clips offered in more than one
 * file, raised to the whole 256 B above it.
 */
const BUDGET_BYTES = 39_680;

/**
 * The presenter view's size budget: its IIFE's gzip -9 size as the presenter export option left it (6,220 B: the
 * presenter view, the sync, and the web app's header, theme toggle and tokens) plus 3 %, in whole 256 B. A deck pays
 * it only when it ships a presenter view.
 */
const PRESENTER_BUDGET_BYTES = 6_656;

async function bundle(outfile: string): Promise<string> {
  const options = builds.find((candidate) => candidate.outfile === outfile);
  expect(options).toBeDefined();
  const { outfile: _outfile, ...rest } = options ?? {};
  const result = await esbuild.build({ ...rest, write: false });
  return result.outputFiles[0]?.text ?? '';
}

const gzip = (text: string) => gzipSync(text, { level: 9 }).length;

describe('licence banners', () => {
  it.each(['dist/player.mjs', 'dist/player.iife.js', 'dist/presenter.mjs', 'dist/presenter.iife.js'])(
    '%s starts with its licence; the presenter view names Heroicons, whose icon paths it draws',
    async (outfile) => {
      const script = await bundle(outfile);
      expect(script).toMatch(/^\/\*! slidock [a-z ]+ \| \(c\) 2026 Bruno Macabeus \| MIT License/);
      expect(script.includes('Heroicons (c) Tailwind Labs, Inc. | MIT License')).toBe(outfile.includes('presenter'));
    },
  );
});

describe('player', () => {
  it('exports the package version', () => {
    expect(version).toBe(packageJson.version);
  });

  it('has no runtime dependencies', () => {
    expect(packageJson.dependencies ?? {}).toEqual({});
  });

  it('the IIFE build defines the Slidock global with the deck-page API, and no presenter view', async () => {
    const script = await bundle('dist/player.iife.js');
    const context: { Slidock?: Record<string, unknown> } = {};
    runInNewContext(script, context);
    expect(context.Slidock?.version).toBe(packageJson.version);
    for (const name of ['mount', 'inlineDeck', 'createPlayer', 'htmlDeck', 'renderStatic']) {
      expect(typeof context.Slidock?.[name]).toBe('function');
    }
    expect(context.Slidock?.mountPresenter).toBeUndefined();
    // The presenter view's layout, its sync namespace and channel stay in the presenter's build.
    for (const text of ['sd-presenter', 'slidock-sync@1', 'BroadcastChannel', 'Presenter view']) {
      expect(script).not.toContain(text);
    }
  });

  it.each(['dist/player.mjs', 'dist/player.iife.js'])('%s fits the gzip budget', async (outfile) => {
    const size = gzip(await bundle(outfile));
    expect(size).toBeGreaterThan(1000);
    expect(size).toBeLessThanOrEqual(BUDGET_BYTES);
  });
});

describe('presenter', () => {
  it('the IIFE build defines SlidockPresenter on the page that runs the player first', async () => {
    const [player, presenter] = await Promise.all([bundle('dist/player.iife.js'), bundle('dist/presenter.iife.js')]);
    expect(() => runInNewContext(presenter, {})).toThrow();
    const context: { Slidock?: unknown; SlidockPresenter?: Record<string, unknown> } = {};
    runInNewContext(player, context);
    runInNewContext(presenter, context);
    for (const name of ['session', 'join', 'open', 'mount']) {
      expect(typeof context.SlidockPresenter?.[name]).toBe('function');
    }
  });

  it.each(['dist/presenter.mjs', 'dist/presenter.iife.js'])('%s does not bundle the player again', async (outfile) => {
    const script = await bundle(outfile);
    expect(script).not.toContain('.sd-player{');
    expect(script).not.toContain('slidock-deck@1');
    if (outfile.endsWith('.mjs')) {
      expect(script).toMatch(/from"@slidock\/player"/);
    }
  });

  it.each(['dist/presenter.mjs', 'dist/presenter.iife.js'])('%s fits the gzip budget', async (outfile) => {
    const size = gzip(await bundle(outfile));
    expect(size).toBeGreaterThan(1000);
    expect(size).toBeLessThanOrEqual(PRESENTER_BUDGET_BYTES);
  });
});

describe('slidock-bundle', () => {
  const crate = (name: string) =>
    readFileSync(new URL(`../../../crates/slidock-bundle/player/${name}`, import.meta.url));

  it.each(['player.iife.js', 'presenter.iife.js'])(
    'embeds this build of dist/%s (`pnpm player:build`)',
    async (name) => {
      expect(crate(name).toString('utf8')).toBe(await bundle(`dist/${name}`));
    },
  );

  it('embeds the package version', () => {
    expect(crate('version.txt').toString('utf8')).toBe(`${packageJson.version}\n`);
  });
});
