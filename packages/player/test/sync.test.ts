import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  PING_INTERVAL_MS,
  SYNC_NAMESPACE,
  type Sync,
  type SyncMessage,
  type SyncRole,
  type SyncState,
  type SyncTransport,
  broadcastTransport,
  createSync,
  windowTransport,
} from '../src/presenter/sync';

const DECK = 'd'.repeat(64);

/**
 * A window as `windowTransport` sees it: message listeners and `postMessage`, delivering asynchronously with the
 * other window of its pair as the source.
 */
class FakeWindow {
  closed = false;
  partner: FakeWindow | null = null;
  /** The target origin of each message posted to this window. */
  readonly targets: string[] = [];
  readonly #listeners = new Set<(event: MessageEvent) => void>();
  readonly location: { origin: string; protocol: string } | undefined;

  constructor(origin?: string) {
    this.location =
      origin === undefined ? undefined : { origin, protocol: origin === 'null' ? 'file:' : `${origin.split(':')[0]}:` };
  }

  addEventListener(_type: 'message', listener: (event: MessageEvent) => void): void {
    this.#listeners.add(listener);
  }

  removeEventListener(_type: 'message', listener: (event: MessageEvent) => void): void {
    this.#listeners.delete(listener);
  }

  postMessage(data: unknown, targetOrigin = '*'): void {
    this.targets.push(targetOrigin);
    this.deliver(data, this.partner);
  }

  /** A message from `source`, with its origin as a browser stamps it. */
  deliver(data: unknown, source: FakeWindow | null): void {
    const origin = source?.location?.origin ?? 'null';
    queueMicrotask(() => {
      for (const listener of Array.from(this.#listeners)) {
        listener({ data, source, origin } as unknown as MessageEvent);
      }
    });
  }
}

/** The deck window (opener) and the presenter window it opened. */
function windowPair(origin?: string): { deck: FakeWindow; presenter: FakeWindow } {
  const deck = new FakeWindow(origin);
  const presenter = new FakeWindow(origin);
  deck.partner = presenter;
  presenter.partner = deck;
  return { deck, presenter };
}

function fakeWindowTransport(self: FakeWindow, peer: () => FakeWindow | null): SyncTransport {
  return windowTransport(self as unknown as Window, () => peer() as unknown as Window | null);
}

/** A same-origin channel: every member but the sender receives each message, asynchronously. */
class FakeChannel {
  readonly #members = new Set<(message: unknown) => void>();

  transport(): SyncTransport {
    let mine: ((message: unknown) => void) | null = null;
    const send = (message: SyncMessage) => {
      for (const member of Array.from(this.#members)) {
        if (member !== mine) {
          queueMicrotask(() => member(message));
        }
      }
    };
    return {
      send,
      listen: (handler) => {
        mine = (message) => handler(message, send);
        this.#members.add(mine);
        return () => {
          if (mine !== null) {
            this.#members.delete(mine);
          }
        };
      },
    };
  }
}

interface SimWindow {
  state: SyncState;
  applied: SyncState[];
  sync: Sync;
}

function simulate(
  role: SyncRole,
  session: string,
  transports: SyncTransport[],
  state: SyncState = { slide: 0, step: 0, blank: null },
  deck = DECK,
): SimWindow {
  const window: SimWindow = { state, applied: [], sync: null as unknown as Sync };
  window.sync = createSync({
    deck,
    role,
    session,
    transports,
    current: () => window.state,
    apply: (next) => {
      window.state = next;
      window.applied.push(next);
    },
    pingMs: 0,
  });
  return window;
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
const at = (slide: number, step = 0, blank: SyncState['blank'] = null): SyncState => ({ slide, step, blank });

afterEach(() => {
  vi.useRealTimers();
});

describe('presenter sync', () => {
  it('a starting presenter follows the deck window, then either window drives', async () => {
    const channel = new FakeChannel();
    const deck = simulate('deck', 's1', [channel.transport()], at(4, 1));
    const presenter = simulate('presenter', 's1', [channel.transport()]);
    await flush();
    expect(presenter.state).toEqual(at(4, 1));

    deck.state = at(5);
    deck.sync.publish(deck.state);
    await flush();
    expect(presenter.state).toEqual(at(5));

    presenter.state = at(5, 0, 'black');
    presenter.sync.publish(presenter.state);
    await flush();
    expect(deck.state).toEqual(at(5, 0, 'black'));
  });

  it('ignores other decks, other sessions and its own role', async () => {
    const channel = new FakeChannel();
    const presenter = simulate('presenter', 's1', [channel.transport()]);
    const foreign = simulate('deck', 's1', [channel.transport()], at(9), 'e'.repeat(64));
    const otherSession = simulate('deck', 's2', [channel.transport()], at(8));
    const otherPresenter = simulate('presenter', 's1', [channel.transport()], at(7));
    await flush();
    foreign.sync.publish(at(9));
    otherSession.sync.publish(at(8));
    otherPresenter.sync.publish(at(7));
    await flush();
    expect(presenter.applied).toEqual([]);
  });

  it('drops malformed messages', async () => {
    const channel = new FakeChannel();
    const presenter = simulate('presenter', 's1', [channel.transport()]);
    const raw = channel.transport();
    raw.listen(() => undefined);
    const base: SyncMessage = {
      ns: SYNC_NAMESPACE,
      deck: DECK,
      session: 's1',
      role: 'deck',
      sender: 'x',
      seq: 1,
      kind: 'state',
      state: at(3),
    };
    raw.send({ ...base, state: { slide: 1.5, step: 0, blank: null } });
    raw.send({ ...base, seq: 2, ns: 'other' as typeof SYNC_NAMESPACE });
    raw.send({ ...base, seq: 3, state: { slide: 1, step: 0, blank: 'red' as 'black' } });
    await flush();
    expect(presenter.applied).toEqual([]);
    raw.send({ ...base, seq: 4 });
    await flush();
    expect(presenter.applied).toEqual([at(3)]);
  });

  it('applies a message that arrives over both transports once', async () => {
    const channel = new FakeChannel();
    const { deck: deckWindow, presenter: presenterWindow } = windowPair();
    const deck = simulate('deck', 's1', [fakeWindowTransport(deckWindow, () => presenterWindow), channel.transport()]);
    const presenter = simulate('presenter', 's1', [
      fakeWindowTransport(presenterWindow, () => deckWindow),
      channel.transport(),
    ]);
    await flush();
    presenter.applied.length = 0;
    deck.sync.publish(at(2));
    await flush();
    expect(presenter.applied).toEqual([at(2)]);
  });

  it('a page with an origin posts to its own origin and hears only windows of it; a file page posts to any', async () => {
    const origin = 'https://decks.example';
    const { deck: deckWindow, presenter: presenterWindow } = windowPair(origin);
    const deck = simulate('deck', 's1', [fakeWindowTransport(deckWindow, () => presenterWindow)], at(3));
    const presenter = simulate('presenter', 's1', [fakeWindowTransport(presenterWindow, () => deckWindow)]);
    await flush();
    expect(presenter.state).toEqual(at(3));
    expect(new Set([...deckWindow.targets, ...presenterWindow.targets])).toEqual(new Set([origin]));

    // Another origin's window, even one that names the deck and the session, is not heard.
    const stranger = new FakeWindow('https://elsewhere.example');
    const message: SyncMessage = {
      ns: SYNC_NAMESPACE,
      deck: DECK,
      session: 's1',
      role: 'deck',
      sender: 'stranger',
      seq: 1,
      kind: 'state',
      state: at(9),
    };
    presenterWindow.deliver(message, stranger);
    await flush();
    expect(presenter.state).toEqual(at(3));
    presenterWindow.deliver({ ...message, sender: 'same-origin' }, new FakeWindow(origin));
    await flush();
    expect(presenter.state).toEqual(at(9));
    deck.sync.destroy();
    presenter.sync.destroy();

    // A file:// page's origin is opaque (WebKit spells it `file://`): it posts to any origin, and the ids alone gate
    // what it hears.
    const files = windowPair('file://');
    simulate('deck', 's2', [fakeWindowTransport(files.deck, () => files.presenter)], at(4));
    const filePresenter = simulate('presenter', 's2', [fakeWindowTransport(files.presenter, () => files.deck)]);
    await flush();
    expect(filePresenter.state).toEqual(at(4));
    expect(new Set([...files.deck.targets, ...files.presenter.targets])).toEqual(new Set(['*']));
    files.presenter.deliver({ ...message, session: 's2', sender: 'file' }, stranger);
    await flush();
    expect(filePresenter.state).toEqual(at(9));
  });

  it('a reloaded deck window rejoins over the channel and follows the presenter', async () => {
    const channel = new FakeChannel();
    const first = simulate('deck', 's1', [channel.transport()]);
    const presenter = simulate('presenter', 's1', [channel.transport()]);
    await flush();
    presenter.state = at(6, 2);
    presenter.sync.publish(presenter.state);
    await flush();
    first.sync.leave();

    const reloaded = simulate('deck', 's2', [channel.transport()]);
    await flush();
    expect(presenter.sync.session).toBe('s2');
    expect(reloaded.state).toEqual(at(6, 2));
    reloaded.state = at(7);
    reloaded.sync.publish(reloaded.state);
    await flush();
    expect(presenter.state).toEqual(at(7));
  });

  it('a second deck window does not take over a presenter whose deck window is live', async () => {
    const channel = new FakeChannel();
    simulate('deck', 's1', [channel.transport()]);
    const presenter = simulate('presenter', 's1', [channel.transport()]);
    await flush();
    simulate('deck', 's2', [channel.transport()], at(4));
    await flush();
    expect(presenter.sync.session).toBe('s1');
    expect(presenter.applied).toEqual([at(0)]);
  });

  it('a crashed deck window is replaced once it misses its pongs, through the opener alone', async () => {
    vi.useFakeTimers();
    const { deck: deckWindow, presenter: presenterWindow } = windowPair();
    const first = simulate('deck', 's1', [fakeWindowTransport(deckWindow, () => presenterWindow)]);
    const presenterState = { value: at(0) };
    const presenter = createSync({
      deck: DECK,
      role: 'presenter',
      session: 's1',
      transports: [fakeWindowTransport(presenterWindow, () => deckWindow)],
      current: () => presenterState.value,
      apply: (state) => (presenterState.value = state),
    });
    await vi.advanceTimersByTimeAsync(0);
    presenterState.value = at(3, 1);
    presenter.publish(presenterState.value);
    await vi.advanceTimersByTimeAsync(0);
    expect(first.state).toEqual(at(3, 1));
    await vi.advanceTimersByTimeAsync(PING_INTERVAL_MS);
    // A crash: no bye.
    first.sync.destroy();

    // The reloaded deck no longer knows the presenter window: its hello reaches nobody.
    const reloaded = simulate('deck', 's2', [fakeWindowTransport(deckWindow, () => null)]);
    await vi.advanceTimersByTimeAsync(0);
    expect(reloaded.state).toEqual(at(0));
    await vi.advanceTimersByTimeAsync(PING_INTERVAL_MS);
    expect(presenter.session).toBe('s1');
    await vi.advanceTimersByTimeAsync(2 * PING_INTERVAL_MS);
    expect(presenter.session).toBe('s2');
    expect(reloaded.state).toEqual(at(3, 1));
    presenter.destroy();
  });

  it('a reloaded presenter window takes its session from the URL and follows the deck', async () => {
    const channel = new FakeChannel();
    simulate('deck', 's1', [channel.transport()], at(2, 1, 'white'));
    const first = simulate('presenter', 's1', [channel.transport()]);
    await flush();
    first.sync.leave();
    const reloaded = simulate('presenter', 's1', [channel.transport()]);
    await flush();
    expect(reloaded.state).toEqual(at(2, 1, 'white'));
  });

  it('a presenter opened without a session adopts the deck window’s', async () => {
    const { deck: deckWindow, presenter: presenterWindow } = windowPair();
    simulate('deck', 's1', [fakeWindowTransport(deckWindow, () => presenterWindow)], at(5));
    const presenter = simulate('presenter', '', [fakeWindowTransport(presenterWindow, () => deckWindow)]);
    await flush();
    await flush();
    expect(presenter.sync.session).toBe('s1');
  });

  it('runs over a real BroadcastChannel keyed by the deck id', async () => {
    const deckChannel = broadcastTransport(DECK);
    const presenterChannel = broadcastTransport(DECK);
    expect(deckChannel).not.toBeNull();
    const deck = simulate('deck', 's1', [deckChannel as SyncTransport], at(3));
    const presenter = simulate('presenter', 's1', [presenterChannel as SyncTransport]);
    await vi.waitFor(() => expect(presenter.state).toEqual(at(3)));
    deck.sync.destroy();
    presenter.sync.destroy();
  });
});
