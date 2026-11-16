// The player's builds: ESM for the web app, and an IIFE (global `Slidock`) that slidock-bundle inlines in decks; and
// the presenter view's (src/presenter/), which a deck embeds only when it ships one: ESM (`@slidock/player/presenter`)
// and an IIFE (global `SlidockPresenter`).
import { readFileSync } from 'node:fs';

/** The licence banner of the player's builds (legal comments are otherwise left out). */
const PLAYER_BANNER = '/*! slidock player | (c) 2026 Bruno Macabeus | MIT License */';

/** The presenter view's: it also carries Heroicons' icon paths (src/presenter/window.ts). */
const PRESENTER_BANNER =
  '/*! slidock presenter view | (c) 2026 Bruno Macabeus | MIT License | Heroicons (c) Tailwind Labs, Inc. | MIT License */';

/** @type {import('esbuild').BuildOptions} */
const common = {
  entryPoints: ['src/index.ts'],
  bundle: true,
  minify: true,
  target: 'es2022',
  platform: 'browser',
  legalComments: 'none',
  charset: 'utf8',
  banner: { js: PLAYER_BANNER },
};

const PLAYER_API = new URL('./src/presenter/player.ts', import.meta.url);

/**
 * The presenter's builds take the player from the player's own build instead of bundling it again: its one module
 * that imports the player (`src/presenter/player.ts`, `export { … } from '../index'`) becomes an import of
 * `@slidock/player` (ESM) or a read of the `Slidock` global the deck page's player script defines (IIFE).
 *
 * @param {'esm' | 'iife'} format
 * @returns {import('esbuild').Plugin}
 */
function playerFromItsBuild(format) {
  return {
    name: 'player-from-its-build',
    setup(build) {
      build.onLoad({ filter: /[\\/]src[\\/]presenter[\\/]player\.ts$/ }, () => {
        const source = readFileSync(PLAYER_API, 'utf8');
        const names = /export \{([^}]*)\} from '\.\.\/index';/.exec(source)?.[1];
        if (names === undefined) {
          throw new Error(`${PLAYER_API.pathname}: expected one \`export { … } from '../index'\``);
        }
        const list = names
          .split(',')
          .map((name) => name.trim())
          .filter((name) => name !== '');
        return {
          contents:
            format === 'esm'
              ? `export { ${list.join(', ')} } from '@slidock/player';`
              : `export const { ${list.join(', ')} } = globalThis.Slidock;`,
          loader: 'js',
        };
      });
    },
  };
}

/** @type {import('esbuild').BuildOptions[]} */
export const builds = [
  { ...common, format: 'esm', outfile: 'dist/player.mjs' },
  { ...common, entryPoints: ['src/global.ts'], format: 'iife', outfile: 'dist/player.iife.js' },
  {
    ...common,
    entryPoints: ['src/presenter/index.ts'],
    format: 'esm',
    outfile: 'dist/presenter.mjs',
    banner: { js: PRESENTER_BANNER },
    external: ['@slidock/player'],
    plugins: [playerFromItsBuild('esm')],
  },
  {
    ...common,
    entryPoints: ['src/presenter/global.ts'],
    format: 'iife',
    outfile: 'dist/presenter.iife.js',
    banner: { js: PRESENTER_BANNER },
    plugins: [playerFromItsBuild('iife')],
  },
];
