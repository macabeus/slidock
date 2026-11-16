// @vitest-environment happy-dom
import type { Deck, DeckStep, DeckTransition } from '@slidock/types';
import { afterEach, beforeEach, describe, expect, it, onTestFinished, vi } from 'vitest';

import { type Player, RT, createPlayer, htmlDeck, inlineDeck, mount } from '../src/index';
import { mountPresenter, presenter as presenterView } from '../src/presenter/support';
import { deck, deckPage, slide, slideHtml, step, transition } from './fixtures';

const flush = async () => {
  for (let i = 0; i < 5; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
};

function key(k: string, target: EventTarget = document): KeyboardEvent {
  const event = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true });
  target.dispatchEvent(event);
  return event;
}

function layers(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>('.sd-layer'));
}

function current(): HTMLElement | undefined {
  return layers().find((layer) => layer.hasAttribute('data-sd-current'));
}

function counter(): string {
  return document.querySelector('.sd-counter')?.textContent ?? '';
}

let players: Player[] = [];

interface FakeAnimation {
  readonly keyframes: Keyframe[];
  readonly options: KeyframeAnimationOptions;
  finished: Promise<void>;
  finish(): void;
  cancel(): void;
  done: boolean;
  cancelled: boolean;
  /** `finished` once it has played to its end by its own time, before its promise settles; then `finish` settles it. */
  playState: AnimationPlayState;
}

/** Replaces `Element.animate` with animations that finish when the test says (or at once with `instant`). */
function fakeAnimate(instant: boolean): { animations: FakeAnimation[]; restore: () => void } {
  const animations: FakeAnimation[] = [];
  const original = Element.prototype.animate;
  Element.prototype.animate = function (keyframes: Keyframe[], options: KeyframeAnimationOptions) {
    let resolve: () => void = () => undefined;
    const animation: FakeAnimation = {
      keyframes,
      options,
      done: false,
      cancelled: false,
      playState: 'running',
      finished: new Promise<void>((r) => (resolve = r)),
      finish: () => {
        animation.done = true;
        animation.playState = 'finished';
        resolve();
      },
      cancel: () => {
        animation.cancelled = true;
      },
    };
    if (instant) {
      animation.finish();
    }
    animations.push(animation);
    return animation as unknown as Animation;
  } as typeof Element.prototype.animate;
  return { animations, restore: () => (Element.prototype.animate = original) };
}

function track(player: Player): Player {
  players.push(player);
  return player;
}

beforeEach(() => {
  document.head.replaceChildren();
  document.body.replaceChildren();
  window.location.hash = '';
});

afterEach(() => {
  for (const player of players) {
    player.destroy();
  }
  players = [];
});

describe('deck sources', () => {
  it('reads the deck page: #sd-deck and the slide templates', () => {
    const data = deck([slide('s1'), slide('s2')]);
    deckPage(document, data);
    const source = inlineDeck(document);
    expect(source.deck.slides.map((s) => s.id)).toEqual(['s1', 's2']);
    const fragment = source.slide('s2');
    expect(fragment?.querySelector('main')?.getAttribute('data-slide')).toBe('s2');
    expect(source.slide('s2')).not.toBe(fragment);
    expect(source.slide('nope')).toBeNull();
  });

  it("keeps the page's faces in the head, so a host that clears the body keeps them", () => {
    deckPage(document, deck([slide('s1')]));
    const fonts = document.createElement('style');
    fonts.id = 'sd-fonts';
    fonts.textContent = '@font-face{font-family:"X";src:url(data:font/woff2;base64,AA==)}';
    document.body.prepend(fonts);
    const source = inlineDeck(document);
    document.body.replaceChildren();
    expect(document.getElementById('sd-fonts')?.parentNode).toBe(document.head);
    expect(source.slide('s1')?.querySelector('main')?.getAttribute('data-slide')).toBe('s1');
  });

  it('refuses a deck format it does not read', () => {
    deckPage(document, { ...deck([slide('s1')]), format: 'slidock-deck@2' as Deck['format'] });
    expect(() => inlineDeck(document)).toThrow(/slidock-deck@1/);
  });
});

describe('player', () => {
  it('mounts each slide in its own shadow root and shows the first', async () => {
    deckPage(document, deck([slide('s1'), slide('s2'), slide('s3')]));
    track(mount(document.body, inlineDeck(document)));
    await flush();
    expect(current()?.getAttribute('data-sd-layer')).toBe('s1');
    const root = current()?.shadowRoot;
    expect(root?.querySelector('main[data-slide="s1"]')).not.toBeNull();
    expect(document.querySelector('main[data-slide]')).toBeNull();
    expect(document.querySelector('.sd-frame')?.getAttribute('style')).toContain('width: 960px');
  });

  it('keeps the current slide and its neighbours mounted, and unmounts far ones', async () => {
    deckPage(document, deck(['a', 'b', 'c', 'd', 'e', 'f'].map((id) => slide(id))));
    const player = track(mount(document.body, inlineDeck(document)));
    await flush();
    expect(layers().map((l) => l.getAttribute('data-sd-layer'))).toEqual(['a', 'b']);
    player.goTo(3);
    await flush();
    expect(
      layers()
        .map((l) => l.getAttribute('data-sd-layer'))
        .sort(),
    ).toEqual(['c', 'd', 'e']);
  });

  it('navigates with PowerPoint keys, skipping hidden slides and numbering shown ones', async () => {
    deckPage(document, deck([slide('s1'), slide('s2', { hidden: true }), slide('s3'), slide('s4')]));
    const player = track(mount(document.body, inlineDeck(document)));
    await flush();
    expect(counter()).toBe('1 / 3');
    expect(key('ArrowRight').defaultPrevented).toBe(true);
    await flush();
    expect(player.position.slide).toBe(2);
    expect(counter()).toBe('2 / 3');
    key('End');
    await flush();
    expect(player.position.slide).toBe(3);
    key('Home');
    await flush();
    expect(player.position.slide).toBe(0);
    key('3');
    expect(counter()).toBe('→ 3');
    key('Enter');
    await flush();
    expect(player.position.slide).toBe(3);
    key('PageUp');
    await flush();
    expect(player.position.slide).toBe(2);
    expect(current()?.getAttribute('data-sd-layer')).toBe('s3');
  });

  it('numbers shown slides only while viewing, where it shows the hidden ones', async () => {
    deckPage(document, deck([slide('s1'), slide('s2', { hidden: true }), slide('s3'), slide('s4')]));
    const player = track(mount(document.body, inlineDeck(document), { mode: 'view' }));
    const live = () => document.querySelector('.sd-sr')?.textContent ?? '';
    await flush();
    expect(counter()).toBe('1 / 3');
    key('ArrowRight');
    await flush();
    expect(player.position.slide).toBe(1);
    expect(counter()).toBe('hidden');
    expect(live()).toMatch(/^Slide – of 3/);
    key('ArrowRight');
    await flush();
    expect(counter()).toBe('2 / 3');
    expect(live()).toMatch(/^Slide 2 of 3/);
    key('3');
    key('Enter');
    await flush();
    expect(player.position.slide).toBe(3);
    expect(counter()).toBe('3 / 3');
  });

  it('ignores keys typed in a field', async () => {
    deckPage(document, deck([slide('s1'), slide('s2')]));
    const player = track(mount(document.body, inlineDeck(document)));
    const input = document.createElement('input');
    document.body.append(input);
    await flush();
    key('ArrowRight', input);
    await flush();
    expect(player.position.slide).toBe(0);
  });

  it('blanks black and white and un-blanks on next', async () => {
    deckPage(document, deck([slide('s1'), slide('s2')]));
    const player = track(mount(document.body, inlineDeck(document)));
    await flush();
    const blank = document.querySelector('.sd-blank');
    key('b');
    expect(blank?.getAttribute('data-sd-blank')).toBe('black');
    key('w');
    expect(blank?.getAttribute('data-sd-blank')).toBe('white');
    key('ArrowRight');
    await flush();
    expect(blank?.getAttribute('data-sd-blank')).toBe('');
    expect(player.position.slide).toBe(0);
  });

  it('Escape stops presenting: hidden slides show again', async () => {
    deckPage(document, deck([slide('s1'), slide('s2', { hidden: true })]));
    const player = track(mount(document.body, inlineDeck(document)));
    await flush();
    expect(player.position.presenting).toBe(true);
    key('Escape');
    await flush();
    expect(player.position.presenting).toBe(false);
    key('ArrowRight');
    await flush();
    expect(player.position.slide).toBe(1);
  });

  it('opens and closes the help overlay', async () => {
    deckPage(document, deck([slide('s1')]));
    track(mount(document.body, inlineDeck(document)));
    const help = document.querySelector<HTMLElement>('.sd-help');
    expect(help?.hidden).toBe(true);
    key('?');
    expect(help?.hidden).toBe(false);
    key('Escape');
    expect(help?.hidden).toBe(true);
  });

  it('pre-hides entrance builds, plays click groups forward and cuts back', async () => {
    const timeline = [
      step('s1.title', { target: { element: 's1.title', paragraphs: [0, 0] } }),
      step('s1.title', { target: { element: 's1.title', paragraphs: [1, 1] } }),
    ];
    deckPage(document, deck([slide('s1', { timeline }), slide('s2')]));
    const fake = fakeAnimate(true);
    onTestFinished(fake.restore);
    const player = track(mount(document.body, inlineDeck(document)));
    await flush();
    const root = current()?.shadowRoot;
    const hidden = () =>
      Array.from(root?.querySelectorAll('[data-p]') ?? []).map((p) => p.getAttribute('data-sd-state') === 'hidden');
    expect(hidden()).toEqual([true, true]);
    player.next();
    await flush();
    expect(player.position).toMatchObject({ slide: 0, step: 1 });
    expect(hidden()).toEqual([false, true]);
    player.next();
    await flush();
    expect(hidden()).toEqual([false, false]);
    player.previous();
    await flush();
    expect(player.position).toMatchObject({ slide: 0, step: 1 });
    expect(hidden()).toEqual([false, true]);
    expect(fake.animations[0]?.keyframes).toEqual([
      { offset: 0, opacity: 0 },
      { offset: 1, opacity: 1 },
    ]);
    expect(fake.animations[0]?.options).toMatchObject({ duration: 500, fill: 'both' });
  });

  it("leaving a slide cuts its endless repeats: each holds its one round's end through the transition", async () => {
    const blast = step('s1.title', {
      kind: 'emphasis',
      fallback: 'none',
      effect: 'behaviors',
      durationMs: 500,
      repeat: 'untilEndOfSlide',
      params: [
        {
          property: 'scale',
          beginMs: 0,
          durationMs: 500,
          frames: [
            { offset: 0, value: { x: 1, y: 1 } },
            { offset: 1, value: { x: 1.1, y: 1.1 } },
          ],
        },
      ],
    } as Partial<DeckStep>);
    deckPage(
      document,
      deck([slide('s1', { timeline: [blast] }), slide('s2', { transition: transition({ effect: 'fade' }) })]),
    );
    const fake = fakeAnimate(false);
    onTestFinished(fake.restore);
    const player = track(mount(document.body, inlineDeck(document)));
    await flush();
    player.next();
    await flush();
    const loop = fake.animations.find((a) => a.options.iterations === Infinity);
    expect(loop?.cancelled, 'it loops while the slide shows').toBe(false);
    expect(player.position).toMatchObject({ slide: 0, step: 1 });
    const before = fake.animations.length;
    player.next();
    await flush();
    expect(player.position).toMatchObject({ slide: 1, step: 0 });
    expect(loop?.cancelled, 'the click that leaves the slide cuts it').toBe(true);
    const held = fake.animations
      .slice(before)
      .filter((a) => a.keyframes.length === 1 && String(a.keyframes[0]?.transform).includes('scale(1.1, 1.1)'));
    expect(held.length, 'it holds its end while the transition plays').toBe(1);
    expect(
      fake.animations.slice(before).some((a) => !a.done && !a.cancelled && a.options.iterations !== Infinity),
    ).toBe(true);
  });

  it('a next during a running build finishes it and does not advance', async () => {
    deckPage(document, deck([slide('s1', { timeline: [step('s1.title')] }), slide('s2')]));
    const fake = fakeAnimate(false);
    onTestFinished(fake.restore);
    const player = track(mount(document.body, inlineDeck(document)));
    await flush();
    player.next();
    await flush();
    expect(fake.animations.map((a) => a.done)).toEqual([false]);
    player.next();
    await flush();
    expect(fake.animations.map((a) => a.done)).toEqual([true]);
    expect(player.position).toMatchObject({ slide: 0, step: 1 });
    player.next();
    await flush();
    expect(player.position).toMatchObject({ slide: 1, step: 0 });
  });

  it('a next once the running build has played to its end goes on to the next step', async () => {
    const timeline = [step('s1.title', { target: { element: 's1.title', paragraphs: [0, 0] } })];
    timeline.push(step('s1.title', { target: { element: 's1.title', paragraphs: [1, 1] } }));
    deckPage(document, deck([slide('s1', { timeline }), slide('s2')]));
    const fake = fakeAnimate(false);
    onTestFinished(fake.restore);
    const player = track(mount(document.body, inlineDeck(document)));
    await flush();
    player.next();
    await flush();
    expect(fake.animations.map((a) => a.done)).toEqual([false]);
    // The build's time is up, and the engine has not yet settled its promise (it does so on its next frame).
    for (const animation of fake.animations) {
      animation.playState = 'finished';
    }
    player.next();
    await flush();
    expect(player.position).toMatchObject({ slide: 0, step: 2 });
    expect(fake.animations.map((a) => a.done)).toEqual([true, false]);
    // While the second build still plays, a next finishes it and stays.
    player.next();
    await flush();
    expect(player.position).toMatchObject({ slide: 0, step: 2 });
    expect(fake.animations.map((a) => a.done)).toEqual([true, true]);
  });

  it('presses once the running build has played to its end go on one step each', async () => {
    const timeline = [0, 1].map((p) => step('s1.title', { target: { element: 's1.title', paragraphs: [p, p] } }));
    timeline.push(step('s1.title'));
    deckPage(document, deck([slide('s1', { timeline }), slide('s2')]));
    const fake = fakeAnimate(false);
    onTestFinished(fake.restore);
    const player = track(mount(document.body, inlineDeck(document)));
    await flush();
    player.next();
    await flush();
    for (const animation of fake.animations) {
      animation.playState = 'finished';
    }
    // Two presses before the engine settles the first build: the second build plays, and the second press finishes it.
    player.next();
    player.next();
    await flush();
    expect(player.position).toMatchObject({ slide: 0, step: 2 });
    expect(fake.animations.map((a) => a.done)).toEqual([true, true]);
    // A press once the second build is shown plays the third.
    player.next();
    await flush();
    expect(player.position).toMatchObject({ slide: 0, step: 3 });
    expect(fake.animations.map((a) => a.done)).toEqual([true, true, false]);
  });

  it("a press at a transition's end finishes the automatic group, as a press during it does", async () => {
    const timeline = [
      step('s2.title', { trigger: 'afterPrevious', target: { element: 's2.title', paragraphs: [0, 0] } }),
      step('s2.title', { target: { element: 's2.title', paragraphs: [1, 1] } }),
    ];
    const pressAt = async (when: 'end' | 'group'): Promise<{ step: number; done: boolean[] }> => {
      // The first deck page's moves wrote its slide into the hash, where the second would start.
      window.location.hash = '';
      deckPage(document, deck([slide('s1'), slide('s2', { transition: transition(), timeline })]));
      const fake = fakeAnimate(false);
      try {
        const player = mount(document.body, inlineDeck(document));
        await flush();
        player.next();
        await flush();
        const transitions = fake.animations.length;
        expect(transitions).toBeGreaterThan(0);
        if (when === 'end') {
          // The transition's time is up, and the engine has not yet settled it.
          for (const animation of fake.animations) {
            animation.playState = 'finished';
          }
        } else {
          for (const animation of fake.animations) {
            animation.finish();
          }
          await flush();
          expect(fake.animations.length).toBeGreaterThan(transitions);
        }
        player.next();
        await flush();
        const result = {
          step: player.position.step,
          done: fake.animations.slice(transitions).map((a) => a.done),
        };
        player.destroy();
        return result;
      } finally {
        fake.restore();
      }
    };
    const group = await pressAt('group');
    expect(group).toEqual({ step: 0, done: [true] });
    expect(await pressAt('end')).toEqual(group);
  });

  it('a press at the end of a transition another press finished goes on', async () => {
    const timeline = [
      step('s2.title', { trigger: 'afterPrevious', target: { element: 's2.title', paragraphs: [0, 0] } }),
      step('s2.title', { target: { element: 's2.title', paragraphs: [1, 1] } }),
    ];
    deckPage(document, deck([slide('s1'), slide('s2', { transition: transition(), timeline })]));
    const fake = fakeAnimate(false);
    onTestFinished(fake.restore);
    const player = track(mount(document.body, inlineDeck(document)));
    await flush();
    player.next();
    await flush();
    const transitions = fake.animations.length;
    // The first press finishes the transition and its automatic group; the second, before they settle, goes on.
    player.next();
    player.next();
    await flush();
    expect(player.position).toMatchObject({ slide: 1, step: 1 });
    expect(fake.animations.slice(transitions).map((a) => a.done)).toEqual([true, false]);
  });

  it('views slides fully built', async () => {
    const timeline = [step('s1.title')];
    deckPage(document, deck([slide('s1', { timeline })]));
    const player = track(createPlayer(document.body, inlineDeck(document)));
    await flush();
    expect(player.position).toMatchObject({ slide: 0, step: 1, presenting: false });
    expect(current()?.shadowRoot?.querySelector('[data-sd-state]')).toBeNull();
  });

  it('a next during a transition finishes it and does not advance', async () => {
    deckPage(document, deck([slide('s1'), slide('s2', { transition: transition() }), slide('s3')]));
    const finished: Animation[] = [];
    const animate = vi.fn(function (this: Element) {
      let resolve: () => void = () => undefined;
      const animation = {
        finished: new Promise<void>((r) => (resolve = r)),
        finish: () => {
          finished.push(animation as unknown as Animation);
          resolve();
        },
        cancel: () => undefined,
      };
      return animation as unknown as Animation;
    });
    const original = Element.prototype.animate;
    Element.prototype.animate = animate as unknown as typeof Element.prototype.animate;
    try {
      const player = track(mount(document.body, inlineDeck(document)));
      await flush();
      player.next();
      await flush();
      // The curve rides on the keyframes, the effect itself linear (WebKit's compositor drops an effect's easing).
      expect(animate).toHaveBeenCalledWith(
        [{ opacity: 0, easing: 'cubic-bezier(0.42, 0, 0.58, 1)' }, { opacity: 1 }],
        expect.objectContaining({ duration: 700, easing: 'linear' }),
      );
      expect(
        layers()
          .find((l) => l.hasAttribute('data-sd-incoming'))
          ?.getAttribute('data-sd-layer'),
      ).toBe('s2');
      player.next();
      await flush();
      expect(finished.length).toBeGreaterThan(0);
      expect(player.position.slide).toBe(1);
      expect(current()?.getAttribute('data-sd-layer')).toBe('s2');
    } finally {
      Element.prototype.animate = original;
    }
  });

  it("a transition fast-forwarded by next still starts the slide's automatic media", async () => {
    const timeline = [step('s2.v', { trigger: 'afterPrevious', kind: 'media', native: 'keynote:movie-start' })];
    const data = deck([slide('s1'), slide('s2', { transition: transition(), timeline })]);
    deckPage(document, data, (id) =>
      slideHtml(id, id === 's2' ? '<video data-el="s2.v" src="media/v.mp4"></video>' : ''),
    );
    const fake = fakeAnimate(false);
    onTestFinished(fake.restore);
    const played: string[] = [];
    const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockImplementation(function (this: HTMLMediaElement) {
      played.push(this.getAttribute('src') ?? '');
      return Promise.resolve();
    });
    onTestFinished(() => play.mockRestore());
    const player = track(mount(document.body, inlineDeck(document)));
    await flush();
    player.next();
    await flush();
    expect(fake.animations.some((animation) => animation.options.duration === 700)).toBe(true);
    player.next();
    await flush();
    expect(player.position.slide).toBe(1);
    expect(played).toEqual(['media/v.mp4']);
  });

  it('a clip that plays across slides plays along, muted, while the transition plays', async () => {
    const start = (id: string) => step(`${id}.v`, { kind: 'media', native: 'keynote:movie-start' });
    deckPage(
      document,
      deck([
        slide('s1', { timeline: [start('s1')] }),
        slide('s2', { transition: transition(), timeline: [start('s2')] }),
      ]),
      (id) => slideHtml(id, `<video data-el="${id}.v" data-across-slides src="media/v.mp4"></video>`),
    );
    const fake = fakeAnimate(false);
    onTestFinished(fake.restore);
    // An engine's clip: it plays once played, and shows the frame it was sought to at once.
    const playing = new WeakSet<HTMLMediaElement>();
    const spies = [
      vi.spyOn(HTMLMediaElement.prototype, 'play').mockImplementation(function (this: HTMLMediaElement) {
        playing.add(this);
        return Promise.resolve();
      }),
      vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(function (this: HTMLMediaElement) {
        playing.delete(this);
      }),
      vi.spyOn(HTMLMediaElement.prototype, 'paused', 'get').mockImplementation(function (this: HTMLMediaElement) {
        return !playing.has(this);
      }),
      vi.spyOn(HTMLMediaElement.prototype, 'readyState', 'get').mockReturnValue(4),
    ];
    onTestFinished(() => spies.forEach((spy) => spy.mockRestore()));
    const clip = (id: string) =>
      layers()
        .find((layer) => layer.getAttribute('data-sd-layer') === id)
        ?.shadowRoot?.querySelector('video');
    const player = track(mount(document.body, inlineDeck(document)));
    await flush();
    player.next();
    await flush();
    for (const cue of fake.animations) {
      cue.finish();
    }
    await flush();
    expect(clip('s1')?.paused, "the outgoing slide's clip plays").toBe(false);
    const before = fake.animations.length;
    player.next();
    await flush();
    expect(fake.animations.slice(before).some((animation) => !animation.done)).toBe(true);
    expect(current()?.getAttribute('data-sd-layer'), 'the transition still plays').toBe('s1');
    expect([clip('s2')?.paused, clip('s2')?.muted], 'the incoming clip plays along, muted').toEqual([false, true]);
  });

  it('reports effects it plays as their fallback, and out-of-contract timing, once each', async () => {
    deckPage(
      document,
      deck([
        slide('s1', {
          timeline: [
            step('s1.title', { effect: 'vortex', delayMs: -1 } as Partial<DeckStep>),
            step('s1.title', { kind: 'exit', effect: 'flyIn', params: { dir: 'up' } } as unknown as Partial<DeckStep>),
          ],
        }),
        slide('s2', { transition: transition({ effect: 'shred' } as unknown as Partial<DeckTransition>) }),
      ]),
    );
    const fake = fakeAnimate(true);
    try {
      const player = track(mount(document.body, inlineDeck(document)));
      const codes: string[] = [];
      player.on('diagnostic', ({ code, message }) => codes.push(`${code} ${message}`));
      await flush();
      for (let i = 0; i < 3; i++) {
        player.next();
        await flush();
      }
      player.first();
      await flush();
      player.next();
      await flush();
      expect(player.position.slide).toBe(0);
      expect(codes).toEqual([
        'RT0006 slide s1 step 0: unknown effect vortex, played dissolve',
        'RT0007 slide s1 step 0: delayMs is out of range',
        'RT0006 slide s1 step 1: invalid effect flyIn, played dissolve',
        'RT0006 slide s2 transition: unknown transition shred, played dissolve',
      ]);
    } finally {
      fake.restore();
    }
  });

  it('fails closed on a malformed transition or step and keeps moving forward', async () => {
    deckPage(
      document,
      deck([
        slide('s1'),
        slide('s2', {
          transition: transition({ fallback: 'cover', effect: 'vortex' } as unknown as Partial<DeckTransition>),
          timeline: [
            { ...step('s2.title'), target: null } as unknown as DeckStep,
            step('s2.title', { trigger: 'onMediaEnd' } as unknown as Partial<DeckStep>),
            step('s2.title', { kind: 'teleport' } as unknown as Partial<DeckStep>),
            step('s2.title', { fallback: 'cover', delivery: 'byLine' } as unknown as Partial<DeckStep>),
          ],
        }),
        slide('s3'),
      ]),
    );
    const fake = fakeAnimate(true);
    const errors: unknown[] = [];
    const onError = (event: ErrorEvent) => errors.push(event.error);
    window.addEventListener('error', onError);
    try {
      const player = track(mount(document.body, inlineDeck(document)));
      const codes: string[] = [];
      player.on('diagnostic', ({ code, message }) => codes.push(`${code} ${message}`));
      await flush();
      player.next();
      await flush();
      expect(player.position).toMatchObject({ slide: 1, step: 0 });
      // Only the step with a default kept its click.
      player.next();
      await flush();
      expect(player.position).toMatchObject({ slide: 1, step: 1 });
      player.next();
      await flush();
      expect(player.position).toMatchObject({ slide: 2, step: 0 });
      expect(errors).toEqual([]);
      expect(codes).toEqual([
        'RT0008 slide s2 step 0: its target names no element; dropped',
        'RT0008 slide s2 step 1: its trigger is not in the contract (an interactive trigger); dropped',
        'RT0008 slide s2 step 2: its kind is not in the contract; dropped',
        'RT0007 slide s2 step 3: its delivery is not in the contract; built whole',
        'RT0007 slide s2 step 3: its fallback is not in the contract; played appear',
        'RT0006 slide s2 transition: unknown transition vortex, unknown fallback, played none',
      ]);
    } finally {
      window.removeEventListener('error', onError);
      fake.restore();
    }
  });

  it('keeps a slide with huge paragraph ranges cheap to open and to build', async () => {
    const timeline = Array.from({ length: 2_000 }, (_, i) =>
      step(`s2.el${i}`, { target: { element: `s2.el${i}`, paragraphs: [0, 9_999] } }),
    );
    deckPage(document, deck([slide('s1'), slide('s2', { timeline })]));
    const fake = fakeAnimate(true);
    try {
      const started = performance.now();
      const player = track(mount(document.body, inlineDeck(document)));
      const codes: string[] = [];
      player.on('diagnostic', ({ code, message }) => codes.push(`${code} ${message}`));
      await flush();
      player.next();
      await flush();
      player.next();
      await flush();
      expect(performance.now() - started).toBeLessThan(2_000);
      expect(player.position).toMatchObject({ slide: 1, step: 1 });
      expect(codes).toContain(
        'RT0008 slide s2 1999 steps from step 1: past the 10000 build units a slide addresses; dropped',
      );
    } finally {
      fake.restore();
    }
  });

  it('drops a move that throws, reports it, and goes on from where it stopped', async () => {
    deckPage(document, deck([slide('s1'), slide('s2'), slide('s3')]));
    const source = inlineDeck(document);
    const broken = {
      ...source,
      slide: (id: string) => {
        if (id === 's2') {
          throw new Error('broken markup');
        }
        return source.slide(id);
      },
    };
    const player = track(createPlayer(document.body, broken, { mode: 'present', keyboard: 'off' }));
    const codes: string[] = [];
    player.on('diagnostic', ({ code, message }) => codes.push(`${code} ${message}`));
    await flush();
    player.next();
    await flush();
    expect(codes).toContain('RT0009 slide s2: the move failed (broken markup)');
    expect(player.position.slide).toBe(1);
    player.next();
    await flush();
    expect(player.position.slide).toBe(2);
  });

  it('reduced motion plays a 150 ms fade', async () => {
    deckPage(document, deck([slide('s1'), slide('s2', { transition: transition({ fallback: 'push' }) })]));
    const animate = vi.fn(() => ({
      finished: Promise.resolve(),
      finish: () => undefined,
      cancel: () => undefined,
    }));
    const original = Element.prototype.animate;
    Element.prototype.animate = animate as unknown as typeof Element.prototype.animate;
    try {
      const player = track(mount(document.body, inlineDeck(document), { reducedMotion: 'always' }));
      await flush();
      player.next();
      await flush();
      expect(animate).toHaveBeenCalledWith(
        [
          { offset: 0, opacity: 0 },
          { offset: 1, opacity: 1 },
        ],
        expect.objectContaining({ duration: 150 }),
      );
      expect(animate).toHaveBeenCalledTimes(1);
    } finally {
      Element.prototype.animate = original;
    }
  });

  it('sanitizes slide markup at mount and reports it', async () => {
    deckPage(document, deck([slide('s1')]), (id) =>
      slideHtml(id, '<script>window.bad = 1</script><img src="media/a.png" onerror="x()">'),
    );
    const player = track(createPlayer(document.body, inlineDeck(document)));
    const diagnostics: string[] = [];
    player.on('diagnostic', (event) => diagnostics.push(event.message));
    await flush();
    const root = current()?.shadowRoot;
    expect(root?.querySelector('script')).toBeNull();
    expect(root?.querySelector('img')?.hasAttribute('onerror')).toBe(false);
    expect(diagnostics).toEqual(['removed from slide s1: script, img@onerror']);
  });

  it('resolves media through the host resolver', async () => {
    const data = deck([slide('s1')]);
    const source = htmlDeck(
      document,
      data,
      new Map([['s1', slideHtml('s1', '<img src="media/ab.png" alt="">')]]),
      (path) => `blob:test/${path}`,
    );
    track(createPlayer(document.body, source));
    await flush();
    expect(current()?.shadowRoot?.querySelector('img')?.getAttribute('src')).toBe('blob:test/media/ab.png');
  });

  it('renders a still slide at a step', () => {
    const timeline = [step('s1.title'), step('s1.title', { kind: 'exit' })];
    const data = deck([slide('s1', { timeline })]);
    const source = htmlDeck(document, data, [{ id: 's1', html: slideHtml('s1') }]);
    const target = document.createElement('div');
    document.body.append(target);
    const player = track(createPlayer(document.createElement('div'), source));
    const title = () => target.querySelector('.sd-layer')?.shadowRoot?.querySelector('[data-el="s1.title"]');
    let dispose = player.renderStatic('s1', 0, target);
    expect(title()?.getAttribute('data-sd-state')).toBe('hidden');
    dispose();
    expect(target.childElementCount).toBe(0);
    dispose = player.renderStatic(0, 1, target, 'width');
    expect(title()?.hasAttribute('data-sd-state')).toBe(false);
    dispose();
    dispose = player.renderStatic(0, 2, target);
    expect(title()?.getAttribute('data-sd-state')).toBe('hidden');
    dispose();
  });

  it('follows a link to another slide', async () => {
    deckPage(document, deck([slide('s1'), slide('s2'), slide('s3')]), (id) =>
      slideHtml(id, '<a href="#slide=s3" id="jump">jump</a>'),
    );
    const player = track(mount(document.body, inlineDeck(document)));
    await flush();
    const link = current()?.shadowRoot?.querySelector<HTMLElement>('#jump');
    link?.dispatchEvent(new MouseEvent('click', { bubbles: true, composed: true, cancelable: true }));
    await flush();
    expect(player.position.slide).toBe(2);
  });

  it('a click in the left third goes back, elsewhere forward', async () => {
    deckPage(document, deck([slide('s1'), slide('s2'), slide('s3')]));
    const player = track(mount(document.body, inlineDeck(document)));
    await flush();
    const wrapper = document.querySelector<HTMLElement>('.sd-player');
    if (wrapper === null) {
      throw new Error('no player');
    }
    wrapper.getBoundingClientRect = () => ({ left: 0, top: 0, width: 900, height: 500 }) as DOMRect;
    wrapper.dispatchEvent(new MouseEvent('click', { bubbles: true, clientX: 600 }));
    await flush();
    expect(player.position.slide).toBe(1);
    wrapper.dispatchEvent(new MouseEvent('click', { bubbles: true, clientX: 100 }));
    await flush();
    expect(player.position.slide).toBe(0);
  });

  it('follows another window: forward animates as next, anything else cuts', async () => {
    deckPage(document, deck([slide('s1', { timeline: [step('s1.title')] }), slide('s2'), slide('s3')]));
    const player = track(mount(document.body, inlineDeck(document)));
    await flush();
    const motions: string[] = [];
    player.on('position', (event) => motions.push(event.motion));
    player.follow({ slide: 0, step: 1, blank: null }, true);
    player.follow({ slide: 2, step: 0, blank: 'black' }, true);
    player.follow({ slide: 2, step: 0, blank: 'white' }, true);
    player.follow({ slide: 2, step: 0, blank: 'white' }, true);
    await flush();
    expect(motions).toEqual(['build', 'cut', 'none']);
    expect(player.position).toMatchObject({ slide: 2, step: 0, blank: 'white' });
  });

  it('a move during a running build leaves the slide in the state of its position', async () => {
    const timeline = [step('s1.title'), step('s1.title', { kind: 'exit' })];
    deckPage(document, deck([slide('s1', { timeline }), slide('s2')]));
    const fake = fakeAnimate(false);
    onTestFinished(fake.restore);
    const player = track(mount(document.body, inlineDeck(document)));
    await flush();
    const state = () => current()?.shadowRoot?.querySelector('[data-el="s1.title"]')?.getAttribute('data-sd-state');
    player.next();
    await flush();
    player.next();
    await flush();
    player.next();
    await flush();
    expect(player.position.step).toBe(2);
    expect(state()).toBeNull();
    // The exit is running; blanking fast-forwards it.
    player.blank('black');
    await flush();
    expect(state()).toBe('hidden');
    player.blank(null);
    player.previous();
    await flush();
    expect(player.position.step).toBe(1);
    expect(state()).toBeNull();
  });
});

describe('presenter view', () => {
  it('mounts on #presenter with notes, counter and the next preview, in the window frame', async () => {
    window.location.hash = '#presenter=abc';
    deckPage(
      document,
      deck([slide('s1', { notes: ['Say hello', 'Then this'] }), slide('s2', { hidden: true }), slide('s3')], {
        title: 'Talk',
      }),
    );
    const player = track(mount(document.body, inlineDeck(document), { presenter: presenterView }));
    await flush();
    const frame = document.querySelector('.sd-pw');
    expect(frame?.querySelector('.sd-pw-title')?.textContent).toBe('slidock');
    expect(frame?.querySelector('.sd-pw-subtitle')?.textContent).toBe('Presenter view · Talk');
    expect(frame?.querySelector('.sd-pw-theme')?.getAttribute('aria-label')).toBe(
      'System theme. Switch to light theme',
    );
    const view = frame?.querySelector('main.sd-pw-main > .sd-presenter');
    expect(view).not.toBeNull();
    expect(document.title).toBe('Presenter · Talk');
    expect(Array.from(view?.querySelectorAll('.sd-notes p') ?? []).map((p) => p.textContent)).toEqual([
      'Say hello',
      'Then this',
    ]);
    expect(view?.querySelector('.sd-time')?.textContent).toBe('Slide 1 of 2');
    const preview = () => view?.querySelector('.sd-next .sd-layer')?.getAttribute('data-sd-layer');
    expect(preview()).toBe('s3');
    player.next();
    await flush();
    expect(view?.querySelector('.sd-time')?.textContent).toBe('Slide 2 of 2');
    expect(view?.querySelector('.sd-next')?.textContent).toContain('End of presentation');
    player.destroy();
    players = [];
    expect(document.querySelector('.sd-presenter')).toBeNull();
    expect(document.querySelector('.sd-pw')).toBeNull();
  });

  it('P opens the presenter window on the same page with the session, and help lists it', async () => {
    deckPage(document, deck([slide('s1')]));
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    const player = track(mount(document.body, inlineDeck(document), { presenter: presenterView }));
    const diagnostics: string[] = [];
    player.on('diagnostic', (event) => diagnostics.push(event.code));
    expect(key('p').defaultPrevented).toBe(true);
    expect(open).toHaveBeenCalledTimes(1);
    const [url] = open.mock.calls[0] ?? [];
    expect(String(url)).toMatch(/#presenter=[\w-]+$/);
    expect(diagnostics).toEqual([RT.presenterBlocked]);
    expect(Array.from(document.querySelectorAll('.sd-help dt'), (dt) => dt.textContent)).toContain('P');
    open.mockRestore();
  });

  it('without the presenter view: #presenter shows the deck, P is not its key, and help does not list it', async () => {
    window.location.hash = '#presenter=abc';
    deckPage(document, deck([slide('s1', { notes: ['Private'] }), slide('s2')]));
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    onTestFinished(() => open.mockRestore());
    const player = track(mount(document.body, inlineDeck(document)));
    await flush();
    expect(document.querySelector('.sd-presenter, .sd-pw')).toBeNull();
    expect(document.querySelector('.sd-player.sd-page')).not.toBeNull();
    expect(current()?.getAttribute('data-sd-layer')).toBe('s1');
    expect(key('p').defaultPrevented).toBe(false);
    expect(player.openPresenter()).toBeNull();
    expect(open).not.toHaveBeenCalled();
    expect(Array.from(document.querySelectorAll('.sd-help dt'), (dt) => dt.textContent)).not.toContain('P');
    key('ArrowRight');
    await flush();
    expect(current()?.getAttribute('data-sd-layer')).toBe('s2');
  });

  it('keeps presenting when the browser leaves full screen for the presenter window', async () => {
    // happy-dom has no Fullscreen API: a full screen the browser can leave by itself, entered at once or when the
    // test says.
    let fullscreen: Element | null = null;
    let deferred = false;
    let enter: () => void = () => undefined;
    Object.defineProperty(document, 'fullscreenElement', { configurable: true, get: () => fullscreen });
    Object.defineProperty(Element.prototype, 'requestFullscreen', {
      configurable: true,
      value: () =>
        new Promise<void>((resolve) => {
          enter = () => {
            fullscreen = document.querySelector('.sd-player');
            resolve();
          };
          if (!deferred) {
            enter();
          }
        }),
    });
    onTestFinished(() => {
      Reflect.deleteProperty(document, 'fullscreenElement');
      Reflect.deleteProperty(Element.prototype, 'requestFullscreen');
    });
    const leave = () => {
      fullscreen = null;
      document.dispatchEvent(new Event('fullscreenchange'));
    };
    deckPage(document, deck([slide('s1'), slide('s2')]));
    const player = track(createPlayer(document.body, inlineDeck(document), { mode: 'view', presenter: presenterView }));
    await flush();

    // Leaving full screen otherwise ends the show.
    await player.present();
    expect(fullscreen).not.toBeNull();
    leave();
    expect(player.position.presenting).toBe(false);

    const open = vi.spyOn(window, 'open').mockReturnValue({ closed: false } as Window);
    onTestFinished(() => open.mockRestore());
    await player.present();
    player.openPresenter();
    leave();
    expect(player.position.presenting).toBe(true);
    expect(document.querySelector('.sd-player')?.classList.contains('sd-fill')).toBe(true);
    player.stop();
    expect(player.position.presenting).toBe(false);

    // The presenter window opens while the full screen request still runs.
    deferred = true;
    const presenting = player.present();
    player.openPresenter();
    enter();
    await presenting;
    leave();
    expect(player.position.presenting).toBe(true);
    expect(document.querySelector('.sd-player')?.classList.contains('sd-fill')).toBe(true);
  });
});

describe('present and full screen', () => {
  /** A Fullscreen API that answers each request when the test says: it enters full screen, or refuses. */
  function fakeFullscreen() {
    let fullscreen: Element | null = null;
    const pending: { element: Element; resolve: () => void; reject: (error: Error) => void }[] = [];
    const change = () => document.dispatchEvent(new Event('fullscreenchange'));
    Object.defineProperty(document, 'fullscreenElement', { configurable: true, get: () => fullscreen });
    Object.defineProperty(document, 'exitFullscreen', {
      configurable: true,
      value: () => {
        if (fullscreen === null) {
          return Promise.reject(new TypeError('not in full screen'));
        }
        fullscreen = null;
        change();
        return Promise.resolve();
      },
    });
    Object.defineProperty(Element.prototype, 'requestFullscreen', {
      configurable: true,
      value(this: Element) {
        return new Promise<void>((resolve, reject) => pending.push({ element: this, resolve, reject }));
      },
    });
    onTestFinished(() => {
      Reflect.deleteProperty(document, 'fullscreenElement');
      Reflect.deleteProperty(document, 'exitFullscreen');
      Reflect.deleteProperty(Element.prototype, 'requestFullscreen');
    });
    return {
      get fullscreen() {
        return fullscreen;
      },
      get requests() {
        return pending.length;
      },
      /** Answers the oldest request still pending. */
      answer: async (enter: boolean) => {
        const request = pending.shift();
        if (request === undefined) {
          throw new Error('no request is pending');
        }
        if (enter) {
          fullscreen = request.element;
          change();
          request.resolve();
        } else {
          request.reject(new TypeError('refused'));
        }
        await flush();
      },
    };
  }

  const fill = () => document.querySelector('.sd-player')?.classList.contains('sd-fill') ?? false;

  async function viewing() {
    deckPage(document, deck([slide('s1'), slide('s2'), slide('s3')]));
    const player = track(createPlayer(document.body, inlineDeck(document), { mode: 'view' }));
    await flush();
    return player;
  }

  it('plays full screen once the browser enters it, and filling the window when it refuses', async () => {
    const screen = fakeFullscreen();
    const player = await viewing();
    const entered = player.present();
    expect(player.position.presenting).toBe(true);
    await screen.answer(true);
    await entered;
    expect(screen.fullscreen).not.toBeNull();
    expect(fill()).toBe(false);
    key('Escape');
    expect(player.position.presenting).toBe(false);
    expect(screen.fullscreen).toBeNull();

    const refused = player.present();
    await screen.answer(false);
    await refused;
    expect(player.position.presenting).toBe(true);
    expect(fill()).toBe(true);
    key('Escape');
    expect(player.position.presenting).toBe(false);
    expect(fill()).toBe(false);
  });

  for (const enter of [true, false]) {
    it(`a stop while the request is pending leaves the page as before present(), the browser ${
      enter ? 'entering full screen' : 'refusing'
    } after`, async () => {
      const screen = fakeFullscreen();
      const player = await viewing();
      const before = player.position;
      const presented = player.present();
      key('Escape');
      expect(player.position).toEqual(before);
      await screen.answer(enter);
      await presented;
      expect(screen.fullscreen).toBeNull();
      expect(fill()).toBe(false);
      expect(player.position).toEqual(before);

      // Two present/stop pairs ask once, and leave nothing either.
      const first = player.present();
      player.stop();
      const second = player.present();
      player.stop();
      expect(screen.requests).toBe(1);
      await screen.answer(enter);
      await Promise.all([first, second]);
      expect(screen.fullscreen).toBeNull();
      expect(fill()).toBe(false);
      expect(player.position).toEqual(before);

      player.next();
      expect(player.position.slide).toBe(before.slide + 1);
    });
  }

  it('a present after a stop while the request is pending takes it over', async () => {
    const screen = fakeFullscreen();
    const player = await viewing();
    const first = player.present();
    player.stop();
    const second = player.present();
    expect(screen.requests).toBe(1);
    await screen.answer(true);
    await Promise.all([first, second]);
    expect(player.position.presenting).toBe(true);
    expect(screen.fullscreen).not.toBeNull();
    expect(fill()).toBe(false);
  });

  it('a player destroyed while the request is pending leaves full screen when the browser enters it', async () => {
    const screen = fakeFullscreen();
    const player = await viewing();
    const presented = player.present();
    player.destroy();
    await screen.answer(true);
    await presented;
    expect(screen.fullscreen).toBeNull();
  });
});

describe('presenter sync', () => {
  const settle = async () => {
    for (let i = 0; i < 4; i++) {
      await new Promise((resolve) => setTimeout(resolve, 15));
    }
  };

  /** A deck window's player and a presenter-view player on one deck, joined over the deck's BroadcastChannel. */
  async function pair(data: Deck, linked = true): Promise<{ deck: Player; presenter: Player }> {
    deckPage(document, data);
    const source = inlineDeck(document);
    const host = () => document.body.appendChild(document.createElement('div'));
    const deckPlayer = track(
      createPlayer(host(), source, {
        mode: 'present',
        keyboard: 'off',
        sync: { role: 'deck' },
        ...(linked ? { presenter: presenterView } : {}),
      }),
    );
    const presenter = track(
      createPlayer(host(), source, {
        mode: 'present',
        keyboard: 'off',
        muted: true,
        sync: { role: 'presenter', session: '' },
        ...(linked ? { presenter: presenterView } : {}),
      }),
    );
    await settle();
    return { deck: deckPlayer, presenter };
  }

  const at = (player: Player) => ({
    slide: player.position.slide,
    step: player.position.step,
    blank: player.position.blank,
  });

  it('sends every local move, also one back to the state last received', async () => {
    const { deck: deckPlayer, presenter } = await pair(deck(['a', 'b', 'c', 'd', 'e'].map((id) => slide(id))));
    presenter.goTo(3);
    await settle();
    expect(at(deckPlayer)).toEqual({ slide: 3, step: 0, blank: null });
    deckPlayer.previous();
    await settle();
    expect(at(presenter).slide).toBe(2);
    deckPlayer.next();
    await settle();
    expect(at(presenter).slide).toBe(3);
    presenter.blank('black');
    await settle();
    expect(at(deckPlayer).blank).toBe('black');
    deckPlayer.blank(null);
    await settle();
    expect(at(presenter).blank).toBeNull();
    deckPlayer.blank('black');
    await settle();
    expect(at(presenter).blank).toBe('black');
  });

  it('players without the presenter view join no session', async () => {
    const { deck: deckPlayer, presenter } = await pair(deck(['a', 'b', 'c'].map((id) => slide(id))), false);
    presenter.goTo(2);
    await settle();
    expect(at(deckPlayer).slide).toBe(0);
    deckPlayer.next();
    await settle();
    expect(at(presenter).slide).toBe(2);
  });

  it('a presenter view that adopts the session starts where the deck window is, and keeps it there', async () => {
    deckPage(document, deck(['a', 'b', 'c', 'd'].map((id) => slide(id))));
    const source = inlineDeck(document);
    const deckPlayer = track(
      createPlayer(document.body.appendChild(document.createElement('div')), source, {
        mode: 'present',
        keyboard: 'off',
        sync: { role: 'deck' },
        presenter: presenterView,
      }),
    );
    deckPlayer.goTo(2);
    await settle();
    const host = document.body.appendChild(document.createElement('div'));
    const view = mountPresenter(host, source, { session: '', startAt: { slide: 2 } });
    onTestFinished(() => view.destroy());
    await settle();
    expect(at(view.player)).toEqual({ slide: 2, step: 0, blank: null });
    expect(at(deckPlayer)).toEqual({ slide: 2, step: 0, blank: null });
    view.player.next();
    await settle();
    expect(at(deckPlayer).slide).toBe(3);
  });

  it('the presenter view keeps presenting; a deck window that stops keeps its moves to itself', async () => {
    const timeline = [step('a.title'), step('a.title', { kind: 'exit' })];
    const { deck: deckPlayer, presenter } = await pair(
      deck([slide('a', { timeline }), slide('b', { hidden: true }), slide('c')]),
    );
    presenter.stop();
    await settle();
    expect(presenter.position).toMatchObject({ slide: 0, step: 0, presenting: true });
    expect(deckPlayer.position).toMatchObject({ slide: 0, step: 0, presenting: true });
    deckPlayer.stop();
    await settle();
    expect(deckPlayer.position).toMatchObject({ slide: 0, step: 2, presenting: false });
    deckPlayer.next();
    await settle();
    expect(deckPlayer.position.slide).toBe(1);
    expect(at(presenter)).toEqual({ slide: 0, step: 0, blank: null });
    // The presenter drives on, through shown slides only; presenting again sends the deck window's position.
    presenter.next();
    await settle();
    expect(at(presenter)).toEqual({ slide: 0, step: 1, blank: null });
    await deckPlayer.present();
    await settle();
    expect(deckPlayer.position).toMatchObject({ slide: 0, step: 0, presenting: true });
    expect(at(presenter)).toEqual({ slide: 0, step: 0, blank: null });
  });
});

describe('media', () => {
  const video = '<video data-el="v" data-autoplay="" src="media/v.mp4"></video>';
  const muted = (host: ParentNode) => host.querySelector('.sd-layer')?.shadowRoot?.querySelector('video')?.muted;

  it('a muted player plays media muted; another with their sound', async () => {
    const source = htmlDeck(document, deck([slide('s1')]), new Map([['s1', slideHtml('s1', video)]]));
    const mutedHost = document.body.appendChild(document.createElement('div'));
    const soundHost = document.body.appendChild(document.createElement('div'));
    track(createPlayer(mutedHost, source, { mode: 'present', keyboard: 'off', muted: true }));
    track(createPlayer(soundHost, source, { mode: 'present', keyboard: 'off' }));
    await flush();
    expect(muted(mutedHost)).toBe(true);
    expect(muted(soundHost)).toBe(false);
  });

  it("the presenter view is muted: the sound is the deck window's", async () => {
    window.location.hash = '#presenter=abc';
    deckPage(document, deck([slide('s1')]), (id) => slideHtml(id, video));
    track(mount(document.body, inlineDeck(document), { presenter: presenterView }));
    await flush();
    expect(document.querySelector('.sd-current')).not.toBeNull();
    expect(muted(document.querySelector('.sd-current') ?? document)).toBe(true);
  });
});

describe('keyboard: host', () => {
  it('reads keys aimed at the player, and every key while it presents', async () => {
    const data = deck([slide('s1'), slide('s2'), slide('s3'), slide('s4')]);
    const source = htmlDeck(document, data, [{ id: 's1', html: slideHtml('s1') }]);
    const stage = document.body.appendChild(document.createElement('div'));
    const player = track(createPlayer(stage, source, { mode: 'view', keyboard: 'host', chrome: false }));
    const wrapper = stage.querySelector<HTMLElement>('.sd-player');
    await flush();
    key('ArrowRight');
    await flush();
    expect(player.position.slide).toBe(0);
    key('ArrowRight', wrapper ?? document);
    await flush();
    expect(player.position.slide).toBe(1);
    await player.present();
    expect(document.activeElement).toBe(wrapper);
    wrapper?.blur();
    key('ArrowRight');
    await flush();
    expect(player.position).toMatchObject({ slide: 2, presenting: true });
    // A key aimed at the player while it presents acts once: B blanks rather than toggling twice.
    key('b', wrapper ?? document);
    expect(player.position.blank).toBe('black');
  });
});

describe('help overlay', () => {
  it('is modal: it takes focus, the deck behind does not move, and focus goes back on close', async () => {
    deckPage(document, deck([slide('s1'), slide('s2')]));
    const player = track(mount(document.body, inlineDeck(document)));
    const button = document.body.appendChild(document.createElement('button'));
    button.focus();
    await flush();
    const help = document.querySelector<HTMLElement>('.sd-help');
    key('?');
    expect(help?.hidden).toBe(false);
    expect(document.activeElement).toBe(help);
    key('ArrowRight');
    key(' ');
    await flush();
    expect(player.position.slide).toBe(0);
    key('Escape');
    expect(help?.hidden).toBe(true);
    expect(document.activeElement).toBe(button);
    expect(player.position.presenting).toBe(true);
    key('ArrowRight');
    await flush();
    expect(player.position.slide).toBe(1);
  });
});
