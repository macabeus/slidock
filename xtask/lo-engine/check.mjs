// The iris and split clips of the motion probes as LibreOffice's own presentation engine plays them, against what
// slidock plays. Run by `cargo xtask lo-engine`, which exports each probe deck to SVG with the pinned LibreOffice
// (its SVG export embeds the engine, `presentation_engine.js`, and the deck's animations as LibreOffice read them),
// compiles it with slidock (`--emit ir --native-motion`) and bundles the player's clip generators.
//
// usage: node check.mjs <report.json> <clip.mjs> <name> <deck.svg> <presentation-ir.json> [<name> <svg> <ir>]...
//
// Every step whose LibreOffice node holds an `irisWipe`, `ellipseWipe`, `fourBoxWipe` or `barnDoorWipe` filter is a
// row. Its frames, at a quarter, half and three quarters of the step, are sampled on a grid of a square element box:
// LibreOffice's from `ClippingFunctor.perform` (built as `createShapeTransition` builds it for a shape, the clip
// applied with its default nonzero rule), slidock's from the player's clip generator (an exit, or a track played
// `out`, runs it backwards). Each side is read as two facts. Its motion: whether the shape's own centre shows, and
// whether the shown part grows or shrinks. Its shape: the outline the clip moves (box, circle, diamond or plus, or a
// horizontal or vertical band for a split), classified by points at fixed places of the outline's own bounding box
// in its smallest frame. The engine places the box, diamond and barn-door shapes off the element's centre (its
// `IrisWipePath`, `BarnDoorWipePath` and diamond compose their matrices in the other order than LibreOffice's C++),
// so LibreOffice's facts come from the shape's own frame (the clip without its subtraction from the box: its bounding
// box, its centre, its outline, where the four boxes of `fourBoxWipe` frame the plus between them); slidock's shapes
// are centred, and their outline is the shown part when the centre shows, else the hidden part. A row agrees when
// both sides read as the same motion and the same shape; the check fails on any other row, an unaligned slide, or no
// rows. A step LibreOffice reads as its random choice (`ooo-entrance-random`, `ooo-exit-random`) is skipped and
// counted: the engine draws another preset for it each time it loads the deck. slidock must read the same steps as
// its random build; a step only one side reads as random is a problem.
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { isRandomPreset, randomStep } from './rows.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(join(here, '../../packages/player/package.json'));
const { chromium } = require('@playwright/test');

const [reportPath, clipPath, ...deckArgs] = process.argv.slice(2);
if (!reportPath || !clipPath || deckArgs.length === 0 || deckArgs.length % 3 !== 0) {
  console.error('usage: node check.mjs <report.json> <clip.mjs> <name> <deck.svg> <presentation-ir.json>...');
  process.exit(2);
}
const clip = await import(pathToFileURL(clipPath).href);

const TIMES = [0.25, 0.5, 0.75];
const GRID = 40;
const CLIP_TYPES = ['irisWipe', 'ellipseWipe', 'fourBoxWipe', 'barnDoorWipe'];
/** Fine grid for slidock's outline bounding box. */
const FINE = 100;
/** Places in an outline's bounding box (0 to 1 on each axis) that tell the shapes apart. */
const POINTS = {
  corners: [
    [0.04, 0.04],
    [0.96, 0.04],
    [0.04, 0.96],
    [0.96, 0.96],
  ],
  centre: [[0.5, 0.5]],
  // Inside a circle, outside a diamond and a plus.
  round: [
    [0.2, 0.2],
    [0.8, 0.2],
    [0.2, 0.8],
    [0.8, 0.8],
  ],
  // On a plus's arms, outside a diamond.
  arms: [
    [0.3, 0.06],
    [0.06, 0.3],
    [0.7, 0.94],
    [0.94, 0.7],
  ],
  // Inside a diamond.
  diamond: [
    [0.3, 0.3],
    [0.7, 0.7],
  ],
};

/** Which of POINTS an outline holds, `at(u, v)` testing a place of its bounding box. */
function probeOf(at, points, aspect, area) {
  const held = Object.fromEntries(Object.entries(points).map(([key, list]) => [key, list.map(([u, v]) => at(u, v))]));
  return { held, aspect, area };
}

/** The shape an outline probe reads as. */
function shapeOf({ held, aspect }) {
  const all = (key) => held[key].every(Boolean);
  const none = (key) => held[key].every((x) => !x);
  if (all('corners')) {
    if (none('centre')) {
      return 'plus';
    }
    return aspect > 1.5 ? 'horizontal band' : aspect < 1 / 1.5 ? 'vertical band' : 'box';
  }
  if (!none('corners') || !all('centre')) {
    return 'unclear';
  }
  if (all('round')) {
    return 'circle';
  }
  if (all('arms')) {
    return 'plus';
  }
  if (all('diamond') && none('arms')) {
    return 'diamond';
  }
  return 'unclear';
}

/** The shape of the smallest of `frames`' outlines. */
function framesShape(frames) {
  const smallest = frames.reduce((a, b) => (b.outline.area < a.outline.area ? b : a));
  return shapeOf(smallest.outline);
}

/** The shape slidock's reading names. */
function namedShape(reveal) {
  if (reveal.effect === 'split') {
    return reveal.params.orient === 'horz' ? 'horizontal band' : 'vertical band';
  }
  return reveal.params.shape;
}

/** A motion read from frames: whether the shape's centre shows, and how the shown part changes. */
function motion(frames) {
  const centres = new Set(frames.map((f) => f.centre));
  const first = frames[0].shown;
  const last = frames[frames.length - 1].shown;
  const trend = last - first > 0.02 ? 'grows' : first - last > 0.02 ? 'shrinks' : 'holds';
  if (centres.size !== 1 || trend === 'holds') {
    return `unclear (centre ${[...centres].join('/')}, ${trend})`;
  }
  const shape = centres.has(true);
  if (trend === 'grows') {
    return shape ? 'grows from the centre' : 'closes in from the edges';
  }
  return shape ? 'closes to the centre' : 'a hole opens from the centre';
}

// --- slidock's side: the player's clip generators, sampled in Node.

function parsePercent(text) {
  const value = Number.parseFloat(text);
  if (!text.trim().endsWith('%') || Number.isNaN(value)) {
    throw new Error(`not a percentage: ${text}`);
  }
  return value / 100;
}

/** A point test for a CSS `clip-path` of the forms the generators write (`polygon()`, `circle()`). */
function region(css) {
  const polygon = /^polygon\((.*)\)$/s.exec(css);
  if (polygon) {
    const parts = polygon[1].split(',').map((p) => p.trim());
    const evenodd = parts[0] === 'evenodd';
    const points = parts.slice(parts[0] === 'evenodd' || parts[0] === 'nonzero' ? 1 : 0).map((p) => {
      const [x, y] = p.split(/\s+/);
      return [parsePercent(x), parsePercent(y)];
    });
    return (x, y) => {
      let winding = 0;
      let crossings = 0;
      for (let i = 0; i < points.length; i++) {
        const [x0, y0] = points[i];
        const [x1, y1] = points[(i + 1) % points.length];
        if (y0 <= y !== y1 <= y) {
          // From the lower end, so an edge and its way back (a zero-width bridge) cross at the same x and cancel.
          const [[ax, ay], [bx, by]] =
            y0 < y1
              ? [points[i], points[(i + 1) % points.length]]
              : [
                  [x1, y1],
                  [x0, y0],
                ];
          const xi = ax + ((y - ay) / (by - ay)) * (bx - ax);
          if (xi > x) {
            crossings += 1;
            winding += y1 > y0 ? 1 : -1;
          }
        }
      }
      return evenodd ? crossings % 2 === 1 : winding !== 0;
    };
  }
  const circle = /^circle\((\S+) at (\S+) (\S+)\)$/.exec(css);
  if (circle) {
    // A percentage radius is of the box's diagonal over the square root of 2: the side, for a square box.
    const [r, cx, cy] = circle.slice(1).map(parsePercent);
    return (x, y) => (x - cx) ** 2 + (y - cy) ** 2 <= r * r;
  }
  throw new Error(`a clip-path the check does not read: ${css}`);
}

function sampleCss(css) {
  const inside = region(css);
  let shown = 0;
  for (let j = 0; j < GRID; j++) {
    for (let i = 0; i < GRID; i++) {
      shown += inside((i + 0.5) / GRID, (j + 0.5) / GRID) ? 1 : 0;
    }
  }
  const centre = inside(0.5, 0.5);
  // The outline: the shown part when the centre shows, else the hidden part, within the box.
  const outline = (x, y) => x >= 0 && x <= 1 && y >= 0 && y <= 1 && inside(x, y) === centre;
  let [x0, y0, x1, y1] = [1, 1, 0, 0];
  for (let j = 0; j < FINE; j++) {
    for (let i = 0; i < FINE; i++) {
      const [x, y] = [(i + 0.5) / FINE, (j + 0.5) / FINE];
      if (outline(x, y)) {
        [x0, y0, x1, y1] = [Math.min(x0, x), Math.min(y0, y), Math.max(x1, x), Math.max(y1, y)];
      }
    }
  }
  const half = 0.5 / FINE;
  [x0, y0, x1, y1] = [x0 - half, y0 - half, x1 + half, y1 + half];
  const [w, h] = [Math.max(0, x1 - x0), Math.max(0, y1 - y0)];
  return {
    shown: shown / (GRID * GRID),
    centre,
    outline: probeOf((u, v) => outline(x0 + u * w, y0 + v * h), POINTS, w / h, w * h),
  };
}

/** The clip slidock plays for a step, as `{ reveal, backwards }`, or `null` when it plays no iris or split. */
function slidockClip(step) {
  const native = step.effect?.native;
  if (!native) {
    return null;
  }
  if (native.effect === 'iris' || native.effect === 'split') {
    return { reveal: { effect: native.effect, params: native.params }, backwards: step.kind === 'exit' };
  }
  if (native.effect === 'behaviors') {
    const track = (native.params ?? []).find(
      (t) => t.property === 'clipPath' && (t.reveal?.effect === 'iris' || t.reveal?.effect === 'split'),
    );
    return track ? { reveal: track.reveal, backwards: track.transition === 'out' } : null;
  }
  return null;
}

function slidockFrames({ reveal, backwards }) {
  const frames =
    reveal.effect === 'iris'
      ? clip.iris(reveal.params.shape, reveal.params.inOut, 1)
      : clip.split(reveal.params.orient, reveal.params.inOut);
  return TIMES.map((t) => ({ t, ...sampleCss(frames.at(backwards ? 1 - t : t)) }));
}

// --- LibreOffice's side: its engine, in the exported SVG.

/** Runs in the SVG page: every step of every slide, and the frames of each iris or split filter. */
function engineFrames({ times, grid, clipTypes, points }) {
  /* global NSS, aTransitionTypeInMap, aTransitionSubtypeInMap, aTransitionInfoTable, ClippingFunctor,
     createClipPolyPolygon */
  const root = document.documentElement;
  const size = grid * 10;
  const at = (path, x, y) => {
    const point = root.createSVGPoint();
    point.x = x;
    point.y = y;
    return path.isPointInFill(point);
  };
  const frame = (filter, t) => {
    const type = aTransitionTypeInMap[filter.type];
    const subtype = aTransitionSubtypeInMap[filter.subtype ?? 'default'];
    const info = aTransitionInfoTable[type][subtype];
    const make = () => new ClippingFunctor(createClipPolyPolygon(type, subtype), info, !filter.reverse, !filter.out);
    const path = make().perform(t, size, size);
    root.appendChild(path);
    let shown = 0;
    for (let j = 0; j < grid; j++) {
      for (let i = 0; i < grid; i++) {
        shown += at(path, ((i + 0.5) * size) / grid, ((j + 0.5) * size) / grid) ? 1 : 0;
      }
    }
    // The shape's own frame: the same clip without the subtraction from the bounding box.
    const bare = make();
    bare.bSubtractPolygon = false;
    const shape = bare.perform(t, size, size);
    root.appendChild(shape);
    const box = shape.getBBox();
    const held = Object.fromEntries(
      Object.entries(points).map(([key, list]) => [
        key,
        list.map(([u, v]) => at(shape, box.x + u * box.width, box.y + v * box.height)),
      ]),
    );
    shape.remove();
    const centre = at(path, box.x + box.width / 2, box.y + box.height / 2);
    path.remove();
    const outline = { held, aspect: box.width / box.height, area: (box.width * box.height) / (size * size) };
    return { t, shown: shown / (grid * grid), centre, outline, info: { outInvertsSweep: info.outInvertsSweep } };
  };
  const order = [...document.querySelectorAll('g.Slide')].map((g) => g.id).filter((id) => id !== 'dummy-slide');
  const slides = [];
  for (const defs of document.querySelectorAll('defs.Animations')) {
    const index = order.indexOf(defs.getAttributeNS(NSS.ooo, 'slide')) + 1;
    const steps = [];
    for (const node of defs.getElementsByTagNameNS(NSS.anim, '*')) {
      const presetClass = node.getAttributeNS(NSS.presentation, 'preset-class');
      if (!presetClass) {
        continue;
      }
      const step = {
        presetClass,
        presetId: node.getAttributeNS(NSS.presentation, 'preset-id'),
        presetSubtype: node.getAttributeNS(NSS.presentation, 'preset-sub-type'),
      };
      const filters = [...node.getElementsByTagNameNS(NSS.anim, 'transitionFilter')].filter((f) =>
        clipTypes.includes(f.getAttributeNS(NSS.smil, 'type')),
      );
      if (filters.length > 1) {
        step.error = `${filters.length} clip filters`;
      } else if (filters.length === 1) {
        const f = filters[0];
        step.filter = {
          type: f.getAttributeNS(NSS.smil, 'type'),
          subtype: f.getAttributeNS(NSS.smil, 'subtype'),
          reverse: f.getAttributeNS(NSS.smil, 'direction') === 'reverse',
          out: f.getAttributeNS(NSS.smil, 'mode') === 'out',
        };
        step.frames = times.map((t) => frame(step.filter, t));
      }
      steps.push(step);
    }
    slides.push({ index, steps });
  }
  return slides;
}

/** Whether slidock's step is the step LibreOffice read: the same class, and for an ODP preset the same preset. */
function aligned(name, ours, theirs) {
  const kind = { entrance: 'entrance', exit: 'exit', emphasis: 'emphasis', 'motion-path': 'motion' }[
    theirs.presetClass
  ];
  if (kind !== undefined && ours.kind !== kind) {
    return false;
  }
  if (name.endsWith('odp') && theirs.presetId && ours.effect?.id?.startsWith('odp:ooo-')) {
    const id = ours.effect.id.slice('odp:'.length).split('/')[0];
    return id === theirs.presetId;
  }
  return true;
}

const browser = await chromium.launch();
const rows = [];
const problems = [];
const skipped = [];
try {
  for (let k = 0; k < deckArgs.length; k += 3) {
    const [name, svg, irPath] = deckArgs.slice(k, k + 3);
    const ir = JSON.parse(readFileSync(irPath, 'utf8'));
    const page = await browser.newPage();
    await page.route('**/*', (route) => (route.request().url().startsWith('file:') ? route.continue() : route.abort()));
    await page.goto(pathToFileURL(svg).href);
    await page.waitForFunction(() => typeof ClippingFunctor === 'function');
    const slides = await page.evaluate(engineFrames, {
      times: TIMES,
      grid: GRID,
      clipTypes: CLIP_TYPES,
      points: POINTS,
    });
    await page.close();
    for (const slide of slides) {
      const clipped = slide.steps.filter((s) => s.filter || s.error || isRandomPreset(s.presetId));
      if (clipped.length === 0) {
        continue;
      }
      const ours = ir.slides[slide.index - 1]?.timeline ?? [];
      if (ours.length !== slide.steps.length || slide.steps.some((s, i) => !aligned(name, ours[i], s))) {
        problems.push(
          `${name} slide ${slide.index}: slidock has ${ours.length} steps, LibreOffice ${slide.steps.length}`,
        );
        continue;
      }
      slide.steps.forEach((theirs, i) => {
        const step = ours[i];
        const random = randomStep(step, theirs);
        if (random === 'skip') {
          skipped.push(`${name} slide ${slide.index} step ${i + 1}: ${theirs.presetId}`);
          return;
        }
        if (random === 'mismatch') {
          problems.push(
            `${name} slide ${slide.index} step ${i + 1}: LibreOffice reads ${theirs.presetId ?? 'no preset'}, slidock ${step.effect?.id ?? 'no effect'}; only one is a random choice`,
          );
          return;
        }
        if (!theirs.filter && !theirs.error) {
          return;
        }
        const row = {
          deck: name,
          slide: slide.index,
          step: i + 1,
          kind: step.kind,
          native: step.effect?.id ?? null,
          libreoffice: theirs.filter ?? theirs.error,
          slidock: null,
          libreofficeMotion: theirs.error ?? motion(theirs.frames),
          libreofficeShape: theirs.frames ? framesShape(theirs.frames) : 'unclear',
          slidockMotion: 'another effect',
          slidockShape: null,
          libreofficeFrames: theirs.frames ?? [],
          slidockFrames: [],
        };
        const ourClip = slidockClip(step);
        if (ourClip) {
          row.slidock = { ...ourClip.reveal, backwards: ourClip.backwards };
          row.slidockFrames = slidockFrames(ourClip);
          row.slidockMotion = motion(row.slidockFrames);
          row.slidockShape = framesShape(row.slidockFrames);
          // The frames hold what the reading names.
          if (row.slidockShape !== namedShape(ourClip.reveal)) {
            problems.push(
              `${name} slide ${slide.index} step ${i + 1}: slidock's frames read as ${row.slidockShape}, its reading names ${namedShape(ourClip.reveal)}`,
            );
          }
        }
        row.agrees =
          row.libreofficeMotion === row.slidockMotion &&
          !row.libreofficeMotion.startsWith('unclear') &&
          row.libreofficeShape === row.slidockShape &&
          row.libreofficeShape !== 'unclear';
        rows.push(row);
      });
    }
  }
} finally {
  await browser.close();
}

const filterText = (f) =>
  typeof f === 'string' ? f : `${f.type} ${f.subtype}${f.reverse ? ' reverse' : ''}${f.out ? ' out' : ''}`;
const readText = (s) =>
  s === null ? '-' : `${s.effect}{${Object.values(s.params).join(', ')}}${s.backwards ? ' backwards' : ''}`;
for (const row of rows) {
  console.log(
    [
      row.agrees ? 'agree' : 'DIFFER',
      `${row.deck} slide ${row.slide} step ${row.step}`,
      row.kind,
      row.native,
      `LibreOffice ${filterText(row.libreoffice)}: ${row.libreofficeShape}, ${row.libreofficeMotion}`,
      `slidock ${readText(row.slidock)}: ${row.slidockShape ?? '-'}, ${row.slidockMotion}`,
    ].join(' | '),
  );
}
for (const step of skipped) {
  console.log(`skip | ${step} | LibreOffice draws a random preset at each run`);
}
for (const problem of problems) {
  console.log(`PROBLEM | ${problem}`);
}
const tally = {};
for (const row of rows) {
  const key = `${row.deck} ${row.kind}`;
  tally[key] ??= { agree: 0, differ: 0 };
  tally[key][row.agrees ? 'agree' : 'differ'] += 1;
}
for (const [key, { agree, differ }] of Object.entries(tally)) {
  console.log(`${key}: ${agree} agree, ${differ} differ`);
}
console.log(`skipped: ${skipped.length} random steps`);
writeFileSync(reportPath, `${JSON.stringify({ rows, skipped, problems }, null, 2)}\n`);
const failed = rows.length === 0 || problems.length > 0 || rows.some((row) => !row.agrees);
process.exit(failed ? 1 : 0);
