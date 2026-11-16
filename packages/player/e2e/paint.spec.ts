// Colour and font changes: text colour through the recolouring rule, fill, line and font switches
// through the deck's paint hooks (`var(--sd-fill, <static>)`…). The static render shows each hook's fallback, the
// change interpolates colours and switches fonts frame by frame, its end holds, and moving back cuts to the static
// paint. A target whose markup has no hook plays its fallback, reported.
import { type Page, expect, test } from '@playwright/test';
import type { DeckStep } from '@slidock/types';

import { deckSlide, fixture, slideHtml, step } from './support/deck';
import { type Harness, diff, disallowedKeys, open, settle, shot, staticShot } from './support/harness';

const AA = 96;
const SPECKS = 256;
/** A settled frame is the still render, pixel for pixel. */
const STILL = 0;

const FILL = [124, 58, 237] as const; // #7c3aed
const LINE = [17, 24, 39] as const; // #111827
const PATH_FILL = [22, 163, 74] as const; // #16a34a
const RED = [220, 38, 38] as const; // #dc2626

/**
 * The fixture slide with a hooked text box and a hooked SVG shape, painted as the deck path prints them: each target
 * declares its hooks with their static values, and its paint (or its SVG path's) reads them.
 */
function hooked(id: string): string {
  return slideHtml(id).replace(
    '</main>',
    `<div data-el="${id}.hook" data-kind="shape" data-sd-text style="--sd-fill:#7c3aed;--sd-line:#111827;` +
      '--sd-bold:400;--sd-italic:normal;--sd-underline:none;position:absolute;left:380pt;top:250pt;' +
      'width:300pt;height:110pt;box-sizing:border-box;padding:12pt;background:var(--sd-fill,#7c3aed);' +
      'border:9pt solid var(--sd-line,#111827);color:#fde68a;font:20pt/1.2 sans-serif;' +
      'font-weight:var(--sd-bold,400);font-style:var(--sd-italic,normal);' +
      'text-decoration-line:var(--sd-underline,none)"><p data-p="0">Hooked <span style="color:#ffffff">text</span></p></div>' +
      `<div data-el="${id}.svg" data-kind="shape" style="--sd-fill:#16a34a;--sd-line:#111827;position:absolute;` +
      'left:40pt;top:250pt;width:200pt;height:110pt">' +
      '<svg width="100%" height="100%" viewBox="0 0 200 110" style="display:block">' +
      '<path d="M12 12H188V98H12Z" stroke-width="16" style="fill:var(--sd-fill,#16a34a);stroke:var(--sd-line,#111827)"></path>' +
      '</svg></div></main>',
  );
}

const PX = 4 / 3;
/** Points inside the hooked box's fill band (below its text), its border, the SVG's fill and its stroke, in px. */
const AT = {
  fill: [(380 + 150) * PX, (250 + 95) * PX],
  border: [(380 + 150) * PX, (250 + 3) * PX],
  pathFill: [(40 + 100) * PX, (250 + 55) * PX],
  pathLine: [(40 + 100) * PX, (250 + 110 * (12 / 110)) * PX],
} as const;

async function pixel(page: Page, image: string, [x, y]: readonly [number, number]): Promise<[number, number, number]> {
  return page.evaluate(
    async ([b64, px, py]) => {
      const bitmap = await createImageBitmap(await (await fetch(`data:image/png;base64,${b64}`)).blob());
      const ctx = new OffscreenCanvas(bitmap.width, bitmap.height).getContext('2d');
      if (ctx === null) {
        return [-1, -1, -1] as [number, number, number];
      }
      ctx.drawImage(bitmap, 0, 0);
      const d = ctx.getImageData(Math.round(px), Math.round(py), 1, 1).data;
      return [d[0] ?? -1, d[1] ?? -1, d[2] ?? -1] as [number, number, number];
    },
    [image, x, y] as const,
  );
}

function near(actual: readonly number[], expected: readonly number[], tolerance = 3): boolean {
  return actual.every((v, i) => Math.abs(v - (expected[i] ?? 0)) <= tolerance);
}

/** Strictly between two colours: inside their channel ranges, and away from both. */
function between(actual: readonly number[], a: readonly number[], b: readonly number[]): boolean {
  const inside = actual.every(
    (v, i) => v >= Math.min(a[i] ?? 0, b[i] ?? 0) - 3 && v <= Math.max(a[i] ?? 0, b[i] ?? 0) + 3,
  );
  const away = (c: readonly number[]) => actual.some((v, i) => Math.abs(v - (c[i] ?? 0)) > 20);
  return inside && away(a) && away(b);
}

async function at(page: Page, fraction: number): Promise<string> {
  await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), fraction);
  return shot(page);
}

async function start(page: Page): Promise<number> {
  return page.evaluate(() => {
    const { sd } = window as unknown as Harness;
    sd.player.next();
    return sd.pause();
  });
}

/** A computed style of the element `selector` finds under the current slide's `data-el` element `id` (or itself). */
async function computed(page: Page, id: string, selector: string | null, property: string): Promise<string> {
  return page.evaluate(
    ([target, sel, prop]) => {
      const root = (window as unknown as Harness).sd.target(target);
      const el = sel === null ? root : (root?.querySelector(sel) ?? null);
      return el === null ? '' : getComputedStyle(el).getPropertyValue(prop).trim();
    },
    [id, selector, property] as const,
  );
}

/**
 * What the shown slide keeps that its still render has none of: a link, an animation of a paint or
 * any animation still running, an inline value of a registered property, `will-change`, a stacking context of a
 * paint's own.
 */
async function leftovers(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const root = document.querySelector('.sd-layer[data-sd-current]')?.shadowRoot;
    const found: string[] = [];
    for (const element of Array.from(root?.querySelectorAll('*') ?? [])) {
      const id = element.closest('[data-el]')?.getAttribute('data-el') ?? element.tagName;
      if (element.hasAttribute('data-sd-paint')) {
        found.push(`${id} linked`);
      }
      const style = element.getAttribute('style') ?? '';
      for (const what of ['--sd-anim-', 'will-change', 'isolation']) {
        if (style.includes(what)) {
          found.push(`${id} ${what} inline`);
        }
      }
      if (getComputedStyle(element).isolation === 'isolate' && !element.hasAttribute('data-sd-box')) {
        found.push(`${id} isolated`);
      }
    }
    for (const animation of root?.getAnimations() ?? []) {
      const keys = (animation.effect as KeyframeEffect).getKeyframes().flatMap((k) => Object.keys(k));
      if (animation.playState === 'running' || keys.some((key) => key.startsWith('--sd-anim-'))) {
        found.push(`animation ${animation.playState} ${keys.join(',')}`);
      }
    }
    return found;
  });
}

const emphasis = (element: string, overrides: Record<string, unknown>): DeckStep =>
  step(element, { kind: 'emphasis', fallback: 'none', ...overrides });

interface ColorCase {
  readonly name: string;
  readonly step: DeckStep;
  readonly point: readonly [number, number];
  readonly from: readonly [number, number, number];
}

const COLOR_CASES: readonly ColorCase[] = [
  {
    name: 'a box fill',
    step: emphasis('s1.hook', { effect: 'colorChange', params: { property: 'fill', to: '#dc2626' } }),
    point: AT.fill,
    from: FILL,
  },
  {
    name: 'a box border',
    step: emphasis('s1.hook', { effect: 'colorChange', params: { property: 'line', to: '#dc2626' } }),
    point: AT.border,
    from: LINE,
  },
  {
    name: 'an SVG fill',
    step: emphasis('s1.svg', { effect: 'colorChange', params: { property: 'fill', to: '#dc2626' } }),
    point: AT.pathFill,
    from: PATH_FILL,
  },
  {
    name: 'an SVG stroke',
    step: emphasis('s1.svg', { effect: 'colorChange', params: { property: 'line', to: '#dc2626' } }),
    point: AT.pathLine,
    from: LINE,
  },
];

test('the static render paints every hook with its fallback', async ({ page }) => {
  await open(page, fixture([deckSlide('s1', { timeline: [COLOR_CASES[0]?.step as DeckStep] })], { s1: hooked('s1') }));
  // The player registered its colour properties: the hooks stay unregistered, so their fallbacks still apply.
  const still = await staticShot(page, 0, 0);
  expect(await pixel(page, still, AT.fill)).toEqual([...FILL]);
  expect(await pixel(page, still, AT.border)).toEqual([...LINE]);
  expect(await pixel(page, still, AT.pathFill)).toEqual([...PATH_FILL]);
  expect(await pixel(page, still, AT.pathLine)).toEqual([...LINE]);
  expect(await computed(page, 's1.hook', null, 'font-weight')).toBe('400');
  expect(await computed(page, 's1.hook', null, 'font-style')).toBe('normal');
  expect(await computed(page, 's1.hook', null, 'text-decoration-line')).toBe('none');
});

for (const c of COLOR_CASES) {
  test(`a colour change interpolates ${c.name} through its hook and holds it`, async ({ page }) => {
    await open(page, fixture([deckSlide('s1', { timeline: [c.step] })], { s1: hooked('s1') }));
    const before = await staticShot(page, 0, 0);
    const after = await staticShot(page, 0, 1);
    expect(await pixel(page, before, c.point), 'static: the fallback').toEqual([...c.from]);
    expect(near(await pixel(page, after, c.point), RED), 'fully built: the new colour').toBe(true);
    expect(await start(page)).toBeGreaterThan(0);
    const first = await at(page, 0);
    expect(await pixel(page, first, c.point), 'first frame').toEqual([...c.from]);
    expect(await diff(page, first, before, AA), 'first frame = state before').toBeLessThanOrEqual(SPECKS);
    const middle = await pixel(page, await at(page, 0.5), c.point);
    expect(between(middle, c.from, RED), `middle frame between: ${middle.join(',')}`).toBe(true);
    const last = await at(page, 1);
    expect(near(await pixel(page, last, c.point), RED), 'last frame').toBe(true);
    expect(await diff(page, last, after, AA), 'last frame = state after').toBeLessThanOrEqual(SPECKS);
    expect(await disallowedKeys(page)).toEqual([]);
    await page.evaluate(() => (window as unknown as Harness).sd.resume());
    await settle(page);
    expect(await diff(page, await shot(page), after, STILL), 'settled = renderStatic').toBe(0);
    expect(await leftovers(page), 'settled: nothing the still render lacks').toEqual([]);
    await page.evaluate(() => (window as unknown as Harness).sd.player.previous());
    await settle(page);
    expect(await diff(page, await shot(page), before, STILL), 'back = state before').toBe(0);
    expect(
      await page.evaluate(
        () =>
          (window as unknown as Harness).sd.target('s1.hook')?.closest('main')?.querySelector('[data-sd-paint]') ??
          null,
      ),
    ).toBeNull();
    expect(await page.evaluate(() => (window as unknown as Harness).sd.diagnostics)).toEqual([]);
  });
}

test('an endless colour loop keeps showing after its group settles, until the slide ends', async ({ page }) => {
  const loop = emphasis('s1.hook', {
    effect: 'colorChange',
    params: { property: 'fill', to: '#dc2626' },
    repeat: 'untilEndOfSlide',
  });
  await open(page, fixture([deckSlide('s1', { timeline: [loop] }), deckSlide('s2')], { s1: hooked('s1') }));
  const fill = () => computed(page, 's1.hook', null, 'background-color');
  const still = await fill();
  await page.evaluate(() => (window as unknown as Harness).sd.player.next());
  // The group has nothing to wait for: it settles at once, and the loop runs on.
  const seen = new Set<string>();
  for (let i = 0; i < 12; i++) {
    await page.waitForTimeout(60);
    seen.add(await fill());
  }
  seen.delete(still);
  expect(seen.size, `the fill changes while the loop runs (static ${still})`).toBeGreaterThan(1);
  expect(
    await page.evaluate(() => (window as unknown as Harness).sd.target('s1.hook')?.getAttribute('data-sd-paint')),
  ).toBe('fill');
  await page.evaluate(() => (window as unknown as Harness).sd.player.next());
  await settle(page);
  expect(
    await page.evaluate(
      () =>
        document.querySelector('.sd-layer[data-sd-layer="s1"]')?.shadowRoot?.querySelector('[data-sd-paint]') ?? null,
    ),
    'the slide ended: nothing is linked',
  ).toBeNull();
});

test('tier-2 colour tracks play each frame on text, fill and line', async ({ page }) => {
  const track = (property: string) => ({
    property,
    beginMs: 0,
    durationMs: 1000,
    frames: [
      { offset: 0, value: '#000000' },
      { offset: 0.5, value: '#ff0000' },
      { offset: 1, value: '#0000ff' },
    ],
  });
  const timeline = [
    emphasis('s1.hook', { effect: 'behaviors', params: [track('text'), track('fill'), track('line')] }),
  ];
  await open(page, fixture([deckSlide('s1', { timeline })], { s1: hooked('s1') }));
  const after = await staticShot(page, 0, 1);
  expect(await start(page)).toBeGreaterThan(0);
  const read = async () => ({
    text: await computed(page, 's1.hook', 'p', 'color'),
    run: await computed(page, 's1.hook', 'span', 'color'),
    fill: await computed(page, 's1.hook', null, 'background-color'),
    line: await computed(page, 's1.hook', null, 'border-top-color'),
  });
  await at(page, 0);
  expect(await read()).toEqual({
    text: 'rgb(0, 0, 0)',
    run: 'rgb(0, 0, 0)',
    fill: 'rgb(0, 0, 0)',
    line: 'rgb(0, 0, 0)',
  });
  await at(page, 0.5);
  const red = 'rgb(255, 0, 0)';
  expect(await read()).toEqual({ text: red, run: red, fill: red, line: red });
  await at(page, 0.25);
  const quarter = await read();
  expect(quarter.fill).toBe('rgb(128, 0, 0)');
  expect(quarter.line).toBe(quarter.fill);
  await page.evaluate(() => (window as unknown as Harness).sd.resume());
  await settle(page);
  const blue = 'rgb(0, 0, 255)';
  expect(await read()).toEqual({ text: blue, run: blue, fill: blue, line: blue });
  expect(await diff(page, await shot(page), after, STILL), 'settled = renderStatic').toBe(0);
  expect(await leftovers(page), 'settled: nothing the still render lacks').toEqual([]);
});

test('tier-2 font tracks switch bold, italic and underline, each value holding until the next frame', async ({
  page,
}) => {
  const track = (property: string) => ({
    property,
    beginMs: 0,
    durationMs: 1000,
    frames: [
      { offset: 0, value: true },
      { offset: 0.6, value: false },
      { offset: 0.8, value: true },
      { offset: 1, value: true },
    ],
  });
  const timeline = [
    emphasis('s1.hook', { effect: 'behaviors', params: [track('bold'), track('italic'), track('underline')] }),
  ];
  await open(page, fixture([deckSlide('s1', { timeline })], { s1: hooked('s1') }));
  const before = await staticShot(page, 0, 0);
  const after = await staticShot(page, 0, 1);
  expect(await diff(page, before, after, 0), 'the held switches change the text').toBeGreaterThan(0);
  expect(await start(page)).toBeGreaterThan(0);
  const read = async () =>
    [
      await computed(page, 's1.hook', 'p', 'font-weight'),
      await computed(page, 's1.hook', null, 'font-style'),
      await computed(page, 's1.hook', null, 'text-decoration-line'),
    ].join(' ');
  for (const [fraction, expected] of [
    [0, '700 italic underline'],
    [0.3, '700 italic underline'],
    [0.59, '700 italic underline'],
    [0.61, '400 normal none'],
    [0.79, '400 normal none'],
    [0.81, '700 italic underline'],
  ] as const) {
    await at(page, fraction);
    expect(await read(), `at ${fraction}`).toBe(expected);
  }
  await page.evaluate(() => (window as unknown as Harness).sd.resume());
  await settle(page);
  expect(await read()).toBe('700 italic underline');
  expect(await diff(page, await shot(page), after, STILL), 'settled = renderStatic').toBe(0);
  expect(await leftovers(page), 'settled: nothing the still render lacks').toEqual([]);
  await page.evaluate(() => (window as unknown as Harness).sd.player.previous());
  await settle(page);
  expect(await read()).toBe('400 normal none');
  expect(await diff(page, await shot(page), before, STILL), 'back = state before').toBe(0);
});

test('a fill change on a target without the hook plays its fallback, reported', async ({ page }) => {
  // `s1.box` paints a literal background: no hook to animate.
  const timeline = [emphasis('s1.box', { effect: 'colorChange', params: { property: 'fill', to: '#dc2626' } })];
  await open(page, fixture([deckSlide('s1', { timeline })], { s1: hooked('s1') }));
  const before = await staticShot(page, 0, 0);
  await page.evaluate(() => (window as unknown as Harness).sd.player.next());
  await settle(page);
  expect(await diff(page, await shot(page), before, STILL)).toBe(0);
  const codes = await page.evaluate(() => (window as unknown as Harness).sd.diagnostics.map((d) => d.code));
  expect(codes).toEqual(['RT0006']);
});

test('a tier-2 font track on a target without its hook is left out, reported; the other tracks play', async ({
  page,
}) => {
  const timeline = [
    emphasis('s1.box', {
      effect: 'behaviors',
      params: [
        {
          property: 'bold',
          beginMs: 0,
          durationMs: 400,
          frames: [
            { offset: 0, value: true },
            { offset: 1, value: true },
          ],
        },
        {
          property: 'opacity',
          beginMs: 0,
          durationMs: 400,
          frames: [
            { offset: 0, value: 1 },
            { offset: 1, value: 0.5 },
          ],
        },
      ],
    }),
  ];
  await open(page, fixture([deckSlide('s1', { timeline })], { s1: hooked('s1') }));
  const after = await staticShot(page, 0, 1);
  await page.evaluate(() => (window as unknown as Harness).sd.player.next());
  await settle(page);
  expect(await diff(page, await shot(page), after, STILL)).toBe(0);
  expect(Number(await computed(page, 's1.box', null, 'opacity'))).toBeCloseTo(0.5);
  const diagnostics = await page.evaluate(() => (window as unknown as Harness).sd.diagnostics);
  expect(diagnostics.map((d) => d.code)).toEqual(['RT0006']);
  expect(diagnostics[0]?.message).toContain('bold track');
});

test('tier-2 switches show bold, italic, underline and colours from their begin, nothing before, and hold', async ({
  page,
}) => {
  const set = (property: string, value: unknown) => ({
    property: 'set',
    beginMs: 400,
    durationMs: 600,
    to: { property, value },
  });
  const timeline = [
    emphasis('s1.hook', {
      effect: 'behaviors',
      params: [
        set('bold', true),
        set('italic', true),
        set('underline', true),
        set('fill', '#dc2626'),
        set('text', '#0000ff'),
      ],
    }),
  ];
  await open(page, fixture([deckSlide('s1', { timeline })], { s1: hooked('s1') }));
  const before = await staticShot(page, 0, 0);
  const after = await staticShot(page, 0, 1);
  const read = async () =>
    [
      await computed(page, 's1.hook', 'p', 'font-weight'),
      await computed(page, 's1.hook', null, 'font-style'),
      await computed(page, 's1.hook', null, 'text-decoration-line'),
      await computed(page, 's1.hook', null, 'background-color'),
      await computed(page, 's1.hook', 'span', 'color'),
    ].join(' ');
  const off = '400 normal none rgb(124, 58, 237) rgb(255, 255, 255)';
  const on = '700 italic underline rgb(220, 38, 38) rgb(0, 0, 255)';
  expect(await diff(page, before, after, 0), 'the held switches change the box').toBeGreaterThan(0);
  expect(await start(page)).toBeGreaterThan(0);
  // The switches end 1000 ms in: 0.39 is 390 ms, just before their begin, and 0.41 just after it.
  for (const [fraction, expected] of [
    [0, off],
    [0.39, off],
    [0.41, on],
    [0.7, on],
    [1, on],
  ] as const) {
    const frame = await at(page, fraction);
    expect(await read(), `at ${fraction}`).toBe(expected);
    if (fraction === 0) {
      expect(await diff(page, frame, before, AA), 'first frame = state before').toBeLessThanOrEqual(SPECKS);
    }
  }
  expect(await disallowedKeys(page)).toEqual([]);
  await page.evaluate(() => (window as unknown as Harness).sd.resume());
  await settle(page);
  expect(await read()).toBe(on);
  expect(await diff(page, await shot(page), after, STILL), 'settled = renderStatic').toBe(0);
  expect(await leftovers(page), 'settled: nothing the still render lacks').toEqual([]);
  await page.evaluate(() => (window as unknown as Harness).sd.player.previous());
  await settle(page);
  expect(await read()).toBe(off);
  expect(await diff(page, await shot(page), before, STILL), 'back = state before').toBe(0);
  expect(await page.evaluate(() => (window as unknown as Harness).sd.diagnostics)).toEqual([]);
});

test('switches and tracks of one paint, in one step or one group, each show in their time', async ({ page }) => {
  const set = (property: string, value: unknown, beginMs: number, durationMs = 500) => ({
    property: 'set',
    beginMs,
    durationMs,
    to: { property, value },
  });
  const cases: { name: string; timeline: DeckStep[]; expected: [number, string][] }[] = [
    {
      // A switch that does not hold, as the compilers write it: a second switch back to the element's own value.
      name: 'a switch and its switch back',
      timeline: [
        emphasis('s1.hook', {
          effect: 'behaviors',
          durationMs: 1000,
          params: [
            set('fill', '#dc2626', 0),
            set('fill', '#7c3aed', 500),
            set('bold', true, 0),
            set('bold', false, 500),
          ],
        }),
      ],
      expected: [
        [0.25, '700 rgb(220, 38, 38)'],
        [0.75, '400 rgb(124, 58, 237)'],
        [1, '400 rgb(124, 58, 237)'],
      ],
    },
    {
      name: 'a colour track, then a switch',
      timeline: [
        emphasis('s1.hook', {
          effect: 'behaviors',
          durationMs: 1000,
          params: [
            {
              property: 'fill',
              beginMs: 0,
              durationMs: 500,
              frames: [
                { offset: 0, value: '#ff0000' },
                { offset: 1, value: '#0000ff' },
              ],
            },
            set('fill', '#00ff00', 500),
          ],
        }),
      ],
      expected: [
        [0, '400 rgb(255, 0, 0)'],
        [0.25, '400 rgb(128, 0, 128)'],
        [0.75, '400 rgb(0, 255, 0)'],
      ],
    },
    {
      name: 'two steps of one group',
      timeline: [
        emphasis('s1.hook', {
          effect: 'behaviors',
          durationMs: 500,
          params: [set('fill', '#dc2626', 100, 400), set('bold', true, 100, 400)],
        }),
        emphasis('s1.hook', {
          trigger: 'afterPrevious',
          effect: 'behaviors',
          durationMs: 500,
          params: [set('fill', '#0000ff', 0)],
        }),
      ],
      expected: [
        [0.05, '400 rgb(124, 58, 237)'],
        [0.25, '700 rgb(220, 38, 38)'],
        [0.75, '700 rgb(0, 0, 255)'],
      ],
    },
  ];
  for (const c of cases) {
    await test.step(c.name, async () => {
      await open(page, fixture([deckSlide('s1', { timeline: c.timeline })], { s1: hooked('s1') }));
      const after = await staticShot(page, 0, 1);
      const read = async () =>
        `${await computed(page, 's1.hook', 'p', 'font-weight')} ${await computed(page, 's1.hook', null, 'background-color')}`;
      expect(await start(page)).toBeGreaterThan(0);
      for (const [fraction, expected] of c.expected) {
        await at(page, fraction);
        expect(await read(), `at ${fraction}`).toBe(expected);
      }
      await page.evaluate(() => (window as unknown as Harness).sd.resume());
      await settle(page);
      expect(await diff(page, await shot(page), after, STILL), 'settled = renderStatic').toBe(0);
      expect(await leftovers(page), 'settled: nothing the still render lacks').toEqual([]);
    });
  }
});

/** `#rrggbb` as 0–255 channels. */
function rgb(hex: string): [number, number, number] {
  return [1, 3, 5].map((i) => Number.parseInt(hex.slice(i, i + 2), 16)) as [number, number, number];
}

/** The colour halfway from `a` to `b`, mixed in gamma-encoded sRGB or in Oklab (CSS Color 4 §12), as 0–255 channels. */
function halfway(a: string, b: string, space: 'srgb' | 'oklab'): [number, number, number] {
  if (space === 'srgb') {
    return rgb(a).map((v, i) => Math.round((v + (rgb(b)[i] ?? 0)) / 2)) as [number, number, number];
  }
  const linear = (v: number) => {
    const c = v / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  const toLab = (hex: string) => {
    const [r, g, bl] = rgb(hex).map(linear) as [number, number, number];
    const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * bl);
    const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * bl);
    const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * bl);
    return [
      0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
      1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
      0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
    ];
  };
  const [L, A, B] = toLab(a).map((v, i) => (v + (toLab(b)[i] ?? 0)) / 2) as [number, number, number];
  const l = (L + 0.3963377774 * A + 0.2158037573 * B) ** 3;
  const m = (L - 0.1055613458 * A - 0.0638541728 * B) ** 3;
  const s = (L - 0.0894841775 * A - 1.291485548 * B) ** 3;
  const encode = (c: number) => {
    const v = c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055;
    return Math.round(Math.min(1, Math.max(0, v)) * 255);
  };
  return [
    encode(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
    encode(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
    encode(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
  ];
}

test('a registered <color> animated between two #rrggbb keyframes is mixed in sRGB, as the compiler folds', async ({
  page,
}, info) => {
  // The pairs the compiler folds: primaries, black to white, and the probe decks' fill and accent.
  const PAIRS = [
    ['#ff0000', '#0000ff'],
    ['#000000', '#ffffff'],
    ['#2f6fdf', '#e0442e'],
  ] as const;
  await page.setContent('<!doctype html><html><head><meta charset="utf-8"></head><body></body></html>');
  const measured = await page.evaluate((pairs) => {
    CSS.registerProperty({ name: '--sd-probe', syntax: '<color>', inherits: false, initialValue: '#000000' });
    return pairs.map(([a, b]) => {
      const el = document.createElement('div');
      el.style.cssText = 'width:10px;height:10px;background-color:var(--sd-probe)';
      document.body.append(el);
      const animation = el.animate([{ '--sd-probe': a }, { '--sd-probe': b }], { duration: 1000, fill: 'both' });
      animation.pause();
      animation.currentTime = 500;
      const value = getComputedStyle(el).backgroundColor;
      animation.cancel();
      el.remove();
      return value;
    });
  }, PAIRS);
  const spaces = PAIRS.map(([a, b], i) => {
    const got =
      (measured[i] ?? '')
        .match(/[\d.]+/g)
        ?.slice(0, 3)
        .map(Number) ?? [];
    const close = (c: readonly number[]) => got.length === 3 && c.every((v, j) => Math.abs(v - (got[j] ?? -9)) <= 1);
    return close(halfway(a, b, 'srgb')) ? 'srgb' : close(halfway(a, b, 'oklab')) ? 'oklab' : `neither: ${measured[i]}`;
  });
  info.annotations.push({
    type: 'color-interpolation',
    description: `${info.project.name}: ${PAIRS.map(([a, b], i) => `${a}→${b} mid ${measured[i]} (${spaces[i]})`).join('; ')}`,
  });
  // The two spaces disagree on every pair, so the check tells them apart.
  for (const [a, b] of PAIRS) {
    expect(halfway(a, b, 'srgb')).not.toEqual(halfway(a, b, 'oklab'));
  }
  expect(spaces).toEqual(PAIRS.map(() => 'srgb'));
});
