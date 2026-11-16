// The presenter view in its own window, in the web app and from file://: it opens, follows the deck window, and
// drives it (next, previous, go to, blank), both ways. The two windows draw the same frame (`@slidock/player/presenter`):
// the same elements with the same computed styles and the same deck title. Reconnection after reloading the presenter window, and the
// BroadcastChannel alone (a presenter page with no opener), are recorded per browser as annotations; README.md has
// the table. The presenter view's preview of the next click (the next build step, then the next slide) is checked
// on a corpus deck with builds.
import { type BrowserContext, type Page, type TestInfo, expect, test } from '@playwright/test';
import type { Deck } from '@slidock/types';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import {
  type DeckCase,
  GOOGLE_OFF,
  SMOKE,
  clickGroups,
  compiledDeck,
  deckCases,
  siteDir,
  smokeCase,
  timeoutFor,
} from './support/decks';
import { currentSlide, expectBlank, expectSlide, settle, watch } from './support/player';
import { untilCompiled } from './support/progress';

const smoke = smokeCase();

/** The smoke deck's slides: two shown, a hidden one, a last shown one. */
function slides(deck: Deck) {
  const [first, second, hidden, last] = deck.slides;
  if (!first || !second || !hidden?.hidden || !last) {
    throw new Error('the smoke deck has changed');
  }
  return { first, second, last };
}

function presenterButton(presenter: Page, title: string) {
  return presenter.locator(`.sd-presenter header button[title="${title}"]`);
}

/** Both windows show `id`. */
async function both(deckPage: Page, presenter: Page, id: string) {
  await expectSlide(deckPage, id);
  await expectSlide(presenter, id);
}

async function exerciseSync(deckPage: Page, presenter: Page, deck: Deck) {
  const { first, second, last } = slides(deck);
  await test.step('the presenter view opens on the deck window’s slide, with its notes', async () => {
    await both(deckPage, presenter, first.id);
    await expect(presenter.locator('.sd-notes p')).toHaveText(first.notes);
    await expect(presenter.locator('.sd-next .sd-static')).toHaveCount(1);
  });
  await test.step('the presenter view follows the deck window', async () => {
    await deckPage.keyboard.press('ArrowRight');
    await both(deckPage, presenter, second.id);
    await deckPage.keyboard.press('ArrowLeft');
    await both(deckPage, presenter, first.id);
    await deckPage.keyboard.press('3');
    await deckPage.keyboard.press('Enter');
    await both(deckPage, presenter, last.id);
    await deckPage.keyboard.press('Home');
    await both(deckPage, presenter, first.id);
    await deckPage.keyboard.press('b');
    await expectBlank(presenter, 'black');
    await deckPage.keyboard.press('b');
    await expectBlank(presenter, '');
    await deckPage.keyboard.press('w');
    await expectBlank(presenter, 'white');
    await deckPage.keyboard.press('w');
    await expectBlank(presenter, '');
  });
  await test.step('the presenter view drives the deck window', async () => {
    await presenterButton(presenter, 'Next').click();
    await both(deckPage, presenter, second.id);
    // The hidden slide is skipped while presenting.
    await presenterButton(presenter, 'Next').click();
    await both(deckPage, presenter, last.id);
    await presenterButton(presenter, 'Previous').click();
    await both(deckPage, presenter, second.id);
    await presenter.getByRole('spinbutton', { name: 'Go to slide' }).fill('1');
    await presenter.getByRole('button', { name: 'Go', exact: true }).click();
    await both(deckPage, presenter, first.id);
    await presenter.keyboard.press('End');
    await both(deckPage, presenter, last.id);
    await presenterButton(presenter, 'Black screen (B)').click();
    await expectBlank(deckPage, 'black');
    await presenterButton(presenter, 'White screen (W)').click();
    await expectBlank(deckPage, 'white');
    await presenterButton(presenter, 'White screen (W)').click();
    await expectBlank(deckPage, '');
  });
}

/** Whether `presenter` shows the deck window's slide `id` within `timeout`. */
async function shows(presenter: Page, id: string, timeout: number): Promise<boolean> {
  return expect
    .poll(() => currentSlide(presenter), { timeout })
    .toBe(id)
    .then(
      () => true,
      () => false,
    );
}

/** Reloads the presenter window: does it find the deck window again, and drive it? */
async function recordReload(deckPage: Page, presenter: Page, deck: Deck, testInfo: TestInfo) {
  const { first, second } = slides(deck);
  await deckPage.keyboard.press('Home');
  await deckPage.keyboard.press('ArrowRight');
  await expectSlide(deckPage, second.id);
  await presenter.reload();
  const found = await shows(presenter, second.id, 15_000);
  let drives = false;
  if (found) {
    await presenterButton(presenter, 'Previous').click();
    drives = await shows(deckPage, first.id, 10_000);
  }
  testInfo.annotations.push({
    type: 'presenter reload',
    description: found ? (drives ? 'reconnects and drives' : 'reconnects, does not drive') : 'does not reconnect',
  });
}

/** A presenter page opened by hand (no opener): only the BroadcastChannel can reach the deck window. */
async function recordBroadcastOnly(
  context: BrowserContext,
  deckPage: Page,
  url: string,
  deck: Deck,
  testInfo: TestInfo,
) {
  const { first, second, last } = slides(deck);
  const lone = await context.newPage();
  await lone.goto(url);
  await deckPage.keyboard.press('End');
  await expectSlide(deckPage, last.id);
  const follows = await shows(lone, last.id, 12_000);
  let drives = false;
  if (follows) {
    await presenterButton(lone, 'Previous').click();
    drives = await shows(deckPage, second.id, 10_000);
  }
  testInfo.annotations.push({
    type: 'broadcastchannel only',
    description: follows ? (drives ? 'follows and drives' : 'follows, does not drive') : 'does not connect',
  });
  await lone.close();
  await deckPage.keyboard.press('Home');
  await expectSlide(deckPage, first.id);
}

/** Waits until the app has not replaced its player for `quietMs`. */
async function playerSettled(page: Page, quietMs = 750) {
  await page.evaluate(
    (quiet) =>
      new Promise<void>((resolve) => {
        const done = () => {
          observer.disconnect();
          resolve();
        };
        let timer = setTimeout(done, quiet);
        const observer = new MutationObserver((records) => {
          const replaced = records.some((record) =>
            Array.from(record.addedNodes).some((node) => node instanceof Element && node.matches('.sd-player')),
          );
          if (replaced) {
            clearTimeout(timer);
            timer = setTimeout(done, quiet);
          }
        });
        observer.observe(document.body, { subtree: true, childList: true });
      }),
    quietMs,
  );
}

/** Opens the smoke deck in the app, on its first slide. */
async function openInApp(page: Page, deck: Deck) {
  await page.goto('/');
  await page.locator('input[type="file"]').setInputFiles(smoke.input);
  await expect(page.getByRole('toolbar', { name: 'Deck' })).toBeVisible({ timeout: 60_000 });
  await expectSlide(page, slides(deck).first.id);
}

test(
  'presenter view from file:// follows and drives the deck window',
  { tag: ['@smoke'] },
  async ({ page, context }, testInfo) => {
    const deck = compiledDeck(smoke);
    const problems = watch(page);
    await page.goto(pathToFileURL(join(siteDir(smoke), 'index.html')).href);
    await expectSlide(page, slides(deck).first.id);
    const popup = page.waitForEvent('popup');
    await page.keyboard.press('p');
    const presenter = await popup;
    const presenterProblems = watch(presenter);
    await expect(presenter).toHaveURL(/#presenter=[\w-]+$/);
    await expect(presenter.locator('.sd-presenter')).toBeVisible();
    await exerciseSync(page, presenter, deck);
    const url = presenter.url();
    await recordReload(page, presenter, deck, testInfo);
    await recordBroadcastOnly(context, page, url, deck, testInfo);
    expect(problems, 'deck window problems').toEqual([]);
    expect(presenterProblems, 'presenter window problems').toEqual([]);
  },
);

test(
  'presenter view in the app follows and drives the deck window',
  { tag: ['@smoke'] },
  async ({ page, context }, testInfo) => {
    const deck = compiledDeck(smoke);
    const problems = watch(page);
    await openInApp(page, deck);
    const popup = page.waitForEvent('popup');
    await page.getByRole('button', { name: 'Presenter view' }).click();
    const presenter = await popup;
    const presenterProblems = watch(presenter);
    // The app at `#presenter=<session>`: the deck window's sync session, which the presenter view joins.
    await expect(presenter).toHaveURL(/#presenter=[\w-]+$/);
    await expect(presenter.locator('.sd-presenter')).toBeVisible();
    // Opening the presenter view presents from the deck window.
    await expect(page).toHaveURL(/present=1/);
    await exerciseSync(page, presenter, deck);
    await recordReload(page, presenter, deck, testInfo);
    // The app's presenter window gets the deck itself from its opener, so a page without one has nothing to show.
    const lone = await context.newPage();
    await lone.goto('/#presenter');
    await expect(lone.getByText('Nothing to present here')).toBeVisible();
    await lone.close();
    testInfo.annotations.push({ type: 'broadcastchannel only', description: 'n/a: the deck comes from the opener' });
    expect(problems, 'deck window problems').toEqual([]);
    expect(presenterProblems, 'presenter window problems').toEqual([]);
  },
);

/**
 * The presenter window's frame and view as a tree: each element's tag, class, role, accessible name and the computed
 * styles that draw it, with its box, and the window's title; the slides left out.
 */
function frameSnapshot(presenter: Page) {
  return presenter.evaluate(() => {
    const props = [
      'display',
      'position',
      'boxSizing',
      'fontFamily',
      'fontSize',
      'fontWeight',
      'lineHeight',
      'letterSpacing',
      'color',
      'backgroundColor',
      'backgroundImage',
      'borderTopColor',
      'borderTopWidth',
      'borderRadius',
      'boxShadow',
      'paddingTop',
      'paddingLeft',
      'marginTop',
      'gap',
      'filter',
      'cursor',
      'opacity',
    ] as const;
    const walk = (element: Element): unknown => {
      const style = getComputedStyle(element);
      const box = element.getBoundingClientRect();
      return {
        tag: element.tagName.toLowerCase(),
        class: element.getAttribute('class'),
        role: element.getAttribute('role'),
        label: element.getAttribute('aria-label') ?? element.getAttribute('title'),
        text: element.children.length > 0 ? null : element.textContent,
        box: [box.x, box.y, box.width, box.height].map(Math.round),
        style: Object.fromEntries(props.map((prop) => [prop, style[prop]])),
        children: Array.from(element.children)
          .filter((child) => !child.matches('.sd-player, .sd-static') && child.tagName.toLowerCase() !== 'svg')
          .map(walk),
      };
    };
    const root = document.querySelector('.sd-pw');
    return root === null ? null : { title: document.title, frame: walk(root) };
  });
}

test(
  'the app and a downloaded deck draw the same presenter window',
  { tag: ['@smoke'] },
  async ({ page, browser }, testInfo) => {
    const deck = compiledDeck(smoke);
    const size = { width: 1280, height: 800 };
    const snapshot = async (open: (deckPage: Page) => Promise<void>, scheme: 'dark' | 'light') => {
      const context = await browser.newContext({ colorScheme: scheme, baseURL: testInfo.project.use.baseURL ?? '' });
      const deckPage = await context.newPage();
      await open(deckPage);
      const popup = deckPage.waitForEvent('popup');
      await deckPage.keyboard.press('p');
      const presenter = await popup;
      await presenter.setViewportSize(size);
      await expect(
        presenter.locator('.sd-pw main > .sd-presenter > .sd-current .sd-layer[data-sd-current]'),
      ).toHaveCount(1);
      await expect(presenter.locator('.sd-notes p')).toHaveText(slides(deck).first.notes);
      const shot = await frameSnapshot(presenter);
      await context.close();
      return shot;
    };
    const fromFile = async (deckPage: Page) => {
      await deckPage.goto(pathToFileURL(join(siteDir(smoke), 'index.html')).href);
    };
    const inApp = async (deckPage: Page) => {
      await openInApp(deckPage, deck);
      await deckPage.getByRole('button', { name: 'Present', exact: true }).click();
    };
    for (const scheme of ['dark', 'light'] as const) {
      const exported = await snapshot(fromFile, scheme);
      expect(exported, 'the downloaded deck draws the frame').not.toBeNull();
      expect(await snapshot(inApp, scheme), `the same window in the ${scheme} scheme`).toEqual(exported);
    }
    await page.close();
  },
);

// The player's own P key opens the presenter window too: while presenting, and while viewing with the stage focused
// (the player reads its keys there first).
for (const how of ['presenting', 'viewing with the stage focused'] as const) {
  test(`presenter view in the app opens with the P key, ${how}`, { tag: ['@smoke'] }, async ({ page }) => {
    const deck = compiledDeck(smoke);
    const { first, second } = slides(deck);
    const problems = watch(page);
    await openInApp(page, deck);
    if (how === 'presenting') {
      await page.getByRole('button', { name: 'Present', exact: true }).click();
      await expect(page).toHaveURL(/present=1/);
    } else {
      // The fonts pass swaps the deck, and with it the player, shortly after the deck first shows: a player focused
      // before that is gone, and P then reaches the app (which presents) instead of the player.
      await playerSettled(page);
      const stage = page.getByTestId('stage').locator('.sd-player');
      await stage.focus();
      await expect(stage).toBeFocused();
    }
    const popup = page.waitForEvent('popup');
    await page.keyboard.press('p');
    const presenter = await popup;
    const presenterProblems = watch(presenter);
    await expect(presenter.locator('.sd-presenter')).toBeVisible();
    await both(page, presenter, first.id);
    await page.keyboard.press('ArrowRight');
    await both(page, presenter, second.id);
    await presenterButton(presenter, 'Previous').click();
    await both(page, presenter, first.id);
    expect(problems, 'deck window problems').toEqual([]);
    expect(presenterProblems, 'presenter window problems').toEqual([]);
  });
}

/** A presenter view's slide state: the slide a stage shows and its hidden elements and paragraphs. */
function stageState(presenter: Page, stage: 'next' | 'current') {
  return presenter.evaluate((which) => {
    const layer = document.querySelector(
      which === 'next' ? '.sd-presenter .sd-next .sd-layer' : '.sd-presenter .sd-current .sd-layer[data-sd-current]',
    );
    const hidden = Array.from(layer?.shadowRoot?.querySelectorAll('[data-sd-state="hidden"]') ?? [], (element) => {
      const owner = element.closest('[data-el]')?.getAttribute('data-el') ?? '';
      return `${owner}/${element.getAttribute('data-p') ?? ''}`;
    }).sort();
    return { slide: layer?.getAttribute('data-sd-layer') ?? null, hidden };
  }, stage);
}

/**
 * How long the app may take to compile a corpus deck: a minute, 6 s a slide and 50 ms a MiB (about 10 minutes for
 * retro: 48 slides, 4.5 GiB). Docker Firefox compiles retro in two to three minutes, ten times slower than Chromium.
 */
function compileTimeout(deck: DeckCase, slides: number): number {
  return 60_000 + slides * 6_000 + Math.ceil(deck.bytes / (1024 * 1024)) * 50;
}

/** The corpus deck with the fewest slides that has a shown slide of two click groups or more, and that slide. */
function deckWithBuilds(): { deck: DeckCase; compiled: Deck; slide: number } {
  let best: { deck: DeckCase; compiled: Deck; slide: number } | null = null;
  for (const deck of deckCases()) {
    if (deck.smoke) {
      continue;
    }
    const compiled = compiledDeck(deck);
    const slide = compiled.slides.findIndex((s) => !s.hidden && clickGroups(s) >= 2);
    if (slide >= 0 && (best === null || compiled.slides.length < best.compiled.slides.length)) {
      best = { deck, compiled, slide };
    }
  }
  if (best === null) {
    throw new Error('no corpus deck has a slide with two click builds');
  }
  return best;
}

// The smoke deck has no builds (the PPTX frontend shows its slides fully built), so this runs on the corpus.
if (!SMOKE) {
  test('presenter view in the app previews the next build step, then the next slide', async ({ page }, testInfo) => {
    const { deck, compiled, slide } = deckWithBuilds();
    const compiling = compileTimeout(deck, compiled.slides.length);
    test.setTimeout(compiling + timeoutFor(deck, compiled.slides.length, 120_000));
    const target = compiled.slides[slide];
    const groups = target === undefined ? 0 : clickGroups(target);
    const following = compiled.slides.slice(slide + 1).find((s) => !s.hidden);
    if (target === undefined || target.number === null) {
      throw new Error('the slide has no number');
    }
    await page.addInitScript(({ key, value }) => {
      try {
        localStorage.setItem(key, value);
      } catch {
        // No storage in this frame.
      }
    }, GOOGLE_OFF);
    await page.goto('/');
    await page.locator('input[type="file"]').setInputFiles(deck.input);
    const started = Date.now();
    const phases = await untilCompiled(page, compiling);
    testInfo.annotations.push({
      type: 'compile',
      description: `${deck.name}: ${Date.now() - started} ms of ${compiling} (${phases.join(' → ')})`,
    });
    const popup = page.waitForEvent('popup');
    await page.getByRole('button', { name: 'Presenter view' }).click();
    const presenter = await popup;
    await expect(presenter.locator('.sd-presenter')).toBeVisible();
    await settle(page);
    for (const digit of String(target.number)) {
      await page.keyboard.press(digit);
    }
    await page.keyboard.press('Enter');
    await both(page, presenter, target.id);
    const label = presenter.locator('.sd-presenter .sd-next .sd-label').first();
    for (let step = 1; step <= groups; step++) {
      await expect(label).toHaveText(`Next: build ${step} of ${groups}`);
      await expect(presenter.locator('.sd-next')).toHaveAttribute('data-sd-next', 'build');
      const preview = await stageState(presenter, 'next');
      expect(preview.slide).toBe(target.id);
      expect(preview, 'the preview is not the slide as it is').not.toEqual(await stageState(presenter, 'current'));
      await settle(page);
      await page.keyboard.press('ArrowRight');
      // The presenter view cuts to the deck window's step: its stage now shows what the preview showed.
      await expect.poll(() => stageState(presenter, 'current')).toEqual(preview);
    }
    if (following !== undefined) {
      await expect(label).toHaveText(`Next: slide ${following.number}`);
      await expect(presenter.locator('.sd-next')).toHaveAttribute('data-sd-next', 'slide');
      expect((await stageState(presenter, 'next')).slide).toBe(following.id);
    }
  });
}
