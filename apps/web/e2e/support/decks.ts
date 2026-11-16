// The decks the suite runs on: the smoke deck it writes itself, and, unless `SLIDOCK_E2E_SMOKE=1`, every deck of
// the private corpus (`SLIDOCK_CORPUS_DIR`, default `$SLIDOCK_PRIVATE/corpus`, `~/.slidock/corpus`). The global setup
// compiles each with the CLI and unpacks the zip; the tests read what it wrote under `<cargo target>/e2e/`.
import type { Deck, DeckSlide } from '@slidock/types';
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { homedir } from 'node:os';
import { basename, extname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { envPort } from '../../../../packages/player/e2e/support/browserServer';

export const REPO = fileURLToPath(new URL('../../../..', import.meta.url));
export const APP = join(REPO, 'apps/web');
export const TARGET = resolve(REPO, process.env.CARGO_TARGET_DIR ?? 'target');
/** Everything the suite writes: compiled decks, downloads, results. */
export const OUT = join(TARGET, 'e2e');
export const CLI = join(TARGET, 'release', process.platform === 'win32' ? 'slidock.exe' : 'slidock');

export const SMOKE = process.env.SLIDOCK_E2E_SMOKE === '1';

/**
 * The CLI's font arguments for the suite's decks: Google Fonts off, every other source on (embedded fonts, the
 * bundled substitutes). The app runs with its Google Fonts switch off (`GOOGLE_OFF`), so the two ship the same faces:
 * Google serves the CLI static files and a browser per-script chunks of other builds.
 */
export const CLI_FONT_ARGS = ['--no-google'];

/** The app's remembered Google Fonts switch (src/fonts/controller.ts `GOOGLE_PREFERENCE`), off. */
export const GOOGLE_OFF = { key: 'slidock.googleFonts', value: 'off' };
/** The download dialog's "Play transparent videos in every browser" off: a zip the CLI's can equal. */
export const COPIES_OFF = { key: 'slidock.download.copies', value: 'off' };
/** `$SLIDOCK_PRIVATE`, the maintainer's private data (default `~/.slidock`). */
export const PRIVATE = resolve(process.env.SLIDOCK_PRIVATE ?? join(homedir(), '.slidock'));
export const CORPUS = resolve(process.env.SLIDOCK_CORPUS_DIR ?? join(PRIVATE, 'corpus'));
/** The corpus manifest, which names each presentation's source files (`SLIDOCK_CORPUS_MANIFEST`, as the bench). */
export const CORPUS_MANIFEST = resolve(
  process.env.SLIDOCK_CORPUS_MANIFEST ?? join(PRIVATE, 'bench/corpus/manifest.json'),
);

interface CorpusManifest {
  presentations: { id: string; sources: { frontend: string; file: string }[] }[];
}

/**
 * The file of a corpus presentation's source for `frontend` (`pptx`, `odp`, `keynote`), by the presentation's id in
 * the corpus manifest: the repository names corpus decks by id only. `undefined` when the manifest or the file is
 * missing.
 */
export function corpusSource(id: string, frontend: string): string | undefined {
  if (!existsSync(CORPUS_MANIFEST)) {
    return undefined;
  }
  const manifest = JSON.parse(readFileSync(CORPUS_MANIFEST, 'utf8')) as CorpusManifest;
  const file = manifest.presentations
    .find((presentation) => presentation.id === id)
    ?.sources.find((source) => source.frontend === frontend)?.file;
  const input = file === undefined ? undefined : join(CORPUS, file);
  return input !== undefined && existsSync(input) ? input : undefined;
}

/** The browser server in the pinned image (`SLIDOCK_E2E_FIREFOX=docker`, `SLIDOCK_E2E_WEBKIT=docker`). */
export const BROWSER_SERVER_PORT = envPort('SLIDOCK_E2E_BROWSER_PORT', 3555);
/** The production build's server (`vite preview`). */
export const APP_PORT = envPort('SLIDOCK_E2E_PORT', 4317);
/** The dev server that serves the demo page. */
export const DEMO_PORT = envPort('SLIDOCK_E2E_DEMO_PORT', 4318);

const EXTENSIONS = new Set(['.pptx', '.key', '.odp']);

export interface DeckCase {
  /** A file-system-safe name, unique in the run. */
  key: string;
  /** The input's file name. */
  name: string;
  input: string;
  bytes: number;
  smoke: boolean;
}

export const SMOKE_NAME = 'slidock-smoke.pptx';

export function smokeCase(): DeckCase {
  const input = join(OUT, 'fixture', SMOKE_NAME);
  return {
    key: 'smoke',
    name: SMOKE_NAME,
    input,
    bytes: existsSync(input) ? statSync(input).size : 0,
    smoke: true,
  };
}

function corpusCases(): DeckCase[] {
  if (!existsSync(CORPUS)) {
    throw new Error(
      `The corpus folder ${CORPUS} is missing. Set SLIDOCK_CORPUS_DIR, ` +
        'or run the smoke subset (pnpm --filter @slidock/web e2e:smoke).',
    );
  }
  const names = readdirSync(CORPUS)
    .filter((name) => EXTENSIONS.has(extname(name).toLowerCase()) && statSync(join(CORPUS, name)).isFile())
    .sort();
  if (names.length === 0) {
    throw new Error(`The corpus folder ${CORPUS} holds no .pptx, .key or .odp file`);
  }
  const used = new Set<string>();
  return names.map((name) => {
    let key = basename(name)
      .replace(/[^A-Za-z0-9.-]+/g, '-')
      .toLowerCase();
    while (used.has(key)) {
      key += '_';
    }
    used.add(key);
    const input = join(CORPUS, name);
    return { key, name, input, bytes: statSync(input).size, smoke: false };
  });
}

/** Every deck of this run, the smoke deck first. */
export function deckCases(): DeckCase[] {
  return SMOKE ? [smokeCase()] : [smokeCase(), ...corpusCases()];
}

export function deckDir(deck: DeckCase): string {
  return join(OUT, 'decks', deck.key);
}

/** The zip the CLI wrote. */
export function cliZip(deck: DeckCase): string {
  return join(deckDir(deck), 'deck.zip');
}

/** The CLI zip unpacked: `index.html`, `media/`, `LICENSES.txt`. */
export function siteDir(deck: DeckCase): string {
  return join(deckDir(deck), 'site');
}

/** The deck JSON of the CLI's `index.html`. */
export function compiledDeck(deck: DeckCase): Deck {
  return JSON.parse(readFileSync(join(deckDir(deck), 'deck.json'), 'utf8')) as Deck;
}

/** A slide's click groups: its steps that start on a click. */
export function clickGroups(slide: DeckSlide): number {
  return slide.timeline.filter((step) => step.trigger === 'click').length;
}

/** A per-test timeout that grows with the input. */
export function timeoutFor(deck: DeckCase, slides: number, base = 60_000): number {
  return base + slides * 3_000 + Math.ceil(deck.bytes / (1024 * 1024)) * 1_000;
}
