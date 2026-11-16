// The effect suite's fixture decks: a 720 x 405 pt deck (960 x 540 px) whose slides hold a background, a coloured
// shape, a rotated shape and a text box with two paragraphs, in the markup slidock-html prints. A build test puts
// one step on a slide; a transition test moves between two slides that differ everywhere.
import type { Deck, DeckSlide, DeckStep, DeckTransition } from '@slidock/types';

export const WIDTH = 960;
export const HEIGHT = 540;

export function slideHtml(id: string, variant: 'a' | 'b' = 'a'): string {
  const bg = variant === 'a' ? '#f8fafc' : '#fde68a';
  const fill = variant === 'a' ? '#2563eb' : '#dc2626';
  const left = variant === 'a' ? 300 : 120;
  return (
    `<main data-slide="${id}" style="position:relative;width:720pt;height:405pt;overflow:hidden">` +
    `<div data-sd-bg style="position:absolute;inset:0;background:${bg}"></div>` +
    `<div data-el="${id}.box" data-kind="shape" data-sd-match="box" style="position:absolute;left:${left}pt;top:150pt;width:200pt;height:100pt;background:${fill}"></div>` +
    `<div data-el="${id}.rot" data-kind="shape" style="position:absolute;left:560pt;top:280pt;width:90pt;height:60pt;rotate:30deg;scale:-1 1;background:#16a34a;opacity:0.8"></div>` +
    `<div data-el="${id}.text" data-kind="shape" data-sd-text style="position:absolute;left:40pt;top:30pt;width:420pt;height:90pt;color:#111827;font:24pt/1.2 sans-serif;white-space:pre-wrap">` +
    `<p data-p="0">Hello effects</p><p data-p="1">Second line</p></div>` +
    `</main>`
  );
}

export function step(element: string, overrides: Partial<DeckStep> & Record<string, unknown> = {}): DeckStep {
  return {
    target: { element, paragraphs: null },
    trigger: 'click',
    kind: 'entrance',
    delivery: 'all',
    delayMs: 0,
    durationMs: 1000,
    native: 'test:effect',
    fallback: 'dissolve',
    ...overrides,
  } as DeckStep;
}

export function deckSlide(id: string, overrides: Partial<DeckSlide> = {}): DeckSlide {
  return {
    id,
    layout: null,
    hidden: false,
    number: null,
    title: id,
    notes: [],
    failed: null,
    timeline: [],
    transition: null,
    fonts: [],
    ...overrides,
  };
}

export function transition(overrides: Partial<DeckTransition> & Record<string, unknown>): DeckTransition {
  return {
    native: 'test:transition',
    fallback: 'dissolve',
    durationMs: 1000,
    advance: null,
    ...overrides,
  } as DeckTransition;
}

export interface Fixture {
  readonly deck: Deck;
  readonly slides: Record<string, string>;
  /** The URL `media/<file>` paths resolve against; relative paths when absent. */
  readonly mediaBase?: string;
}

export function fixture(slides: DeckSlide[], html?: Record<string, string>): Fixture {
  let shown = 0;
  return {
    deck: {
      format: 'slidock-deck@1',
      player: null,
      id: 'e'.repeat(64),
      title: 'Effects',
      lang: null,
      size: { w: 720, h: 405 },
      source: { format: 'pptx', name: 'effects.pptx', sha256: 'f'.repeat(64), bytes: 1 },
      slides: slides.map((s) => ({ ...s, number: s.hidden ? null : ++shown })),
      media: [],
      fonts: [],
      diagnostics: { total: 0, error: 0, decline: 0, lossy: 0, warning: 0, info: 0, codes: [] },
    },
    slides: Object.fromEntries(slides.map((s, i) => [s.id, html?.[s.id] ?? slideHtml(s.id, i % 2 === 0 ? 'a' : 'b')])),
  };
}
