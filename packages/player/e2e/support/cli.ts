// The CLI for the suites that play compiled decks: `SLIDOCK_CLI`, or the workspace's release build, which
// `cargo build --release -p slidock-cli` brings up to date first. A compiled page can be given this build's player
// (`embedPlayer`), so the suite plays what the package builds whichever player the CLI embeds; its presenter view
// too, when the page ships one.
import type { Page } from '@playwright/test';
import type { Deck } from '@slidock/types';
import { execFileSync, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const REPO = fileURLToPath(new URL('../../../..', import.meta.url));
const TARGET = resolve(REPO, process.env.CARGO_TARGET_DIR ?? 'target');
export const OUT = join(TARGET, 'player-e2e', 'decks');
const PLAYER = fileURLToPath(new URL('../../dist/player.iife.js', import.meta.url));
const PRESENTER = fileURLToPath(new URL('../../dist/presenter.iife.js', import.meta.url));

export function cli(): string {
  const named = process.env.SLIDOCK_CLI;
  if (named !== undefined && named !== '') {
    return named;
  }
  execFileSync('cargo', ['build', '--release', '-p', 'slidock-cli'], { cwd: REPO, stdio: 'inherit' });
  return join(TARGET, 'release', process.platform === 'win32' ? 'slidock.exe' : 'slidock');
}

/**
 * Compiles `input` (a path under the repository), with `args` for the CLI, to a new deck folder named after `name`;
 * returns the folder. Each
 * call gets a folder of its own, so workers, projects and repeats never write one another's deck (the run's output
 * folder, which holds them, is emptied when a run starts).
 */
export function compile(binary: string, input: string, name: string, args: readonly string[] = []): string {
  mkdirSync(OUT, { recursive: true });
  const dir = mkdtempSync(join(OUT, `${name}-`));
  const run = spawnSync(binary, ['compile', join(REPO, input), '-o', dir, ...args], {
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
  });
  if (run.status !== 0) {
    throw new Error(`slidock compile ${input} exited ${run.status}: ${run.stderr.slice(-2000)}`);
  }
  return dir;
}

const hash = (text: string) => `'sha256-${createHash('sha256').update(text, 'utf8').digest('base64')}'`;

/**
 * The deck page with this build's player, and its presenter view when the page ships one, in place of the ones the CLI
 * embedded, their CSP hashes updated to match.
 */
export function embedPlayer(html: string): string {
  const scripts: { start: number; end: number; built: string }[] = [];
  // The player's build opens with its licence banner.
  const player = html.indexOf('<script>/*! slidock player |');
  const playerEnd = html.indexOf('</script>', player);
  if (player < 0 || playerEnd < 0) {
    throw new Error('the deck page has no embedded player');
  }
  scripts.push({ start: player, end: playerEnd, built: PLAYER });
  // The presenter view's script follows the player's.
  const next = playerEnd + '</script>\n'.length;
  const nextEnd = html.indexOf('</script>', next);
  if (html.startsWith('<script>', next) && html.slice(next, nextEnd).includes('SlidockPresenter=')) {
    scripts.push({ start: next, end: nextEnd, built: PRESENTER });
  }
  let out = html;
  // From the last, so the earlier offsets hold.
  for (const { start, end, built } of scripts.reverse()) {
    const old = out.slice(start + '<script>'.length, end);
    const script = readFileSync(built, 'utf8');
    if (!out.includes(hash(old)) || script.includes('</script')) {
      throw new Error('the deck page does not hash its scripts as expected');
    }
    // The CSP sits in the head, before the scripts.
    const head = out.slice(0, start).replace(hash(old), hash(script));
    out = `${head}<script>${script}</script>${out.slice(end + '</script>'.length)}`;
  }
  return out;
}

/** The deck page with its deck JSON replaced (the JSON is data: the page's CSP does not cover it). */
export function withDeck(html: string, edit: (deck: Record<string, unknown>) => void): string {
  const match = /(<script type="application\/json" id="sd-deck">)([\s\S]*?)(<\/script>)/.exec(html);
  if (match === null) {
    throw new Error('the deck page has no #sd-deck');
  }
  const deck = JSON.parse(match[2] ?? '') as Record<string, unknown>;
  edit(deck);
  const json = JSON.stringify(deck).replace(/</g, '\\u003c');
  return html.slice(0, match.index) + match[1] + json + match[3] + html.slice(match.index + match[0].length);
}

/** A compiled deck page's deck JSON and slide templates, read by the page's own parser. */
export async function readDeckPage(page: Page, html: string): Promise<{ deck: Deck; slides: Record<string, string> }> {
  await page.setContent('<!doctype html><html><head><meta charset="utf-8"></head><body></body></html>');
  const { deck, slides } = await page.evaluate((text) => {
    const doc = new DOMParser().parseFromString(text, 'text/html');
    const data = JSON.parse(doc.getElementById('sd-deck')?.textContent ?? 'null') as unknown;
    const templates: Record<string, string> = {};
    for (const template of doc.querySelectorAll('template[data-sd-slide]')) {
      templates[template.getAttribute('data-sd-slide') ?? ''] = template.innerHTML;
    }
    return { deck: data, slides: templates };
  }, html);
  return { deck: deck as Deck, slides };
}
