import { type Player as DeckPlayer, type DeckSource, createPlayer, htmlDeck, renderStatic } from '@slidock/player';
import { mountPresenter, presenter } from '@slidock/player/presenter';
import type { DeckDiagnostic } from '@slidock/types';

import { presenterUrl } from '../presenter/bridge';
import type {
  DeckInput,
  Player,
  PlayerEvent,
  PlayerFactory,
  PlayerOptions,
  Presenter,
  PresenterOptions,
} from './types';

// The app's player API over `@slidock/player`. The deck window's player joins a presenter-view sync session as it is
// created; the presenter window (the app at `#presenter`, fed the deck by `PresenterLink`) joins with an empty
// session, which adopts the deck window's, so each deck the presenter window is handed finds its player at once.

/** Every click group played: `renderStatic` clamps the step to the slide's groups. */
const FULLY_BUILT = Number.MAX_SAFE_INTEGER;

function source(input: DeckInput): DeckSource {
  return htmlDeck(document, input.deck, input.slides, input.mediaUrl);
}

/** Runtime diagnostics as the app lists them: warnings about the slide they name. */
function runtimeDiagnostic(event: { code: string; message: string; slide: string | null }): DeckDiagnostic {
  return {
    code: event.code,
    severity: 'warning',
    message: event.message,
    feature: null,
    container: event.slide,
    node: null,
  };
}

/** Reports the player's events to `onEvent`; returns the listeners' removal. */
function forward(player: DeckPlayer, onEvent: ((event: PlayerEvent) => void) | undefined): () => void {
  if (onEvent === undefined) {
    return () => undefined;
  }
  const off = [
    player.on('position', ({ position }) => onEvent({ type: 'position', position })),
    player.on('ended', () => onEvent({ type: 'ended' })),
    player.on('diagnostic', (event) => onEvent({ type: 'diagnostic', diagnostic: runtimeDiagnostic(event) })),
    player.on('presenter', ({ window }) => onEvent({ type: 'presenter', window })),
  ];
  return () => off.forEach((remove) => remove());
}

function createDeckPlayer(host: HTMLElement, input: DeckInput, options: PlayerOptions): Player {
  const deckSource = source(input);
  const player = createPlayer(host, deckSource, {
    mode: options.mode,
    keyboard: 'host',
    pointer: 'present',
    chrome: true,
    sync: { role: 'deck' },
    presenter,
    presenterUrl,
    ...(options.startAt === undefined ? {} : { startAt: options.startAt }),
  });
  const off = forward(player, options.onEvent);
  const indexOf = (slideId: string) => input.deck.slides.findIndex((slide) => slide.id === slideId);
  return {
    next: () => player.next(),
    previous: () => player.previous(),
    first: () => player.first(),
    last: () => player.last(),
    goTo: (slide, step) => player.goTo(slide, step),
    present: () => player.present(),
    stop: () => player.stop(),
    blank: (kind) => player.blank(kind),
    get position() {
      return player.position;
    },
    renderStatic: (slideId, target, fit) => renderStatic(deckSource, indexOf(slideId), FULLY_BUILT, target, fit),
    openPresenter: () => player.openPresenter(),
    destroy: () => {
      off();
      player.destroy();
    },
  };
}

function createPresenterView(host: HTMLElement, input: DeckInput, options: PresenterOptions): Presenter {
  const view = mountPresenter(host, source(input), {
    session: options.session,
    ...(options.startAt === undefined ? {} : { startAt: options.startAt }),
  });
  const off = forward(view.player, options.onEvent);
  return {
    destroy: () => {
      off();
      view.destroy();
    },
  };
}

export const slidockPlayer: PlayerFactory = {
  createPlayer: createDeckPlayer,
  createPresenter: createPresenterView,
};
