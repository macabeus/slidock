import { act, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { App } from '../src/App';
import { testFontsEnvironment } from '../src/testing/fake-fonts';
import { FakePlayerFactory } from '../src/testing/fake-player';
import { FakeWorker } from '../src/testing/fake-worker';
import { SAMPLE_IMAGE_PATH, sampleDeck } from '../src/testing/sample-deck';
import { objectUrls } from './setup';

afterEach(() => {
  Object.defineProperty(window, 'opener', { value: null, configurable: true });
});

function renderPresenter() {
  window.history.replaceState(null, '', '/#presenter');
  const player = new FakePlayerFactory();
  const startWorker = vi.fn(() => new FakeWorker());
  const utils = render(<App services={{ startWorker, player, fonts: testFontsEnvironment() }} />);
  return { ...utils, player, startWorker };
}

describe('presenter window', () => {
  it('asks its opener for the deck and mounts the presenter view on it', () => {
    const opener = { postMessage: vi.fn() };
    Object.defineProperty(window, 'opener', { value: opener, configurable: true });
    const app = renderPresenter();
    expect(app.startWorker).not.toHaveBeenCalled();
    expect(opener.postMessage).toHaveBeenCalledWith({ type: 'slidock:presenter-ready' }, location.origin);
    expect(screen.getByText('Waiting for the deck window…')).toBeTruthy();

    const { deck, slides, media } = sampleDeck();
    const answer = new MessageEvent('message', {
      data: {
        type: 'slidock:presenter-deck',
        deck,
        slides,
        media,
        fonts: [],
        position: { slide: 1, step: 0 },
        serial: 1,
      },
      origin: location.origin,
    });
    Object.defineProperty(answer, 'source', { value: opener });
    act(() => {
      window.dispatchEvent(answer);
    });
    const presenter = app.player.presenters[0];
    expect(presenter?.options.startAt).toEqual({ slide: 1, step: 0 });
    expect(presenter?.input.deck.id).toBe(deck.id);
    const url = presenter?.input.mediaUrl(SAMPLE_IMAGE_PATH) ?? '';
    expect(objectUrls.has(url)).toBe(true);
    expect(screen.getByText('Presenter view · Quarterly review')).toBeTruthy();

    app.unmount();
    expect(presenter?.destroyed).toBe(true);
    expect(objectUrls.has(url)).toBe(false);
  });

  it('follows the deck window to each deck it opens, and lets go of a closed one', () => {
    const opener = { postMessage: vi.fn() };
    Object.defineProperty(window, 'opener', { value: opener, configurable: true });
    const app = renderPresenter();
    const send = (data: unknown) => {
      const event = new MessageEvent('message', { data, origin: location.origin });
      Object.defineProperty(event, 'source', { value: opener });
      act(() => {
        window.dispatchEvent(event);
      });
    };
    const first = sampleDeck();
    send({ type: 'slidock:presenter-deck', ...first, position: { slide: 0, step: 0 }, serial: 1 });
    const firstUrl = app.player.presenters[0]?.input.mediaUrl(SAMPLE_IMAGE_PATH) ?? '';

    const second = sampleDeck();
    second.deck = { ...second.deck, id: 'second-deck', title: 'Second deck' };
    send({ type: 'slidock:presenter-deck', ...second, position: { slide: 2, step: 0 }, serial: 2 });
    expect(opener.postMessage).toHaveBeenCalledTimes(1);
    expect(app.player.presenters[0]?.destroyed).toBe(true);
    expect(objectUrls.has(firstUrl)).toBe(false);
    expect(app.player.presenters[1]?.input.deck.id).toBe('second-deck');
    expect(app.player.presenters[1]?.options.startAt).toEqual({ slide: 2, step: 0 });
    expect(screen.getByText('Presenter view · Second deck')).toBeTruthy();
    const secondUrl = app.player.presenters[1]?.input.mediaUrl(SAMPLE_IMAGE_PATH) ?? '';

    send({ type: 'slidock:presenter-closed' });
    expect(app.player.presenters[1]?.destroyed).toBe(true);
    expect(objectUrls.has(secondUrl)).toBe(false);
    expect(screen.getByText('Waiting for the deck window…')).toBeTruthy();
  });

  it('keeps showing a deck its opener sends again, at the position it has reached', () => {
    const opener = { postMessage: vi.fn() };
    Object.defineProperty(window, 'opener', { value: opener, configurable: true });
    const app = renderPresenter();
    const send = (data: unknown) => {
      const event = new MessageEvent('message', { data, origin: location.origin });
      Object.defineProperty(event, 'source', { value: opener });
      act(() => {
        window.dispatchEvent(event);
      });
    };
    const deck = sampleDeck();
    send({ type: 'slidock:presenter-deck', ...deck, position: { slide: 0, step: 0 }, serial: 3 });
    // Sent again (the answer to `ready`, after the deck window has sent it on taking the window), with the position
    // the deck window had then: the players have moved on together since.
    send({ type: 'slidock:presenter-deck', ...deck, position: { slide: 0, step: 0 }, serial: 3 });
    expect(app.player.presenters).toHaveLength(1);
    expect(app.player.presenters[0]?.destroyed).toBe(false);
    // The deck window serves the deck anew (its fonts changed): the presenter view takes it.
    send({ type: 'slidock:presenter-deck', ...deck, position: { slide: 1, step: 0 }, serial: 4 });
    expect(app.player.presenters).toHaveLength(2);
    expect(app.player.presenters[1]?.options.startAt).toEqual({ slide: 1, step: 0 });
    // After a closed deck, the same deck sent again shows.
    send({ type: 'slidock:presenter-closed' });
    send({ type: 'slidock:presenter-deck', ...deck, position: { slide: 1, step: 0 }, serial: 4 });
    expect(app.player.presenters).toHaveLength(3);
  });

  it('ignores answers from other windows and other origins', () => {
    const opener = { postMessage: vi.fn() };
    Object.defineProperty(window, 'opener', { value: opener, configurable: true });
    const app = renderPresenter();
    const { deck, slides, media } = sampleDeck();
    const data = {
      type: 'slidock:presenter-deck',
      deck,
      slides,
      media,
      fonts: [],
      position: { slide: 0, step: 0 },
      serial: 1,
    };
    const foreign = new MessageEvent('message', { data, origin: 'https://evil.example' });
    Object.defineProperty(foreign, 'source', { value: opener });
    const stranger = new MessageEvent('message', { data, origin: location.origin });
    Object.defineProperty(stranger, 'source', { value: {} });
    const malformed = new MessageEvent('message', {
      data: { type: 'slidock:presenter-deck' },
      origin: location.origin,
    });
    Object.defineProperty(malformed, 'source', { value: opener });
    act(() => {
      window.dispatchEvent(foreign);
      window.dispatchEvent(stranger);
      window.dispatchEvent(malformed);
    });
    expect(app.player.presenters).toEqual([]);
  });

  it('explains itself when opened without a deck window', () => {
    renderPresenter();
    expect(screen.getByText('Nothing to present here')).toBeTruthy();
  });
});
