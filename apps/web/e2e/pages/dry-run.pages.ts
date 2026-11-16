// The Pages build (pages.config.ts) under its base path: the PPTX and ODP motion probe decks (and the deck
// `SLIDOCK_PAGES_DECK` names, a path, when set) each open, show every slide,
// present, open the presenter window and download a zip that then plays from file://. Every request stays under
// /slidock/ or is the deck's own blob or data URL (the zip's page: its own folder), and no page logs a console error,
// throws or violates its CSP. Google Fonts is switched off, as a user can, so no request leaves the site.
import { type Page, expect, test } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, rmSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

import { GOOGLE_OFF, OUT, REPO } from '../support/decks';
import { currentSlide, expectMounted, expectSettled, settle, watch } from '../support/player';

const EXTRA = process.env.SLIDOCK_PAGES_DECK;
const DECKS = [
  { key: 'probe-pptx', input: join(REPO, 'probes/out/motion-probe.pptx') },
  { key: 'probe-odp', input: join(REPO, 'probes/out/motion-probe.odp') },
  ...(EXTRA ? [{ key: 'extra', input: resolve(EXTRA) }] : []),
];

function site(): string {
  const url = process.env.SLIDOCK_PAGES_URL;
  if (!url) {
    throw new Error('SLIDOCK_PAGES_URL is unset: run with --config e2e/pages/pages.config.ts');
  }
  return url;
}

/** Steps through every slide of the rail with the arrow key, each mounted in turn. */
async function viewEverySlide(page: Page, slides: number) {
  for (let index = 0; index < slides; index++) {
    if (index > 0) {
      await page.keyboard.press('ArrowRight');
      await settle(page);
    }
    const id = await currentSlide(page);
    expect(id, `slide ${index + 1} shows`).not.toBeNull();
    await expectMounted(page, id ?? '');
  }
}

for (const deck of DECKS) {
  test(`${deck.key}: ${basename(deck.input)} opens, presents, opens the presenter window and downloads a zip that plays from file://`, async ({
    page,
    context,
  }, testInfo) => {
    const out = join(OUT, 'pages', deck.key);
    rmSync(out, { recursive: true, force: true });
    mkdirSync(out, { recursive: true });
    const unpacked = join(out, 'site');
    const allowed = [site(), 'blob:', 'data:', pathToFileURL(unpacked).href];
    const requests: string[] = [];
    context.on('request', (request) => requests.push(request.url()));
    const problems = watch(page);
    await context.addInitScript(({ key, value }) => {
      try {
        localStorage.setItem(key, value);
      } catch {
        // A file:// page or an opaque origin may have no storage; the app's own document does.
      }
    }, GOOGLE_OFF);

    await page.goto(site());
    await expect(page.getByTestId('wasm-status')).toHaveText(/^slidock-wasm \S+$/, { timeout: 60_000 });

    await test.step('open the deck and view every slide', async () => {
      await page.locator('input[type="file"]').setInputFiles(deck.input);
      await expect(page.getByRole('toolbar', { name: 'Deck' })).toBeVisible({ timeout: testInfo.timeout });
      await expectSettled(page, testInfo.timeout);
      const slides = await page.locator('nav[aria-label="Slides"] > ol > li').count();
      expect(slides).toBeGreaterThan(0);
      await viewEverySlide(page, slides);
    });

    await test.step('present', async () => {
      // Escape follows the click at once, while the browser may still be entering full screen; then the show starts,
      // moves and stops. Each time nothing fills the screen (full screen, or the window) once the show stops, and a
      // late answer of the browser fills nothing either. A key pressed after the click reaches the show before that
      // answer only sometimes; e2e/present.spec.ts and the player's own present.spec.ts catch the race every time.
      const showing = () =>
        page.evaluate(
          () => document.fullscreenElement !== null || document.querySelector('.sd-player.sd-fill') !== null,
        );
      for (const keys of [['Escape'], ['ArrowRight', 'Escape']]) {
        await page.getByRole('button', { name: 'Present', exact: true }).click();
        if (keys.length > 1) {
          await expect(page).toHaveURL(/present=1/);
        }
        for (const key of keys) {
          await page.keyboard.press(key);
        }
        await expect(page).not.toHaveURL(/present=1/);
        await expect.poll(showing).toBe(false);
        await page.waitForTimeout(1_000);
        expect(await showing(), `nothing fills the screen a second after ${keys.join(', ')} stopped the show`).toBe(
          false,
        );
      }
    });

    await test.step('open the presenter window', async () => {
      const popup = page.waitForEvent('popup');
      await page.getByRole('button', { name: 'Presenter view' }).click();
      const presenter = await popup;
      problems.push(...watch(presenter));
      expect(presenter.url().startsWith(site())).toBe(true);
      await expect(presenter.locator('.sd-presenter')).toBeVisible();
      await presenter.close();
    });

    const zip = join(out, 'deck.zip');
    await test.step('download the zip', async () => {
      await page.getByRole('button', { name: 'Download zip' }).click();
      const dialog = page.getByRole('dialog', { name: 'Download zip' });
      const download = page.waitForEvent('download', { timeout: testInfo.timeout });
      await dialog.getByRole('button', { name: 'Build and save' }).click();
      await (await download).saveAs(zip);
      await expect(dialog.getByText(/^Saved /)).toBeVisible();
    });

    await test.step('the zip plays from file://', async () => {
      execFileSync('unzip', ['-q', zip, '-d', unpacked]);
      const deckPage = await context.newPage();
      const deckProblems = watch(deckPage);
      await deckPage.goto(pathToFileURL(join(unpacked, 'index.html')).href);
      await expect.poll(() => currentSlide(deckPage)).not.toBeNull();
      const first = await currentSlide(deckPage);
      await expectMounted(deckPage, first ?? '');
      await deckPage.keyboard.press('ArrowRight');
      await settle(deckPage);
      expect(deckProblems, 'console errors, page errors and CSP violations of the zip page').toEqual([]);
      await deckPage.close();
    });

    const outside = requests.filter((url) => !allowed.some((prefix) => url.startsWith(prefix)));
    expect(outside, `requests outside ${site()}, the deck's blobs and the zip's folder`).toEqual([]);
    expect(
      requests.some((url) => url.startsWith(`${site()}assets/`) && url.endsWith('.wasm')),
      'the compiler loads from the site',
    ).toBe(true);
    expect(problems, 'console errors, page errors and CSP violations').toEqual([]);
    testInfo.annotations.push({ type: 'requests', description: String(requests.length) });
  });
}
