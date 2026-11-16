// Where a player's deck comes from: the deck JSON and a fresh, inert copy of each slide's markup. The zip's page
// holds both inline (`inlineDeck`); the web app hands them over as strings with a `blob:` media resolver (`htmlDeck`).
import type { Deck, SlideTemplate } from '@slidock/types';

export const DECK_FORMAT = 'slidock-deck@1';

/** Turns a deck media path (`media/<sha256>.<ext>`) into the URL the page loads. */
export type MediaResolver = (path: string) => string;

export interface DeckSource {
  readonly deck: Deck;
  /**
   * A fresh copy of the slide's composed `<main data-slide>`, owned by an inert document (its media do not load
   * until it is adopted), or `null` when the deck has no markup for it.
   */
  slide(id: string): DocumentFragment | null;
  /** The media resolver; relative paths when absent (the zip). */
  readonly mediaUrl?: MediaResolver;
}

/** Reads and checks deck JSON; throws on a format this player does not read. */
export function parseDeck(json: string): Deck {
  const value: unknown = JSON.parse(json);
  if (typeof value !== 'object' || value === null) {
    throw new Error('slidock: the deck data is not an object');
  }
  const format = (value as { format?: unknown }).format;
  if (format !== DECK_FORMAT) {
    throw new Error(`slidock: this player reads ${DECK_FORMAT} decks, not ${JSON.stringify(format)}`);
  }
  const slides = (value as { slides?: unknown }).slides;
  if (!Array.isArray(slides)) {
    throw new Error('slidock: the deck data has no slides');
  }
  return value as Deck;
}

/** The deck of a slidock page: `<script id="sd-deck">` and its `<template data-sd-slide>` elements. */
export function inlineDeck(document: Document, options: { mediaUrl?: MediaResolver } = {}): DeckSource {
  const script = document.getElementById('sd-deck');
  if (script === null) {
    throw new Error('slidock: the page has no #sd-deck');
  }
  const deck = parseDeck(script.textContent ?? '');
  // The deck page writes its faces after its text view, in the body: they belong to the document, so a host that
  // replaces the body's content (to mount the deck elsewhere) keeps them.
  const fonts = document.getElementById('sd-fonts');
  if (fonts !== null && document.head !== null && fonts.parentNode !== document.head) {
    document.head.append(fonts);
  }
  const templates = new Map<string, HTMLTemplateElement>();
  for (const template of Array.from(document.querySelectorAll<HTMLTemplateElement>('template[data-sd-slide]'))) {
    const id = template.getAttribute('data-sd-slide');
    if (id !== null && !templates.has(id)) {
      templates.set(id, template);
    }
  }
  return {
    deck,
    slide: (id) => {
      const template = templates.get(id);
      return template === undefined ? null : (template.content.cloneNode(true) as DocumentFragment);
    },
    ...(options.mediaUrl === undefined ? {} : { mediaUrl: options.mediaUrl }),
  };
}

/** A deck from its JSON and its slides' composed markup, as the web app holds them. */
export function htmlDeck(
  document: Document,
  deck: Deck,
  slides: Iterable<SlideTemplate> | ReadonlyMap<string, string>,
  mediaUrl?: MediaResolver,
): DeckSource {
  const html = new Map<string, string>();
  if (isMap(slides)) {
    for (const [id, text] of slides) {
      html.set(id, text);
    }
  } else {
    for (const slide of slides) {
      html.set(slide.id, slide.html);
    }
  }
  const parsed = new Map<string, HTMLTemplateElement>();
  return {
    deck,
    slide: (id) => {
      let template = parsed.get(id);
      if (template === undefined) {
        const text = html.get(id);
        if (text === undefined) {
          return null;
        }
        template = document.createElement('template');
        template.innerHTML = text;
        parsed.set(id, template);
      }
      return template.content.cloneNode(true) as DocumentFragment;
    },
    ...(mediaUrl === undefined ? {} : { mediaUrl }),
  };
}

function isMap(value: unknown): value is ReadonlyMap<string, string> {
  return value instanceof Map;
}

const MEDIA_PATH = /^(?:\.\.?\/)*(media\/[^?#]+)$/;

/** The deck media path a markup URL names, or `null` for any other URL. */
export function mediaPath(url: string): string | null {
  return MEDIA_PATH.exec(url.trim())?.[1] ?? null;
}

const MEDIA_ATTRIBUTES: readonly string[] = ['src', 'poster', 'href', 'xlink:href'];

/** Points the markup's media references at `resolve`'s URLs: attributes and `url()` in inline styles. */
export function resolveMedia(root: ParentNode, resolve: MediaResolver): void {
  for (const element of Array.from(root.querySelectorAll('*'))) {
    for (const name of MEDIA_ATTRIBUTES) {
      const value = element.getAttribute(name);
      const path = value === null ? null : mediaPath(value);
      if (path !== null) {
        element.setAttribute(name, resolve(path));
      }
    }
    const style = element.getAttribute('style');
    if (style !== null && style.includes('media/')) {
      element.setAttribute(
        'style',
        style.replace(/url\(\s*(['"]?)(.*?)\1\s*\)/g, (whole, _quote: string, url: string) => {
          const path = mediaPath(url);
          return path === null ? whole : `url(${JSON.stringify(resolve(path))})`;
        }),
      );
    }
  }
}
