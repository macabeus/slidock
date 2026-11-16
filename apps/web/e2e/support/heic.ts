// HEIC photos in the suite: which files of an app zip the browser decoded (the photos the CLI omits), and the zip
// the compiler writes, under Node through the compile worker, for the same input and those files. A zip with photos
// this browser decoded is held to that one: the output is the same for the same inputs, the decoded files included.
import { execFileSync } from 'node:child_process';
import { mkdirSync, rmSync } from 'node:fs';
import { basename, join } from 'node:path';

import { REPO } from './decks';

/** The names of a zip's entries. */
export function zipNames(zip: string): string[] {
  return execFileSync('unzip', ['-Z1', zip], { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
    .split('\n')
    .filter((name) => name !== '');
}

/** A decoded photo of an app zip: the need's id (its source's SHA-256) and the file, extracted to `dir`. */
export interface DecodedPhoto {
  need: string;
  file: string;
}

/** The media of `appZip` that `cliZip` lacks (the photos this browser decoded), extracted to `dir`. */
export function decodedPhotos(appZip: string, cliZip: string, dir: string): DecodedPhoto[] {
  const cli = new Set(zipNames(cliZip));
  const added = zipNames(appZip).filter((name) => name.startsWith('media/') && !cli.has(name));
  if (added.length === 0) {
    return [];
  }
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  execFileSync('unzip', ['-q', '-o', '-j', appZip, ...added, '-d', dir]);
  return added.map((name) => ({ need: basename(name).replace(/\.[a-z]+$/, ''), file: join(dir, basename(name)) }));
}

/**
 * The zip the compile worker writes under Node for `input` with `photos` added (as the app adds them) and the
 * bundled substitutes the deck names (as the app adds them with Google Fonts off): `<out>/deck/deck.zip`.
 */
export function wasmZip(input: string, photos: readonly DecodedPhoto[], out: string): string {
  execFileSync(
    'node',
    [
      '--import',
      './packages/wasm/test/node/ts-resolve.ts',
      'packages/wasm/test/node/compile.ts',
      out,
      `--core=${join(REPO, 'crates/slidock-substitutes/fonts')}`,
      ...photos.map(({ need, file }) => `--media=${need}=${file}`),
      `deck=${input}`,
    ],
    { cwd: REPO, stdio: 'pipe', maxBuffer: 64 * 1024 * 1024 },
  );
  return join(out, 'deck', 'deck.zip');
}
