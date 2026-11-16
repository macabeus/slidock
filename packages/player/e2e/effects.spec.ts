// Every effect of the contract, played by the built player and seeked to 0%, 50% and 100%. The
// first frame must equal the still render of the state before, the last frame and the settled end state the still
// render of the state after (`renderStatic`), pixel for pixel. The middle frame must show motion, and where the
// catalog says what it shows (a side, a spread, an axis, a pose) it must show that: a mirrored or wrong-direction
// recipe fails. No animation may touch a property outside the allowed set.
import { type Page, expect, test } from '@playwright/test';
import type { BuildEffect, StepKind } from '@slidock/types';

import {
  type BuildCase,
  EMPHASES,
  EMPHASIS_EXITS,
  ENTRANCES,
  EXIT_BEHAVIORS,
  type Mid,
  PROJECTIVE,
  type Pose,
  type Side,
  TRANSITIONS,
  opposite,
} from './catalog';
import { deckSlide, fixture, step, transition } from './support/deck';
import {
  type Harness,
  type Quads,
  type Region,
  diff,
  disallowedKeys,
  open,
  pauseOnStart,
  settle,
  shot,
  staticShot,
} from './support/harness';

const MIDDLE = 0.5;
/** The least shift, in px, a side check accepts; antialiasing noise stays under it. */
const SHIFT = 5;
/** Spread ratios past these gather at the centre or at the edges. */
const CENTER = 0.95;
const EDGES = 1.05;
/** An axis whose spread ratio moves this far changed; one within `KEPT` kept its extent. */
const CHANGED = 0.08;
const KEPT = 0.05;
/** The incoming slide's background in the fixture (variant b). */
const INCOMING_BG = [253, 230, 138] as const;
/** The fixture text's own colour. */
const TEXT_COLOR = 'rgb(17, 24, 39)';
/** Antialiasing a composited frame may change, per channel. */
const AA = 96;
/** Antialiased pixels past `AA` a composited frame may still have: glyph edges, 0.05% of the stage. */
const SPECKS = 256;
/** A change the middle frame must show, per channel. */
const MOVED = 16;

function effectFields(effect: BuildEffect): Record<string, unknown> {
  return 'params' in effect ? { effect: effect.effect, params: effect.params } : { effect: effect.effect };
}

async function at(page: Page, fraction: number): Promise<string> {
  await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), fraction);
  return shot(page);
}

async function frames(page: Page, middle = MIDDLE) {
  return { start: await at(page, 0), middle: await at(page, middle), end: await at(page, 1) };
}

function vector(side: Side): [number, number] {
  return [side.includes('l') ? -1 : side.includes('r') ? 1 : 0, side.includes('u') ? -1 : side.includes('d') ? 1 : 0];
}

/** Checks a measured middle frame against the catalog: its side, spread and axis. */
function expectMid(region: { full: Region; seen: Region }, mid: Mid, side: Side | undefined): void {
  const { full, seen } = region;
  const at = JSON.stringify({ full, seen });
  expect(seen.count, `something shows partway: ${at}`).toBeGreaterThan(0);
  if (side !== undefined) {
    const [vx, vy] = vector(side);
    if (vx !== 0) {
      expect((seen.cx - full.cx) * vx, `shows on side ${side}: ${at}`).toBeGreaterThanOrEqual(SHIFT);
    }
    if (vy !== 0) {
      expect((seen.cy - full.cy) * vy, `shows on side ${side}: ${at}`).toBeGreaterThanOrEqual(SHIFT);
    }
  }
  if (mid.spread !== undefined) {
    const ratio = seen.spread / full.spread;
    if (mid.spread === 'center') {
      expect(ratio, `gathers at the centre: ${at}`).toBeLessThanOrEqual(CENTER);
    } else {
      expect(ratio, `gathers at the edges: ${at}`).toBeGreaterThanOrEqual(EDGES);
    }
  }
  if (mid.axis !== undefined) {
    const rx = Math.abs(seen.spreadX / full.spreadX - 1);
    const ry = Math.abs(seen.spreadY / full.spreadY - 1);
    if (mid.axis === 'horz' || mid.axis === 'both') {
      expect(rx, `changes along x: ${at}`).toBeGreaterThanOrEqual(CHANGED);
    } else {
      expect(rx, `keeps its width: ${at}`).toBeLessThanOrEqual(KEPT);
    }
    if (mid.axis === 'vert' || mid.axis === 'both') {
      expect(ry, `changes along y: ${at}`).toBeGreaterThanOrEqual(CHANGED);
    } else {
      expect(ry, `keeps its height: ${at}`).toBeLessThanOrEqual(KEPT);
    }
  }
}

/**
 * Checks a middle frame where the catalog says what it shows. WebKit's screenshot paints a projective transform (one
 * with perspective) as a flat affine shape, while the page itself renders it in perspective (the `projective
 * transform` test below holds both), so where the target carries one at that frame, WebKit measures the quad its
 * computed transform projects to, and `perspective.spec.ts` checks that the page draws that quad (the test must be
 * listed in `PROJECTIVE`). Chromium and Firefox measure both ways, and both must pass.
 */
async function expectMidFrame(
  page: Page,
  browserName: string,
  at: { fraction: number; frame: string; hidden: string; shown: string },
  target: string | null,
  sample: { mode: 'differ' | 'color'; moved: number; color: readonly [number, number, number] | null },
  mid: Mid,
  side: Side | undefined,
): Promise<void> {
  const measure = (quads: Quads | null) =>
    page.evaluate(
      ([f, h, s, mode, moved, color, q]) => (window as unknown as Harness).sd.motion(f, h, s, mode, moved, color, q),
      [at.frame, at.hidden, at.shown, sample.mode, sample.moved, sample.color, quads] as const,
    );
  await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), at.fraction);
  const quads = await page.evaluate((id) => (window as unknown as Harness).sd.projected(id), target);
  if (quads !== null) {
    expect(PROJECTIVE, 'a projective middle frame is checked on screen').toContain(test.info().title);
    expectMid(await measure(quads), mid, side);
  }
  if (quads === null || browserName !== 'webkit') {
    expectMid(await measure(null), mid, side);
  }
}

/** The side an exit shows: the entrance's picture played backwards with the opposite direction. */
function exitSide(c: BuildCase): Side | undefined {
  const side = c.mid?.side;
  if (c.mid?.exitSide !== undefined || side === undefined) {
    return c.mid?.exitSide;
  }
  const params: unknown = 'params' in c.effect ? c.effect.params : undefined;
  const directed = typeof params === 'object' && params !== null && ('dir' in params || 'corner' in params);
  return directed ? opposite(side) : side;
}

/** The 2D matrix of a computed `transform`, with its rotation (clockwise positive), scales and translation. */
function pose(transform: string, translate: string) {
  const values = /matrix(3d)?\(([^)]*)\)/.exec(transform)?.[2]?.split(',').map(Number);
  const is3d = transform.startsWith('matrix3d');
  const [a, b, c, d, e, f] =
    values === undefined
      ? [1, 0, 0, 1, 0, 0]
      : is3d
        ? [values[0], values[1], values[4], values[5], values[12], values[13]]
        : values;
  const [tx, ty] = translate === 'none' ? [0, 0] : translate.split(' ').map((v) => Number.parseFloat(v) || 0);
  return {
    rotate: (Math.atan2(b ?? 0, a ?? 1) * 180) / Math.PI,
    scaleX: Math.hypot(a ?? 1, b ?? 0),
    scaleY: Math.hypot(c ?? 0, d ?? 1),
    x: (e ?? 0) + (tx ?? 0),
    y: (f ?? 0) + (ty ?? 0),
  };
}

function expectPose(style: Record<string, string> | null, expected: Pose): void {
  expect(style).not.toBeNull();
  const s = style ?? {};
  const at = JSON.stringify(s);
  const p = pose(s.transform ?? 'none', s.translate ?? 'none');
  if (expected.rotate !== undefined) {
    expect(p.rotate * (expected.rotate === 'cw' ? 1 : -1), `turns ${expected.rotate}: ${at}`).toBeGreaterThan(1);
  }
  if (expected.moved !== undefined) {
    const [vx, vy] = vector(expected.moved);
    if (vx !== 0) {
      expect(p.x * vx, `moved ${expected.moved}: ${at}`).toBeGreaterThanOrEqual(SHIFT);
    }
    if (vy !== 0) {
      expect(p.y * vy, `moved ${expected.moved}: ${at}`).toBeGreaterThanOrEqual(SHIFT);
    }
  }
  for (const [axis, scale] of [
    [expected.scaleX, p.scaleX],
    [expected.scaleY, p.scaleY],
  ] as const) {
    if (axis === 'grows') {
      expect(scale, `grows: ${at}`).toBeGreaterThan(1.02);
    } else if (axis === 'shrinks') {
      expect(scale, `shrinks: ${at}`).toBeLessThan(0.98);
    }
  }
  if (expected.fades === true) {
    expect(Number(s.opacity), `fades: ${at}`).toBeLessThan(0.95);
  }
  if (expected.recolors === true) {
    expect(s.color, `recolours: ${at}`).not.toBe(TEXT_COLOR);
  }
  if (expected.filters === true) {
    expect(s.filter, `filters: ${at}`).not.toBe('none');
  }
}

// Tier-2 tracks describe the exit itself, so an exit plays them as they are (EXIT_BEHAVIORS).
const exits: BuildCase[] = ENTRANCES.filter((c) => c.delivery === undefined && c.effect.effect !== 'behaviors').map(
  (c) => ({ ...c, kind: 'exit' as StepKind }),
);

for (const c of [...ENTRANCES, ...exits, EXIT_BEHAVIORS, ...EMPHASIS_EXITS, ...EMPHASES]) {
  test(`build ${c.kind} ${c.name}`, async ({ page, browserName }) => {
    const target = `s1.${c.target ?? 'box'}`;
    const data = fixture([
      deckSlide('s1', {
        timeline: [
          step(target, {
            kind: c.kind,
            delivery: c.delivery ?? 'all',
            fallback: c.kind === 'entrance' || c.kind === 'exit' ? 'dissolve' : 'none',
            ...effectFields(c.effect),
          }),
        ],
      }),
    ]);
    await open(page, data);
    const before = await staticShot(page, 0, 0);
    const after = await staticShot(page, 0, 1);
    const started = await page.evaluate(() => {
      const { sd } = window as unknown as Harness;
      sd.player.next();
      return sd.pause();
    });
    if (c.still !== true) {
      expect(started).toBeGreaterThan(0);
    }
    const seen = await frames(page, c.middle);
    expect(await disallowedKeys(page)).toEqual([]);
    if (c.instantStart !== true) {
      expect(await diff(page, seen.start, before, AA), 'first frame = state before').toBeLessThanOrEqual(SPECKS);
    }
    expect(await diff(page, seen.end, after, AA), 'last frame = state after').toBeLessThanOrEqual(SPECKS);
    if (c.still !== true) {
      expect(await diff(page, seen.middle, before, MOVED), 'middle frame moved from the start').toBeGreaterThan(0);
    }
    if (c.mid !== undefined) {
      const frame = c.mid.at === undefined ? seen.middle : await at(page, c.mid.at);
      const [hidden, shown] = c.kind === 'entrance' ? [before, after] : [after, before];
      await expectMidFrame(
        page,
        browserName,
        { fraction: c.mid.at ?? c.middle ?? MIDDLE, frame, hidden, shown },
        target,
        { mode: 'differ', moved: MOVED, color: null },
        c.mid,
        c.kind === 'entrance' ? c.mid.side : exitSide(c),
      );
    }
    if (c.pose !== undefined) {
      await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), c.middle ?? MIDDLE);
      expectPose(await page.evaluate((id) => (window as unknown as Harness).sd.style(id), target), c.pose);
    }
    await page.evaluate(() => (window as unknown as Harness).sd.resume());
    await settle(page);
    expect(await diff(page, await shot(page), after), 'settled end state = renderStatic').toBe(0);
    const diagnostics = await page.evaluate(() => (window as unknown as Harness).sd.diagnostics.map((d) => d.code));
    if (c.fallback === true) {
      expect(diagnostics).toContain('RT0006');
    } else {
      expect(diagnostics).not.toContain('RT0006');
    }
  });
}

for (const c of TRANSITIONS) {
  test(`transition ${c.name}`, async ({ page, browserName }) => {
    const data = fixture([
      deckSlide('s1'),
      deckSlide('s2', { transition: transition({ fallback: 'dissolve', ...effectFields(c.effect as BuildEffect) }) }),
    ]);
    await open(page, data);
    const outgoing = await staticShot(page, 0, 0);
    const incoming = await staticShot(page, 1, 0);
    await page.evaluate(() => (window as unknown as Harness).sd.player.next());
    if (c.instant !== true) {
      await pauseOnStart(page);
      const seen = await frames(page, c.middle);
      expect(await disallowedKeys(page)).toEqual([]);
      if (c.startsElsewhere !== true) {
        expect(await diff(page, seen.start, outgoing, AA), 'first frame = outgoing slide').toBeLessThanOrEqual(SPECKS);
      }
      expect(await diff(page, seen.middle, outgoing, MOVED), 'middle frame moved').toBeGreaterThan(0);
      expect(await diff(page, seen.middle, incoming, MOVED), 'middle frame not yet arrived').toBeGreaterThan(0);
      expect(await diff(page, seen.end, incoming, AA), 'last frame = incoming slide').toBeLessThanOrEqual(SPECKS);
      if (c.mid !== undefined) {
        const frame = c.mid.at === undefined ? seen.middle : await at(page, c.mid.at);
        await expectMidFrame(
          page,
          browserName,
          { fraction: c.mid.at ?? c.middle ?? MIDDLE, frame, hidden: outgoing, shown: incoming },
          null,
          { mode: 'color', moved: 24, color: INCOMING_BG },
          c.mid,
          c.mid.side,
        );
      }
      if (c.pose !== undefined) {
        await page.evaluate((f) => (window as unknown as Harness).sd.seek(f), MIDDLE);
        expectPose(await page.evaluate(() => (window as unknown as Harness).sd.incoming()), c.pose);
      }
      await page.evaluate(() => (window as unknown as Harness).sd.resume());
    }
    await settle(page);
    expect(await page.evaluate(() => (window as unknown as Harness).sd.player.position.slide)).toBe(1);
    expect(await diff(page, await shot(page), incoming), 'settled end state = renderStatic').toBe(0);
    const diagnostics = await page.evaluate(() => (window as unknown as Harness).sd.diagnostics.map((d) => d.code));
    expect(diagnostics).not.toContain('RT0006');
  });
}

// A box turned 45° about y in 800 px of perspective: its nearer (left) edge is 243 px tall and its farther one 171 px.
// Chromium and Firefox screenshot that; WebKit renders it on screen (its video frames match Chromium's) but its
// screenshot paints an affine shape with both edges 200 px tall, which is why `expectMidFrame` measures geometry there.
test('a projective transform in a screenshot', async ({ page, browserName }) => {
  await page.setViewportSize({ width: 600, height: 400 });
  await page.setContent(
    '<!doctype html><body style="margin:0;background:#fff"><div style="position:absolute;left:100px;top:100px;' +
      'width:400px;height:200px;background:#000;transform:perspective(800px) rotateY(45deg)"></div></body>',
  );
  const edges = await page.evaluate(
    async (b64) => {
      const image = await createImageBitmap(await (await fetch(`data:image/png;base64,${b64}`)).blob());
      const ctx = new OffscreenCanvas(image.width, image.height).getContext('2d');
      if (ctx === null) {
        return null;
      }
      ctx.drawImage(image, 0, 0);
      const data = ctx.getImageData(0, 0, image.width, image.height).data;
      const dark = (x: number, y: number) => (data[(y * image.width + x) * 4] ?? 255) < 128;
      const columns = Array.from({ length: image.width }, (_, x) => x).filter((x) =>
        Array.from({ length: image.height }, (_, y) => y).some((y) => dark(x, y)),
      );
      const height = (x: number) => Array.from({ length: image.height }, (_, y) => y).filter((y) => dark(x, y)).length;
      const [first, last] = [columns[0] ?? 0, columns[columns.length - 1] ?? 0];
      return { left: height(first + 2), right: height(last - 2) };
    },
    (await page.screenshot()).toString('base64'),
  );
  expect(edges).not.toBeNull();
  const { left, right } = edges ?? { left: 0, right: 0 };
  if (browserName === 'webkit') {
    expect(Math.abs(left - right), `flat in WebKit's screenshot: ${left} / ${right}`).toBeLessThanOrEqual(4);
  } else {
    expect(left - right, `in perspective: ${left} / ${right}`).toBeGreaterThan(50);
  }
});
