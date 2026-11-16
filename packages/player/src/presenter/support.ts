// The presenter view as the deck player takes it (`PlayerOptions.presenter`): the session in a `#presenter` URL, the
// sync between the two windows (sync.ts), the opening and placing of the presenter window, and the presenter window's
// page, its frame (window.ts) around the view (view.ts).
import type { DeckSource } from '../deck';
import type { JoinOptions, PresenterOptions, PresenterSupport, PresenterView } from '../player';
import { broadcastTransport, createSync, randomId, windowTransport } from './sync';
import { mountView } from './view';
import { createPresenterWindow } from './window';

/** The session a `#presenter[=<session>]` hash names, or `null` when the page is not a presenter view. */
export function presenterSession(hash: string): string | null {
  const match = /^#presenter(?:=([^&]*))?$/.exec(hash);
  if (match === null) {
    return null;
  }
  try {
    return decodeURIComponent(match[1] ?? '');
  } catch {
    return '';
  }
}

function join(view: Window, options: JoinOptions) {
  const transports = [windowTransport(view, options.peer, options.onPeer)];
  const channel = broadcastTransport(options.deck);
  if (channel !== null) {
    transports.push(channel);
  }
  return createSync({
    deck: options.deck,
    role: options.role,
    session: options.session ?? randomId(),
    transports,
    current: options.current,
    apply: options.apply,
  });
}

interface ScreenLike {
  readonly availLeft: number;
  readonly availTop: number;
  readonly availWidth: number;
  readonly availHeight: number;
}

interface ScreenDetailsLike {
  readonly screens: readonly ScreenLike[];
  readonly currentScreen: ScreenLike;
}

/** Window Management API placement: the deck full screen on another screen, the presenter on this one. */
function placeWindows(view: Window, presenter: Window, deckElement: HTMLElement): void {
  const getScreenDetails = (view as Window & { getScreenDetails?: () => Promise<ScreenDetailsLike> }).getScreenDetails;
  if (typeof getScreenDetails !== 'function') {
    return;
  }
  void (async () => {
    const details = await getScreenDetails.call(view);
    const current = details.currentScreen;
    const other = details.screens.find((screen) => screen !== current);
    if (other === undefined) {
      return;
    }
    presenter.moveTo(current.availLeft, current.availTop);
    presenter.resizeTo(current.availWidth, current.availHeight);
    await deckElement.requestFullscreen({ screen: other } as FullscreenOptions);
  })().catch(() => undefined);
}

function open(view: Window, url: string, deck: string, stage: HTMLElement): Window | null {
  const opened = view.open(url, `slidock-presenter-${deck.slice(0, 16)}`, 'popup,width=1100,height=720');
  if (opened !== null) {
    placeWindows(view, opened, stage);
  }
  return opened;
}

/** The presenter window's page: its frame, titled with the deck, around the presenter view. */
function mountWindow(host: HTMLElement, source: DeckSource, options: PresenterOptions): PresenterView {
  const frame = createPresenterWindow(host);
  frame.setTitle(source.deck.title);
  const view = mountView(frame.main, source, options, presenter);
  return {
    player: view.player,
    destroy: () => {
      view.destroy();
      frame.destroy();
    },
  };
}

/** The presenter view for `PlayerOptions.presenter` and `mount`; a deck page's `SlidockPresenter`. */
export const presenter: PresenterSupport = {
  help: ['P', 'Presenter view'],
  session: presenterSession,
  join,
  open,
  mount: mountWindow,
};

/** The presenter view alone (no window frame) of `source` in `host`, joined to its deck window. */
export function mountPresenter(host: HTMLElement, source: DeckSource, options: PresenterOptions): PresenterView {
  return mountView(host, source, options, presenter);
}
