// The presenter-view sync: the deck window and the presenter window exchange `{slide, step, blank}` and follow
// each other; either drives. Messages go over `postMessage` between the opener and the window it opened,
// and over a `BroadcastChannel` keyed by the deck id, which lets a reloaded window find the other again. Every message
// carries the deck id and the session id; any other deck's or session's message is ignored.
//
// A window that starts says `hello`; the window already running answers with its state, which the new one follows.
// The presenter pings its deck window; the deck answers `pong`. A deck window that reloads starts a new session and
// says `bye` as it leaves. The presenter adopts another deck window's session only once its own deck window is gone
// (it said `bye`, or missed its pongs) or when it never had one: on that window's `hello`, or on the `hello` a deck
// window sends back to a ping of another session, which is how a reloaded deck is found when only `window.opener`
// connects the two.
import type { Blank } from '../machine';

export const SYNC_NAMESPACE = 'slidock-sync@1';
export const PING_INTERVAL_MS = 3000;

export type SyncRole = 'deck' | 'presenter';

export interface SyncState {
  readonly slide: number;
  readonly step: number;
  readonly blank: Blank | null;
}

export interface SyncMessage {
  readonly ns: typeof SYNC_NAMESPACE;
  readonly deck: string;
  readonly session: string;
  readonly role: SyncRole;
  /** The sending instance, so a window drops its own messages and duplicates from a second transport. */
  readonly sender: string;
  readonly seq: number;
  readonly kind: 'hello' | 'ping' | 'pong' | 'state' | 'bye';
  readonly state: SyncState | null;
}

/** A way to reach the other window. `reply` answers on the transport and to the window a message came from. */
export interface SyncTransport {
  send(message: SyncMessage): void;
  listen(handler: (message: unknown, reply: (message: SyncMessage) => void) => void): () => void;
}

export interface SyncOptions {
  readonly deck: string;
  readonly role: SyncRole;
  readonly session: string;
  readonly transports: readonly SyncTransport[];
  /** The local position, sent to a window that says hello. */
  readonly current: () => SyncState;
  /** Follows the other window. */
  readonly apply: (state: SyncState) => void;
  /** Called when the presenter adopts a reloaded deck window's session. */
  readonly onSession?: (session: string) => void;
  readonly instance?: string;
  /** The presenter's ping period; 0 turns pings off, and with them the detection of a deck window that is gone. */
  readonly pingMs?: number;
}

export interface Sync {
  readonly session: string;
  /** Sends the local position after a local move; the other window follows. */
  publish(state: SyncState): void;
  /** Says `bye` (the page is going away), then stops. */
  leave(): void;
  /** Stops without a word. */
  destroy(): void;
}

export function randomId(): string {
  const crypto = globalThis.crypto as Crypto | undefined;
  if (crypto !== undefined && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return Math.random().toString(36).slice(2) + Date.now().toString(36);
}

function isMessage(value: unknown): value is SyncMessage {
  if (typeof value !== 'object' || value === null) {
    return false;
  }
  const m = value as Partial<SyncMessage>;
  return (
    m.ns === SYNC_NAMESPACE &&
    typeof m.deck === 'string' &&
    typeof m.session === 'string' &&
    (m.role === 'deck' || m.role === 'presenter') &&
    typeof m.sender === 'string' &&
    typeof m.seq === 'number' &&
    (m.kind === 'hello' || m.kind === 'ping' || m.kind === 'pong' || m.kind === 'state' || m.kind === 'bye')
  );
}

function isState(value: unknown): value is SyncState {
  if (typeof value !== 'object' || value === null) {
    return false;
  }
  const s = value as Partial<SyncState>;
  return (
    Number.isInteger(s.slide) &&
    Number.isInteger(s.step) &&
    (s.blank === null || s.blank === 'black' || s.blank === 'white')
  );
}

export function createSync(options: SyncOptions): Sync {
  const instance = options.instance ?? randomId();
  let session = options.session;
  let seq = 0;
  let destroyed = false;
  const lastSeq = new Map<string, number>();

  const message = (kind: SyncMessage['kind'], state: SyncState | null): SyncMessage => ({
    ns: SYNC_NAMESPACE,
    deck: options.deck,
    session,
    role: options.role,
    sender: instance,
    seq: ++seq,
    kind,
    state,
  });
  const broadcast = (kind: SyncMessage['kind'], state: SyncState | null): void => {
    const m = message(kind, state);
    for (const transport of options.transports) {
      try {
        transport.send(m);
      } catch {
        // A closed window or channel: the other transport may still reach it.
      }
    }
  };

  const pingMs = options.pingMs ?? PING_INTERVAL_MS;
  const staleMs = pingMs > 0 ? 2 * pingMs + 500 : Infinity;
  // When the presenter last heard its deck window; -Infinity once that window said bye, or before it was ever heard.
  let lastHeard = options.session === '' ? -Infinity : Date.now();
  const orphaned = (): boolean => lastHeard === -Infinity || Date.now() - lastHeard > staleMs;

  const receive = (value: unknown, reply: (message: SyncMessage) => void): void => {
    if (destroyed || !isMessage(value) || value.deck !== options.deck || value.sender === instance) {
      return;
    }
    if (value.role === options.role) {
      return;
    }
    if ((lastSeq.get(value.sender) ?? 0) >= value.seq) {
      return;
    }
    lastSeq.set(value.sender, value.seq);
    if (value.session !== session) {
      if (options.role === 'presenter' && value.kind === 'hello' && orphaned()) {
        session = value.session;
        lastHeard = Date.now();
        options.onSession?.(session);
        reply(message('state', options.current()));
      } else if (options.role === 'deck' && (value.kind === 'hello' || value.kind === 'ping')) {
        reply(message('hello', null));
      }
      return;
    }
    if (options.role === 'presenter') {
      lastHeard = value.kind === 'bye' ? -Infinity : Date.now();
    }
    if (value.kind === 'hello') {
      reply(message('state', options.current()));
    } else if (value.kind === 'ping') {
      reply(message('pong', null));
    } else if (value.kind === 'state' && isState(value.state)) {
      options.apply({ slide: value.state.slide, step: value.state.step, blank: value.state.blank });
    }
  };

  const unlisten = options.transports.map((transport) => transport.listen(receive));
  broadcast('hello', null);
  const pinger =
    options.role === 'presenter' && pingMs > 0 ? setInterval(() => broadcast('ping', null), pingMs) : undefined;
  const destroy = (): void => {
    destroyed = true;
    if (pinger !== undefined) {
      clearInterval(pinger);
    }
    for (const stop of unlisten) {
      stop();
    }
  };

  return {
    get session() {
      return session;
    },
    publish: (state) => {
      if (!destroyed) {
        broadcast('state', state);
      }
    },
    leave: () => {
      if (!destroyed) {
        broadcast('bye', null);
        destroy();
      }
    },
    destroy,
  };
}

/**
 * A window's origin, or `null` when it has none to match: a `file://` page (whose origin is opaque, though WebKit
 * spells it `file://`), a sandboxed frame, or one that cannot be read.
 */
export function originOf(view: Window): string | null {
  try {
    const { origin, protocol } = view.location;
    const opaque = protocol === 'file:' || origin === '' || origin === 'null';
    return typeof origin === 'string' && !opaque ? origin : null;
  } catch {
    return null;
  }
}

/**
 * `postMessage` to the window `peer` returns (the opener, or the window this one opened). A page with an origin
 * sends only to, and hears only from, windows of its own origin (the presenter window opens the deck's own page);
 * a `file://` page, whose origin is opaque, sends to any and hears any, gated by the deck and session ids (positions
 * are not secret).
 */
export function windowTransport(
  self: Window,
  peer: () => Window | null,
  onPeer?: (peer: Window) => void,
): SyncTransport {
  const origin = originOf(self);
  const target = origin ?? '*';
  return {
    send: (message) => {
      const window = peer();
      if (window !== null && !window.closed) {
        window.postMessage(message, target);
      }
    },
    listen: (handler) => {
      const listener = (event: MessageEvent): void => {
        const source = event.source as Window | null;
        if (source === null || source === self || (origin !== null && event.origin !== origin)) {
          return;
        }
        handler(event.data, (reply) => {
          onPeer?.(source);
          source.postMessage(reply, target);
        });
      };
      self.addEventListener('message', listener);
      return () => self.removeEventListener('message', listener);
    },
  };
}

/** A `BroadcastChannel` named after the deck id, for same-origin windows. */
export function broadcastTransport(deck: string): SyncTransport | null {
  if (typeof BroadcastChannel !== 'function') {
    return null;
  }
  let channel: BroadcastChannel;
  try {
    channel = new BroadcastChannel(`slidock:${deck}`);
  } catch {
    return null;
  }
  const handlers = new Set<(event: MessageEvent) => void>();
  return {
    send: (message) => channel.postMessage(message),
    listen: (handler) => {
      const listener = (event: MessageEvent): void => handler(event.data, (reply) => channel.postMessage(reply));
      handlers.add(listener);
      channel.addEventListener('message', listener);
      return () => {
        channel.removeEventListener('message', listener);
        handlers.delete(listener);
        if (handlers.size === 0) {
          channel.close();
        }
      };
    },
  };
}
