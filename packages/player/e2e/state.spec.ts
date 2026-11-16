// State is a function of (slide, step): a fixture deck with several kinds of steps is played forward, cut back and
// mounted at a step, and every settled state must equal `renderStatic` of that step pixel for pixel. Reduced motion
// turns builds into appear and transitions into a 150 ms fade.
import { type Page, expect, test } from '@playwright/test';
import type { DeckStep } from '@slidock/types';

import { deckSlide, fixture, step, transition } from './support/deck';
import { type Harness, diff, disallowedKeys, open, pauseOnStart, settle, shot, staticShot } from './support/harness';

const fields = (f: Record<string, unknown>) => f as Partial<DeckStep>;

const TIMELINE: DeckStep[] = [
  step(
    's1.box',
    fields({ trigger: 'afterPrevious', effect: 'flyIn', params: { dir: 'u', distance: 'offSlide' }, durationMs: 400 }),
  ),
  step(
    's1.text',
    fields({
      delivery: 'byWord',
      effect: 'fade',
      stagger: { perUnit: { ms: 60 }, order: 'fromCenter' },
      durationMs: 300,
    }),
  ),
  step(
    's1.box',
    fields({
      kind: 'emphasis',
      fallback: 'none',
      effect: 'spin',
      params: { deg: 45, cw: true, hold: true },
      durationMs: 300,
    }),
  ),
  step(
    's1.rot',
    fields({
      trigger: 'withPrevious',
      kind: 'emphasis',
      fallback: 'none',
      effect: 'growShrink',
      params: { scale: { x: 1.3, y: 1.3 }, hold: true },
      repeat: { count: 1.5 },
      durationMs: 200,
    }),
  ),
  step(
    's1.text',
    fields({
      kind: 'emphasis',
      fallback: 'none',
      effect: 'pulse',
      params: { scale: 1.1 },
      after: { dim: '#9ca3af' },
      durationMs: 200,
    }),
  ),
  step('s1.box', fields({ kind: 'exit', effect: 'wipe', params: { dir: 'r', soft: false }, durationMs: 300 })),
  step(
    's1.rot',
    fields({
      kind: 'motion',
      fallback: 'none',
      effect: 'path',
      params: {
        d: [
          ['M', 0, 0],
          ['L', -200, -60],
        ],
        autoRotate: false,
        hold: true,
      },
      durationMs: 300,
    }),
  ),
];

const CLICKS = TIMELINE.filter((s) => s.trigger === 'click').length;

test('every settled step equals renderStatic, forward, back and mounted', async ({ page }) => {
  const data = fixture([deckSlide('s1', { timeline: TIMELINE }), deckSlide('s2')]);
  await open(page, data);
  const statics: string[] = [];
  for (let s = 0; s <= CLICKS; s++) {
    statics.push(await staticShot(page, 0, s));
  }
  // Each step changes something.
  for (let s = 1; s <= CLICKS; s++) {
    expect(
      await diff(page, statics[s - 1] as string, statics[s] as string),
      `step ${s} changes the slide`,
    ).toBeGreaterThan(0);
  }
  expect(await diff(page, await shot(page), statics[0] as string), 'mounted at step 0').toBe(0);
  for (let s = 1; s <= CLICKS; s++) {
    await page.evaluate(() => (window as unknown as Harness).sd.player.next());
    await settle(page);
    expect(await page.evaluate(() => (window as unknown as Harness).sd.player.position.step)).toBe(s);
    expect(await diff(page, await shot(page), statics[s] as string), `played to step ${s}`).toBe(0);
  }
  expect(await disallowedKeys(page)).toEqual([]);
  for (let s = CLICKS - 1; s >= 0; s--) {
    await page.evaluate(() => (window as unknown as Harness).sd.player.previous());
    await settle(page);
    expect(await diff(page, await shot(page), statics[s] as string), `cut back to step ${s}`).toBe(0);
  }
  for (const s of [3, 1, 5]) {
    await page.evaluate((target) => (window as unknown as Harness).sd.player.goTo(0, target), s);
    await settle(page);
    expect(await diff(page, await shot(page), statics[s] as string), `jumped to step ${s}`).toBe(0);
  }
  // Leaving the slide and coming back to its last step fast-forwards everything.
  await page.evaluate(() => (window as unknown as Harness).sd.player.goTo(1));
  await settle(page);
  await page.evaluate(() => (window as unknown as Harness).sd.player.previous());
  await settle(page);
  expect(await diff(page, await shot(page), statics[CLICKS] as string), 'came back fully built').toBe(0);
});

test('a player started at a step shows that step', async ({ page }) => {
  const data = fixture([deckSlide('s1', { timeline: TIMELINE })]);
  await open(page, data, { startAt: { slide: 0, step: 4 } });
  const expected = await staticShot(page, 0, 4);
  expect(await diff(page, await shot(page), expected)).toBe(0);
});

test('a click while a group plays finishes it, then the next click advances', async ({ page }) => {
  const data = fixture([deckSlide('s1', { timeline: TIMELINE })]);
  await open(page, data);
  const two = await staticShot(page, 0, 2);
  await page.evaluate(() => (window as unknown as Harness).sd.player.next());
  await page.evaluate(() => (window as unknown as Harness).sd.player.next());
  await page.evaluate(() => (window as unknown as Harness).sd.player.next());
  await settle(page);
  expect(await page.evaluate(() => (window as unknown as Harness).sd.player.position.step)).toBe(2);
  expect(await diff(page, await shot(page), two)).toBe(0);
});

test('a click once a group has played to its end advances, before the engine settles it', async ({ page }) => {
  const data = fixture([deckSlide('s1', { timeline: TIMELINE })]);
  await open(page, data);
  const two = await staticShot(page, 0, 2);
  // An engine that settles an ended animation's promise late (a loaded WebKit can take a while): a second after it
  // ends, or at once when it is finished.
  await page.evaluate(() => {
    const animate = Element.prototype.animate;
    Element.prototype.animate = function late(this: Element, ...args: Parameters<Element['animate']>) {
      const animation = animate.apply(this, args);
      const finish = animation.finish.bind(animation);
      let release = () => {};
      const told = new Promise<void>((resolve) => (release = resolve));
      const finished = animation.finished.then((value) =>
        Promise.race([told, new Promise((resolve) => setTimeout(resolve, 1000))]).then(() => value),
      );
      Object.defineProperty(animation, 'finished', { get: () => finished });
      animation.finish = () => {
        finish();
        release();
      };
      return animation;
    };
  });
  await page.evaluate(() => (window as unknown as Harness).sd.player.next());
  await settle(page);
  const step = () => page.evaluate(() => (window as unknown as Harness).sd.player.position.step);
  expect(await step()).toBe(1);
  await page.evaluate(() => (window as unknown as Harness).sd.player.next());
  await expect.poll(step, { timeout: 500 }).toBe(2);
  await page.waitForTimeout(1200);
  await settle(page);
  expect(await step()).toBe(2);
  expect(await diff(page, await shot(page), two)).toBe(0);
});

for (const how of ['option', 'media query'] as const) {
  test(`reduced motion (${how}): builds appear, transitions are a 150 ms fade`, async ({ page }) => {
    if (how === 'media query') {
      await page.emulateMedia({ reducedMotion: 'reduce' });
    }
    const data = fixture([
      deckSlide('s1', { timeline: TIMELINE.slice(0, 3) }),
      deckSlide('s2', {
        transition: transition({ effect: 'cube', params: { dir: 'l' }, fallback: 'push', durationMs: 2000 }),
      }),
    ]);
    await open(page, data, how === 'option' ? { reducedMotion: 'always' } : {});
    const timings = await page.evaluate(() => {
      const { sd } = window as unknown as Harness;
      sd.player.next();
      return document
        .querySelector('.sd-layer[data-sd-current]')
        ?.shadowRoot?.getAnimations()
        .map((a) => (a.effect as KeyframeEffect).getTiming().duration);
    });
    expect(timings?.every((d) => d === 0)).toBe(true);
    await settle(page);
    await page.evaluate(() => (window as unknown as Harness).sd.player.next());
    await settle(page);
    await page.evaluate(() => (window as unknown as Harness).sd.player.next());
    // Paused as it starts: on a loaded machine the 150 ms fade could otherwise end before it is read.
    await pauseOnStart(page);
    const played = await page.evaluate(() =>
      document.getAnimations().map((a) => ({
        duration: (a.effect as KeyframeEffect).getTiming().duration,
        keys: (a.effect as KeyframeEffect).getKeyframes().flatMap((k) => Object.keys(k)),
      })),
    );
    expect(played.length).toBe(1);
    expect(played[0]?.duration).toBe(150);
    expect(played[0]?.keys).toContain('opacity');
    expect(played[0]?.keys).not.toContain('transform');
    await page.evaluate(() => (window as unknown as Harness).sd.resume());
    await settle(page);
    const incoming = await staticShot(page, 1, 0);
    expect(await diff(page, await shot(page), incoming)).toBe(0);
  });
}

test('held paths settle on the still render after a group is paused partway', async ({ page }) => {
  // Two paths that move shapes by translations alone, across each other and the text, each holding where it ends.
  const path = (element: string, x: number, y: number) =>
    step(
      element,
      fields({
        kind: 'motion',
        fallback: 'none',
        effect: 'path',
        params: {
          d: [
            ['M', 0, 0],
            ['L', x, y],
          ],
          autoRotate: false,
          hold: true,
        },
        durationMs: 300,
      }),
    );
  const timeline = [path('s1.box', 320, 180), path('s1.rot', -320, -180), path('s1.text', 200, 300)];
  await open(page, fixture([deckSlide('s1', { timeline })]));
  for (let at = 1; at <= timeline.length; at++) {
    await page.evaluate(() => {
      const { sd } = window as unknown as Harness;
      sd.player.next();
      sd.pause();
    });
    for (const fraction of [0, 0.5, 1]) {
      await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), fraction);
      await shot(page);
    }
    await page.evaluate(() => (window as unknown as Harness).sd.resume());
    await settle(page);
    const settled = await shot(page);
    expect(await diff(page, settled, await staticShot(page, 0, at)), `step ${at}`).toBe(0);
  }
});

test('an appear shown for a while (Flash Once) shows its target from its start and hides it when it ends', async ({
  page,
}) => {
  const timeline = [step('s1.box', fields({ effect: 'appear', fallback: 'appear', durationMs: 600, after: 'hide' }))];
  await open(page, fixture([deckSlide('s1', { timeline })]));
  const before = await staticShot(page, 0, 0);
  const after = await staticShot(page, 0, 1);
  expect(await diff(page, before, after), 'the step leaves its target hidden').toBe(0);
  await page.evaluate(() => {
    const { sd } = window as unknown as Harness;
    sd.player.next();
    sd.pause();
  });
  const opacity = () =>
    page.evaluate(() => {
      const { sd } = window as unknown as Harness;
      const el = sd.target('s1.box');
      return el === null ? '' : `${getComputedStyle(el).visibility} ${getComputedStyle(el).opacity}`;
    });
  // The group's span is the 600 ms the target shows.
  for (const [fraction, expected] of [
    [0, 'visible 1'],
    [0.5, 'visible 1'],
    [0.99, 'visible 1'],
    [1, 'visible 0'],
  ] as const) {
    await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), fraction);
    expect(await opacity(), `at ${fraction}`).toBe(expected);
  }
  await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), 0.5);
  expect(await diff(page, await shot(page), before), 'shown while it lasts').toBeGreaterThan(0);
  await page.evaluate(() => (window as unknown as Harness).sd.resume());
  await settle(page);
  expect(await diff(page, await shot(page), after), 'settled = renderStatic').toBe(0);
});

/** LibreOffice's blast on `s1.box`: a scale to 1.1 over the whole 500 ms step, held. */
const blast = (repeat: unknown) =>
  step(
    's1.box',
    fields({
      kind: 'emphasis',
      fallback: 'none',
      effect: 'behaviors',
      durationMs: 500,
      repeat,
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
    }),
  );

/** The scale `s1.box` shows on slide layer `s1`, current or outgoing. */
const boxScale = (page: Page) =>
  page.evaluate(() => {
    const layer = document.querySelector('.sd-layer[data-sd-layer="s1"]');
    const el = [...(layer?.shadowRoot?.querySelectorAll('[data-el]') ?? [])].find(
      (e) => e.getAttribute('data-el') === 's1.box',
    );
    return el === undefined ? Number.NaN : new DOMMatrix(getComputedStyle(el).transform).a;
  });

test('a held tier-2 scale that repeats three times settles at its held value', async ({ page }) => {
  const timeline = [blast({ count: 3 })];
  await open(page, fixture([deckSlide('s1', { timeline })]));
  const after = await staticShot(page, 0, 1);
  const transform = () =>
    page.evaluate(() => {
      const el = (window as unknown as Harness).sd.target('s1.box');
      return el === null ? '' : getComputedStyle(el).transform;
    });
  await page.evaluate(() => {
    const { sd } = window as unknown as Harness;
    sd.player.next();
    sd.pause();
  });
  await page.evaluate(() => (window as unknown as Harness).sd.seek(1));
  const last = await shot(page);
  const end = await transform();
  expect(end).toBe('matrix(1.1, 0, 0, 1.1, 0, 0)');
  await page.evaluate(() => (window as unknown as Harness).sd.resume());
  await settle(page);
  expect(await transform(), 'settles where it ends').toBe(end);
  expect(await diff(page, await shot(page), after), 'settled = renderStatic').toBe(0);
  expect(await diff(page, last, after, 96), 'last frame = renderStatic').toBeLessThanOrEqual(256);
});

/** Plays the next click paused, checks the loop's rounds at `ms`, then lets it run: it is still looping later. */
async function loops(page: Page): Promise<void> {
  await page.evaluate(() => {
    const { sd } = window as unknown as Harness;
    sd.player.next();
    sd.pause();
  });
  // Rounds of 500 ms: the middle of the first, of the second, and just into the third.
  for (const [ms, expected] of [
    [250, 1.05],
    [750, 1.05],
    [1050, 1.01],
  ] as const) {
    await page.evaluate((at) => (window as unknown as Harness).sd.seekMs(at), ms);
    expect(await boxScale(page), `at ${ms} ms`).toBeCloseTo(expected, 3);
  }
  await page.evaluate(() => (window as unknown as Harness).sd.resume());
  await settle(page);
  const seen = new Set<number>();
  for (let i = 0; i < 8; i++) {
    await page.waitForTimeout(70);
    seen.add(Math.round((await boxScale(page)) * 1000));
  }
  expect(seen.size, 'it loops while the slide shows').toBeGreaterThan(2);
}

test('an endless tier-2 repeat until the next click loops, and the click cuts it to its held end', async ({ page }) => {
  const timeline = [
    blast('untilNextClick'),
    step(
      's1.text',
      fields({ kind: 'emphasis', fallback: 'none', effect: 'spin', params: { deg: 10, cw: true, hold: true } }),
    ),
  ];
  await open(page, fixture([deckSlide('s1', { timeline })]));
  const after = await staticShot(page, 0, 2);
  await loops(page);
  await page.evaluate(() => {
    const { sd } = window as unknown as Harness;
    sd.player.next();
    sd.pause();
  });
  for (const fraction of [0, 0.5, 1]) {
    await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), fraction);
    expect(await boxScale(page), `cut: held at ${fraction} of the next group`).toBeCloseTo(1.1, 6);
  }
  await page.evaluate(() => (window as unknown as Harness).sd.resume());
  await settle(page);
  expect(await boxScale(page)).toBeCloseTo(1.1, 6);
  expect(await diff(page, await shot(page), after), 'settled = renderStatic').toBe(0);
});

test('an endless tier-2 repeat until the end of the slide loops through the next click, and leaving cuts it', async ({
  page,
}) => {
  const timeline = [
    blast('untilEndOfSlide'),
    step(
      's1.text',
      fields({ kind: 'emphasis', fallback: 'none', effect: 'spin', params: { deg: 10, cw: true, hold: true } }),
    ),
  ];
  await open(
    page,
    fixture([
      deckSlide('s1', { timeline }),
      deckSlide('s2', { transition: transition({ effect: 'fade', params: { through: 'none' }, durationMs: 1000 }) }),
    ]),
  );
  const after = await staticShot(page, 0, 2);
  await loops(page);
  await page.evaluate(() => (window as unknown as Harness).sd.player.next());
  await settle(page);
  const seen = new Set<number>();
  for (let i = 0; i < 8; i++) {
    await page.waitForTimeout(70);
    seen.add(Math.round((await boxScale(page)) * 1000));
  }
  expect(seen.size, 'it loops on through the next click').toBeGreaterThan(2);
  // Leaving the slide cuts it: the transition's first frame shows the outgoing slide as its still render (the fade
  // composites it, which may change the antialiasing of a few edges), and the slide holds its one round's end through
  // the transition.
  await page.evaluate(() => (window as unknown as Harness).sd.player.next());
  await pauseOnStart(page);
  for (const fraction of [0, 0.5]) {
    await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), fraction);
    if (fraction === 0) {
      expect(await diff(page, await shot(page), after, 96), 'the cut = renderStatic').toBeLessThanOrEqual(256);
    }
    expect(await boxScale(page), `held at ${fraction} of the transition`).toBeCloseTo(1.1, 6);
  }
  await page.evaluate(() => (window as unknown as Harness).sd.resume());
  await settle(page);
  await page.evaluate(() => (window as unknown as Harness).sd.player.previous());
  await settle(page);
  expect(await boxScale(page)).toBeCloseTo(1.1, 6);
  expect(await diff(page, await shot(page), after), 'back = renderStatic').toBe(0);
});

test("a tier-2 step's own repeat plays the whole step every round, and ends held", async ({ page }) => {
  const timeline = [
    step(
      's1.box',
      fields({
        kind: 'emphasis',
        fallback: 'none',
        effect: 'behaviors',
        durationMs: 500,
        repeat: { count: 3 },
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
      }),
    ),
  ];
  await open(page, fixture([deckSlide('s1', { timeline })]));
  const after = await staticShot(page, 0, 1);
  const scale = () =>
    page.evaluate(() => {
      const el = (window as unknown as Harness).sd.target('s1.box');
      return el === null ? Number.NaN : new DOMMatrix(getComputedStyle(el).transform).a;
    });
  await page.evaluate(() => {
    const { sd } = window as unknown as Harness;
    sd.player.next();
    sd.pause();
  });
  // Three rounds of 500 ms: 750 ms is the middle of the second, 1050 ms just into the third.
  for (const [fraction, expected] of [
    [0.5, 1.05],
    [0.7, 1.01],
    [1, 1.1],
  ] as const) {
    await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), fraction);
    expect(await scale(), `at ${fraction}`).toBeCloseTo(expected, 3);
  }
  await page.evaluate(() => (window as unknown as Harness).sd.resume());
  await settle(page);
  expect(await scale()).toBeCloseTo(1.1, 6);
  expect(await diff(page, await shot(page), after), 'settled = renderStatic').toBe(0);
});

test('a held repeat cut by the next click holds its end through the next group', async ({ page }) => {
  const timeline = [
    step(
      's1.box',
      fields({
        kind: 'emphasis',
        fallback: 'none',
        effect: 'spin',
        params: { deg: 90, cw: true, hold: true },
        durationMs: 400,
        repeat: 'untilNextClick',
      }),
    ),
    step(
      's1.text',
      fields({
        kind: 'emphasis',
        fallback: 'none',
        effect: 'spin',
        params: { deg: 10, cw: true, hold: true },
        durationMs: 1000,
      }),
    ),
  ];
  await open(page, fixture([deckSlide('s1', { timeline })]));
  const after = await staticShot(page, 0, 2);
  const turn = () =>
    page.evaluate(() => {
      const el = (window as unknown as Harness).sd.target('s1.box');
      return el === null ? '' : getComputedStyle(el).transform;
    });
  await page.evaluate(() => (window as unknown as Harness).sd.player.next());
  await page.waitForTimeout(300);
  expect(await turn(), 'the loop runs').not.toBe('none');
  await page.evaluate(() => {
    const { sd } = window as unknown as Harness;
    sd.player.next();
    sd.pause();
  });
  for (const fraction of [0, 0.5]) {
    await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), fraction);
    expect(await turn(), `at ${fraction} of the next group`).toBe('matrix(0, 1, -1, 0, 0, 0)');
  }
  await page.evaluate(() => (window as unknown as Harness).sd.resume());
  await settle(page);
  expect(await turn()).toBe('matrix(0, 1, -1, 0, 0, 0)');
  expect(await diff(page, await shot(page), after), 'settled = renderStatic').toBe(0);
});
