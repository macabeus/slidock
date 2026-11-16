// @slidock/player: plays a slidock deck (stage, shadow-root slide mounts, builds and transitions). It has no
// dependencies and is part of the deck format: decks embed the IIFE build and record its version. The presenter view
// is `@slidock/player/presenter` (src/presenter/), a build of its own that a deck embeds only when it ships one.
import { TEXT_VIEW_ID } from './chrome';
import type { DeckSource } from './deck';
import { numbering, slideForNumber } from './machine';
import { type Player, type PlayerOptions, createPlayer } from './player';

export type { Blank, MachineModel, Motion, Position } from './machine';
export type { DeckSource, MediaResolver } from './deck';
export type { PlayerEvents } from './events';
export type { JoinOptions, Player, PlayerOptions, PresenterOptions, PresenterSupport, PresenterView } from './player';
export type { Sync, SyncRole, SyncState } from './presenter/sync';
export { CHROME_CSS } from './chrome';
export { DECK_FORMAT, htmlDeck, inlineDeck, parseDeck } from './deck';
export { RT, createPlayer, renderStatic } from './player';
export { stepCount } from './timeline';
export { clickGroups, numbering, reduce, slideForNumber } from './machine';

/** The player version, recorded in each deck's data. */
export const version = '0.1.0';

/** A deep link to a shown slide: `#slide-<n>`, its 1-based number among the shown slides. */
const SLIDE_HASH = /^#slide-(\d+)$/;

/** The slide index a `#slide-<n>` hash names, or `null`. */
function slideOfHash(source: DeckSource, hash: string): number | null {
  const match = SLIDE_HASH.exec(hash);
  return match?.[1] === undefined ? null : slideForNumber(source.deck.slides, Number(match[1]));
}

/**
 * The page's deep links: `#slide-<n>` starts there, follows the shown slide (by `replaceState`, so no history entry and
 * no `hashchange`) while the hash is empty or a slide's, and a later `#slide-<n>` (a heading of the text view) goes to
 * that slide; each brings the stage into view. Any other hash is left alone. Returns its disposer.
 */
function deepLinks(view: Window, host: HTMLElement, source: DeckSource, player: Player): () => void {
  const numbers = numbering(source.deck.slides);
  const toStage = (): void => {
    const stage = Array.from(host.children).find((child) => child.classList.contains('sd-player'));
    if (stage === undefined) {
      return;
    }
    // `scrollTo`, which WebKit lets end the fragment's anchoring (it keeps a fragment's target in view through later
    // layouts otherwise). The deck page's stage is fixed to the window: the page's top is where nothing covers it.
    if (stage.classList.contains('sd-page')) {
      view.scrollTo(view.scrollX, 0);
    } else {
      view.scrollTo(
        view.scrollX + stage.getBoundingClientRect().left,
        view.scrollY + stage.getBoundingClientRect().top,
      );
    }
  };
  const goTo = (hash: string): void => {
    const index = slideOfHash(source, hash);
    if (index !== null) {
      if (index !== player.position.slide) {
        player.goTo(index);
      }
      toStage();
    }
  };
  const replace = (hash: string): void => {
    try {
      view.history.replaceState(view.history.state, '', `${view.location.href.split('#')[0] ?? ''}${hash}`);
    } catch {
      // Some file:// contexts refuse it; the deck plays on.
    }
  };
  const cleanups: (() => void)[] = [];
  const initial = view.location.hash;
  if (slideOfHash(source, initial) !== null) {
    // The browser would scroll to the fragment's section of the text view once parsed, and WebKit keeps it in view
    // through later layouts: the page leaves the fragment out until it has loaded, then puts it back.
    replace('');
    const restore = (): void => {
      if (view.location.hash === '') {
        replace(initial);
      }
      toStage();
    };
    const onLoad = (): void => void view.setTimeout(restore, 0);
    if (view.document.readyState === 'complete') {
      restore();
    } else {
      view.addEventListener('load', onLoad, { once: true });
      cleanups.push(() => view.removeEventListener('load', onLoad));
    }
  }
  cleanups.push(
    player.on('position', ({ position }) => {
      const number = numbers[position.slide] ?? null;
      const hash = view.location.hash;
      if (number === null || (hash !== '' && !SLIDE_HASH.test(hash)) || hash === `#slide-${number}`) {
        return;
      }
      replace(`#slide-${number}`);
    }),
  );
  // Only the hash the page holds now: a stale event (an engine that reports `replaceState`, late) moves nothing.
  const onHashChange = (event: HashChangeEvent): void => {
    if (new URL(event.newURL, view.location.href).hash === view.location.hash) {
      goTo(view.location.hash);
    }
  };
  // A heading of the slide already in the hash changes nothing the browser reports.
  const onClick = (event: MouseEvent): void => {
    const link = (event.target as Partial<Element> | null)?.closest?.(`#${TEXT_VIEW_ID} a[href^="#slide-"]`);
    if (link !== null && link !== undefined && link.getAttribute('href') === view.location.hash) {
      event.preventDefault();
      goTo(view.location.hash);
    }
  };
  view.addEventListener('hashchange', onHashChange);
  view.document.addEventListener('click', onClick);
  cleanups.push(() => {
    view.removeEventListener('hashchange', onHashChange);
    view.document.removeEventListener('click', onClick);
  });
  return () => {
    for (const cleanup of cleanups) {
      cleanup();
    }
  };
}

/**
 * Plays a deck in `host` the way a deck page does: presenting, keys on the whole document, clicks and swipes
 * navigating, and its deep links (`#slide-<n>`). With `options.presenter` it joins its presenter-view session, and on
 * a `#presenter` URL it mounts the presenter window instead, in place of the page's text view, and returns its player;
 * without one, a `#presenter` URL shows the deck.
 */
export function mount(host: HTMLElement, deck: DeckSource, options: PlayerOptions = {}): Player {
  const hash = host.ownerDocument.defaultView?.location.hash ?? '';
  const session = options.presenter?.session(hash) ?? null;
  if (options.presenter !== undefined && session !== null) {
    // The presenter window is the presenter view alone: the deck page's text view would sit in its focus order, its
    // accessibility tree and its scroll beneath the layout.
    const text = host.ownerDocument.getElementById(TEXT_VIEW_ID);
    if (text !== null && host.contains(text)) {
      text.remove();
    }
    const view = options.presenter.mount(host, deck, {
      session,
      ...(options.reducedMotion === undefined ? {} : { reducedMotion: options.reducedMotion }),
      ...(options.readyTimeoutMs === undefined ? {} : { readyTimeoutMs: options.readyTimeoutMs }),
      ...(options.loop === undefined ? {} : { loop: options.loop }),
    });
    return Object.create(view.player, { destroy: { value: view.destroy } }) as Player;
  }
  const start = slideOfHash(deck, hash);
  const player = createPlayer(host, deck, {
    mode: 'present',
    keyboard: 'document',
    pointer: 'always',
    chrome: true,
    sync: { role: 'deck' },
    ...(start === null ? {} : { startAt: { slide: start } }),
    ...options,
  });
  const view = host.ownerDocument.defaultView;
  if (view === null) {
    return player;
  }
  const stop = deepLinks(view, host, deck, player);
  return Object.create(player, {
    destroy: {
      value: () => {
        stop();
        player.destroy();
      },
    },
  }) as Player;
}
