import { stepCount } from '@slidock/player';
import type { Deck, DeckDiagnostic, SlideTemplate } from '@slidock/types';

// The player API the app codes against (docs/player-api.md), adapted to the deck the worker hands
// over (packages/wasm/README.md). `slidock-player.ts` implements it over `@slidock/player`. The player announces each
// move to assistive technology itself (a live region inside its host), and reports its runtime diagnostics as
// `diagnostic` events, which the viewer adds to the conversion's.

/** A deck to play: the deck JSON, the composed slides in show order, and the media as URLs. */
export interface DeckInput {
  deck: Deck;
  slides: readonly SlideTemplate[];
  /** The URL of a `media/<sha256>.<ext>` path the slide markup references (a `blob:` URL in the app). */
  mediaUrl(path: string): string;
}

export type Blank = 'black' | 'white' | null;

/** Where the player is. `slide` indexes `deck.slides` (hidden slides included); `step` counts played click groups. */
export interface Position {
  slide: number;
  step: number;
  presenting: boolean;
  blank: Blank;
}

export type PlayerEvent =
  | { type: 'position'; position: Position }
  | { type: 'ended' }
  | { type: 'diagnostic'; diagnostic: DeckDiagnostic }
  /** The player opened the presenter window, from `openPresenter` or its own P key (`null`: the browser blocked it). */
  | { type: 'presenter'; window: Window | null };

export interface PlayerOptions {
  /** `view`: slides fully built, no motion; `present` plays builds and transitions. */
  mode: 'view' | 'present';
  startAt?: { slide: number; step?: number };
  onEvent?(event: PlayerEvent): void;
}

export interface Player {
  next(): void;
  previous(): void;
  first(): void;
  last(): void;
  /** A cut, no animation. */
  goTo(slide: number, step?: number): void;
  /** Full screen on the host, playing builds and transitions; ends with `stop` or by leaving full screen. */
  present(): Promise<void>;
  stop(): void;
  blank(kind: Blank): void;
  readonly position: Position;
  /** Draws a slide fully built and still into `host` (thumbnails, previews); returns its cleanup. */
  renderStatic(slideId: string, host: HTMLElement, fit: 'contain' | 'width'): () => void;
  /**
   * Opens the presenter window (the app at `#presenter`), presenting from here if the player is not yet; `null` when
   * the browser blocked it. The two windows' players keep their positions in step.
   */
  openPresenter(): Window | null;
  destroy(): void;
}

export interface PresenterOptions {
  /** Where the deck window was when it sent the deck. */
  startAt?: { slide: number; step?: number };
  /**
   * The deck window's sync session (from the presenter window's URL): the presenter view joins it and takes the deck
   * window's position as it is now. Empty: adopted from the first deck window that answers.
   */
  session: string;
  onEvent?(event: PlayerEvent): void;
}

/** The presenter view in its own window, following and driving the deck window on the same deck. */
export interface Presenter {
  destroy(): void;
}

export interface PlayerFactory {
  createPlayer(host: HTMLElement, deck: DeckInput, options: PlayerOptions): Player;
  createPresenter(host: HTMLElement, deck: DeckInput, options: PresenterOptions): Presenter;
}

/**
 * The number of click groups of a slide, as `@slidock/player` counts them: a step the player drops from a malformed
 * deck does not count, so the step clamp and the counter agree with what the player plays.
 */
export function clickGroups(deck: Deck, slide: number): number {
  const shown = deck.slides[slide];
  return shown === undefined ? 0 : stepCount(shown);
}
