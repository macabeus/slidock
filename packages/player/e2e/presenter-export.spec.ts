// The presenter view of a CLI deck, from file:// and over HTTP, with this build's player and presenter view: P opens
// the presenter window, which draws the web app's frame (the header with the deck's title and the theme toggle) above
// the presenter view, follows the deck window and drives it (next, previous, builds, blank), and runs its timers. A
// deck compiled with `--no-presenter` has no way in: no P key, no help row, no `SlidockPresenter`, and its
// `#presenter` URL shows the deck.
import { type Page, expect, test } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
import { type Server, createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { extname, join, relative } from 'node:path';
import { pathToFileURL } from 'node:url';

import { OUT, cli, compile, embedPlayer, withDeck } from './support/cli';

test.describe.configure({ timeout: 5 * 60_000 });
// The window's tokens follow the system preference until the toggle forces one: start from a dark system.
test.use({ colorScheme: 'dark' });

const NOTES = ['Say hello first.', 'Then the agenda.'];

interface Deck {
  dir: string;
  title: string;
  first: string;
  second: string;
}

let withPresenter: Deck | null = null;
let withoutPresenter: Deck | null = null;
let server: Server | null = null;

/** The motion probe, compiled with `args`, with this build's scripts, two click builds and notes on its first slide. */
function deck(name: string, args: readonly string[]): Deck {
  const dir = compile(cli(), 'probes/out/motion-probe.pptx', name, args);
  const html = readFileSync(join(dir, 'index.html'), 'utf8');
  let title = '';
  let first = '';
  let second = '';
  const edited = withDeck(embedPlayer(html), (data) => {
    title = data.title as string;
    const slides = data.slides as { id: string; timeline: unknown[]; notes: string[] }[];
    const slide = slides[0];
    if (slide === undefined) {
      throw new Error('the probe deck has no slides');
    }
    first = slide.id;
    second = slides[1]?.id ?? '';
    slide.notes = NOTES;
    const build = (element: string) => ({
      target: { element, paragraphs: null },
      trigger: 'click',
      kind: 'entrance',
      delivery: 'all',
      delayMs: 0,
      durationMs: 200,
      native: 'test:build',
      fallback: 'dissolve',
      effect: 'fade',
    });
    slide.timeline = [build(`${slide.id}.2`), build(`${slide.id}.3`)];
  });
  writeFileSync(join(dir, 'index.html'), edited);
  return { dir, title, first, second };
}

const TYPES: Record<string, string> = {
  '.html': 'text/html; charset=utf-8',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.gif': 'image/gif',
  '.svg': 'image/svg+xml',
  '.mp4': 'video/mp4',
  '.txt': 'text/plain',
};

test.beforeAll(async () => {
  test.setTimeout(30 * 60_000);
  withPresenter = deck('presenter-on', []);
  withoutPresenter = deck('presenter-off', ['--no-presenter']);
  // The decks over HTTP: a static server on the run's output folder.
  server = createServer((request, response) => {
    const path = join(OUT, decodeURIComponent(new URL(request.url ?? '/', 'http://x').pathname));
    if (relative(OUT, path).startsWith('..')) {
      response.writeHead(403).end();
      return;
    }
    try {
      const body = readFileSync(path);
      response.writeHead(200, { 'content-type': TYPES[extname(path)] ?? 'application/octet-stream' }).end(body);
    } catch {
      response.writeHead(404).end();
    }
  });
  await new Promise<void>((resolve) => server?.listen(0, '127.0.0.1', resolve));
});

test.afterAll(async () => {
  await new Promise<void>((resolve) => (server === null ? resolve() : server.close(() => resolve())));
});

function url(deck: Deck | null, over: 'file' | 'http'): string {
  if (deck === null || server === null) {
    throw new Error('no deck');
  }
  const index = join(deck.dir, 'index.html');
  if (over === 'file') {
    return pathToFileURL(index).href;
  }
  const { port } = server.address() as AddressInfo;
  return `http://127.0.0.1:${port}/${relative(OUT, index).split('\\').join('/')}`;
}

/** Errors a page reports: uncaught exceptions, console errors (CSP violations among them). */
function watch(page: Page): string[] {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') {
      errors.push(message.text());
    }
  });
  return errors;
}

async function shown(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const stage = document.querySelector('.sd-presenter .sd-current') ?? document;
    return (
      stage.querySelector('.sd-player > .sd-frame > .sd-layer[data-sd-current]')?.getAttribute('data-sd-layer') ?? null
    );
  });
}

async function step(page: Page): Promise<number> {
  return page.evaluate(() => {
    const stage = document.querySelector('.sd-presenter .sd-current') ?? document;
    const layer = stage.querySelector('.sd-player > .sd-frame > .sd-layer[data-sd-current]');
    return layer?.shadowRoot?.querySelectorAll('[data-sd-state="hidden"]').length ?? -1;
  });
}

/** Waits until nothing animates on `page` (a key during a build finishes it instead of advancing). */
async function idle(page: Page): Promise<void> {
  await page.waitForFunction(
    () =>
      [...document.querySelectorAll('.sd-layer')].every((layer) =>
        (layer.shadowRoot?.getAnimations() ?? []).every((a) => a.playState !== 'running'),
      ) && document.getAnimations().every((a) => a.playState !== 'running'),
  );
}

async function blank(page: Page): Promise<string> {
  return page.evaluate(() => {
    const stage = document.querySelector('.sd-presenter .sd-current') ?? document;
    return stage.querySelector('.sd-blank')?.getAttribute('data-sd-blank') ?? '';
  });
}

for (const over of ['file', 'http'] as const) {
  test(`the presenter window from ${over}: the app's frame, in step with the deck window`, async ({ page }) => {
    const deckErrors = watch(page);
    await page.setViewportSize({ width: 1280, height: 720 });
    await page.goto(url(withPresenter, over));
    await expect(page.locator('.sd-player')).toBeVisible();
    const { title, first, second } = withPresenter as Deck;
    await expect.poll(() => shown(page)).toBe(first);
    const popup = page.waitForEvent('popup');
    await page.keyboard.press('p');
    const presenter = await popup;
    const errors = watch(presenter);
    await presenter.setViewportSize({ width: 1280, height: 800 });
    await expect(presenter).toHaveURL(/#presenter=[\w-]+$/);

    await test.step('the frame: the header card over the presenter view', async () => {
      const frame = presenter.locator('.sd-pw');
      await expect(frame.locator('header.sd-pw-header .sd-pw-title')).toHaveText('slidock');
      await expect(frame.locator('.sd-pw-subtitle')).toHaveText(`Presenter view · ${title}`);
      await expect(frame.getByRole('button', { name: 'System theme. Switch to light theme' })).toBeVisible();
      await expect(frame.locator('main.sd-pw-main > .sd-presenter')).toBeVisible();
      const header = await frame.locator('.sd-pw-header').boundingBox();
      const view = await frame.locator('.sd-presenter').boundingBox();
      expect(
        header !== null && view !== null && header.y + header.height <= view.y,
        'the view is below the header',
      ).toBe(true);
      await expect(presenter.locator('#sd-text')).toHaveCount(0);
      await expect(presenter).toHaveTitle(`Presenter · ${title}`);
    });

    await test.step('notes, the next click and the counter', async () => {
      await expect(presenter.locator('.sd-notes p')).toHaveText(NOTES);
      await expect(presenter.locator('.sd-next .sd-label').first()).toHaveText('Next: build 1 of 2');
      await expect.poll(() => shown(presenter)).toBe(first);
    });

    await test.step('builds and slides, both ways', async () => {
      const hidden = await step(page);
      await presenter.getByRole('button', { name: '→' }).click();
      await expect.poll(() => step(page)).toBe(hidden - 1);
      await expect.poll(() => step(presenter)).toBe(hidden - 1);
      await expect(presenter.locator('.sd-next .sd-label').first()).toHaveText('Next: build 2 of 2');
      await idle(page);
      await page.keyboard.press('ArrowRight');
      await expect.poll(() => step(presenter)).toBe(hidden - 2);
      await idle(page);
      await page.keyboard.press('ArrowRight');
      await expect.poll(() => shown(presenter)).toBe(second);
      await expect(presenter.locator('.sd-notes')).toHaveText('No notes');
      await idle(presenter);
      await presenter.keyboard.press('ArrowLeft');
      await expect.poll(() => shown(page)).toBe(first);
      await expect(presenter.locator('.sd-notes p')).toHaveText(NOTES);
    });

    await test.step('blank, both ways', async () => {
      await page.keyboard.press('b');
      await expect.poll(() => blank(presenter)).toBe('black');
      await expect(presenter.getByRole('button', { name: 'Black' })).toHaveAttribute('aria-pressed', 'true');
      await presenter.getByRole('button', { name: 'White' }).click();
      await expect.poll(() => blank(page)).toBe('white');
      await presenter.getByRole('button', { name: 'White' }).click();
      await expect.poll(() => blank(page)).toBe('');
    });

    await test.step('the timers run and reset', async () => {
      const elapsed = presenter.locator('.sd-time[title="Elapsed"]');
      await expect(elapsed).not.toHaveText('0:00', { timeout: 5000 });
      await presenter.getByRole('button', { name: 'Reset' }).click();
      await expect(elapsed).toHaveText('0:00');
    });

    await test.step('the theme toggle', async () => {
      const background = () => presenter.locator('.sd-pw').evaluate((el) => getComputedStyle(el).backgroundColor);
      expect(await background()).toBe('rgb(15, 23, 42)');
      await presenter.getByRole('button', { name: /^System theme/ }).click();
      await expect(presenter.locator('html')).toHaveAttribute('data-theme', 'light');
      expect(await background()).toBe('rgb(248, 250, 252)');
      await expect(presenter.getByRole('button', { name: 'Light theme. Switch to dark theme' })).toBeVisible();
      await presenter.getByRole('button', { name: /^Light theme/ }).click();
      await presenter.getByRole('button', { name: /^Dark theme/ }).click();
      await expect(presenter.locator('html')).not.toHaveAttribute('data-theme');
    });

    expect(deckErrors, 'deck window errors').toEqual([]);
    expect(errors, 'presenter window errors').toEqual([]);
  });

  test(`a deck without the presenter view from ${over} has no way into one`, async ({ page, context }) => {
    const errors = watch(page);
    await page.setViewportSize({ width: 1280, height: 720 });
    await page.goto(url(withoutPresenter, over));
    const { first, second } = withoutPresenter as Deck;
    await expect.poll(() => shown(page)).toBe(first);
    expect(await page.evaluate(() => 'SlidockPresenter' in window)).toBe(false);
    let popups = 0;
    context.on('page', () => popups++);
    await page.keyboard.press('p');
    await page.keyboard.press('?');
    const help = page.getByRole('dialog', { name: 'Keyboard shortcuts' });
    await expect(help).toBeVisible();
    expect(await help.locator('dt').allTextContents()).not.toContain('P');
    await expect(help.locator('dt', { hasText: /^F$/ })).toHaveCount(1);
    await page.keyboard.press('Escape');
    expect(popups).toBe(0);

    const lone = await context.newPage();
    const loneErrors = watch(lone);
    await lone.setViewportSize({ width: 1280, height: 720 });
    await lone.goto(`${url(withoutPresenter, over)}#presenter=x`);
    await expect(lone.locator('.sd-player.sd-page')).toBeVisible();
    await expect(lone.locator('.sd-pw, .sd-presenter')).toHaveCount(0);
    await expect.poll(() => shown(lone)).toBe(first);
    await lone.keyboard.press('2');
    await lone.keyboard.press('Enter');
    await expect.poll(() => shown(lone)).toBe(second);
    expect(errors).toEqual([]);
    expect(loneErrors).toEqual([]);
  });
}
