// Builds the player into dist/, and copies the IIFE builds slidock-bundle embeds, with the package's version, into
// the crate (`crates/slidock-bundle/player/`), so cargo builds the CLI without Node and a packaged crate carries its
// player. esbuild writes the same bytes for the same sources and version, so a fresh build leaves the copies as they
// are.
//
//   node build.mjs            # dist/ and the crate's copies
//   node build.mjs --check    # builds in memory and fails when the crate's copies are not that build
import * as esbuild from 'esbuild';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

import { builds } from './esbuild.config.mjs';

const PACKAGE = fileURLToPath(new URL('.', import.meta.url));
const CRATE = join(PACKAGE, '..', '..', 'crates', 'slidock-bundle', 'player');
/** The builds the crate embeds, by file name in both places. */
const EMBEDDED = ['player.iife.js', 'presenter.iife.js'];

const check = process.argv.includes('--check');
const results = await Promise.all(builds.map((options) => esbuild.build({ ...options, write: false })));
/** Every output by its path relative to the package (`dist/player.iife.js`). */
const outputs = new Map(
  results.flatMap((result) => result.outputFiles.map((file) => [relative(PACKAGE, file.path), file.contents])),
);

const { version } = JSON.parse(readFileSync(join(PACKAGE, 'package.json'), 'utf8'));
/** The crate's files: each embedded build and `version.txt`. */
const crateFiles = new Map([
  ...EMBEDDED.map((name) => {
    const bytes = outputs.get(join('dist', name));
    if (bytes === undefined) {
      throw new Error(`esbuild.config.mjs has no build writing dist/${name}`);
    }
    return [name, bytes];
  }),
  ['version.txt', new TextEncoder().encode(`${version}\n`)],
]);

if (check) {
  const stale = [...crateFiles].filter(([name, bytes]) => {
    try {
      return !Buffer.from(bytes).equals(readFileSync(join(CRATE, name)));
    } catch {
      return true;
    }
  });
  if (stale.length > 0) {
    const names = stale.map(([name]) => `crates/slidock-bundle/player/${name}`).join(', ');
    console.error(`${names}: not the build of packages/player; run \`pnpm player:build\` and commit them`);
    process.exit(1);
  }
  console.log(`crates/slidock-bundle/player/ holds the build of packages/player (${version})`);
} else {
  for (const [path, bytes] of outputs) {
    mkdirSync(dirname(join(PACKAGE, path)), { recursive: true });
    writeFileSync(join(PACKAGE, path), bytes);
  }
  mkdirSync(CRATE, { recursive: true });
  for (const [name, bytes] of crateFiles) {
    writeFileSync(join(CRATE, name), bytes);
  }
}
