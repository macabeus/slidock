// @vitest-environment happy-dom
import type { DeckTransition } from '@slidock/types';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { DEFAULT_ADVANCE, autoNext, readAdvance } from '../src/advance';
import { type Player, RT, inlineDeck, mount } from '../src/index';
import { type MachineModel, numbering } from '../src/machine';
import { nextClick } from '../src/presenter/view';
import { deck, deckPage, slide, step, transition } from './fixtures';

const withAdvance = (advance: unknown): DeckTransition => ({ ...transition(), advance }) as unknown as DeckTransition;

describe('readAdvance', () => {
  it('reads click and timer advance', () => {
    expect(readAdvance(null)).toEqual(DEFAULT_ADVANCE);
    expect(readAdvance(transition())).toEqual(DEFAULT_ADVANCE);
    expect(readAdvance(withAdvance({ onClick: false, afterMs: 3000 }))).toEqual({
      onClick: false,
      afterMs: 3000,
      issue: null,
    });
    expect(readAdvance(withAdvance({ onClick: true, afterMs: 0 }))).toEqual({ onClick: true, afterMs: 0, issue: null });
  });

  it('plays the declared default for values out of the contract', () => {
    for (const afterMs of [-1, Number.NaN, Infinity, 3_600_001, '500', undefined]) {
      expect(readAdvance(withAdvance({ onClick: false, afterMs }))).toEqual({
        onClick: false,
        afterMs: null,
        issue: 'afterMs',
      });
    }
    expect(readAdvance(withAdvance({ onClick: 'no', afterMs: 10 }))).toEqual({
      onClick: true,
      afterMs: 10,
      issue: 'onClick',
    });
    expect(readAdvance(withAdvance(7))).toEqual({ ...DEFAULT_ADVANCE, issue: 'onClick' });
  });

  it('goes on when the timer ran out, or while groups remain on a slide that ignores clicks', () => {
    const click = { onClick: true, afterMs: 1000, issue: null };
    const kiosk = { onClick: false, afterMs: null, issue: null };
    expect(autoNext(click, false, true)).toBe(false);
    expect(autoNext(click, true, true)).toBe(true);
    expect(autoNext(click, true, false)).toBe(true);
    expect(autoNext(kiosk, false, true)).toBe(true);
    expect(autoNext(kiosk, false, false)).toBe(false);
  });
});

describe('nextClick', () => {
  const model: MachineModel = {
    slides: [
      { hidden: false, clickGroups: 2 },
      { hidden: true, clickGroups: 0 },
      { hidden: false, clickGroups: 0 },
    ],
    builds: false,
    loop: false,
  };
  const numbers = numbering(model.slides);
  const at = (slide: number, step: number) => ({ slide, step, presenting: true, blank: null });

  it('names the next build step, the next shown slide, or the end', () => {
    expect(nextClick(model, numbers, at(0, 0))).toEqual({
      kind: 'build',
      slide: 0,
      step: 1,
      label: 'Next: build 1 of 2',
    });
    expect(nextClick(model, numbers, at(0, 1))).toEqual({
      kind: 'build',
      slide: 0,
      step: 2,
      label: 'Next: build 2 of 2',
    });
    expect(nextClick(model, numbers, at(0, 2))).toEqual({ kind: 'slide', slide: 2, step: 0, label: 'Next: slide 2' });
    expect(nextClick(model, numbers, at(2, 0))).toEqual({ kind: 'end', label: 'Next' });
  });

  it('previews past a blank screen', () => {
    expect(nextClick(model, numbers, { ...at(0, 0), blank: 'black' })).toMatchObject({ kind: 'build', step: 1 });
  });
});

describe('auto-advance', () => {
  let players: Player[] = [];
  const flush = async () => {
    for (let i = 0; i < 5; i++) {
      await vi.advanceTimersByTimeAsync(0);
    }
  };
  /** Animations that finish at once: the timing under test is the player's own. */
  const original = Element.prototype.animate;
  beforeEach(() => {
    Element.prototype.animate = function () {
      return {
        finished: Promise.resolve(),
        finish: () => undefined,
        cancel: () => undefined,
      } as unknown as Animation;
    } as typeof Element.prototype.animate;
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'queueMicrotask'] });
    document.head.replaceChildren();
    document.body.replaceChildren();
    window.location.hash = '';
  });
  afterEach(() => {
    for (const player of players) {
      player.destroy();
    }
    players = [];
    vi.useRealTimers();
    Element.prototype.animate = original;
  });
  const start = async (slides: Parameters<typeof deck>[0]) => {
    deckPage(document, deck(slides));
    const player = mount(document.body, inlineDeck(document));
    players.push(player);
    const codes: string[] = [];
    player.on('diagnostic', (d) => codes.push(d.code));
    await flush();
    return { player, codes };
  };

  it('advances after afterMs from the arrival on the slide', async () => {
    const { player } = await start([
      slide('s1', { transition: withAdvance({ onClick: true, afterMs: 2000 }) }),
      slide('s2'),
      slide('s3'),
    ]);
    expect(player.position.slide).toBe(0);
    await vi.advanceTimersByTimeAsync(1999);
    expect(player.position.slide).toBe(0);
    await vi.advanceTimersByTimeAsync(1);
    await flush();
    expect(player.position.slide).toBe(1);
    // The second slide has no timer: it waits for a click.
    await vi.advanceTimersByTimeAsync(10_000);
    expect(player.position.slide).toBe(1);
  });

  it('plays the remaining builds before it leaves the slide', async () => {
    const { player } = await start([
      slide('s1', {
        timeline: [step('s1.title'), step('s1.title')],
        transition: withAdvance({ onClick: true, afterMs: 500 }),
      }),
      slide('s2'),
    ]);
    const steps: string[] = [];
    player.on('position', ({ position }) => steps.push(`${position.slide}.${position.step}`));
    await vi.advanceTimersByTimeAsync(500);
    for (let i = 0; i < 10; i++) {
      await flush();
    }
    expect(steps).toEqual(['0.1', '0.2', '1.0']);
  });

  it('a slide that ignores clicks plays its groups one after another, then waits for its timer', async () => {
    const { player } = await start([
      slide('s1', {
        timeline: [step('s1.title'), step('s1.title')],
        transition: withAdvance({ onClick: false, afterMs: 5000 }),
      }),
      slide('s2'),
    ]);
    for (let i = 0; i < 10; i++) {
      await flush();
    }
    expect(player.position).toMatchObject({ slide: 0, step: 2 });
    await vi.advanceTimersByTimeAsync(5000);
    await flush();
    expect(player.position).toMatchObject({ slide: 1, step: 0 });
  });

  it('a click forward does nothing on a slide that ignores clicks; keys still advance', async () => {
    const { player } = await start([
      slide('s1', { transition: withAdvance({ onClick: false, afterMs: null }) }),
      slide('s2'),
    ]);
    const wrapper = document.querySelector<HTMLElement>('.sd-player');
    wrapper?.dispatchEvent(new MouseEvent('click', { bubbles: true, clientX: 900, button: 0 }));
    await flush();
    expect(player.position.slide).toBe(0);
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }));
    await flush();
    expect(player.position.slide).toBe(1);
  });

  it('waits while blanked, and a move cancels the timer', async () => {
    const { player } = await start([
      slide('s1', { transition: withAdvance({ onClick: true, afterMs: 1000 }) }),
      slide('s2', { transition: withAdvance({ onClick: true, afterMs: 1000 }) }),
      slide('s3'),
    ]);
    player.blank('black');
    await vi.advanceTimersByTimeAsync(3000);
    await flush();
    expect(player.position).toMatchObject({ slide: 0, blank: 'black' });
    player.blank(null);
    await flush();
    expect(player.position.slide).toBe(1);
    // Stopping ends auto-advance.
    player.stop();
    await vi.advanceTimersByTimeAsync(3000);
    await flush();
    expect(player.position.slide).toBe(1);
  });

  it('reports an advance out of the contract and plays the default (a click)', async () => {
    const { player, codes } = await start([
      slide('s1', { transition: withAdvance({ onClick: true, afterMs: -5 }) }),
      slide('s2'),
    ]);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(player.position.slide).toBe(0);
    expect(codes).toEqual([RT.timingDefault]);
  });

  it('stops at the end of the show', async () => {
    const ended = vi.fn();
    const { player } = await start([
      slide('s1'),
      slide('s2', { transition: withAdvance({ onClick: true, afterMs: 100 }) }),
    ]);
    player.on('ended', ended);
    player.next();
    await flush();
    await vi.advanceTimersByTimeAsync(1000);
    await flush();
    expect(player.position.slide).toBe(1);
    expect(ended).toHaveBeenCalledTimes(1);
  });
});
