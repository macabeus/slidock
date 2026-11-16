// What the tests read from a page that runs @slidock/player: the deck page, the app, or a presenter view. The main
// player is `.sd-player`; its stage (`.sd-frame`) holds one layer per mounted slide, the shown one marked
// `data-sd-current`. Thumbnails and previews are `.sd-static` and never match.
import { type Page, type TestInfo, expect } from '@playwright/test';
import type { Deck } from '@slidock/types';

import { differentPixels } from './png';

/** The main player's shown slide layer. */
export const CURRENT = '.sd-player > .sd-frame > .sd-layer[data-sd-current]';

/** Console errors, uncaught exceptions and CSP violations of a page, from before its first script on. */
export function watch(page: Page): string[] {
  const problems: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') {
      problems.push(`console: ${message.text()}`);
    }
  });
  page.on('pageerror', (error) => problems.push(`pageerror: ${error.message}`));
  void page.addInitScript(() => {
    document.addEventListener('securitypolicyviolation', (event) => {
      console.error(`CSP violation: ${event.violatedDirective} blocked ${event.blockedURI || '(inline)'}`);
    });
  });
  return problems;
}

/** The id of the slide the page's player shows, or `null`. */
export function currentSlide(page: Page): Promise<string | null> {
  return page.evaluate((selector) => document.querySelector(selector)?.getAttribute('data-sd-layer') ?? null, CURRENT);
}

/** The player's blank screen: `black`, `white` or `''`. */
export function blankOf(page: Page): Promise<string> {
  return page.evaluate(() => document.querySelector('.sd-player > .sd-blank')?.getAttribute('data-sd-blank') ?? '');
}

/**
 * Waits until the app's fonts flow can no longer swap the deck under the test: each re-finish (a substitute or
 * another font arriving) replaces the player, and with it the layer a check is reading.
 */
export async function expectSettled(page: Page, timeout: number): Promise<void> {
  await expect(page.getByTestId('stage'), 'the fonts flow settles').toHaveAttribute('aria-busy', 'false', { timeout });
}

/** Counts the players the page creates from now on; resolves with a reader of the count. */
export async function countPlayers(page: Page): Promise<() => Promise<number>> {
  const key = '__sdPlayersMade';
  await page.evaluate((key) => {
    const counter = { made: 0 };
    (window as unknown as Record<string, typeof counter>)[key] = counter;
    new MutationObserver((records) => {
      for (const record of records) {
        for (const node of Array.from(record.addedNodes)) {
          if (node instanceof HTMLElement && node.classList.contains('sd-player')) {
            counter.made += 1;
          }
        }
      }
    }).observe(document.body, { subtree: true, childList: true });
  }, key);
  return () => page.evaluate((key) => (window as unknown as Record<string, { made: number }>)[key]?.made ?? -1, key);
}

export async function expectSlide(page: Page, id: string, timeout = 15_000): Promise<void> {
  await expect.poll(() => currentSlide(page), { timeout, message: `the player shows ${id}` }).toBe(id);
}

export async function expectBlank(page: Page, kind: string, timeout = 10_000): Promise<void> {
  await expect.poll(() => blankOf(page), { timeout, message: `the screen is blank "${kind}"` }).toBe(kind);
}

/**
 * Waits until no animation runs (a transition or a build, in the page or a slide's shadow root), for at most
 * `timeout`.
 */
export async function settle(page: Page, timeout = 5_000): Promise<void> {
  await page
    .waitForFunction(
      () =>
        [
          ...document.getAnimations(),
          ...Array.from(
            document.querySelectorAll('.sd-layer'),
            (layer) => layer.shadowRoot?.getAnimations() ?? [],
          ).flat(),
        ].every((animation) => animation.playState !== 'running') && !document.querySelector('[data-sd-incoming]'),
      undefined,
      { timeout, polling: 50 },
    )
    .catch(() => undefined);
}

/** The shown slide's markup is mounted in its shadow root. */
export async function expectMounted(page: Page, id: string): Promise<void> {
  const mounted = await page.evaluate(
    ({ selector, id }) =>
      document.querySelector(selector)?.shadowRoot?.querySelector(`main[data-slide="${id}"]`) !== null,
    { selector: CURRENT, id },
  );
  expect(mounted, `slide ${id} is mounted in its shadow root`).toBe(true);
}

interface Override {
  /** Makes every element of the slide but its background transparent. */
  content?: boolean;
  /** Paints the stage behind the slide this colour. */
  backdrop?: string;
}

const RESTORE = '__sdRestore';

/**
 * Screenshots the shown slide's layer, with the player's chrome (the counter, the help, the live region) hidden and
 * `override` applied. The styles are inline and `!important`, set through the CSSOM: they win over the slide's
 * animations, and no CSP blocks them.
 */
async function shoot(page: Page, override: Override): Promise<Buffer> {
  await page.evaluate(
    ({ selector, override, restore }) => {
      const undo: (() => void)[] = [];
      const set = (element: Element, property: string, value: string) => {
        const { style } = element as HTMLElement;
        const old = style.getPropertyValue(property);
        const priority = style.getPropertyPriority(property);
        style.setProperty(property, value, 'important');
        undo.push(() => (old === '' ? style.removeProperty(property) : style.setProperty(property, old, priority)));
      };
      const layer = document.querySelector(selector);
      layer?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
      const player = layer?.closest('.sd-player');
      for (const chrome of Array.from(player?.children ?? [])) {
        if (!chrome.classList.contains('sd-frame')) {
          set(chrome, 'visibility', 'hidden');
        }
      }
      if (player && override.backdrop !== undefined) {
        set(player, 'background', override.backdrop);
      }
      const main = layer?.shadowRoot?.querySelector('main[data-slide]');
      for (const child of Array.from(override.content ? (main?.children ?? []) : [])) {
        if (!child.hasAttribute('data-sd-bg')) {
          set(child, 'opacity', '0');
        }
      }
      (window as unknown as Record<string, () => void>)[restore] = () => undo.reverse().forEach((step) => step());
    },
    { selector: CURRENT, override, restore: RESTORE },
  );
  try {
    // The layer's box on the page, at one pixel per CSS pixel: a locator screenshot waits for the element to be
    // stable and scrolls it, and at the device's scale doubles the pixels to compare.
    const box = await page.locator(CURRENT).boundingBox();
    if (box === null) {
      throw new Error('the shown slide has no box');
    }
    return await page.screenshot({ clip: box, scale: 'css' });
  } finally {
    await page.evaluate((restore) => (window as unknown as Record<string, () => void>)[restore]?.(), RESTORE);
  }
}

/** Whether the shown slide has elements besides its background (the `data-sd-bg` child). */
async function hasContent(page: Page): Promise<boolean> {
  return page.evaluate((selector) => {
    const main = document.querySelector(selector)?.shadowRoot?.querySelector('main[data-slide]');
    return Array.from(main?.children ?? []).some((child) => !child.hasAttribute('data-sd-bg'));
  }, CURRENT);
}

/**
 * The shown slide renders: its elements draw something its background alone does not, or, on a slide that is only a
 * background, the background covers the stage (the slide looks the same over two backdrop colours). The outermost
 * pixel ring of the layer is left out: the scaled box can cut through it.
 */
export async function expectDrawn(page: Page, id: string): Promise<void> {
  await settle(page);
  if (await hasContent(page)) {
    const drawn = await shoot(page, {});
    const background = await shoot(page, { content: true });
    expect(differentPixels(drawn, background, 1), `slide ${id} draws more than its background`).toBeGreaterThan(0);
  } else {
    const onMagenta = await shoot(page, { backdrop: '#f0f' });
    const onGreen = await shoot(page, { backdrop: '#0f0' });
    expect(differentPixels(onMagenta, onGreen, 1), `slide ${id} paints its background over the stage`).toBe(0);
  }
}

export interface MediaCheck {
  /** Media that failed to load. */
  failed: string[];
  /** Videos the browser cannot decode (`canPlayType` says no). */
  unsupported: string[];
}

/**
 * Loads the shown slide's media: each image must decode, and each video must read its metadata (a probe element
 * with the same URL) unless the browser cannot play its type.
 */
export async function checkMedia(page: Page, deck: Deck): Promise<MediaCheck> {
  const mimes = Object.fromEntries(deck.media.map((medium) => [medium.path.split('/').pop() ?? '', medium.mime]));
  const videoMimes = [...new Set(deck.media.map((medium) => medium.mime).filter((mime) => mime.startsWith('video/')))];
  return page.evaluate(
    async ({ selector, mimes, videoMimes }) => {
      const root = document.querySelector(selector)?.shadowRoot;
      const failed: string[] = [];
      const unsupported: string[] = [];
      const name = (url: string) => decodeURIComponent(url.split(/[?#]/)[0]?.split('/').pop() ?? url);
      const timeout = <T>(promise: Promise<T>, ms: number, value: T) =>
        Promise.race([promise, new Promise<T>((resolve) => setTimeout(() => resolve(value), ms))]);
      for (const img of Array.from(root?.querySelectorAll('img') ?? [])) {
        const ok = await timeout(
          img.decode().then(
            () => true,
            () => false,
          ),
          15_000,
          false,
        );
        if (!ok || img.naturalWidth === 0) {
          failed.push(`img ${name(img.currentSrc || img.src)}`);
        }
      }
      for (const video of Array.from(root?.querySelectorAll('video') ?? [])) {
        const src = video.currentSrc || video.src;
        const mime = mimes[name(src)] ?? '';
        const probe = document.createElement('video');
        probe.muted = true;
        probe.preload = 'metadata';
        const loaded = new Promise<boolean>((resolve) => {
          probe.addEventListener('loadedmetadata', () => resolve(true), { once: true });
          probe.addEventListener('error', () => resolve(false), { once: true });
        });
        probe.src = src;
        const ok = await timeout(loaded, 20_000, false);
        // A `blob:` URL (the app) names no file: a source the browser refuses counts as unsupported when the deck
        // has a video type this browser cannot play.
        const cannot =
          mime === ''
            ? probe.error?.code === MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED &&
              videoMimes.some((type) => probe.canPlayType(type) === '')
            : probe.canPlayType(mime) === '';
        if (!ok) {
          (cannot ? unsupported : failed).push(`video ${name(src)} (${mime || 'type unknown'})`);
        }
        probe.removeAttribute('src');
        probe.load();
      }
      return { failed, unsupported };
    },
    { selector: CURRENT, mimes, videoMimes },
  );
}

/** Records media a browser cannot decode on the test. */
export function noteUnsupported(testInfo: TestInfo, slide: string, media: MediaCheck): void {
  for (const item of media.unsupported) {
    testInfo.annotations.push({ type: 'unsupported media', description: `${slide}: ${item}` });
  }
}
