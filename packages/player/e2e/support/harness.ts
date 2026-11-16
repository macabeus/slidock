// The page side of the effect suite: mounts the built player (`dist/player.iife.js`) on a fixture deck, finds every
// animation (shadow roots included), pauses and seeks them together, and compares screenshots pixel by pixel.
import type { Frame, Locator, Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { ALLOWED_KEYFRAME_KEYS } from '../../src/effects/properties';
import { type Fixture, HEIGHT, WIDTH } from './deck';

const PLAYER = fileURLToPath(new URL('../../dist/player.iife.js', import.meta.url));
const PRESENTER = fileURLToPath(new URL('../../dist/presenter.iife.js', import.meta.url));

/** Keys `getKeyframes()` adds beside the animated properties. */
const COMPUTED_KEYS = ['computedOffset'];

const HARNESS = `
window.sd = (() => {
  const roots = () => {
    const out = [document];
    for (const layer of document.querySelectorAll('.sd-layer')) {
      if (layer.shadowRoot) out.push(layer.shadowRoot);
    }
    return out;
  };
  const animations = () => {
    const seen = new Set();
    for (const root of roots()) {
      for (const a of root.getAnimations()) seen.add(a);
    }
    return [...seen];
  };
  // An endless repeat counts one round toward the group's length.
  const finiteEnd = (a) => {
    if (!a.effect) return 0;
    const end = a.effect.getComputedTiming().endTime;
    if (Number.isFinite(end)) return end;
    const timing = a.effect.getTiming();
    return timing.delay + timing.duration;
  };
  let paused = [];
  return {
    diagnostics: [],
    player: null,
    mount(fixture, options) {
      document.body.style.margin = '0';
      const host = document.createElement('div');
      host.id = 'host';
      host.style.cssText = 'position:fixed;left:0;top:0;width:${WIDTH}px;height:${HEIGHT}px';
      document.body.append(host);
      const media = fixture.mediaBase ? (path) => fixture.mediaBase + path : undefined;
      const source = Slidock.htmlDeck(document, fixture.deck, new Map(Object.entries(fixture.slides)), media);
      this.source = source;
      this.player = Slidock.createPlayer(host, source, Object.assign({ mode: 'present', keyboard: 'off', pointer: 'off', chrome: false }, options));
      this.player.on('diagnostic', (d) => this.diagnostics.push(d));
      this.events = [];
      this.player.on('position', (e) => this.events.push({ type: 'position', t: performance.now(), slide: e.position.slide, step: e.position.step }));
      this.player.on('ready', (e) => this.events.push({ type: 'ready', t: performance.now(), slide: e.slide, step: -1 }));
      return true;
    },
    showStatic(slide, step) {
      this.hideStatic();
      const host = document.createElement('div');
      host.id = 'static';
      host.style.cssText = 'position:fixed;left:0;top:0;width:${WIDTH}px;height:${HEIGHT}px;z-index:10;background:#000';
      document.body.append(host);
      this.disposeStatic = this.player.renderStatic(slide, step, host);
    },
    hideStatic() {
      if (this.disposeStatic) this.disposeStatic();
      this.disposeStatic = null;
      document.getElementById('static')?.remove();
    },
    // The animations still to end: an endless repeat runs while its slide shows, until a click cuts it.
    count() {
      return animations().filter(
        (a) => a.playState === 'running' && Number.isFinite(a.effect ? a.effect.getComputedTiming().endTime : 0),
      ).length;
    },
    pause() {
      paused = animations().filter((a) => a.playState !== 'finished' || a.effect.getComputedTiming().endTime > 0);
      for (const a of paused) a.pause();
      return paused.length;
    },
    seek(fraction) {
      const total = Math.max(0, ...paused.map(finiteEnd));
      for (const a of paused) a.currentTime = fraction * total;
      return total;
    },
    seekMs(ms) {
      for (const a of paused) a.currentTime = ms;
    },
    // Cuts every endless repeat as a click does: it ends with its round (forward then back when it alternates) and
    // holds there, until the player cancels it. As the player holds a cut repeat (by its state), the hold is a still
    // frame of that end made after a style read with nothing animating, and an end that shows what the element shows
    // without it holds nothing (the motion lane's \`cutLoop\`).
    cut() {
      for (const a of animations()) {
        if (!a.effect || Number.isFinite(a.effect.getComputedTiming().endTime)) continue;
        const effect = a.effect;
        const direction = effect.getTiming().direction;
        effect.updateTiming({ iterations: direction === 'alternate' || direction === 'alternate-reverse' ? 2 : 1 });
        a.finish();
        const target = effect.target;
        if (!target) continue;
        const keys = new Set();
        for (const keyframe of effect.getKeyframes()) {
          for (const key of Object.keys(keyframe)) {
            if (!['offset', 'computedOffset', 'easing', 'composite'].includes(key)) keys.add(key);
          }
        }
        const read = () => {
          const style = getComputedStyle(target);
          return Object.fromEntries(
            [...keys].map((k) => [k, k.startsWith('--') ? style.getPropertyValue(k).trim() : String(style[k])]),
          );
        };
        const frame = read();
        a.cancel();
        const rest = read();
        if ([...keys].some((k) => frame[k] !== rest[k])) {
          a.effect = new KeyframeEffect(target, [frame], { duration: 0, fill: 'forwards' });
          a.finish();
        }
      }
    },
    // Finishes what can end; an endless repeat plays on.
    resume() {
      for (const a of paused) {
        const end = a.effect ? a.effect.getComputedTiming().endTime : 0;
        if (Number.isFinite(end)) a.finish(); else a.play();
      }
      paused = [];
    },
    keyframeKeys() {
      const keys = new Set();
      for (const a of animations()) {
        for (const frame of a.effect.getKeyframes()) for (const key of Object.keys(frame)) keys.add(key);
      }
      return [...keys];
    },
    target(id) {
      for (const layer of document.querySelectorAll('.sd-layer[data-sd-current]')) {
        const el = [...layer.shadowRoot.querySelectorAll('[data-el]')].find((e) => e.getAttribute('data-el') === id);
        if (el) return el;
      }
      return null;
    },
    incoming() {
      const layer = document.querySelector('.sd-layer[data-sd-incoming]');
      if (!layer) return null;
      const cs = getComputedStyle(layer);
      return { opacity: cs.opacity, transform: cs.transform, translate: cs.translate };
    },
    style(id) {
      const el = this.target(id);
      if (!el) return null;
      const cs = getComputedStyle(el);
      return { opacity: cs.opacity, transform: cs.transform, translate: cs.translate, rotate: cs.rotate, scale: cs.scale, color: cs.color, clipPath: cs.clipPath, maskImage: cs.maskImage || cs.webkitMaskImage, filter: cs.filter, visibility: cs.visibility };
    },
    // Where a middle frame shows what moved. \`differ\`: \`full\` is where the shown and hidden states differ, \`seen\`
    // where the frame differs from the hidden state. \`color\` (\`[r, g, b]\`): \`full\` is where the shown state has
    // that colour, \`seen\` where the frame has it. Each region has its pixel count, centroid, bounding box, and mean
    // distance (overall, along x, along y) from the full region's centroid.
    // The quad a target covers at rest and the quad its computed transform projects it to now, in viewport px, when
    // that transform is projective (it carries perspective) and the target is opaque with no other transform; else
    // null. \`flat\` is the quad without the perspective divide (the transform's affine part). \`id\` null names the
    // incoming layer of a transition.
    projected(id) {
      const el = id === null ? document.querySelector('.sd-layer[data-sd-incoming]') : this.target(id);
      const parent = el && el.offsetParent;
      if (!el || !parent) return null;
      const cs = getComputedStyle(el);
      if (cs.transform === 'none' || cs.translate !== 'none' || cs.rotate !== 'none' || cs.scale !== 'none') return null;
      if (Number(cs.opacity) !== 1) return null;
      const m = new DOMMatrix(cs.transform);
      if (m.m14 === 0 && m.m24 === 0) return null;
      const box = parent.getBoundingClientRect();
      const k = box.width / parent.offsetWidth;
      const left = box.left + (parent.clientLeft + el.offsetLeft) * k;
      const top = box.top + (parent.clientTop + el.offsetTop) * k;
      const [ox, oy] = cs.transformOrigin.split(' ').map(parseFloat);
      const corners = [[0, 0], [el.offsetWidth, 0], [el.offsetWidth, el.offsetHeight], [0, el.offsetHeight]];
      const now = [];
      const flat = [];
      for (const [x, y] of corners) {
        const p = m.transformPoint(new DOMPoint(x - ox, y - oy, 0, 1));
        if (!(p.w > 0)) return null;
        now.push([left + (p.x / p.w + ox) * k, top + (p.y / p.w + oy) * k]);
        flat.push([left + (p.x + ox) * k, top + (p.y + oy) * k]);
      }
      return { rest: corners.map(([x, y]) => [left + x * k, top + y * k]), now, flat };
    },
    // How well the pixels of \`color\` (within \`tolerance\` per channel) in a frame cover the projected quad and the
    // flat one: each region's intersection over union with them, and the two quads' own. \`view\` (\`[x0, y0, x1, y1]\`)
    // is where the viewport sits in the frame: the quads are cut to it, as the page is.
    async fit(frame, quads, color, tolerance, view) {
      const image = await createImageBitmap(await (await fetch('data:image/png;base64,' + frame)).blob());
      const { width, height } = image;
      const ctx = new OffscreenCanvas(width, height).getContext('2d', { willReadFrequently: true });
      ctx.drawImage(image, 0, 0);
      const px = ctx.getImageData(0, 0, width, height).data;
      const mask = (quad) => {
        ctx.clearRect(0, 0, width, height);
        ctx.save();
        if (view) {
          ctx.beginPath();
          ctx.rect(view[0], view[1], view[2] - view[0], view[3] - view[1]);
          ctx.clip();
        }
        ctx.beginPath();
        quad.forEach(([x, y], i) => (i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y)));
        ctx.closePath();
        ctx.fill();
        ctx.restore();
        return ctx.getImageData(0, 0, width, height).data;
      };
      const [now, flat] = [mask(quads.now), mask(quads.flat)];
      let count = 0, nowBoth = 0, nowAny = 0, flatBoth = 0, flatAny = 0, quadsBoth = 0, quadsAny = 0;
      for (let i = 0; i < px.length; i += 4) {
        const hit = Math.max(Math.abs(px[i] - color[0]), Math.abs(px[i + 1] - color[1]), Math.abs(px[i + 2] - color[2])) <= tolerance;
        const inNow = now[i + 3] >= 128, inFlat = flat[i + 3] >= 128;
        if (hit) count++;
        if (hit && inNow) nowBoth++;
        if (hit || inNow) nowAny++;
        if (hit && inFlat) flatBoth++;
        if (hit || inFlat) flatAny++;
        if (inNow && inFlat) quadsBoth++;
        if (inNow || inFlat) quadsAny++;
      }
      const iou = (both, any) => (any ? both / any : 0);
      return { count, now: iou(nowBoth, nowAny), flat: iou(flatBoth, flatAny), quads: iou(quadsBoth, quadsAny) };
    },
    async motion(frame, hidden, shown, mode, moved, color, quads) {
      const load = async (b64) => createImageBitmap(await (await fetch('data:image/png;base64,' + b64)).blob());
      const images = await Promise.all([load(frame), load(hidden), load(shown)]);
      const { width, height } = images[0];
      const canvas = new OffscreenCanvas(width, height);
      const ctx = canvas.getContext('2d');
      const [m, h, s] = images.map((image) => {
        ctx.clearRect(0, 0, width, height);
        ctx.drawImage(image, 0, 0);
        return ctx.getImageData(0, 0, width, height).data;
      });
      const delta = (a, b, i) => Math.max(Math.abs(a[i] - b[i]), Math.abs(a[i + 1] - b[i + 1]), Math.abs(a[i + 2] - b[i + 2]));
      const full = new Uint8Array(width * height);
      const seen = new Uint8Array(width * height);
      const near = (a, i) => Math.max(Math.abs(a[i] - color[0]), Math.abs(a[i + 1] - color[1]), Math.abs(a[i + 2] - color[2])) <= moved;
      // With \`quads\` (from \`projected\`), the regions are the target's quad at rest and the quad it projects to now.
      const fill = (quad) => {
        ctx.clearRect(0, 0, width, height);
        ctx.beginPath();
        quad.forEach(([x, y], i) => (i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y)));
        ctx.closePath();
        ctx.fillStyle = '#000';
        ctx.fill();
        return ctx.getImageData(0, 0, width, height).data;
      };
      const [fullQuad, seenQuad] = quads ? [fill(quads.rest), fill(quads.now)] : [null, null];
      for (let p = 0; p < width * height; p++) {
        const i = p * 4;
        if (quads) {
          full[p] = fullQuad[i + 3] >= 128 ? 1 : 0;
          seen[p] = seenQuad[i + 3] >= 128 ? 1 : 0;
        } else if (mode === 'differ') {
          full[p] = delta(s, h, i) > moved ? 1 : 0;
          seen[p] = delta(m, h, i) > moved ? 1 : 0;
        } else {
          full[p] = near(s, i) ? 1 : 0;
          seen[p] = near(m, i) ? 1 : 0;
        }
      }
      const stats = (mask) => {
        let count = 0, sx = 0, sy = 0, x0 = Infinity, y0 = Infinity, x1 = -1, y1 = -1;
        for (let p = 0; p < mask.length; p++) {
          if (!mask[p]) continue;
          const x = p % width, y = Math.floor(p / width);
          count++; sx += x; sy += y;
          x0 = Math.min(x0, x); y0 = Math.min(y0, y); x1 = Math.max(x1, x); y1 = Math.max(y1, y);
        }
        return { count, cx: count ? sx / count : 0, cy: count ? sy / count : 0, w: count ? x1 - x0 + 1 : 0, h: count ? y1 - y0 + 1 : 0 };
      };
      const f = stats(full);
      const v = stats(seen);
      const spread = (mask) => {
        let total = 0, tx = 0, ty = 0, count = 0;
        for (let p = 0; p < mask.length; p++) {
          if (!mask[p]) continue;
          const dx = (p % width) - f.cx, dy = Math.floor(p / width) - f.cy;
          total += Math.hypot(dx, dy); tx += Math.abs(dx); ty += Math.abs(dy);
          count++;
        }
        return count ? { spread: total / count, spreadX: tx / count, spreadY: ty / count } : { spread: 0, spreadX: 0, spreadY: 0 };
      };
      return { full: { ...f, ...spread(full) }, seen: { ...v, ...spread(seen) } };
    },
    async diff(a, b, tolerance) {
      const load = async (b64) => createImageBitmap(await (await fetch('data:image/png;base64,' + b64)).blob());
      const [x, y] = await Promise.all([load(a), load(b)]);
      if (x.width !== y.width || x.height !== y.height) return -1;
      const canvas = new OffscreenCanvas(x.width, x.height);
      const ctx = canvas.getContext('2d');
      ctx.drawImage(x, 0, 0);
      const da = ctx.getImageData(0, 0, x.width, x.height).data;
      ctx.clearRect(0, 0, x.width, x.height);
      ctx.drawImage(y, 0, 0);
      const db = ctx.getImageData(0, 0, x.width, x.height).data;
      let count = 0;
      let max = 0;
      let box = [Infinity, Infinity, -1, -1];
      for (let i = 0; i < da.length; i += 4) {
        const d = Math.max(Math.abs(da[i] - db[i]), Math.abs(da[i + 1] - db[i + 1]), Math.abs(da[i + 2] - db[i + 2]));
        if (d > tolerance) {
          count++;
          max = Math.max(max, d);
          const p = i / 4, px = p % x.width, py = Math.floor(p / x.width);
          box = [Math.min(box[0], px), Math.min(box[1], py), Math.max(box[2], px), Math.max(box[3], py)];
        }
      }
      this.lastDiff = { count, max, box };
      return count;
    },
  };
})();
`;

export async function open(page: Page, data: Fixture, options: Record<string, unknown> = {}): Promise<void> {
  await page.setViewportSize({ width: WIDTH, height: HEIGHT });
  await page.setContent('<!doctype html><html><head><meta charset="utf-8"></head><body></body></html>');
  await mountIn(page, data, options);
}

/** Mounts the player and the harness in a document already loaded (a page, or a frame of one). */
export async function mountIn(
  target: Page | Frame,
  data: Fixture,
  options: Record<string, unknown> = {},
): Promise<void> {
  await target.addScriptTag({ content: readFileSync(PLAYER, 'utf8') });
  await target.addScriptTag({ content: readFileSync(PRESENTER, 'utf8') });
  await target.addScriptTag({ content: HARNESS });
  await target.evaluate(([fixture, opts]) => (window as unknown as Harness).sd.mount(fixture, opts), [
    data,
    options,
  ] as const);
  await settle(target);
}

/** Waits until no animation runs and the player is idle (no readiness wait left). */
export async function settle(page: Page | Frame): Promise<void> {
  await page.waitForFunction(() => (window as unknown as Harness).sd.count() === 0);
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}

/**
 * Waits for animations to start and pauses them in the same frame, so a loaded machine cannot let a transition run
 * to its end (and the player tear it down) between the two.
 */
export async function pauseOnStart(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const { sd } = window as unknown as Harness;
    if (sd.count() === 0) {
      return false;
    }
    sd.pause();
    return true;
  });
}

/**
 * Waits for animations to start, lets the page draw them for `ms`, and pauses them: an engine's compositor has then
 * played its own copy of their timing, which is what a viewer sees, and a pause holds that copy where `seek` puts it.
 */
export async function pauseRunning(page: Page, ms = 100): Promise<void> {
  await page.waitForFunction(() => (window as unknown as Harness).sd.count() > 0);
  await page.waitForTimeout(ms);
  await page.evaluate(() => (window as unknown as Harness).sd.pause());
  // A seek in the same frame as the pause can land before the compositor has paused its copy, which then holds the
  // time it paused at (Chromium): the pause is drawn first.
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}

export async function shot(page: Page, mask: Locator[] = []): Promise<string> {
  const buffer = await page.screenshot({
    clip: { x: 0, y: 0, width: WIDTH, height: HEIGHT },
    animations: 'allow',
    mask,
  });
  return buffer.toString('base64');
}

/**
 * Pixels that differ by more than `tolerance` (per channel) between two screenshots. A still state compares exactly;
 * a paused frame allows for antialiasing, which Chromium changes on content it composites while an animation runs.
 */
export async function diff(page: Page, a: string, b: string, tolerance = 0): Promise<number> {
  const count = await page.evaluate(([x, y, t]) => (window as unknown as Harness).sd.diff(x, y, t), [
    a,
    b,
    tolerance,
  ] as const);
  if (count !== 0) {
    console.log(
      'diff',
      JSON.stringify(await page.evaluate(() => (window as unknown as { sd: { lastDiff: unknown } }).sd.lastDiff)),
    );
  }
  return count;
}

/** A still render of the deck at (slide, step), screenshot over the player. */
export async function staticShot(page: Page, slide: number, step: number, mask: Locator[] = []): Promise<string> {
  await page.evaluate(([s, t]) => (window as unknown as Harness).sd.showStatic(s, t), [slide, step] as const);
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const image = await shot(page, mask);
  await page.evaluate(() => (window as unknown as Harness).sd.hideStatic());
  return image;
}

/** Keyframe keys the player used that effects may not animate. */
export async function disallowedKeys(page: Page): Promise<string[]> {
  const keys = await page.evaluate(() => (window as unknown as Harness).sd.keyframeKeys());
  return keys.filter((key) => !ALLOWED_KEYFRAME_KEYS.has(key) && !COMPUTED_KEYS.includes(key));
}

export interface Region {
  readonly count: number;
  readonly cx: number;
  readonly cy: number;
  readonly w: number;
  readonly h: number;
  /** Mean distance of the region's pixels from the full region's centroid, overall and along each axis. */
  readonly spread: number;
  readonly spreadX: number;
  readonly spreadY: number;
}

/** A target's corners at rest, where its transform projects them now, and the same without perspective, in px. */
export interface Quads {
  readonly rest: readonly (readonly [number, number])[];
  readonly now: readonly (readonly [number, number])[];
  readonly flat: readonly (readonly [number, number])[];
}

export interface Harness {
  sd: {
    mount(fixture: Fixture, options: Record<string, unknown>): boolean;
    showStatic(slide: number, step: number): void;
    hideStatic(): void;
    /** The running animations that can end. */
    count(): number;
    pause(): number;
    seek(fraction: number): number;
    /** Seeks every paused animation to `ms`. */
    seekMs(ms: number): void;
    /** Finishes what can end; an endless repeat plays on. */
    resume(): void;
    /** Cuts every endless repeat to the end of its round and holds it there, as a click does. */
    cut(): void;
    keyframeKeys(): string[];
    /** The current slide's `data-el` element. */
    target(id: string): Element | null;
    style(id: string): Record<string, string> | null;
    /** The incoming layer's computed opacity and transform, while a transition runs. */
    incoming(): Record<string, string> | null;
    diff(a: string, b: string, tolerance: number): Promise<number>;
    /** The quads of a target under a projective transform (`id` null: the incoming layer), or null. */
    projected(id: string | null): Quads | null;
    /** The intersection over union of a frame's `color` pixels with the projected and the flat quad. */
    fit(
      frame: string,
      quads: Quads,
      color: readonly [number, number, number],
      tolerance: number,
      view?: readonly [number, number, number, number],
    ): Promise<{ count: number; now: number; flat: number; quads: number }>;
    motion(
      frame: string,
      hidden: string,
      shown: string,
      mode: 'differ' | 'color',
      moved: number,
      color: readonly [number, number, number] | null,
      quads?: Quads | null,
    ): Promise<{ full: Region; seen: Region }>;
    diagnostics: { code: string; message: string }[];
    events: { type: 'position' | 'ready'; t: number; slide: number; step: number }[];
    source: unknown;
    player: {
      next(): void;
      previous(): void;
      goTo(slide: number, step?: number): void;
      present(): Promise<void>;
      stop(): void;
      destroy(): void;
      readonly position: { slide: number; step: number; presenting: boolean; blank: string | null };
    };
  };
}
