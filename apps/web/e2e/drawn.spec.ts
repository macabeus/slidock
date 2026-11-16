// The non-blank check the other specs run on every slide (`expectDrawn`), on the smoke deck from file://: it passes
// on a slide that renders, and fails on one whose elements render nothing or whose background does not paint.
import { type Page, expect, test } from '@playwright/test';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import { compiledDeck, siteDir, smokeCase } from './support/decks';
import { CURRENT, expectDrawn, expectSlide } from './support/player';

/** Applies `change` to each element of the shown slide but its background, or to the background alone. */
function alter(page: Page, which: 'elements' | 'background', change: 'hidden' | 'removed') {
  return page.evaluate(
    ({ selector, which, change }) => {
      const main = document.querySelector(selector)?.shadowRoot?.querySelector('main[data-slide]');
      for (const child of Array.from(main?.children ?? [])) {
        if (child.hasAttribute('data-sd-bg') === (which === 'background')) {
          if (change === 'removed') {
            child.remove();
          } else {
            (child as HTMLElement).style.setProperty('visibility', 'hidden');
          }
        }
      }
    },
    { selector: CURRENT, which, change },
  );
}

test('the non-blank check fails on a slide that renders nothing', { tag: ['@smoke'] }, async ({ page }) => {
  const smoke = smokeCase();
  const id = compiledDeck(smoke).slides[0]?.id ?? '';
  await page.goto(pathToFileURL(join(siteDir(smoke), 'index.html')).href);
  await expectSlide(page, id);
  await expectDrawn(page, id);

  await alter(page, 'elements', 'hidden');
  await expect(expectDrawn(page, id)).rejects.toThrow(/draws more than its background/);

  await alter(page, 'elements', 'removed');
  await expectDrawn(page, id);
  await alter(page, 'background', 'hidden');
  await expect(expectDrawn(page, id)).rejects.toThrow(/paints its background over the stage/);
});
