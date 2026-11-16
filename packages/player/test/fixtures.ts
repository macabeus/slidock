// Deck builders for the tests: a deck JSON and slide markup in the shape slidock-html and slidock-bundle print.
import type { Deck, DeckSlide, DeckStep, DeckTransition } from '@slidock/types';

export function step(element: string, overrides: Partial<DeckStep> = {}): DeckStep {
  return {
    target: { element, paragraphs: null },
    trigger: 'click',
    kind: 'entrance',
    delivery: 'all',
    delayMs: 0,
    durationMs: 500,
    native: 'pptx:fade',
    fallback: 'dissolve',
    ...overrides,
  };
}

export function transition(overrides: Partial<DeckTransition> = {}): DeckTransition {
  return { native: 'pptx:fade', fallback: 'dissolve', durationMs: 700, advance: null, ...overrides };
}

export function slide(id: string, overrides: Partial<DeckSlide> = {}): DeckSlide {
  return {
    id,
    layout: null,
    hidden: false,
    number: null,
    title: `Title ${id}`,
    notes: [],
    failed: null,
    timeline: [],
    transition: null,
    fonts: [],
    ...overrides,
  };
}

export function deck(slides: DeckSlide[], overrides: Partial<Deck> = {}): Deck {
  let shown = 0;
  return {
    format: 'slidock-deck@1',
    player: null,
    id: 'a'.repeat(64),
    title: 'Deck',
    lang: null,
    size: { w: 720, h: 405 },
    source: { format: 'pptx', name: 'deck.pptx', sha256: 'b'.repeat(64), bytes: 1 },
    slides: slides.map((s) => ({ ...s, number: s.hidden ? null : ++shown })),
    media: [],
    fonts: [],
    diagnostics: { total: 0, error: 0, decline: 0, lossy: 0, warning: 0, info: 0, codes: [] },
    ...overrides,
  };
}

/** A composed slide: a title box with paragraphs, as the HTML backend prints it. */
export function slideHtml(id: string, body = ''): string {
  return (
    `<main data-slide="${id}" style="position:relative;width:720pt;height:405pt;overflow:hidden">` +
    `<div data-el="${id}.title" data-sd-text style="position:absolute;left:40pt;top:20pt;width:640pt;height:50pt">` +
    `<p data-p="0">One</p><p data-p="1">Two</p></div>${body}</main>`
  );
}

/** A deck page: the deck JSON and one template per slide, as slidock-bundle writes `index.html`. */
export function deckPage(document: Document, data: Deck, html: (id: string) => string = (id) => slideHtml(id)): void {
  const script = document.createElement('script');
  script.type = 'application/json';
  script.id = 'sd-deck';
  script.textContent = JSON.stringify(data);
  document.head.append(script);
  for (const s of data.slides) {
    const template = document.createElement('template');
    template.setAttribute('data-sd-slide', s.id);
    template.innerHTML = html(s.id);
    document.body.append(template);
  }
}
