// Keynote's Magic Move through the compiler: a two-slide IR presentation of our
// own (written here, compiled with the CLI's IR frontend) whose second slide enters by a Magic Move with "fade
// unmatched objects", as the Keynote frontend lowers one (`effect.native` magicMove, `matchKey` on each matched
// element). The compiled markup (`data-sd-match`, the deck's `matchKeys`, a straight line's box of zero height) is what
// the player reads, so a change on either side shows here: the matched box travels on the in-out curve; the incoming
// slide's unmatched objects (a box, and a line whose stroke and arrowhead overflow its box) stay out until the last
// third, then fade in together, the line's overflow included (in WebKit too); the end is the still render.
import { type Page, expect, test } from '@playwright/test';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';

import { OUT, REPO, cli, compile, readDeckPage } from './support/cli';
import { type Harness, diff, open, pauseOnStart, settle, shot, staticShot } from './support/harness';

test.describe.configure({ mode: 'serial', timeout: 5 * 60_000 });

let binary = '';
test.beforeAll(() => {
  test.setTimeout(30 * 60_000);
  binary = cli();
});

const frame = (x: number, y: number, w: number, h: number) => ({ x, y, w, h, rotation: 0, flipH: false, flipV: false });
const rgb = (r: number, g: number, b: number) => ({ kind: 'rgb', space: 'srgb', r, g, b, a: 1 });
const BLUE = rgb(0.18, 0.44, 0.87);

function box(id: string, at: ReturnType<typeof frame>, color: ReturnType<typeof rgb>, matchKey?: string) {
  return {
    id,
    origin: { kind: 'agent' },
    kind: 'shape',
    frame: at,
    geometry: { kind: 'preset', name: 'rect', adjust: {} },
    fill: { kind: 'solid', color },
    ...(matchKey === undefined ? {} : { matchKey }),
  };
}

/** A straight line with an arrowhead (Keynote's `line-end` path): a frame of zero height. */
function line(id: string, at: ReturnType<typeof frame>, color: ReturnType<typeof rgb>) {
  const shaft = at.w - 30;
  return {
    id,
    origin: { kind: 'agent' },
    kind: 'shape',
    frame: at,
    geometry: {
      kind: 'paths',
      paths: [
        {
          fill: true,
          fillRule: 'nonzero',
          stroke: true,
          segments: [
            ['M', 0, 0],
            ['L', shaft, 0],
          ],
        },
        {
          fill: true,
          fillRule: 'nonzero',
          stroke: false,
          role: 'line-end',
          segments: [['M', shaft, -15], ['L', at.w, 0], ['L', shaft, 15], ['Z']],
        },
      ],
    },
    fill: { kind: 'none' },
    stroke: { cap: 'flat', dash: 'solid', join: 'miter', width: 8, fill: { kind: 'solid', color } },
  };
}

function presentation() {
  const level = {
    para: {
      align: 'left',
      bullet: { kind: 'none' },
      indent: 0,
      lineSpacing: { basis: 'size-1.2', kind: 'multiple', value: 1 },
      marginLeft: 0,
      marginRight: 0,
      rtl: false,
      spaceAfter: { kind: 'pt', value: 0 },
      spaceBefore: { kind: 'pt', value: 0 },
      tabSize: 72,
    },
    run: {
      baseline: 0,
      caps: 'none',
      color: { kind: 'theme', mods: [], slot: 'tx1' },
      font: { kind: 'theme', slot: 'minor' },
      highlight: null,
      italic: false,
      letterSpacing: 0,
      link: null,
      scriptScale: 1,
      size: 18,
      strike: 'none',
      synthesizeBold: true,
      underline: 'none',
      weight: 400,
    },
  };
  const slide = (id: string, elements: unknown[], transition?: unknown) => ({
    id,
    origin: { kind: 'agent' },
    hidden: false,
    showLayout: true,
    timeline: [],
    elements,
    ...(transition === undefined ? {} : { transition }),
  });
  return {
    $schema: 'urn:slidock:presentation-ir:1',
    size: { w: 960, h: 540 },
    defaults: {
      body: {
        anchor: 'top',
        autofit: { kind: 'none' },
        insets: { b: 3.6, l: 7.2, r: 7.2, t: 3.6 },
        overflow: 'spread',
        wrap: 'square',
      },
      text: Object.fromEntries(Array.from({ length: 9 }, (_, i) => [String(i), level])),
    },
    themes: {},
    masters: {},
    layouts: {},
    fonts: {},
    media: {},
    phase: 'source',
    slides: [
      slide('mm-a', [box('mm-a.box', frame(60, 180, 200, 120), BLUE, 'e2e:box')]),
      slide(
        'mm-b',
        [
          box('mm-b.box', frame(640, 180, 200, 120), BLUE, 'e2e:box'),
          box('mm-b.new', frame(60, 380, 200, 100), rgb(0.12, 0.62, 0.42)),
          line('mm-b.line', frame(320, 430, 300, 0), rgb(0.93, 0.38, 0.34)),
        ],
        {
          durationMs: 1000,
          easing: 'in-out',
          effect: {
            fallback: 'dissolve',
            id: 'keynote:apple:magic-move-implied-motion-path',
            native: {
              effect: 'magicMove',
              id: 'keynote:apple:magic-move-implied-motion-path',
              params: { fadeUnmatched: true },
            },
          },
        },
      ),
    ],
  };
}

/** Points in slide pt; on the harness's 960 px host the 960 pt stage (1280 px) is scaled by 3/4, so a point is a pixel. */
const AT = {
  background: [900, 40],
  /** The matched box's centre halfway: (160 + 740) / 2, as the in-out curve is at 0.5 at half time. */
  halfway: [450, 240],
  from: [160, 240],
  to: [740, 240],
  newBox: [160, 430],
  stroke: [450, 432],
  arrowhead: [612, 432],
} as const;

async function pixels(page: Page): Promise<Record<keyof typeof AT, number[]>> {
  const image = await shot(page);
  return page.evaluate(
    async ([b64, points]) => {
      const bitmap = await createImageBitmap(await (await fetch(`data:image/png;base64,${b64}`)).blob());
      const ctx = new OffscreenCanvas(bitmap.width, bitmap.height).getContext('2d');
      ctx?.drawImage(bitmap, 0, 0);
      const out: Record<string, number[]> = {};
      for (const [name, [x, y]] of Object.entries(points)) {
        const d = ctx?.getImageData(x, y, 1, 1).data;
        out[name] = [d?.[0] ?? -1, d?.[1] ?? -1, d?.[2] ?? -1];
      }
      return out;
    },
    [image, AT] as const,
  ) as Promise<Record<keyof typeof AT, number[]>>;
}

/** How far `actual` is from `background` toward `full`, on the channel where they differ most. */
function alpha(actual: readonly number[], background: readonly number[], full: readonly number[]): number {
  const channel = [0, 1, 2].reduce((a, b) =>
    Math.abs((full[b] ?? 0) - (background[b] ?? 0)) > Math.abs((full[a] ?? 0) - (background[a] ?? 0)) ? b : a,
  );
  return ((actual[channel] ?? 0) - (background[channel] ?? 0)) / ((full[channel] ?? 0) - (background[channel] ?? 0));
}

const near = (a: readonly number[], b: readonly number[], tolerance = 3) =>
  a.every((v, i) => Math.abs(v - (b[i] ?? 0)) <= tolerance);

test('a compiled Keynote Magic Move moves the match and fades the unmatched objects in over the last third', async ({
  page,
}) => {
  mkdirSync(OUT, { recursive: true });
  const input = join(mkdtempSync(join(OUT, 'magic-move-ir-')), 'magic-move.presentation-ir.json');
  writeFileSync(input, JSON.stringify(presentation()));
  const dir = compile(binary, relative(REPO, input), 'magic-move');
  const { deck, slides } = await readDeckPage(page, readFileSync(join(dir, 'index.html'), 'utf8'));
  // The markup the player reads.
  expect(deck.slides[1]?.transition?.effect).toBe('magicMove');
  expect(slides['mm-a']).toContain('data-el="mm-a.box" data-kind="shape" data-sd-match="e2e:box"');
  expect(slides['mm-b']).toContain('data-el="mm-b.box" data-kind="shape" data-sd-match="e2e:box"');
  expect(slides['mm-b']).toMatch(/data-el="mm-b\.line"[^>]*height:0pt;box-shadow:0 0 0 1pt transparent/);

  await open(page, { deck, slides });
  const after = await staticShot(page, 1, 0);
  await page.evaluate(() => (window as unknown as Harness).sd.player.next());
  await pauseOnStart(page);
  const seek = async (f: number) => {
    await page.evaluate((x) => (window as unknown as Harness).sd.seek(x), f);
    await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  };
  await seek(1);
  const full = await pixels(page);
  expect(near(full.from, full.background), 'the box has left').toBe(true);

  await seek(0.5);
  const half = await pixels(page);
  const bg = half.background;
  expect(near(half.halfway, full.to), `the match halfway: ${half.halfway}`).toBe(true);
  expect(near(half.from, bg) && near(half.to, bg), 'nothing left behind or ahead').toBe(true);
  for (const name of ['newBox', 'stroke', 'arrowhead'] as const) {
    expect(near(half[name], bg), `${name} not in yet at 0.5: ${half[name]}`).toBe(true);
  }

  // Keynote's last third: the unmatched objects come in together, a line's overflowing stroke and arrowhead with them.
  for (const f of [0.8, 0.9]) {
    await seek(f);
    const now = await pixels(page);
    const box = alpha(now.newBox, now.background, full.newBox);
    expect(box, `the unmatched box at ${f}`).toBeGreaterThan(0.1);
    expect(box, `the unmatched box at ${f}`).toBeLessThan(0.95);
    for (const name of ['stroke', 'arrowhead'] as const) {
      const value = alpha(now[name], now.background, full[name]);
      expect(
        Math.abs(value - box),
        `the line's ${name} at ${f}: ${value.toFixed(3)} vs the box's ${box.toFixed(3)}`,
      ).toBeLessThan(0.06);
    }
  }

  await page.evaluate(() => (window as unknown as Harness).sd.resume());
  await settle(page);
  expect(await diff(page, await shot(page), after), 'settled = renderStatic').toBe(0);
});
