// `present()` and Escape in every engine. Escape (or `stop`) before `present()`, while the browser has yet to answer
// its full screen request, or once the show fills the screen leaves the page as it was before `present()`: not full
// screen, no `sd-fill`, no show running, the deck where it was and still usable. A refused request (a frame without the
// fullscreen permission) plays the show filling the window instead, and a stop before the refusal fills nothing.
import { type Frame, type Page, expect, test } from '@playwright/test';

import { HEIGHT, WIDTH, deckSlide, fixture } from './support/deck';
import { type Harness, mountIn, open } from './support/harness';

const DECK = fixture([deckSlide('s1'), deckSlide('s2'), deckSlide('s3')]);
const OPTIONS = { mode: 'view', keyboard: 'host' };

/** What the button's click runs, all in the one user gesture, before the browser can answer the request. */
type Scenario = 'present' | 'escape' | 'stop' | 'twice' | 'again';

interface Armed {
  /** Every `present()` the last click made, settled. */
  presented: Promise<void>;
}

interface Shown {
  readonly fullscreen: boolean;
  readonly fill: boolean;
  readonly presenting: boolean;
  readonly slide: number;
  readonly step: number;
}

/** Adds a Present button (a click on it is a user gesture) that runs `scenario` on the player. */
async function arm(target: Page | Frame, scenario: Scenario): Promise<void> {
  await target.evaluate((run) => {
    document.getElementById('present')?.remove();
    const button = document.createElement('button');
    button.id = 'present';
    button.textContent = 'Present';
    button.style.cssText = 'position:fixed;right:0;bottom:0;z-index:5';
    button.addEventListener('click', () => {
      const { player } = (window as unknown as Harness).sd;
      const shows = [player.present()];
      if (run === 'escape') {
        const key = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
        document.querySelector('.sd-player')?.dispatchEvent(key);
      } else if (run !== 'present') {
        player.stop();
      }
      if (run === 'twice') {
        shows.push(player.present());
        player.stop();
      } else if (run === 'again') {
        shows.push(player.present());
      }
      (window as unknown as Armed).presented = Promise.all(shows).then(() => undefined);
    });
    document.body.append(button);
  }, scenario);
}

/** Clicks Present and waits for every `present()` of the click to settle. */
async function click(target: Page | Frame): Promise<void> {
  await target.click('#present');
  await answered(target);
}

async function answered(target: Page | Frame): Promise<void> {
  await target.evaluate(async () => {
    await (window as unknown as Armed).presented;
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  });
}

async function shown(target: Page | Frame): Promise<Shown> {
  return target.evaluate(() => {
    const { position } = (window as unknown as Harness).sd.player;
    return {
      fullscreen: document.fullscreenElement !== null,
      fill: document.querySelector('.sd-player')?.classList.contains('sd-fill') ?? false,
      presenting: position.presenting,
      slide: position.slide,
      step: position.step,
    };
  });
}

/** The page is `before` again and stays so: no late answer of the browser changes it. */
async function expectAsBefore(target: Page | Frame, before: Shown): Promise<void> {
  await expect.poll(() => shown(target), { timeout: 5_000 }).toEqual(before);
  await target.evaluate(() => new Promise((resolve) => setTimeout(resolve, 300)));
  expect(await shown(target)).toEqual(before);
}

/** The deck still moves, and a show starts and ends again (`page` presses the keys of `target`'s document). */
async function expectUsable(page: Page, target: Page | Frame, before: Shown, fullscreen: boolean): Promise<void> {
  await target.evaluate(() => (window as unknown as Harness).sd.player.next());
  const moved = await shown(target);
  expect(moved.slide).toBe(before.slide + 1);
  await arm(target, 'present');
  await click(target);
  expect(await shown(target)).toMatchObject({ presenting: true, fullscreen, fill: !fullscreen });
  await page.keyboard.press('Escape');
  await expectAsBefore(target, moved);
}

/** The player in a frame without the fullscreen permission: the browser refuses every request. */
async function openFramed(page: Page): Promise<Frame> {
  await page.setViewportSize({ width: WIDTH, height: HEIGHT });
  await page.setContent(
    `<!doctype html><html><body style="margin:0"><iframe allow="fullscreen 'none'" ` +
      `srcdoc="<!doctype html><meta charset=utf-8><body></body>" ` +
      `style="border:0;width:${WIDTH}px;height:${HEIGHT}px"></iframe></body></html>`,
  );
  const frame = page.frames().find((candidate) => candidate !== page.mainFrame());
  if (frame === undefined) {
    throw new Error('the frame did not load');
  }
  await frame.waitForLoadState();
  await mountIn(frame, DECK, OPTIONS);
  return frame;
}

test.describe('present() and Escape', () => {
  test('Escape before present() does nothing; once the show fills the screen it leaves the page as before', async ({
    page,
  }) => {
    await open(page, DECK, OPTIONS);
    const before = await shown(page);
    expect(before).toMatchObject({ fullscreen: false, fill: false, presenting: false });
    await page.locator('.sd-player').focus();
    await page.keyboard.press('Escape');
    await expectAsBefore(page, before);

    await arm(page, 'present');
    await click(page);
    expect(await shown(page)).toMatchObject({ presenting: true, fullscreen: true, fill: false });
    await page.keyboard.press('Escape');
    await expectAsBefore(page, before);
    await expectUsable(page, page, before, true);
  });

  for (const [scenario, what] of [
    ['escape', 'Escape'],
    ['stop', 'stop()'],
    ['twice', 'a second present() and stop()'],
  ] as const) {
    test(`${what} while the full screen request is pending leaves the page as before present()`, async ({ page }) => {
      await open(page, DECK, OPTIONS);
      const before = await shown(page);
      await arm(page, scenario);
      await click(page);
      await expectAsBefore(page, before);
      await expectUsable(page, page, before, true);
    });
  }

  test('a present() after a stop() while the request is pending takes it over and plays full screen', async ({
    page,
  }) => {
    await open(page, DECK, OPTIONS);
    const before = await shown(page);
    await arm(page, 'again');
    await click(page);
    expect(await shown(page)).toMatchObject({ presenting: true, fullscreen: true, fill: false });
    await page.keyboard.press('Escape');
    await expectAsBefore(page, before);
  });

  test('Escape pressed right after the Present click leaves the page as before, whenever the browser answers', async ({
    page,
  }) => {
    await open(page, DECK, OPTIONS);
    const before = await shown(page);
    await arm(page, 'present');
    for (let round = 0; round < 3; round++) {
      await page.click('#present');
      await page.keyboard.press('Escape');
      await answered(page);
      await expectAsBefore(page, before);
    }
    await expectUsable(page, page, before, true);
  });

  test('a refused request plays the show filling the window; Escape before or after the refusal leaves the page as before', async ({
    page,
  }) => {
    const frame = await openFramed(page);
    const before = await shown(frame);
    await arm(frame, 'present');
    await click(frame);
    expect(await shown(frame)).toMatchObject({ presenting: true, fullscreen: false, fill: true });
    await page.keyboard.press('Escape');
    await expectAsBefore(frame, before);

    for (const scenario of ['escape', 'stop', 'twice'] as const) {
      await arm(frame, scenario);
      await click(frame);
      await expectAsBefore(frame, before);
    }
    await expectUsable(page, frame, before, false);
  });
});
