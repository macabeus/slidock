// The suite's compile-progress reader (support/progress.ts) on a page that stands in for the app: the conversion
// panel, then the deck's toolbar in its place. The app swaps the two in one commit that may land between any two of
// the reader's calls, so each poll must read the panel in one call that does not wait for it.
import { type Locator, type Page, expect, test } from '@playwright/test';

import { untilCompiled } from './support/progress';

const PANEL = (phase: string) =>
  `<section aria-label="Conversion"><ol>` +
  `<li>Reading and lowering the slides</li><li aria-current="step">${phase}</li>` +
  `</ol></section>`;

/** The app's commit that shows the deck: the conversion panel goes, the deck's toolbar takes its place. */
function showDeck(page: Page) {
  return page.evaluate(() => {
    document.querySelector('[aria-label="Conversion"]')?.remove();
    document.body.insertAdjacentHTML('beforeend', '<div role="toolbar" aria-label="Deck">Slide 1 of 1</div>');
  });
}

/**
 * `page`, where the app shows the deck as soon as the first read of the conversion panel's step has answered:
 * before any later call reaches the page, as the trace of the Firefox stall shows it.
 */
function swappingAfterFirstStepRead(page: Page): Page {
  let swapped = false;
  const wrap = (locator: Locator): Locator =>
    new Proxy(locator, {
      get(target, key, receiver) {
        const value: unknown = Reflect.get(target, key, receiver);
        if (typeof value !== 'function') {
          return value;
        }
        return (...args: unknown[]) => {
          const result: unknown = Reflect.apply(value, target, args);
          if (result instanceof Promise) {
            return result.then(async (answer: unknown) => {
              if (!swapped) {
                swapped = true;
                await showDeck(page);
              }
              return answer;
            });
          }
          return typeof result === 'object' && result !== null && 'locator' in result
            ? wrap(result as Locator)
            : result;
        };
      },
    });
  return new Proxy(page, {
    get(target, key, receiver) {
      if (key === 'getByLabel') {
        return (...args: Parameters<Page['getByLabel']>) => wrap(target.getByLabel(...args));
      }
      const value: unknown = Reflect.get(target, key, receiver);
      return typeof value === 'function' ? value.bind(target) : value;
    },
  });
}

test('the progress reader follows the phases and ends when the toolbar replaces the panel', async ({ page }) => {
  await page.setContent(PANEL('Fonts, text metrics and layout'));
  setTimeout(() => void page.setContent(PANEL('Writing the slides')), 1_200);
  setTimeout(() => void showDeck(page), 2_400);
  expect(await untilCompiled(page, 10_000)).toEqual(['Fonts, text metrics and layout', 'Writing the slides']);
});

test('the progress reader ends when the deck replaces the panel between two of its reads', async ({ page }) => {
  await page.setContent(PANEL('Writing the slides'));
  expect(await untilCompiled(swappingAfterFirstStepRead(page), 5_000)).toEqual(['Writing the slides']);
});
