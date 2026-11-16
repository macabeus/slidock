// The font packaging matrix: decks compiled with shipped fonts, unpacked, and
// opened from file:// and from HTTP in every browser project. Every shown slide is presented, and every text node
// must render with a face injected by `<style id="sd-fonts">` (support/fontChecks.ts says how each browser proves
// it), with no console error and no CSP violation. Each click build is played, and the text it shows is checked too.
// The presenter window shows the same faces, in its current slide and its next-slide preview.
import { type BrowserContext, type CDPSession, type Page, type TestInfo, expect, test } from '@playwright/test';
import type { Deck } from '@slidock/types';

import { clickGroups } from './support/decks';
import {
  type Drawn,
  PREVIEW,
  type RenderCheck,
  expectFacesLoaded,
  expectShippedFaces,
  fontSession,
  renderCheck,
  textElements,
  visibleParts,
} from './support/fontChecks';
import { type BuiltDeck, buildFontDeck, fileUrl, fontDeckCases, serveDecks, tamperedDeck } from './support/fontMatrix';
import { CURRENT, currentSlide, expectSlide, settle, watch } from './support/player';

const ORIGINS = ['file://', 'http'] as const;
type Origin = (typeof ORIGINS)[number];

/** A render check that found nothing (support/fontChecks.ts `renderCheck`). */
const PASSED = { notInjected: [], sameAsFallback: [] };

/** What the render check found wrong, without the text it did not compare. */
function failures({ notInjected, sameAsFallback }: RenderCheck): Pick<RenderCheck, 'notInjected' | 'sameAsFallback'> {
  return { notInjected, sameAsFallback };
}

let server: Awaited<ReturnType<typeof serveDecks>> | undefined;

test.beforeAll(async () => {
  server = await serveDecks();
});

test.afterAll(async () => {
  await server?.close();
});

function deckUrl(built: BuiltDeck, origin: Origin, browserName: string): string {
  if (origin === 'file://') {
    return fileUrl(built, browserName);
  }
  if (server === undefined) {
    throw new Error('the deck server is not running');
  }
  return server.url(built.site);
}

/** Presses next until the player shows `id`, at most `presses` times, each press waiting for its slide. */
async function advanceTo(page: Page, id: string, presses: number): Promise<void> {
  for (let press = 0; press < presses && (await currentSlide(page)) !== id; press++) {
    await page.keyboard.press('ArrowRight');
    await expect
      .poll(() => currentSlide(page), { timeout: 10_000 })
      .toBe(id)
      .catch(() => undefined);
  }
  await expectSlide(page, id);
}

/** Plays the shown slide's click groups, so everything it draws is on screen; stops if the slide moves on. */
async function playBuilds(page: Page, id: string, groups: number): Promise<boolean> {
  for (let press = 0; press < groups && (await currentSlide(page)) === id; press++) {
    await page.keyboard.press('ArrowRight');
    await settle(page);
  }
  return (await currentSlide(page)) === id;
}

/**
 * Chromium's message when CDP's CSS domain, turned on in a `file://` page that has already loaded (the presenter
 * window, opened by the deck page), reads the page's own document again: the matrix's probe makes that request, not
 * the page. A session made before the page loads (the deck page) causes none.
 */
const DEVTOOLS_LOAD =
  /^console: Unsafe attempt to load URL (\S+) from frame with URL (\S+)\. 'file:' URLs are treated as unique security origins\.\s*$/;

/** Whether `problem` is that message, about the page at `deckUrl` itself. */
function devToolsLoad(problem: string, deckUrl: string): boolean {
  const [, url, frame] = DEVTOOLS_LOAD.exec(problem) ?? [];
  return url !== undefined && url === frame && url.split('#')[0] === deckUrl;
}

/**
 * Console errors, uncaught exceptions and CSP violations of every page of `context`, from before each page's first
 * script on (support/player.ts `watch` does this for one page; a popup is watched here from its start).
 */
async function watchAll(context: BrowserContext): Promise<(page: Page) => string[]> {
  const problems = new Map<Page | null, string[]>();
  const of = (page: Page | null) => {
    const list = problems.get(page) ?? [];
    problems.set(page, list);
    return list;
  };
  context.on('console', (message) => {
    if (message.type() === 'error') {
      of(message.page()).push(`console: ${message.text()}`);
    }
  });
  context.on('weberror', (error) => of(error.page()).push(`pageerror: ${error.error().message}`));
  await context.addInitScript(() => {
    document.addEventListener('securitypolicyviolation', (event) => {
      console.error(`CSP violation: ${event.violatedDirective} blocked ${event.blockedURI || '(inline)'}`);
    });
  });
  return (page) => of(page);
}

function shownSlides(deck: Deck) {
  return deck.slides.filter((slide) => !slide.hidden);
}

/**
 * Presents every shown slide of the deck `page` has open, plays each click build, and checks the text of each slide,
 * built, with `expectShippedFaces` and `renderCheck`. Text a build shows must appear (it is hidden before its click).
 * Text no pixel of which shows (pictures or opaque boxes in front of it), and emoji a platform colour emoji font draws,
 * are recorded as annotations. Returns how
 * many text elements were checked, and how many of them builds showed.
 */
async function checkEveryShownSlide(
  page: Page,
  built: BuiltDeck,
  client: CDPSession | undefined,
  testInfo: TestInfo,
): Promise<{ checked: number; shownByBuilds: number }> {
  let checked = 0;
  let shownByBuilds = 0;
  const covered: string[] = [];
  const emoji: string[] = [];
  let previous: Deck['slides'][number] | undefined;
  for (const slide of shownSlides(built.deck)) {
    await test.step(`slide ${slide.id}`, async () => {
      if (previous === undefined) {
        await expectSlide(page, slide.id);
      } else {
        await advanceTo(page, slide.id, clickGroups(previous) + 2);
      }
      previous = slide;
      const groups = clickGroups(slide);
      await settle(page);
      const before = new Set((await textElements(page)).map((element) => element.text));
      if (!(await playBuilds(page, slide.id, groups))) {
        testInfo.annotations.push({ type: 'not checked', description: `${slide.id}: moved on by itself` });
        return;
      }
      await settle(page);
      const drawn = await expectShippedFaces(page, built.rules, client, slide.id);
      const { elements } = drawn;
      const check = await renderCheck(page, elements);
      expect(failures(check), `${slide.id}: the render check`).toEqual(PASSED);
      covered.push(...check.covered.map((text) => `${slide.id}: ${text}`));
      emoji.push(...drawn.emoji.map((text) => `${slide.id}: ${text}`));
      const appeared = elements.filter((element) => !before.has(element.text));
      if (groups > 0) {
        expect(
          appeared.map((element) => element.text),
          `${slide.id}: text its click builds show`,
        ).not.toEqual([]);
      }
      checked += elements.length;
      shownByBuilds += appeared.length;
    });
  }
  testInfo.annotations.push({ type: 'text elements checked', description: String(checked) });
  testInfo.annotations.push({ type: 'text elements shown by builds', description: String(shownByBuilds) });
  for (const text of covered) {
    testInfo.annotations.push({ type: 'text covered (no render check)', description: text });
  }
  for (const text of emoji) {
    testInfo.annotations.push({ type: 'emoji drawn by a platform font', description: text });
  }
  return { checked, shownByBuilds };
}

for (const fontDeck of fontDeckCases()) {
  test.describe(`font matrix: ${fontDeck.name}`, () => {
    let built: BuiltDeck;

    test.beforeAll(() => {
      built = buildFontDeck(fontDeck);
      expect(built.rules.length, 'the deck ships faces').toBeGreaterThan(0);
    });

    for (const origin of ORIGINS) {
      test(`${origin}: every text node renders with an injected face`, async ({ page, browserName }, testInfo) => {
        const shown = shownSlides(built.deck);
        test.setTimeout(60_000 + shown.length * 15_000);
        const problems = watch(page);
        const client = browserName === 'chromium' ? await fontSession(page) : undefined;
        await page.goto(deckUrl(built, origin, browserName));
        await expectFacesLoaded(page, built.rules, 'deck page');
        const { checked, shownByBuilds } = await checkEveryShownSlide(page, built, client, testInfo);
        expect(checked, 'text elements checked').toBeGreaterThan(0);
        if (fontDeck.clickBuild !== undefined) {
          expect(shownByBuilds, 'text elements shown by builds').toBeGreaterThan(0);
        }
        expect(problems, 'console errors, page errors and CSP violations').toEqual([]);
      });

      test(`${origin}: the presenter window shows the same faces`, async ({ page, browserName }, testInfo) => {
        const shown = shownSlides(built.deck);
        test.setTimeout(60_000 + shown.length * 25_000);
        const chromium = browserName === 'chromium';
        const problems = await watchAll(page.context());
        const deckClient = chromium ? await fontSession(page) : undefined;
        const url = deckUrl(built, origin, browserName);
        await page.goto(url);
        const first = shown[0];
        if (first === undefined) {
          throw new Error('the deck shows no slide');
        }
        await expectSlide(page, first.id);
        const popup = page.waitForEvent('popup');
        await page.keyboard.press('p');
        const presenter = await popup;
        await expect(presenter.locator('.sd-presenter')).toBeVisible();
        const presenterClient = chromium ? await fontSession(presenter) : undefined;
        await expectFacesLoaded(presenter, built.rules, 'presenter window');
        let previews = 0;
        for (const [i, slide] of shown.entries()) {
          await test.step(`slide ${slide.id}`, async () => {
            if (i > 0) {
              // Go to the slide by its number among the shown ones: the presenter window follows the deck window.
              for (const key of String(i + 1)) {
                await page.keyboard.press(key);
              }
              await page.keyboard.press('Enter');
            }
            // Each build step of the slide, from none played to all: the current slide, then the preview of what the
            // next click shows (this slide's next build, or the next slide).
            for (let step = 0; step <= clickGroups(slide); step++) {
              const at = `${slide.id} step ${step}`;
              if (step > 0) {
                await page.keyboard.press('ArrowRight');
              }
              await expectSlide(page, slide.id);
              await expectSlide(presenter, slide.id);
              await settle(page);
              await settle(presenter);
              const inDeck = await expectShippedFaces(page, built.rules, deckClient, `${at} (deck window)`);
              const inPresenter = await expectShippedFaces(
                presenter,
                built.rules,
                presenterClient,
                `${at} (presenter window)`,
              );
              const faces = (drawn: Drawn[]) => drawn.map((item) => JSON.stringify(item)).sort();
              expect(faces(inPresenter.drawn), `${at}: the presenter window's faces`).toEqual(faces(inDeck.drawn));
              expect(
                failures(await renderCheck(presenter, inPresenter.elements)),
                `${at}: the presenter window's render check`,
              ).toEqual(PASSED);
              const preview = await expectShippedFaces(
                presenter,
                built.rules,
                presenterClient,
                `${at} (next-slide preview)`,
                PREVIEW,
              );
              expect(
                failures(await renderCheck(presenter, preview.elements)),
                `${at}: the preview's render check`,
              ).toEqual(PASSED);
              if (step < clickGroups(slide)) {
                const now = new Set(inPresenter.elements.map((element) => element.text));
                expect(
                  preview.elements.filter((element) => !now.has(element.text)).map((element) => element.text),
                  `${at}: the preview's text the next click shows`,
                ).not.toEqual([]);
              }
              previews += preview.elements.length;
            }
          });
        }
        testInfo.annotations.push({ type: 'preview text elements checked', description: String(previews) });
        if (shown.length > 1) {
          expect(previews, 'preview text elements checked').toBeGreaterThan(0);
        }
        expect(problems(page), 'deck window problems').toEqual([]);
        expect(
          problems(presenter).filter((problem) => !(chromium && origin === 'file://' && devToolsLoad(problem, url))),
          'presenter window problems',
        ).toEqual([]);
      });
    }
  });
}

/** The synthetic deck's first slide title: Pacifico. */
const TITLE_TEXT = 'Crabs & Orcas';
const TITLE = `Pacifico: ${TITLE_TEXT}`;

/**
 * Puts a box over the shown slide's title (support/fontChecks.ts: Pacifico's "Crabs & Orcas"), replacing the one put
 * before: an opaque picture in front of all of it or its left half, a translucent one or an animated GIF (two opaque
 * frames, 20 ms each) in front of all of it, or an opaque white box behind it (in front of the slide's background).
 */
function placeOver(page: Page, kind: 'all' | 'half' | 'translucent' | 'animated' | 'behind'): Promise<void> {
  return page.evaluate(
    ({ selector, title, kind }) => {
      const main = document.querySelector(selector)?.shadowRoot?.querySelector('main[data-slide]');
      const element = Array.from(main?.querySelectorAll('*') ?? []).find(
        (candidate) =>
          Array.from(candidate.childNodes)
            .filter((child) => child.nodeType === Node.TEXT_NODE)
            .map((child) => child.textContent ?? '')
            .join('') === title,
      );
      if (!main || !element) {
        throw new Error('the title is missing');
      }
      main.querySelector('[data-cover]')?.remove();
      // The text's own boxes, with a margin: a script face's glyphs reach past them.
      const range = document.createRange();
      range.selectNodeContents(element);
      const box = range.getBoundingClientRect();
      const origin = main.getBoundingClientRect();
      // The slide is scaled to the stage: lengths inside it are in its own pixels.
      const scale = origin.width / (main as HTMLElement).offsetWidth;
      const margin = 12;
      const width = (kind === 'half' ? box.width / 2 : box.width) + margin * 2;
      const px = (value: number) => `${value / scale}px`;
      const place =
        `position:absolute;left:${px(box.left - origin.left - margin)};top:${px(box.top - origin.top - margin)};` +
        `width:${px(width)};height:${px(box.height + margin * 2)};`;
      if (kind === 'behind') {
        const div = document.createElement('div');
        div.dataset.cover = '';
        div.style.cssText = `${place}background:#fff`;
        main.prepend(div);
        return;
      }
      const img = document.createElement('img');
      img.dataset.cover = '';
      img.src =
        kind === 'animated'
          ? 'data:image/gif;base64,R0lGODlhAQABAIAAABEiM+7dzCH/C05FVFNDQVBFMi4wAwEAAAAh+QQAAgAAACwAAAAAAQABAAAAAgJEAQAh+QQAAgAAACwAAAAAAQABAAAAAgJMAQA7'
          : 'data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg"/>';
      img.style.cssText = `${place}z-index:9;background:#123;opacity:${kind === 'translucent' ? 0.6 : 1}`;
      main.append(img);
    },
    { selector: CURRENT, title: TITLE_TEXT, kind },
  );
}

// The checks themselves: on copies of the synthetic deck whose injection is broken, each check that should see it
// fails.
test.describe('font matrix: the checks catch a deck that does not inject its faces', () => {
  const synthetic = fontDeckCases().find((deck) => deck.key === 'synthetic');
  let built: BuiltDeck;

  test.beforeAll(() => {
    if (synthetic === undefined) {
      throw new Error('the synthetic deck is missing');
    }
    built = buildFontDeck(synthetic);
  });

  /** Opens the tampered deck on its first slide (Pacifico's title), or on slide `at`. */
  async function open(page: Page, tampered: BuiltDeck, browserName: string, at?: string) {
    const client = browserName === 'chromium' ? await fontSession(page) : undefined;
    await page.goto(deckUrl(tampered, 'file://', browserName));
    const shown = shownSlides(tampered.deck);
    const first = shown[0];
    if (first === undefined) {
      throw new Error('the deck shows no slide');
    }
    await expectSlide(page, first.id);
    if (at !== undefined) {
      await advanceTo(page, at, shown.length);
    }
    await settle(page);
    return client;
  }

  test('a CSP that blocks fonts: the faces fail to load, the text draws as the fallback, a violation', async ({
    page,
    browserName,
  }) => {
    const tampered = tamperedDeck(built, 'Pacifico', 'no-font-src');
    const problems = watch(page);
    const client = await open(page, tampered, browserName);
    await expect(expectFacesLoaded(page, tampered.rules, 'tampered')).rejects.toThrow(/did not load/);
    if (client !== undefined) {
      await expect(expectShippedFaces(page, tampered.rules, client, 'tampered')).rejects.toThrow(
        /is not loaded|drew with another face/,
      );
    }
    expect((await renderCheck(page, await textElements(page))).sameAsFallback).toContain(TITLE);
    expect(problems.join('\n')).toMatch(/CSP violation: font-src/);
  });

  test('a face that covers none of its text: the text draws as the fallback', async ({ page, browserName }) => {
    const tampered = tamperedDeck(built, 'Pacifico', 'uncovered');
    const client = await open(page, tampered, browserName);
    if (client !== undefined) {
      await expect(expectShippedFaces(page, tampered.rules, client, 'tampered')).rejects.toThrow(
        /drew with another face/,
      );
    }
    expect((await renderCheck(page, await textElements(page))).sameAsFallback).toContain(TITLE);
    // Seen through a translucent picture, the text is still compared.
    await placeOver(page, 'translucent');
    expect((await renderCheck(page, await textElements(page))).sameAsFallback).toContain(TITLE);
  });

  test('a family that ships no face is found', async ({ page }) => {
    const tampered = tamperedDeck(built, 'Pacifico', 'renamed');
    await open(page, tampered, 'webkit');
    await expect(expectShippedFaces(page, tampered.rules, undefined, 'tampered')).rejects.toThrow(/ships no face/);
  });

  test('a font of this computer drawing in place of the shipped file is found', async ({ page, browserName }) => {
    const tampered = tamperedDeck(built, 'Pacifico', 'shadowed');
    const client = await open(page, tampered, browserName);
    await expect(expectFacesLoaded(page, tampered.rules, 'tampered')).rejects.toThrow(/@font-face count/);
    if (client !== undefined) {
      await expect(expectShippedFaces(page, tampered.rules, client, 'tampered')).rejects.toThrow(
        /drew with another face/,
      );
    }
    expect((await renderCheck(page, await textElements(page))).notInjected).toContain(TITLE);
  });

  test('only text that shows is compared: not text a picture covers, but its part that shows, and text under a translucent picture or over a box behind it; an animated picture is taken out', async ({
    page,
    browserName,
  }) => {
    await open(page, built, browserName);
    /** The slide's text elements, and the title's visible part. */
    const title = async () => {
      const elements = await textElements(page);
      const visible = await visibleParts(page, elements);
      const index = elements.findIndex((element) => element.text === TITLE_TEXT);
      if (index < 0) {
        throw new Error('the title is missing');
      }
      return { elements, visible: visible[index] };
    };
    const { elements, visible: bare } = await title();
    expect(bare, 'the bare title shows').not.toBeNull();
    expect(failures(await renderCheck(page, elements)), 'the bare slide').toEqual(PASSED);

    await placeOver(page, 'all');
    const all = await title();
    expect(all.visible, 'the covered title').toBeNull();
    const check = await renderCheck(page, all.elements);
    expect(failures(check), 'the render check skips the covered title').toEqual(PASSED);
    expect(check.covered, 'what it skips').toEqual([TITLE]);
    expect(all.elements.length, 'the rest of the slide').toBeGreaterThan(check.covered.length);

    await placeOver(page, 'half');
    const half = await title();
    expect(half.visible, 'the half-covered title shows').not.toBeNull();
    expect(half.visible?.width ?? Infinity, 'its visible part').toBeLessThan((bare?.width ?? 0) * 0.75);
    expect(failures(await renderCheck(page, half.elements)), 'the half-covered title, where it shows').toEqual(PASSED);

    for (const kind of ['translucent', 'behind'] as const) {
      await placeOver(page, kind);
      const shown = await title();
      // Through a translucent picture the faintest antialiased edge pixels can blend to the same colour.
      for (const side of ['x', 'y', 'width', 'height'] as const) {
        expect(
          Math.abs((shown.visible?.[side] ?? Infinity) - (bare?.[side] ?? 0)),
          `the title with a ${kind} box: all of it shows (${side})`,
        ).toBeLessThanOrEqual(2);
      }
      const compared = await renderCheck(page, shown.elements);
      expect(compared.covered, `the title with a ${kind} box: compared`).not.toContain(TITLE);
      expect(failures(compared), `the title with a ${kind} box`).toEqual(PASSED);
    }

    // An animated picture is taken out of the slide for the comparison, so the title under it is compared too.
    await placeOver(page, 'animated');
    const animated = await renderCheck(page, await textElements(page));
    expect(animated.covered, 'the title under an animated picture: compared').not.toContain(TITLE);
    expect(failures(animated), 'the title under an animated picture').toEqual(PASSED);
  });

  test('a glyph the face lacks, in a serif family: the platform font that draws it is found', async ({
    page,
    browserName,
  }) => {
    const probe = fontDeckCases().find((deck) => deck.key === 'probe-odp');
    if (probe === undefined) {
      throw new Error('the probe ODP is missing');
    }
    // PT Serif's regular face without "k": "Pack my box" on od-p3 draws its "k" with a font of this computer.
    const tampered = tamperedDeck(buildFontDeck(probe), 'PT Serif', { gap: 0x6b });
    const client = await open(page, tampered, browserName, 'od-p3');
    const elements = await textElements(page);
    const line = elements.find((element) => element.text.includes('Pack my box'));
    expect(line?.stack.at(-1), 'the line falls back to serif').toBe('serif');
    if (client !== undefined) {
      await expect(expectShippedFaces(page, tampered.rules, client, 'tampered')).rejects.toThrow(
        /Pack my box: drawn with .*platform font/,
      );
    }
    expect((await renderCheck(page, elements)).notInjected).toContain(`PT Serif: ${line?.text ?? ''}`);
  });
});
