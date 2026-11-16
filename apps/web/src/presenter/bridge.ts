import type { Deck, FontFile, MediaHandle, SlideTemplate } from '@slidock/types';

// The presenter window is the app itself at `#presenter`, opened by the deck window's player. It asks its opener for
// the deck (`ready`), and the opener answers with the compiled deck and its media blobs (`deck`). The opener also
// sends each deck it opens later (`deck`) and says when it closes one (`closed`), so the presenter window always shows
// the deck window's deck. A deck the opener sends again (on `ready`, or to a window it takes again) carries the serial
// it was first sent with, and the presenter window keeps showing it as it is. All are same-origin `postMessage`s. The
// two players keep their positions in step on their own: the presenter window's URL names the deck window's sync
// session (`#presenter=<session>`), which the presenter view joins at once, so its first word gets the deck window's
// position as it is then, not as it was when the deck was sent (a move in between would otherwise be lost).

const READY = 'slidock:presenter-ready';
const DECK = 'slidock:presenter-deck';
const CLOSED = 'slidock:presenter-closed';

export const PRESENTER_HASH = 'presenter';

export interface PresenterDeck {
  type: typeof DECK;
  deck: Deck;
  slides: SlideTemplate[];
  media: MediaHandle[];
  /** The faces the deck ships, which the presenter window adds to its own document. */
  fonts: FontFile[];
  position: { slide: number; step: number };
  /** Which deck the opener served: the same for each sending of one deck. */
  serial: number;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

export function isPresenterReady(data: unknown): boolean {
  return isObject(data) && data.type === READY;
}

export function isPresenterDeck(data: unknown): data is PresenterDeck {
  return (
    isObject(data) &&
    data.type === DECK &&
    isObject(data.deck) &&
    Array.isArray(data.slides) &&
    Array.isArray(data.media) &&
    Array.isArray(data.fonts) &&
    isObject(data.position) &&
    typeof data.serial === 'number'
  );
}

/** The presenter window's URL: this page at `#presenter=<session>`, `session` being the deck window's sync session. */
export function presenterUrl(session: string): string {
  return `${location.pathname}${location.search}#${PRESENTER_HASH}=${encodeURIComponent(session)}`;
}

/** The deck window's sync session the presenter window's URL names; empty when it names none. */
export function presenterSessionOf(hash: URLSearchParams): string {
  return hash.get(PRESENTER_HASH) ?? '';
}

type Payload = () => Omit<PresenterDeck, 'type' | 'serial'>;

/**
 * The deck window's end of the bridge, for the life of the page (decks come and go under it): it keeps the presenter
 * window, answers its `ready` with the deck being served, sends it each newly served deck, and `closed` when the deck
 * stops being served.
 */
export class PresenterLink {
  #popup: Window | null = null;
  #payload: Payload | null = null;
  #serial = 0;

  /** Starts answering the presenter window; returns the listener's removal. */
  listen(): () => void {
    window.addEventListener('message', this.#onMessage);
    return () => window.removeEventListener('message', this.#onMessage);
  }

  /**
   * Takes `popup` (the presenter window the player opened) as the presenter window, and sends it the deck being
   * served. A previous, different presenter window is told the deck closed.
   */
  attach(popup: Window) {
    const previous = this.#target();
    if (previous && previous !== popup) {
      previous.postMessage({ type: CLOSED }, location.origin);
    }
    this.#popup = popup;
    popup.focus();
    this.#sendDeck();
  }

  /** Serves a deck: sends it to an open presenter window now and on each `ready`. Returns the end of serving it. */
  serve(payload: Payload): () => void {
    this.#payload = payload;
    this.#serial += 1;
    this.#sendDeck();
    return () => {
      if (this.#payload === payload) {
        this.#payload = null;
        this.#send({ type: CLOSED });
      }
    };
  }

  #target(): Window | null {
    if (this.#popup?.closed) {
      this.#popup = null;
    }
    return this.#popup;
  }

  #sendDeck() {
    if (this.#payload) {
      this.#send({ type: DECK, ...this.#payload(), serial: this.#serial });
    }
  }

  #send(message: PresenterDeck | { type: typeof CLOSED }) {
    this.#target()?.postMessage(message, location.origin);
  }

  #onMessage = (event: MessageEvent) => {
    const target = this.#target();
    if (!target || event.source !== target || event.origin !== location.origin || !isPresenterReady(event.data)) {
      return;
    }
    this.#sendDeck();
  };
}

/**
 * From the presenter window: asks the opener for the deck, then calls `onDeck` with each deck it sends (once: not
 * again for the same deck sent again, whose position is the opener's when it was sent, not now) and `onClosed` when
 * it closes one.
 */
export function requestDeck(onDeck: (deck: PresenterDeck) => void, onClosed: () => void): () => void {
  const opener = window.opener as Window | null;
  let shown: number | null = null;
  const onMessage = (event: MessageEvent) => {
    if (event.source !== opener || event.origin !== location.origin) {
      return;
    }
    if (isPresenterDeck(event.data)) {
      if (event.data.serial !== shown) {
        shown = event.data.serial;
        onDeck(event.data);
      }
    } else if (isObject(event.data) && event.data.type === CLOSED) {
      shown = null;
      onClosed();
    }
  };
  window.addEventListener('message', onMessage);
  opener?.postMessage({ type: READY }, location.origin);
  return () => window.removeEventListener('message', onMessage);
}
