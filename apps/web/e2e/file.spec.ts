// The CLI's zip, unpacked and opened from file://: the deck page presents every shown slide with its media, and the
// page's CSP blocks nothing it needs.
import { type Page, expect, test } from '@playwright/test';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import { clickGroups, compiledDeck, deckCases, siteDir, timeoutFor } from './support/decks';
import {
  checkMedia,
  currentSlide,
  expectDrawn,
  expectMounted,
  expectSlide,
  noteUnsupported,
  settle,
  watch,
} from './support/player';

/**
 * Plays the shown slide's `groups` click groups, each press waiting for its build; stops when the slide moves on.
 * A press that lands while a move still runs (a build, a transition, or the readiness gate, which shows no animation)
 * only finishes it, so a group can still be unplayed after this.
 */
async function playBuilds(page: Page, id: string, groups: number): Promise<void> {
  for (let press = 0; press < groups && (await currentSlide(page)) === id; press++) {
    await page.keyboard.press('ArrowRight');
    await settle(page);
  }
}

/**
 * Presses next until the player shows `id`, at most `presses` times: the press meant to bring the slide waits for it
 * before another is sent, so no press can skip past it.
 */
async function advanceTo(page: Page, id: string, presses: number): Promise<void> {
  for (let press = 0; press < presses && (await currentSlide(page)) !== id; press++) {
    await page.keyboard.press('ArrowRight');
    await expect
      .poll(() => currentSlide(page), { timeout: 10_000 })
      .toBe(id)
      .catch(() => undefined);
  }
}

for (const deck of deckCases()) {
  const tag = deck.smoke ? ['@smoke'] : [];
  test(`file://: ${deck.name} presents every slide with its media`, { tag }, async ({ page }, testInfo) => {
    const compiled = compiledDeck(deck);
    const shown = compiled.slides.filter((slide) => !slide.hidden);
    test.setTimeout(timeoutFor(deck, compiled.slides.length));
    const problems = watch(page);

    await page.goto(pathToFileURL(join(siteDir(deck), 'index.html')).href);
    let previous: (typeof shown)[number] | undefined;
    for (const slide of shown) {
      if (previous !== undefined) {
        await advanceTo(page, slide.id, clickGroups(previous) + 2);
      }
      await expectSlide(page, slide.id);
      await expectMounted(page, slide.id);
      const media = await checkMedia(page, compiled);
      expect(media.failed, `media of ${slide.id}`).toEqual([]);
      noteUnsupported(testInfo, slide.id, media);
      // Fully built, as the app shows it: what the slide draws may all be builds.
      await playBuilds(page, slide.id, clickGroups(slide));
      if ((await currentSlide(page)) === slide.id) {
        await expectDrawn(page, slide.id);
      } else {
        testInfo.annotations.push({ type: 'not checked drawn', description: `${slide.id}: moved on by itself` });
      }
      previous = slide;
    }
    expect(problems, 'console errors, page errors and CSP violations').toEqual([]);
  });
}
