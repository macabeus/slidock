// Present in the app, in each engine: Escape while the browser has yet to answer the full screen request, and Escape
// once the show fills the screen, leave the app as it was before: not full screen, nothing filling the window, the
// viewer on the slide it showed, and the deck still moves. An Escape dispatched in the Present click itself, after the
// app's handler, reaches the show before any engine can answer its request; a key pressed right after the click often
// does in Chromium only, as Firefox and WebKit answer the request before the next key arrives.
import { type Page, expect, test } from '@playwright/test';

import { smokeCase } from './support/decks';
import { currentSlide, expectSlide, watch } from './support/player';

const smoke = smokeCase();

const WHEN = ['in the Present click', 'pressed right after Present', 'once the show fills the screen'] as const;

const filling = (page: Page) =>
  page.evaluate(() => document.fullscreenElement !== null || document.querySelector('.sd-player.sd-fill') !== null);

/** The next click's target dispatches Escape once the app's own click handler has run (the window sees it last). */
const escapeInNextClick = (page: Page) =>
  page.evaluate(() => {
    window.addEventListener(
      'click',
      (event) => {
        const key = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
        event.target?.dispatchEvent(key);
      },
      { once: true },
    );
  });

/** The app is the viewer on `slide` again, and stays so: a late answer of the browser fills nothing. */
async function expectViewing(page: Page, slide: string) {
  await expect(page).not.toHaveURL(/present=1/);
  await expect.poll(() => filling(page)).toBe(false);
  await page.waitForTimeout(1_000);
  expect(await filling(page), 'nothing fills the screen a second after the show stopped').toBe(false);
  await expectSlide(page, slide);
}

for (const when of WHEN) {
  test(`Escape ${when} leaves the app as before`, { tag: ['@smoke'] }, async ({ page }) => {
    const problems = watch(page);
    await page.goto('/');
    await page.locator('input[type="file"]').setInputFiles(smoke.input);
    await expect(page.getByRole('toolbar', { name: 'Deck' })).toBeVisible({ timeout: 60_000 });
    await expect.poll(() => currentSlide(page)).not.toBeNull();
    const before = (await currentSlide(page)) ?? '';

    for (let round = 0; round < 3; round++) {
      if (when === 'in the Present click') {
        await escapeInNextClick(page);
      }
      await page.getByRole('button', { name: 'Present', exact: true }).click();
      if (when === 'once the show fills the screen') {
        await expect(page).toHaveURL(/present=1/);
        await expect.poll(() => filling(page)).toBe(true);
      }
      if (when !== 'in the Present click') {
        await page.keyboard.press('Escape');
      }
      await expectViewing(page, before);
    }
    await page.keyboard.press('ArrowRight');
    await expect.poll(() => currentSlide(page)).not.toBe(before);
    expect(problems, 'console errors, page errors and CSP violations').toEqual([]);
  });
}
