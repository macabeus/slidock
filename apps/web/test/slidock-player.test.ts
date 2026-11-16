import { afterEach, describe, expect, it, vi } from 'vitest';

import { slidockPlayer } from '../src/player/slidock-player';
import type { DeckInput, PlayerEvent } from '../src/player/types';
import { SAMPLE_IMAGE_PATH, sampleDeck } from '../src/testing/sample-deck';

const flush = async () => {
  for (let i = 0; i < 5; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
};

function input(): DeckInput {
  const { deck, slides } = sampleDeck();
  return { deck, slides, mediaUrl: (path) => (path === SAMPLE_IMAGE_PATH ? 'blob:test/image' : path) };
}

function host(): HTMLElement {
  return document.body.appendChild(document.createElement('div'));
}

/** The slide the player shows, by its layer. */
function shown(root: ParentNode): Element | null {
  return root.querySelector('.sd-player > .sd-frame > .sd-layer[data-sd-current]');
}

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe('the app player over @slidock/player', () => {
  it('views a slide fully built in a shadow root, with media from the app, and reports moves', async () => {
    const events: PlayerEvent[] = [];
    const target = host();
    const player = slidockPlayer.createPlayer(target, input(), {
      mode: 'view',
      startAt: { slide: 1 },
      onEvent: (event) => events.push(event),
    });
    await flush();
    const layer = shown(target);
    expect(layer?.getAttribute('data-sd-layer')).toBe('s2');
    expect(layer?.shadowRoot?.querySelector('img')?.getAttribute('src')).toBe('blob:test/image');
    expect(player.position).toEqual({ slide: 1, step: 1, presenting: false, blank: null });

    player.next();
    await flush();
    expect(events).toContainEqual({
      type: 'position',
      position: { slide: 2, step: 0, presenting: false, blank: null },
    });
    player.destroy();
    expect(target.querySelector('.sd-player')).toBeNull();
  });

  it('draws thumbnails fully built', () => {
    const target = host();
    const player = slidockPlayer.createPlayer(host(), input(), { mode: 'view' });
    const dispose = player.renderStatic('s2', target, 'contain');
    const layer = target.querySelector('.sd-static .sd-layer');
    expect(layer?.getAttribute('data-sd-layer')).toBe('s2');
    expect(layer?.shadowRoot?.querySelector('[data-sd-state="hidden"]')).toBeNull();
    dispose();
    expect(target.children).toHaveLength(0);
    player.destroy();
  });

  it('reports runtime diagnostics as warnings about their slide', async () => {
    const base = input();
    const hostile: DeckInput = {
      ...base,
      slides: base.slides.map((slide) =>
        slide.id === 's1'
          ? { ...slide, html: slide.html.replace('</main>', '<script>alert(1)</script></main>') }
          : slide,
      ),
    };
    const events: PlayerEvent[] = [];
    const player = slidockPlayer.createPlayer(host(), hostile, {
      mode: 'view',
      onEvent: (event) => events.push(event),
    });
    await flush();
    const diagnostic = events.find((event) => event.type === 'diagnostic');
    expect(diagnostic).toMatchObject({
      type: 'diagnostic',
      diagnostic: { code: 'RT0002', severity: 'warning', container: 's1', feature: null, node: null },
    });
    player.destroy();
  });

  it('opens the presenter window as the app at #presenter, presenting from here', () => {
    window.history.replaceState(null, '', '/deck/?x=1#slide=2');
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    const player = slidockPlayer.createPlayer(host(), input(), { mode: 'view' });
    player.openPresenter();
    expect(open).toHaveBeenCalledTimes(1);
    const url = String(open.mock.calls[0]?.[0]);
    // The URL names the deck window's sync session, which the presenter view joins.
    expect(url).toMatch(/^\/deck\/\?x=1#presenter=[\w-]+$/);
    expect(player.position.presenting).toBe(true);
    player.destroy();
  });

  it("takes the deck window's position as it is when the presenter view joins, not as the deck was sent", async () => {
    // Presenter and deck in one page, connected by the deck's BroadcastChannel as two windows are.
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    const deckHost = host();
    const deck = slidockPlayer.createPlayer(deckHost, input(), { mode: 'view' });
    deck.openPresenter();
    const url = String(open.mock.calls[0]?.[0]);
    const session = decodeURIComponent(url.split('#presenter=')[1] ?? '');
    await flush();
    // The deck was sent at its first slide; the deck window moves on before the presenter view exists.
    deck.next();
    await flush();
    expect(deck.position.slide).toBe(1);
    const target = host();
    const presenter = slidockPlayer.createPresenter(target, input(), { startAt: { slide: 0 }, session });
    await vi.waitFor(() =>
      expect(shown(target.querySelector('.sd-current') ?? target)?.getAttribute('data-sd-layer')).toBe('s2'),
    );
    // The deck window stays where it is: the presenter view followed it, not the other way round.
    expect(deck.position.slide).toBe(1);
    presenter.destroy();
    deck.destroy();
  });

  it('reports the presenter window its own P key opens', async () => {
    const popup = { closed: false } as Window;
    vi.spyOn(window, 'open').mockReturnValue(popup);
    const events: PlayerEvent[] = [];
    const target = host();
    const player = slidockPlayer.createPlayer(target, input(), {
      mode: 'view',
      onEvent: (event) => events.push(event),
    });
    await flush();
    target
      .querySelector('.sd-player')
      ?.dispatchEvent(new KeyboardEvent('keydown', { key: 'p', bubbles: true, cancelable: true }));
    expect(events).toContainEqual({ type: 'presenter', window: popup });
    player.destroy();
  });

  it('mounts the presenter view where the deck window is', async () => {
    const target = host();
    const presenter = slidockPlayer.createPresenter(target, input(), { startAt: { slide: 1 }, session: '' });
    await flush();
    expect(target.querySelector('.sd-presenter')).not.toBeNull();
    expect(shown(target)?.getAttribute('data-sd-layer')).toBe('s2');
    expect(Array.from(target.querySelectorAll('.sd-notes p'), (p) => p.textContent)).toEqual(
      sampleDeck().deck.slides[1]?.notes,
    );
    presenter.destroy();
    expect(target.querySelector('.sd-presenter')).toBeNull();
  });
});
