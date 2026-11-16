// Before the tests (the web servers are up by then): checks that the app's server serves a build for `/`, builds the
// CLI (cargo rebuilds only what changed, and embeds the player the web server's build step just built), writes the
// smoke deck, and compiles every deck with the CLI to a zip, unpacked next to it, with
// Google Fonts off (support/decks.ts `CLI_FONT_ARGS`). With `SLIDOCK_E2E_FIREFOX=docker` or `SLIDOCK_E2E_WEBKIT=docker`
// it also starts the browser server in the pinned image (packages/player/e2e/support/browserServer.ts), and stops it
// after.
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';

import { startBrowserServer } from '../../../packages/player/e2e/support/browserServer';
import { smokePptx } from './fixture/pptx';
import {
  APP_PORT,
  BROWSER_SERVER_PORT,
  CLI,
  CLI_FONT_ARGS,
  OUT,
  REPO,
  cliZip,
  deckCases,
  deckDir,
  siteDir,
  smokeCase,
} from './support/decks';
import { expectBuiltFor } from './support/dist';

const DECK_JSON = /<script type="application\/json" id="sd-deck">([\s\S]*?)<\/script>/;

/**
 * The browser server's one font of its own, beside the image's: the font matrix's `shadowed` deck names it in
 * `local()`, as a font of this computer that must not draw in place of a shipped file. The image has no DejaVu Sans,
 * and a check that relied on the host's fonts would find nothing to catch there.
 */
const LOCAL_FONT = join(REPO, 'crates/slidock-substitutes/fonts/DejaVuSans.woff2');

export default async function globalSetup() {
  const served = await (await fetch(`http://localhost:${APP_PORT}/`)).text();
  expectBuiltFor(
    served,
    '/',
    `the app on port ${APP_PORT}`,
    'this suite needs a build for / (the Pages dry run leaves one for /slidock/ in dist): run it without ' +
      'SLIDOCK_E2E_SKIP_BUILD=1, or stop the server that holds the port',
  );
  if (process.env.SLIDOCK_E2E_SKIP_BUILD !== '1') {
    execFileSync('cargo', ['build', '--release', '-p', 'slidock-cli'], { cwd: REPO, stdio: 'inherit' });
  }
  const smoke = smokeCase().input;
  mkdirSync(dirname(smoke), { recursive: true });
  writeFileSync(smoke, smokePptx());

  for (const deck of deckCases()) {
    const dir = deckDir(deck);
    rmSync(dir, { recursive: true, force: true });
    mkdirSync(dir, { recursive: true });
    const zip = cliZip(deck);
    // Exit 1 reports errors or failed containers in a deck that still printed; 2 is a refused input.
    const run = spawnSync(CLI, ['compile', deck.input, '-o', zip, ...CLI_FONT_ARGS], {
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
    });
    if (run.status !== 0 && run.status !== 1) {
      throw new Error(`slidock compile ${deck.name} exited ${run.status}: ${run.stderr.slice(-2000)}`);
    }
    execFileSync('unzip', ['-q', zip, '-d', siteDir(deck)]);
    const html = readFileSync(join(siteDir(deck), 'index.html'), 'utf8');
    const json = DECK_JSON.exec(html)?.[1];
    if (json === undefined) {
      throw new Error(`${deck.name}: the CLI's index.html has no #sd-deck`);
    }
    writeFileSync(join(dir, 'deck.json'), json);
  }
  mkdirSync(OUT, { recursive: true });
  return startBrowserServer({
    port: BROWSER_SERVER_PORT,
    mounts: [REPO, ...(OUT.startsWith(REPO) ? [] : [OUT])],
    fonts: [[LOCAL_FONT, 'DejaVuSans.woff2']],
  });
}
