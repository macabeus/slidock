// Every screen and state of the app, opened on the demo page (`src/demo.tsx`: the app over a fake worker scripted per
// screen, with the real player): the design review shoots them (review/web.review.ts) and the design suite audits them
// (design.spec.ts).
import { type Page, expect } from '@playwright/test';

import { holdDevServerSocket } from '../../../../packages/ui/src/audit';
import { GOOGLE_OFF } from './decks';

export interface Screen {
  name: string;
  /** Opens the screen from the demo page at `demo` and returns the page that shows it (a popup for the presenter). */
  open(page: Page, demo: string): Promise<Page>;
  /** Shot as the whole page; overlays are shot as the viewport. */
  fullPage: boolean;
  /** Audited by the design suite; a state that only moves the scroll or the focus of another screen is not. */
  audit: boolean;
  /**
   * The side gutter the screen keeps: 16 px, or for the player's own chrome (present mode, the presenter window) the
   * player's 12 px (packages/player; the deck zip carries the same player).
   */
  gutter: number;
}

/** The player's own chrome keeps a 12 px gutter (`.sd-presenter`, present mode). */
export const PLAYER_GUTTER = 12;

/**
 * Google Fonts off: the screens need no network, and the fonts panel shows the switch off. The dev server's HMR socket
 * is held open (`holdDevServerSocket`), so no screen reloads under a check; `drop` closes it.
 */
export async function demoDefaults(page: Page) {
  await page.addInitScript(({ key, value }) => localStorage.setItem(key, value), GOOGLE_OFF);
  return holdDevServerSocket(page.context());
}

async function openDeck(page: Page, demo: string, query = '') {
  await page.goto(`${demo}${query}`);
  await expect(page.getByRole('toolbar', { name: 'Deck' })).toBeVisible();
  await expect(page.getByTestId('stage')).toHaveAttribute('aria-busy', 'false');
}

/** Dispatches a drop (or a drag entering the window) of files with these names on the window. */
async function dropFiles(page: Page, names: string[], type: 'drop' | 'dragenter' = 'drop') {
  await page.evaluate(
    ({ names, type }) => {
      const data = new DataTransfer();
      for (const name of names) {
        data.items.add(new File(['demo'], name));
      }
      window.dispatchEvent(new DragEvent(type, { dataTransfer: data, bubbles: true, cancelable: true }));
    },
    { names, type },
  );
}

async function scrollDialogToEnd(page: Page) {
  await page
    .getByRole('dialog')
    .locator('> div.overflow-y-auto')
    .evaluate((node) => node.scrollTo(0, node.scrollHeight));
}

async function buildZip(page: Page) {
  const dialog = page.getByRole('dialog', { name: 'Download zip' });
  const download = page.waitForEvent('download');
  await dialog.getByRole('button', { name: 'Build and save' }).click();
  await download;
  await expect(dialog.getByRole('status')).toContainText('Saved');
}

const screen = (name: string, open: Screen['open'], fullPage = true, audit = true, gutter = 16): Screen => ({
  name,
  open,
  fullPage,
  audit,
  gutter,
});

export const SCREENS: readonly Screen[] = [
  screen('landing', async (page, demo) => {
    await page.goto(`${demo}?view=landing`);
    await expect(page.getByTestId('wasm-status')).toHaveText('slidock-wasm demo');
    return page;
  }),
  screen('landing-unsupported', async (page, demo) => {
    await page.goto(`${demo}?view=landing`);
    await expect(page.getByTestId('wasm-status')).toHaveText('slidock-wasm demo');
    await dropFiles(page, ['notes.pdf']);
    await expect(page.getByRole('alert')).toBeVisible();
    return page;
  }),
  screen('landing-refused', async (page, demo) => {
    await page.goto(`${demo}?worker=open-error`);
    await expect(page.getByRole('alert')).toContainText("slidock couldn't read this presentation");
    return page;
  }),
  screen('landing-compiler-failed', async (page, demo) => {
    await page.goto(`${demo}?view=landing&worker=load-error`);
    await expect(page.getByRole('alert')).toContainText("The compiler couldn't load");
    return page;
  }),
  screen('converting', async (page, demo) => {
    await page.goto(`${demo}?worker=hold-finish`);
    await expect(page.getByLabel('Conversion')).toBeVisible();
    return page;
  }),
  screen('viewer', async (page, demo) => {
    await openDeck(page, demo);
    return page;
  }),
  screen(
    'viewer-focus',
    async (page, demo) => {
      await openDeck(page, demo);
      await page.keyboard.press('Tab');
      await page.keyboard.press('Tab');
      return page;
    },
    false,
    false,
  ),
  screen('viewer-notes', async (page, demo) => {
    await openDeck(page, demo);
    await page.getByRole('tab', { name: 'Notes' }).click();
    return page;
  }),
  screen('viewer-diagnostics', async (page, demo) => {
    await openDeck(page, demo);
    await page.getByRole('tab', { name: /Diagnostics/ }).click();
    return page;
  }),
  screen(
    'fonts',
    async (page, demo) => {
      await openDeck(page, demo, '?panel=fonts');
      await expect(page.getByRole('dialog', { name: 'Fonts' })).toBeVisible();
      return page;
    },
    false,
  ),
  screen(
    'fonts-end',
    async (page, demo) => {
      await openDeck(page, demo, '?panel=fonts');
      await expect(page.getByRole('dialog', { name: 'Fonts' })).toBeVisible();
      await scrollDialogToEnd(page);
      return page;
    },
    false,
    false,
  ),
  screen(
    'download',
    async (page, demo) => {
      await openDeck(page, demo, '?panel=download');
      await expect(page.getByRole('dialog', { name: 'Download zip' })).toBeVisible();
      return page;
    },
    false,
  ),
  screen(
    'download-done',
    async (page, demo) => {
      await openDeck(page, demo, '?panel=download');
      await buildZip(page);
      return page;
    },
    false,
  ),
  screen(
    'download-done-end',
    async (page, demo) => {
      await openDeck(page, demo, '?panel=download');
      await buildZip(page);
      await scrollDialogToEnd(page);
      return page;
    },
    false,
    false,
  ),
  screen(
    'download-failed',
    async (page, demo) => {
      await openDeck(page, demo, '?panel=download&worker=bundle-error');
      const dialog = page.getByRole('dialog', { name: 'Download zip' });
      await dialog.getByRole('button', { name: 'Build and save' }).click();
      await expect(dialog.getByRole('alert')).toBeVisible();
      await scrollDialogToEnd(page);
      return page;
    },
    false,
  ),
  screen(
    'help',
    async (page, demo) => {
      await openDeck(page, demo, '?panel=help');
      await expect(page.getByRole('dialog', { name: 'Keyboard shortcuts' })).toBeVisible();
      return page;
    },
    false,
  ),
  screen(
    'drop-overlay',
    async (page, demo) => {
      await openDeck(page, demo);
      await dropFiles(page, ['Roadmap.key'], 'dragenter');
      await expect(page.getByText('Drop a deck to open it, or font files to add them')).toBeVisible();
      return page;
    },
    false,
  ),
  screen(
    'replace',
    async (page, demo) => {
      await openDeck(page, demo);
      await dropFiles(page, ['Roadmap.key']);
      await expect(page.getByRole('dialog', { name: 'Replace the open deck?' })).toBeVisible();
      return page;
    },
    false,
  ),
  // Present mode and the presenter window are the player's own chrome (packages/player), audited with its gutter.
  screen(
    'present',
    async (page, demo) => {
      await openDeck(page, demo);
      await page.getByRole('button', { name: 'Present', exact: true }).click();
      await expect.poll(() => page.evaluate(() => document.fullscreenElement !== null)).toBe(true);
      return page;
    },
    false,
    true,
    PLAYER_GUTTER,
  ),
  screen(
    'presenter',
    async (page, demo) => {
      await openDeck(page, demo);
      const popup = page.waitForEvent('popup');
      await page.getByRole('button', { name: 'Presenter view' }).click();
      const presenter = await popup;
      await expect(presenter.locator('.sd-presenter')).toBeVisible();
      return presenter;
    },
    false,
    true,
    PLAYER_GUTTER,
  ),
  screen('presenter-orphan', async (page, demo) => {
    await page.goto(`${demo}#presenter=review`);
    await expect(page.getByRole('alert')).toContainText('Nothing to present here');
    return page;
  }),
];
