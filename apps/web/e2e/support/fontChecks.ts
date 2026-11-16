// What the font matrix asserts about the shown slide of a page that runs @slidock/player (the deck page or the
// presenter view): every text node renders with a face of `<style id="sd-fonts">`.
//
// - Every browser: each text-bearing element names a shipped family first; every `@font-face` loads (none errors);
//   and the pixels of its visible text equal a render through aliases that can only resolve to the shipped files and
//   differ from a render with only the stack's generics (`renderCheck`), so the injected faces draw its glyphs. Text
//   of which no pixel shows (`visibleParts`: a picture or an opaque box in front of it) is left to the other checks.
// - Chromium also asks the renderer (CDP `CSS.getPlatformFontsForNode`) which fonts drew each element's text: every
//   one must be a web font (`isCustomFont`) that is a file of a shipped family of the element's font stack
//   (`shipsFor`). The one exception is a colour emoji font drawing an emoji (`Emoji_Presentation`, or a character with
//   VS16): those are recorded (`emoji`), since no deck ships a colour emoji face.
import { type CDPSession, type Page, expect } from '@playwright/test';

import type { ShippedRule } from './fontMatrix';
import { CURRENT, settle } from './player';
import { differenceBox, differentPixels } from './png';

export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** A text-bearing element of the shown slide: one with a direct child text node that is not only white space. */
export interface TextElement {
  index: number;
  text: string;
  /** The first family of its computed `font-family`. */
  family: string;
  /** The rest of its computed `font-family`: what draws it when the first family has no face. */
  fallback: string;
  /** Every family of its computed `font-family`, unquoted, generics included. */
  stack: string[];
  weight: string;
  style: string;
  /** Its box in the viewport, cut to the viewport. */
  box: Box;
}

const NODES = '__sdFontNodes';

/** The presenter window's next-slide preview: a still render of the next slide, or of this slide's next build. */
export const PREVIEW = '.sd-presenter .sd-next .sd-static > .sd-frame > .sd-layer[data-sd-current]';

/**
 * The visible text-bearing elements of the slide layer `selector` (the main player's shown slide by default), in
 * document order; they are also kept in `window.__sdFontNodes` (by `index`) for CDP to find.
 */
export function textElements(page: Page, selector: string = CURRENT): Promise<TextElement[]> {
  return page.evaluate(
    ({ selector, nodes }) => {
      const main = document.querySelector(selector)?.shadowRoot?.querySelector('main[data-slide]');
      const found: Element[] = [];
      const walk = (element: Element) => {
        if (
          Array.from(element.childNodes).some(
            (child) => child.nodeType === Node.TEXT_NODE && (child.textContent ?? '').trim() !== '',
          )
        ) {
          found.push(element);
        }
        for (const child of Array.from(element.children)) {
          walk(child);
        }
      };
      if (main) {
        walk(main);
      }
      /** The first family of a computed `font-family`, unquoted, and the rest of the list. */
      const split = (list: string): [string, string] => {
        const first = /^\s*(?:"((?:\\.|[^"\\])*)"|'((?:\\.|[^'\\])*)'|([^,]*))\s*(?:,|$)/.exec(list);
        const family = (first?.[1] ?? first?.[2] ?? first?.[3] ?? '').replace(/\\(.)/g, '$1').trim();
        return [family, list.slice(first?.[0].length ?? 0).trim()];
      };
      const families = (list: string): string[] => {
        const out: string[] = [];
        for (let rest = list.trim(); rest !== '';) {
          const [family, next] = split(rest);
          out.push(family);
          rest = next;
        }
        return out;
      };
      const kept: Element[] = [];
      const out = [];
      for (const element of found) {
        const style = getComputedStyle(element);
        const rect = element.getBoundingClientRect();
        const x = Math.max(0, rect.left);
        const y = Math.max(0, rect.top);
        const width = Math.min(window.innerWidth, rect.right) - x;
        const height = Math.min(window.innerHeight, rect.bottom) - y;
        if (style.visibility !== 'visible' || width < 1 || height < 1) {
          continue;
        }
        const [family, fallback] = split(style.fontFamily);
        out.push({
          index: kept.length,
          text: Array.from(element.childNodes)
            .filter((child) => child.nodeType === Node.TEXT_NODE)
            .map((child) => child.textContent ?? '')
            .join(''),
          family,
          fallback,
          stack: families(style.fontFamily),
          weight: style.fontWeight,
          style: style.fontStyle,
          box: { x, y, width, height },
        });
        kept.push(element);
      }
      (window as unknown as Record<string, Element[]>)[nodes] = kept;
      return out;
    },
    { selector, nodes: NODES },
  );
}

/** Each `@font-face` of the page, loaded: its family, weight, style and final status. */
function loadAllFaces(page: Page): Promise<{ face: string; status: string }[]> {
  return page.evaluate(async () => {
    await document.fonts.ready;
    const faces = Array.from(document.fonts);
    return Promise.all(
      faces.map(async (face) => ({
        face: `${face.family} ${face.weight} ${face.style}`,
        status: await face.load().then(
          () => face.status,
          () => 'error',
        ),
      })),
    );
  });
}

/** Every shipped face loads in this page; none errors. */
export async function expectFacesLoaded(page: Page, rules: readonly ShippedRule[], what: string): Promise<void> {
  const faces = await loadAllFaces(page);
  expect(faces.length, `${what}: the page's @font-face count`).toBe(rules.length);
  expect(
    faces.filter((face) => face.status !== 'loaded'),
    `${what}: faces that did not load`,
  ).toEqual([]);
}

/** The face that draws one element, as Chromium reports it. */
interface PlatformFont {
  familyName: string;
  postScriptName: string;
  isCustomFont: boolean;
  glyphCount: number;
}

/** What drew each of the shown slide's text elements (after `textElements`), from Chromium's renderer. */
async function platformFonts(client: CDPSession, count: number): Promise<PlatformFont[][]> {
  await client.send('DOM.getDocument', { depth: 0 });
  const out: PlatformFont[][] = [];
  for (let i = 0; i < count; i++) {
    const { result } = await client.send('Runtime.evaluate', { expression: `window.${NODES}[${i}]` });
    if (result.objectId === undefined) {
      throw new Error(`text element ${i} is gone`);
    }
    const { nodeId } = await client.send('DOM.requestNode', { objectId: result.objectId });
    const { fonts } = await client.send('CSS.getPlatformFontsForNode', { nodeId });
    out.push(fonts.filter((font) => font.glyphCount > 0));
    await client.send('Runtime.releaseObject', { objectId: result.objectId });
  }
  return out;
}

/** A CDP session on `page` with the DOM and CSS domains on. */
export async function fontSession(page: Page): Promise<CDPSession> {
  const client = await page.context().newCDPSession(page);
  await client.send('DOM.enable');
  await client.send('CSS.enable');
  return client;
}

/**
 * Whether `font` is a file that `family`'s rules ship: by PostScript name, or by family for a variable face (its rule
 * spans a weight range, and the browser names the instance it draws, such as `Oswald-Regular_Bold` or an `fvar`
 * instance's own name) or a file with no PostScript name.
 */
function shipsFor(rules: readonly ShippedRule[], family: string, font: PlatformFont): boolean {
  return rules.some((rule) => {
    if (rule.cssFamily !== family) {
      return false;
    }
    const variable = rule.weight.trim().includes(' ');
    return rule.postscript !== '' && !variable
      ? font.postScriptName === rule.postscript
      : font.familyName === rule.family;
  });
}

/** The generic families of CSS Fonts 4, which a `font-family` names unquoted. */
const GENERICS = new Set([
  'serif',
  'sans-serif',
  'monospace',
  'cursive',
  'fantasy',
  'system-ui',
  'ui-serif',
  'ui-sans-serif',
  'ui-monospace',
  'ui-rounded',
  'math',
  'emoji',
  'fangsong',
]);

/** Platform colour emoji fonts: the only fonts besides shipped ones that may draw a slide's text, and only emoji. */
const COLOUR_EMOJI = /^(Apple Color Emoji|Noto Color Emoji|Segoe UI Emoji|Twemoji Mozilla)$/;

/** A character that renders as a colour emoji by default, or one asked to with VS16. */
const EMOJI = /\p{Emoji_Presentation}|\uFE0F/u;

/** Text made only of emoji (and what joins or modifies them) and white space. */
const ONLY_EMOJI =
  /^(?:\p{Emoji_Presentation}|\p{Extended_Pictographic}|\p{Emoji_Modifier}|\u200D|\uFE0F|\s|\p{Regional_Indicator})+$/u;

/** One element's faces, for the test's messages and the deck-vs-presenter comparison. */
export interface Drawn {
  text: string;
  family: string;
  weight: string;
  style: string;
  /** Chromium only: the PostScript names (or families) that drew it, sorted. */
  fonts?: string[];
}

/**
 * The text of slide layer `selector` (the shown slide by default) renders with shipped faces: its elements name a
 * shipped family first, the faces of that family their text needs are loaded (a later shipped family of the stack may
 * draw nothing, so its faces need not load), and in Chromium every font that drew them is a web font of a shipped
 * family of their stack, or a colour emoji font drawing emoji. Returns what drew each element, and the emoji a
 * platform font drew (`family: text (font)`).
 */
export async function expectShippedFaces(
  page: Page,
  rules: readonly ShippedRule[],
  client: CDPSession | undefined,
  what: string,
  selector: string = CURRENT,
): Promise<{ elements: TextElement[]; drawn: Drawn[]; emoji: string[] }> {
  await page.evaluate(() => document.fonts.ready.then(() => undefined));
  const elements = await textElements(page, selector);
  const families = new Set(rules.map((rule) => rule.cssFamily));
  expect(
    elements.filter((element) => !families.has(element.family)).map((element) => `${element.family}: ${element.text}`),
    `${what}: text whose family ships no face`,
  ).toEqual([]);
  const quote = (family: string) => (GENERICS.has(family) ? family : `"${family.replace(/["\\]/g, '\\$&')}"`);
  const checked = await page.evaluate(
    (items) =>
      items.filter(({ font, text }) => !document.fonts.check(font, text)).map(({ font, text }) => `${font}: ${text}`),
    elements.map((element) => ({
      font: `${element.style} ${element.weight} 16px ${quote(element.family)}`,
      text: element.text,
    })),
  );
  expect(checked, `${what}: text whose face is not loaded (document.fonts.check)`).toEqual([]);
  const drawn: Drawn[] = elements.map(({ text, family, weight, style }) => ({ text, family, weight, style }));
  const emoji: string[] = [];
  if (client !== undefined) {
    const fonts = await platformFonts(client, elements.length);
    const wrong: string[] = [];
    elements.forEach((element, i) => {
      const used = fonts[i] ?? [];
      if (used.length === 0) {
        wrong.push(`${element.text}: no font reported`);
      }
      const shipped = element.stack.filter((family) => families.has(family));
      for (const font of used) {
        if (font.isCustomFont && shipped.some((family) => shipsFor(rules, family, font))) {
          continue;
        }
        if (!font.isCustomFont && COLOUR_EMOJI.test(font.familyName) && EMOJI.test(element.text)) {
          emoji.push(`${element.family}: ${element.text} (${font.familyName})`);
          continue;
        }
        wrong.push(
          `${element.text}: drawn with ${font.familyName} (${font.postScriptName}, ` +
            `${font.isCustomFont ? 'web font' : 'platform font'}, ${font.glyphCount} glyphs), ` +
            `not a file of ${shipped.map((family) => `"${family}"`).join(', ')}`,
        );
      }
      const entry = drawn[i];
      if (entry) {
        entry.fonts = used.map((font) => font.postScriptName || font.familyName).sort();
      }
    });
    expect(wrong, `${what}: text Chromium drew with another face`).toEqual([]);
  }
  return { elements, drawn, emoji };
}

async function frames(page: Page): Promise<void> {
  await page.evaluate(
    () => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))),
  );
}

/** Each box's pixels; `undefined` where there is no box. */
async function shootAll(page: Page, boxes: readonly (Box | null)[]): Promise<(Buffer | undefined)[]> {
  const shots: (Buffer | undefined)[] = [];
  for (const box of boxes) {
    shots.push(box === null ? undefined : await page.screenshot({ clip: box, scale: 'css' }));
  }
  return shots;
}

/**
 * The levels (of 255) a channel may move between two renders of the same text and still count as the same pixel: a
 * translucent picture over the text, painted again, can blend a level apart. A different face moves glyph edges by far
 * more.
 */
const TOLERANCE = 2;

/** What makes an element's own text transparent, and what its child elements keep (as they are drawn). */
const TEXT_PAINT: readonly [string, string][] = [
  ['color', 'transparent'],
  ['-webkit-text-fill-color', 'transparent'],
  ['-webkit-text-stroke-color', 'transparent'],
  ['text-decoration-color', 'transparent'],
  ['text-emphasis-color', 'transparent'],
  ['text-shadow', 'none'],
];

/**
 * The part of each element's box (after `textElements`) where its own text shows, as the page draws it now: the box
 * around the pixels that change when its text alone turns transparent (its child elements keep theirs). `null` when
 * no pixel does: text a picture, a filled shape or another opaque box in front of it covers entirely. Text under a
 * translucent picture, or over a shape behind it, shows; a pixel that changes by itself (an animated picture over the
 * text) does not count.
 */
export async function visibleParts(page: Page, elements: readonly TextElement[]): Promise<(Box | null)[]> {
  /** Makes element `index`'s own text transparent, and puts back the one hidden before; `null` only puts it back. */
  const hide = (index: number | null) =>
    page.evaluate(
      async ({ index, nodes, paint }) => {
        const state = window as unknown as Record<string, unknown>;
        const key = `${nodes}Hidden`;
        const saved = (state[key] ?? []) as [HTMLElement, string, string, string][];
        for (const [target, property, value, priority] of saved) {
          target.style.removeProperty(property);
          if (value !== '') {
            target.style.setProperty(property, value, priority);
          }
        }
        state[key] = [];
        const element = index === null ? undefined : ((state[nodes] ?? []) as HTMLElement[])[index];
        if (element !== undefined) {
          const children = Array.from(element.children).filter((child) => child instanceof HTMLElement);
          const kept = children.map((child) => {
            const style = getComputedStyle(child);
            return paint.map(([property]) => style.getPropertyValue(property));
          });
          const next: [HTMLElement, string, string, string][] = [];
          const set = (target: HTMLElement, property: string, value: string) => {
            next.push([
              target,
              property,
              target.style.getPropertyValue(property),
              target.style.getPropertyPriority(property),
            ]);
            target.style.setProperty(property, value, 'important');
          };
          for (const [property, value] of paint) {
            set(element, property, value);
          }
          children.forEach((child, i) => {
            paint.forEach(([property], p) => {
              const value = kept[i]?.[p] ?? '';
              if (value !== '') {
                set(child, property, value);
              }
            });
          });
          state[key] = next;
        }
        await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
      },
      { index, nodes: NODES, paint: TEXT_PAINT },
    );
  // The pictures that may cover the text are drawn before anything is measured (a preview mounts with its slide).
  await page.evaluate(async (nodes) => {
    const elements = ((window as unknown as Record<string, unknown>)[nodes] ?? []) as Element[];
    const roots = new Set(elements.map((element) => element.getRootNode()));
    const images = [...roots].flatMap((root) =>
      root instanceof ShadowRoot || root instanceof Document ? Array.from(root.querySelectorAll('img')) : [],
    );
    await Promise.all(images.map((image) => image.decode().catch(() => undefined)));
  }, NODES);
  await hide(null);
  const shown = await shootAll(
    page,
    elements.map((element) => element.box),
  );
  const out: (Box | null)[] = [];
  try {
    for (const [i, element] of elements.entries()) {
      await hide(i);
      const hidden = await page.screenshot({ clip: element.box, scale: 'css' });
      await hide(null);
      const again = await page.screenshot({ clip: element.box, scale: 'css' });
      const before = shown[i];
      const change = before && differenceBox(before, hidden, again, TOLERANCE);
      if (!change) {
        out.push(null);
        continue;
      }
      // A pixel round the change, inside the box: screenshots of a box at a fractional position round it.
      const { box } = element;
      const x = Math.max(box.x, box.x + change.x - 1);
      const y = Math.max(box.y, box.y + change.y - 1);
      out.push({
        x,
        y,
        width: Math.min(box.x + box.width, box.x + change.x + change.width + 1) - x,
        height: Math.min(box.y + box.height, box.y + change.y + change.height + 1) - y,
      });
    }
  } finally {
    await hide(null);
  }
  return out;
}

/**
 * Takes the moving pictures out of the slide that holds the elements (after `textElements`), so two renders of its
 * text compare: a GIF is hidden (no browser pauses one, and the text under it then shows and is compared too) and a
 * playing video paused; `false` puts them back.
 */
function stillPictures(page: Page, still: boolean): Promise<void> {
  return page.evaluate(
    async ({ still, nodes }) => {
      const state = window as unknown as Record<string, unknown>;
      const key = `${nodes}Still`;
      const saved = (state[key] ?? []) as [HTMLElement, string, string, boolean][];
      for (const [target, value, priority, playing] of saved) {
        target.style.removeProperty('visibility');
        if (value !== '') {
          target.style.setProperty('visibility', value, priority);
        }
        if (playing && target instanceof HTMLVideoElement) {
          await target.play().catch(() => undefined);
        }
      }
      state[key] = [];
      if (!still) {
        return;
      }
      const elements = (state[nodes] ?? []) as Element[];
      const roots = new Set(elements.map((element) => element.getRootNode()));
      const next: [HTMLElement, string, string, boolean][] = [];
      for (const root of roots) {
        if (!(root instanceof ShadowRoot || root instanceof Document)) {
          continue;
        }
        for (const video of Array.from(root.querySelectorAll('video'))) {
          if (!video.paused) {
            video.pause();
            next.push([
              video,
              video.style.getPropertyValue('visibility'),
              video.style.getPropertyPriority('visibility'),
              true,
            ]);
          }
        }
        for (const image of Array.from(root.querySelectorAll('img'))) {
          if (/\.gif(?:[?#]|$)|^data:image\/gif[;,]/i.test(image.currentSrc || image.src)) {
            next.push([
              image,
              image.style.getPropertyValue('visibility'),
              image.style.getPropertyPriority('visibility'),
              false,
            ]);
            image.style.setProperty('visibility', 'hidden', 'important');
          }
        }
      }
      state[key] = next;
    },
    { still, nodes: NODES },
  );
}

/** The render check's findings, as `family: text` per element. */
export interface RenderCheck {
  /** Drawn differently through aliases that can only resolve to the injected files. */
  notInjected: string[];
  /** Drawn the same with the injected families taken out of `font-family` (the stack's generic). */
  sameAsFallback: string[];
  /** Not compared: no pixel of its text shows (`visibleParts`). */
  covered: string[];
}

const ALIAS = 'sd-alias-';

/**
 * The generic the alias render falls back to for a glyph the alias's faces do not cover: one that cannot be what the
 * page itself falls back to (`fallback`, the rest of the element's `font-family`), so such a glyph draws differently
 * in the two renders.
 */
export function aliasFallback(fallback: string): string {
  return /(?:^|,)\s*monospace\s*(?:,|$)/.test(fallback) ? 'serif' : 'monospace';
}

/**
 * Renders each text element three ways, with the slide's moving pictures taken out (`stillPictures`), and compares the
 * pixels of its visible part (`visibleParts`; text no pixel of which shows is not compared, and is listed as
 * `covered`): as the page draws it; with each injected family of its `font-family` set to an alias whose `@font-face`
 * rules are copies of `<style id="sd-fonts">` (so it can only resolve to the injected files), then a generic other
 * than the page's own (`aliasFallback`); and with only the generics of its `font-family` (a named family the deck
 * ships no face for may be installed here, as a substitute's own name often is, and draw like the shipped file). The
 * page's render must equal the alias render (the injected faces draw every glyph, not a font of the same name on this
 * computer, nor the page's fallback for a glyph they lack) and differ from the fallback render (the comparison sees
 * the faces), except for text made only of emoji, which no face ships.
 */
export async function renderCheck(page: Page, elements: readonly TextElement[]): Promise<RenderCheck> {
  if (elements.length === 0) {
    return { notInjected: [], sameAsFallback: [], covered: [] };
  }
  await settle(page);
  /**
   * Sets every element's `font-family` for `mode` (`page`: the slide's own), over a mid grey backdrop (on the player or
   * the still render that holds the slide) so text of any colour on a slide with no background shows; `restore` puts
   * back the page as it was.
   */
  const restyle = (mode: 'page' | 'alias' | 'fallback' | 'restore') =>
    page.evaluate(
      async ({ mode, nodes, alias, items, generics }) => {
        const state = window as unknown as Record<string, unknown>;
        const elements = (state[nodes] ?? []) as HTMLElement[];
        const host = elements[0]?.getRootNode();
        const player = host instanceof ShadowRoot ? host.host.closest('.sd-player, .sd-static') : null;
        const targets: HTMLElement[] = player instanceof HTMLElement ? [player, ...elements] : elements;
        const property = (i: number) => (player instanceof HTMLElement && i === 0 ? 'background' : 'font-family');
        const saved = (state[`${nodes}Saved`] ??= targets.map((target, i) => [
          target.style.getPropertyValue(property(i)),
          target.style.getPropertyPriority(property(i)),
        ])) as [string, string][];
        targets.forEach((target, i) => {
          const [value, priority] = saved[i] ?? ['', ''];
          target.style.removeProperty(property(i));
          if (value !== '') {
            target.style.setProperty(property(i), value, priority);
          }
        });
        document.getElementById(`${alias}fonts`)?.remove();
        if (mode === 'restore') {
          delete state[`${nodes}Saved`];
          return;
        }
        if (player instanceof HTMLElement) {
          player.style.setProperty('background', '#808080', 'important');
        }
        const css = document.getElementById('sd-fonts')?.textContent ?? '';
        if (mode === 'alias') {
          const style = document.createElement('style');
          style.id = `${alias}fonts`;
          style.textContent = css.replaceAll('font-family:"', `font-family:"${alias}`);
          document.head.append(style);
        }
        if (mode !== 'page') {
          const injected = new Set(
            Array.from(css.matchAll(/font-family:"((?:[^"\\]|\\.)*)"/g), ([, family = '']) =>
              family.replace(/\\(.)/g, '$1'),
            ),
          );
          const quote = (family: string) =>
            generics.includes(family) ? family : `"${family.replace(/["\\]/g, '\\$&')}"`;
          elements.forEach((element, i) => {
            const { stack, contrast } = items[i] ?? { stack: [], contrast: 'monospace' };
            const shipped = stack.filter((family) => injected.has(family));
            const rest = stack.filter((family) => !injected.has(family));
            const value =
              mode === 'alias'
                ? [...shipped.map((family) => quote(alias + family)), contrast].join(',')
                : rest.filter((family) => generics.includes(family)).join(',') || 'serif';
            element.style.setProperty('font-family', value, 'important');
          });
        }
        await Promise.all(
          Array.from(document.fonts)
            .filter((face) => face.family.replace(/^["']|["']$/g, '').startsWith(alias))
            .map((face) => face.load().catch(() => undefined)),
        );
        await document.fonts.ready;
      },
      {
        mode,
        nodes: NODES,
        alias: ALIAS,
        items: elements.map(({ stack, fallback }) => ({ stack, contrast: aliasFallback(fallback) })),
        generics: [...GENERICS],
      },
    );
  let visible: (Box | null)[] = [];
  const shoot = async (mode: 'page' | 'alias' | 'fallback') => {
    await restyle(mode);
    await frames(page);
    return shootAll(page, visible);
  };
  try {
    await stillPictures(page, true);
    await restyle('page');
    await frames(page);
    visible = await visibleParts(page, elements);
    const drawn = await shoot('page');
    const aliased = await shoot('alias');
    const fallback = await shoot('fallback');
    const name = (element: TextElement) => `${element.family}: ${element.text}`;
    const differs = (a: Buffer | undefined, b: Buffer | undefined) =>
      a !== undefined && b !== undefined && differentPixels(a, b, 0, TOLERANCE) > 0;
    const shown = (i: number) => visible[i] !== null;
    return {
      notInjected: elements.filter((_, i) => shown(i) && differs(drawn[i], aliased[i])).map(name),
      sameAsFallback: elements
        .filter((element, i) => shown(i) && !ONLY_EMOJI.test(element.text) && !differs(drawn[i], fallback[i]))
        .map(name),
      covered: elements.filter((_, i) => !shown(i)).map(name),
    };
  } finally {
    await stillPictures(page, false);
    await restyle('restore');
    await frames(page);
  }
}
