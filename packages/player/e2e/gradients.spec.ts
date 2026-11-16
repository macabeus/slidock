// Gradients as the HTML backend prints them, measured against their analytic models: a deck of
// lowered gradients of our own (IR written here, compiled with the CLI's IR frontend), each region rendered still and
// scored per pixel in 0–255, its outer 2 px left out. The constructions measured:
// a focal radial inside its circle (A) and at the 0.98 a focus outside is moved to (B), an elliptical focal
// radial (I), rect contours as two ramps in one translated pattern, on a rectangle and on an ellipse (C), ellipse
// bands (between their edges), an sRGB → P3 linear blended in P3 as Keynote does, in SVG and in a table cell's CSS
// (D, E), and an alpha linear over white, unpremultiplied, in SVG and in CSS (G). Each project records its numbers in
// `target/player-e2e/gradients-<project>.json` and is held to its bounds (`BOUNDS`).
import { type Page, expect, test } from '@playwright/test';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';

import { OUT, REPO, cli, compile, readDeckPage } from './support/cli';
import { open, shot } from './support/harness';

test.describe.configure({ mode: 'serial', timeout: 5 * 60_000 });

// Real Chrome for the Chromium project, colour-managed to sRGB as the bench's renderer is.
test.use({
  channel: [
    async ({ browserName }, use) => use(browserName === 'chromium' ? 'chrome' : undefined),
    { scope: 'worker' },
  ],
  launchOptions: [
    async ({ browserName }, use) =>
      use(browserName === 'chromium' ? { args: ['--disable-lcd-text', '--force-color-profile=srgb'] } : {}),
    { scope: 'worker' },
  ],
});

let binary = '';
test.beforeAll(() => {
  test.setTimeout(30 * 60_000);
  binary = cli();
});

type Rgb = [number, number, number];
type Rgba = [number, number, number, number];

const W = 600;
const srgb = (r: number, g: number, b: number, a = 1) => ({ kind: 'rgb', space: 'srgb', r, g, b, a });
const p3 = (r: number, g: number, b: number, a = 1) => ({ kind: 'rgb', space: 'p3', r, g, b, a });
const BLACK_WHITE = [
  { pos: 0, color: srgb(0, 0, 0) },
  { pos: 1, color: srgb(1, 1, 1) },
];
const q5 = (x: number) => Math.round(x * 1e5) / 1e5;
const frame = (x: number, y: number, w: number, h: number) => ({ x, y, w, h, rotation: 0, flipH: false, flipV: false });

function shape(id: string, at: ReturnType<typeof frame>, fill: unknown, preset = 'rect') {
  return {
    id,
    origin: { kind: 'agent' },
    kind: 'shape',
    frame: at,
    geometry: { kind: 'preset', name: preset, adjust: {} },
    fill,
  };
}

// The design's cases, in IR.
/** A: a real circle around the box through its corners, its focus at (0.111, 0.6). */
const R = Math.hypot(W, 200) / 2;
const A = {
  kind: 'radial',
  center: { x: 0.5, y: 0.5 },
  radius: q5(R / W),
  radiusY: q5(R / 200),
  focus: { x: 0.111, y: 0.6 },
};
/** B: the focus (−100, 100) pt, outside the circle, where lowering moves it: 0.98 of the way along its ray. */
const B_FOCUS = (() => {
  const [dx, dy] = [(-100 - W / 2) / R, (100 - 100) / R];
  const k = 0.98 / Math.hypot(dx, dy);
  return { x: q5(0.5 + (dx * k * R) / W), y: q5(0.5 + (dy * k * R) / 200) };
})();
const B = { ...A, focus: B_FOCUS };
/** I: an ellipse rx 0.3 W, ry 0.6 H about (0.4, 0.5), its focus at (0.3, 0.5). */
const I = { kind: 'radial', center: { x: 0.4, y: 0.5 }, radius: 0.3, radiusY: 0.6, focus: { x: 0.3, y: 0.5 } };
/** C: PowerPoint's rect path with fillToRect l10 t30 r80 b20. */
const C = {
  kind: 'rect',
  origin: { x: 0.5, y: 0.5 },
  axisX: { x: 0.5, y: 0 },
  axisY: { x: 0, y: 0.5 },
  inner: { l: -0.8, t: -0.4, r: -0.6, b: 0.6 },
};
/** Ellipse bands: inner (0.5, 0), 8 bands. */
const BANDS = 8;
const E = {
  kind: 'ellipse',
  origin: { x: 0.5, y: 0.5 },
  axisX: { x: 0.5, y: 0 },
  axisY: { x: 0, y: 0.5 },
  inner: { rx: 0.5, ry: 0 },
  steps: BANDS,
};
/** D, E: retro's two colours, an sRGB stop and a P3 one. */
const RETRO: [Rgb, Rgb] = [
  [0.3376, 0.7577, 1],
  [0.0823, 0.2228, 0.5373],
];
const RETRO_STOPS = [
  { pos: 0, color: srgb(...RETRO[0]) },
  { pos: 1, color: p3(...RETRO[1]) },
];
/** G: 20 % green to opaque magenta. */
const ALPHA: [Rgba, Rgba] = [
  [0, 1, 0, 0.2],
  [1, 0, 1, 1],
];
const ALPHA_STOPS = [
  { pos: 0, color: srgb(...ALPHA[0]) },
  { pos: 1, color: srgb(...ALPHA[1]) },
];
const HORIZONTAL = { kind: 'linear', from: { x: 0, y: 0.5 }, to: { x: 1, y: 0.5 } };

function table(id: string, at: ReturnType<typeof frame>, rows: { height: number; fill: unknown }[]) {
  return {
    id,
    origin: { kind: 'agent' },
    kind: 'table',
    frame: at,
    columns: [{ width: at.w }],
    rows: rows.map((row) => ({ height: row.height, cells: [{ text: { body: {}, paragraphs: [] }, fill: row.fill }] })),
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
  const slide = (id: string, elements: unknown[]) => ({
    id,
    origin: { kind: 'agent' },
    hidden: false,
    showLayout: true,
    timeline: [],
    background: { kind: 'solid', color: srgb(1, 1, 1) },
    elements,
  });
  const box = (y: number) => frame(20, y, W, 200);
  return {
    $schema: 'urn:slidock:presentation-ir:1',
    size: { w: 960, h: 540 },
    defaults: {
      body: {
        anchor: 'top',
        autofit: { kind: 'none' },
        insets: { b: 0, l: 0, r: 0, t: 0 },
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
      slide('gr-1', [
        shape('gr-1.a', box(20), { ...A, stops: BLACK_WHITE }),
        shape('gr-1.i', box(260), { ...I, stops: BLACK_WHITE }),
      ]),
      slide('gr-2', [
        shape('gr-2.b', box(20), { ...B, stops: BLACK_WHITE }),
        shape('gr-2.c', box(260), { ...C, stops: BLACK_WHITE }),
      ]),
      slide('gr-3', [
        shape('gr-3.c', box(20), { ...C, stops: BLACK_WHITE }, 'ellipse'),
        shape('gr-3.e', box(260), { ...E, stops: BLACK_WHITE }),
      ]),
      slide('gr-4', [
        shape('gr-4.d', frame(20, 20, W, 60), { ...HORIZONTAL, stops: RETRO_STOPS }),
        shape('gr-4.g', frame(20, 100, W, 40), { ...HORIZONTAL, stops: ALPHA_STOPS }),
        table('gr-4.t', frame(20, 180, W, 100), [
          { height: 60, fill: { kind: 'linear', angle: 0, stops: RETRO_STOPS } },
          { height: 40, fill: { kind: 'linear', angle: 0, stops: ALPHA_STOPS } },
        ]),
      ]),
    ],
  };
}

// Colour (sRGB transfer, CSS Color 4 matrices).
const dec = (c: number) => (Math.abs(c) <= 0.04045 ? c / 12.92 : Math.sign(c) * ((Math.abs(c) + 0.055) / 1.055) ** 2.4);
const enc = (c: number) =>
  Math.abs(c) <= 0.0031308 ? 12.92 * c : Math.sign(c) * (1.055 * Math.abs(c) ** (1 / 2.4) - 0.055);
type M = [Rgb, Rgb, Rgb];
const mul = (m: M, v: Rgb): Rgb => m.map((row) => row[0] * v[0] + row[1] * v[1] + row[2] * v[2]) as Rgb;
const SRGB_XYZ: M = [
  [506752 / 1228815, 87881 / 245763, 12673 / 70218],
  [87098 / 409605, 175762 / 245763, 12673 / 175545],
  [7918 / 409605, 87881 / 737289, 1001167 / 1053270],
];
const XYZ_SRGB: M = [
  [12831 / 3959, -329 / 214, -1974 / 3959],
  [-851781 / 878810, 1648619 / 878810, 36519 / 878810],
  [705 / 12673, -2585 / 12673, 705 / 667],
];
const P3_XYZ: M = [
  [608311 / 1250200, 189793 / 714400, 198249 / 1000160],
  [35783 / 156275, 247089 / 357200, 198249 / 2500400],
  [0, 32229 / 714400, 5220557 / 5000800],
];
const XYZ_P3: M = [
  [446124 / 178915, -333277 / 357830, -72051 / 178915],
  [-14852 / 17905, 63121 / 35810, 423 / 17905],
  [11844 / 330415, -50337 / 660830, 316169 / 330415],
];
const p3ToSrgb = (c: Rgb): Rgb => mul(XYZ_SRGB, mul(P3_XYZ, c.map(dec) as Rgb)).map(enc) as Rgb;
const srgbToP3 = (c: Rgb): Rgb => mul(XYZ_P3, mul(SRGB_XYZ, c.map(dec) as Rgb)).map(enc) as Rgb;

// Models: each gives a pixel's expected sRGB in 0–255 at a point of its box (pt), or null to leave it out.
type Model = (x: number, y: number, w: number, h: number) => Rgb | null;
const grey = (t: number | null): Rgb | null => {
  if (t === null) {
    return null;
  }
  const v = 255 * Math.min(Math.max(t, 0), 1);
  return [v, v, v];
};

/** The circle family lerped from the focus f to the end circle (centre c, radius r): t with |p − (f + t(c − f))| = t·r. */
function focal(c: [number, number], r: number, f: [number, number]) {
  return (x: number, y: number): number | null => {
    const [dx, dy] = [x - f[0], y - f[1]];
    const [ex, ey] = [c[0] - f[0], c[1] - f[1]];
    const a = ex * ex + ey * ey - r * r;
    const b = -2 * (dx * ex + dy * ey);
    const k = dx * dx + dy * dy;
    if (Math.abs(a) < 1e-12) {
      return b === 0 ? null : -k / b;
    }
    const disc = b * b - 4 * a * k;
    if (disc < 0) {
      return null;
    }
    const roots = [1, -1].map((s) => (-b + s * Math.sqrt(disc)) / (2 * a)).filter((t) => t >= 0);
    return roots.length ? Math.max(...roots) : null;
  };
}

/** A lowered radial: the unit circle mapped onto the ellipse, so t is the focal model in its units. */
function radialModel(g: typeof A): Model {
  return (x, y, w, h) => {
    const rx = g.radius * w;
    const ry = g.radiusY * h;
    const [cx, cy] = [g.center.x * w, g.center.y * h];
    const f: [number, number] = [(g.focus.x * w - cx) / rx, (g.focus.y * h - cy) / ry];
    return grey(focal([0, 0], 1, f)((x - cx) / rx, (y - cy) / ry));
  };
}

/** t = max(tx, ty) past the inner rectangle (box fractions l10 t30 r80 b20). */
const rectModel: Model = (x, y, w, h) => {
  const [il, it, ir, ib] = [0.1 * w, 0.3 * h, 0.2 * w, 0.8 * h];
  const tx = x < il ? (il - x) / il : x > ir ? (x - ir) / (w - ir) : 0;
  const ty = y < it ? (it - y) / it : y > ib ? (y - ib) / (h - ib) : 0;
  return grey(Math.max(tx, ty));
};

/** Inside the inscribed ellipse only (95 % of it), the rect model. */
const rectInEllipse: Model = (x, y, w, h) => {
  const [u, v] = [(x - w / 2) / (w / 2), (y - h / 2) / (h / 2)];
  return u * u + v * v < 0.95 ? rectModel(x, y, w, h) : null;
};

/** The contour parameter s of a point in gradient space: the ellipse with semi-axes lerp(inner, 1, s) through it. */
function contour(gx: number, gy: number): number {
  const inside = (s: number) => {
    const [a, b] = [E.inner.rx + s * (1 - E.inner.rx), E.inner.ry + s * (1 - E.inner.ry)];
    return a > 0 && b > 0 && (gx / a) ** 2 + (gy / b) ** 2 <= 1;
  };
  if (!inside(1)) {
    return 1.5;
  }
  let [lo, hi] = [0, 1];
  for (let i = 0; i < 40; i++) {
    const mid = (lo + hi) / 2;
    if (inside(mid)) {
      hi = mid;
    } else {
      lo = mid;
    }
  }
  return hi;
}

/** Flat bands: band k of s ∈ [k/8, (k + 1)/8] in the grey k/7, white past the outer ellipse; within 1.5 pt of a band
 * edge (its antialiasing) left out. */
const bandsModel: Model = (x, y, w, h) => {
  const band = (px: number, py: number) => {
    const s = contour((px / w - 0.5) / 0.5, (py / h - 0.5) / 0.5);
    return s >= 1 ? BANDS : Math.min(Math.floor(s * BANDS), BANDS - 1);
  };
  const corners = [
    [-0.5, -0.5],
    [0.5, -0.5],
    [-0.5, 0.5],
    [0.5, 0.5],
  ].map(([dx, dy]) => band(x + (dx ?? 0) * 3, y + (dy ?? 0) * 3));
  if (new Set(corners).size > 1) {
    return null;
  }
  const k = corners[0] ?? 0;
  return grey(k >= BANDS ? 1 : k / (BANDS - 1));
};

/** Keynote's rule: the stops blended in Display P3. */
const p3Model: Model = (x, _y, w) => {
  const t = x / w;
  const [a, b] = [srgbToP3(RETRO[0]), RETRO[1]];
  const mixed = p3ToSrgb([0, 1, 2].map((i) => (a[i] ?? 0) + ((b[i] ?? 0) - (a[i] ?? 0)) * t) as Rgb);
  return mixed.map((c) => 255 * Math.min(Math.max(c, 0), 1)) as Rgb;
};

/** Colour and alpha blended separately (unpremultiplied), over white. */
const alphaModel: Model = (x, _y, w) => {
  const t = x / w;
  const [c0, c1] = ALPHA;
  const c = [0, 1, 2, 3].map((i) => (c0[i] ?? 0) + ((c1[i] ?? 0) - (c0[i] ?? 0)) * t);
  const a = c[3] ?? 1;
  return [0, 1, 2].map((i) => 255 * (a * (c[i] ?? 0) + (1 - a))) as Rgb;
};

interface Score {
  n: number;
  mean: number;
  max: number;
  /** Where the largest error is, in the box's points. */
  at: [number, number];
}

/** A region to score: an element's box, or a band of it from `top` (pt) down `h` pt; the model works in that box. */
interface Probe {
  name: string;
  id: string;
  h: number;
  top?: number;
  model: Model;
}

/** Scores the probes of a still render of `slide`. */
async function measure(page: Page, slide: number, probes: Probe[]): Promise<Record<string, Score>> {
  type Sd = { sd: { showStatic(s: number, t: number): void; hideStatic(): void } };
  await page.evaluate((s) => (window as unknown as Sd).sd.showStatic(s, 0), slide);
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const image = await shot(page);
  const out: Record<string, Score> = {};
  for (const probe of probes) {
    const box = await page.locator(`#static [data-el="${probe.id}"]`).boundingBox();
    if (box === null) {
      throw new Error(`no ${probe.id}`);
    }
    const scale = box.width / W;
    const [x, y] = [box.x, box.y + (probe.top ?? 0) * scale];
    const [width, height] = [Math.round(W * scale), Math.round(probe.h * scale)];
    const pixels = await page.evaluate(
      async ([b64, px, py, bw, bh]) => {
        const blob = await (await fetch(`data:image/png;base64,${b64}`)).blob();
        const bitmap = await createImageBitmap(blob, { colorSpaceConversion: 'none' });
        const ctx = new OffscreenCanvas(bitmap.width, bitmap.height).getContext('2d', { colorSpace: 'srgb' });
        if (ctx === null) {
          return [];
        }
        ctx.drawImage(bitmap, 0, 0);
        return Array.from(ctx.getImageData(px, py, bw, bh).data);
      },
      [image, Math.round(x), Math.round(y), width, height] as const,
    );
    let [n, sum, max] = [0, 0, 0];
    let at: [number, number] = [0, 0];
    for (let py = 2; py < height - 2; py++) {
      for (let px = 2; px < width - 2; px++) {
        // The pixel's centre in the box's points: the region starts at the box's rounded corner.
        const [mx, my] = [(Math.round(x) + px + 0.5 - x) / scale, (Math.round(y) + py + 0.5 - y) / scale];
        const want = probe.model(mx, my, W, probe.h);
        if (want === null) {
          continue;
        }
        const i = (py * width + px) * 4;
        const got = [pixels[i] ?? 0, pixels[i + 1] ?? 0, pixels[i + 2] ?? 0];
        const error = Math.max(...got.map((g, c) => Math.abs(g - (want[c] ?? 0))));
        n += 1;
        sum += error;
        if (error > max) {
          [max, at] = [error, [Math.round(mx * 10) / 10, Math.round(my * 10) / 10]];
        }
      }
    }
    const round = (v: number) => Math.round(v * 1000) / 1000;
    out[probe.name] = { n, mean: round(sum / Math.max(n, 1)), max: round(max), at };
  }
  await page.evaluate(() => (window as unknown as Sd).sd.hideStatic());
  return out;
}

test('printed gradients match their analytic models', async ({ page }, info) => {
  mkdirSync(OUT, { recursive: true });
  const input = join(mkdtempSync(join(OUT, 'gradients-ir-')), 'gradients.presentation-ir.json');
  writeFileSync(input, JSON.stringify(presentation()));
  const dir = compile(binary, relative(REPO, input), 'gradients');
  const { deck, slides } = await readDeckPage(page, readFileSync(join(dir, 'index.html'), 'utf8'));
  // The printed forms the scores rest on.
  expect.soft(slides['gr-2']).toMatch(/<pattern id="g-gr-2\.c" patternUnits="userSpaceOnUse"/);
  expect.soft(slides['gr-4']).not.toContain('display-p3');
  await open(page, { deck, slides });

  const scores: Record<string, Score> = {
    ...(await measure(page, 0, [
      { name: 'A focal radial', id: 'gr-1.a', h: 200, model: radialModel(A) },
      { name: 'I elliptical focal radial', id: 'gr-1.i', h: 200, model: radialModel(I) },
    ])),
    ...(await measure(page, 1, [
      { name: 'B focus moved inside', id: 'gr-2.b', h: 200, model: radialModel(B) },
      { name: 'C rect contours', id: 'gr-2.c', h: 200, model: rectModel },
    ])),
    ...(await measure(page, 2, [
      { name: 'C rect contours on an ellipse', id: 'gr-3.c', h: 200, model: rectInEllipse },
      { name: 'ellipse bands', id: 'gr-3.e', h: 200, model: bandsModel },
    ])),
    ...(await measure(page, 3, [
      { name: 'D sRGB to P3, SVG', id: 'gr-4.d', h: 60, model: p3Model },
      { name: 'G alpha, SVG', id: 'gr-4.g', h: 40, model: alphaModel },
      { name: 'E sRGB to P3, CSS cell', id: 'gr-4.t', h: 60, model: p3Model },
      { name: 'G alpha, CSS cell', id: 'gr-4.t', top: 60, h: 40, model: alphaModel },
    ])),
  };

  const project = info.project.name;
  const browser = `${page.context().browser()?.browserType().name()} ${page.context().browser()?.version()}`;
  const record = { browser, scores };
  writeFileSync(join(OUT, '..', `gradients-${project}.json`), `${JSON.stringify(record, null, 2)}\n`);
  console.log(project, JSON.stringify(record, null, 2));
  const bounds = BOUNDS[project];
  expect(bounds, `bounds for the ${project} project`).toBeDefined();
  for (const [name, s] of Object.entries(scores)) {
    const [mean, max] = bounds?.cases[name] ?? bounds?.all ?? [0, 0];
    expect(s.n, name).toBeGreaterThan(1000);
    expect.soft(s.mean, `${name} mean`).toBeLessThanOrEqual(mean);
    expect.soft(s.max, `${name} max`).toBeLessThanOrEqual(max);
  }
});

/**
 * Mean and max error (0–255) per project, every project the suite runs: the design's mean ≤ 1, max ≤ 2, and where a
 * construction is measured above it, its own. A focus at 0.98 of the way to its ellipse ramps steeply at the box's
 * corner beside it (B: 3.71 in Chromium and Docker WebKit, 4.12 in WebKit, 3.62 in Firefox); the two rect ramps
 * differ on their mitre lines, where they meet (C: 2.66 in Chromium, 2.49 in Docker WebKit, 3.19 in Firefox on the
 * ellipse). WebKit's own bound is its measured worst (2.24 for I, 2.03 for the CSS alpha ramp) with a margin.
 */
const BOUNDS: Record<string, { all: [number, number]; cases: Record<string, [number, number]> }> = {
  chromium: {
    all: [1, 2],
    cases: {
      'B focus moved inside': [1, 4],
      'C rect contours': [1, 3],
      'C rect contours on an ellipse': [1, 3],
    },
  },
  webkit: { all: [1, 2.5], cases: { 'B focus moved inside': [1, 4.5] } },
  firefox: {
    all: [1, 2],
    cases: {
      'B focus moved inside': [1, 4],
      'C rect contours': [1, 3],
      'C rect contours on an ellipse': [1, 3.5],
    },
  },
  'webkit-docker': {
    all: [1, 2],
    cases: {
      'B focus moved inside': [1, 4],
      'C rect contours': [1, 3],
      'C rect contours on an ellipse': [1, 3],
    },
  },
};
