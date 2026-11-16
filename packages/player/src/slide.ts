// A slide mounted on the stage: a layer element whose shadow root holds a sanitized copy of the slide's composed
// markup, and its build state. The state at a step is a pure function of the timeline: applying it
// cancels every animation but endless repeats, restores split text, marks hidden units with `data-sd-state="hidden"`,
// holds the colours, font switches and dims steps left, and fast-forwards (`Animation.finish()`) the transforms,
// opacities and blurs of the emphases and paths that hold. Playing a group runs each step's
// recipe from the effects library with the Web Animations API, or its canonical fallback.
import type { DeckSlide, DeckStep } from '@slidock/types';

import { type DeckSource, resolveMedia } from './deck';
import { type Split, splitText } from './delivery';
import { type BuildContext, type Recipe, buildRecipe, fallbackRecipe, holds as holdsAfter } from './effects/builds';
import { cssEasing, easeKeyframes } from './effects/easing';
import { maskCss } from './effects/masks';
import { type Paint, paintProperty } from './effects/properties';
import { hashSeed } from './effects/random';
import { family } from './effects/vocab';
import { type MediaPolicy, play, playMedium, stopMedium } from './media';
import { PAINT_CSS, type Run, holdPaint, linkPaint, paintRuns, unlinkPaints } from './paint';
import { type SanitizeReport, sanitize } from './sanitize';
import {
  type Hold,
  type Issue,
  NO_HOLD,
  type ScheduledStep,
  type Unit,
  bounded,
  groups,
  isIdentity,
  slideState,
  stepUnits,
  unitKey,
} from './schedule';

/** The rules every slide's shadow root shares: the host cut from inheritance, the format's reset, the build state. */
export const SLIDE_CSS =
  ':host{all:initial;display:block;position:absolute;left:0;top:0;contain:strict;overflow:hidden}' +
  'p{margin:0}[data-sd-state=hidden]{visibility:hidden!important}' +
  '[data-sd-box]{background:none!important;border-color:transparent!important;box-shadow:none!important}' +
  '[data-sd-box=lifted]{isolation:isolate}' +
  '[data-sd-boxlayer]{position:absolute;z-index:-1;pointer-events:none;box-sizing:border-box}' +
  PAINT_CSS +
  maskCss('');

export interface MountedSlide {
  readonly index: number;
  readonly id: string;
  readonly layer: HTMLElement;
  readonly root: ShadowRoot;
  /** The animations the player started on this slide, cancelled whenever its state is applied. */
  readonly animations: Set<Animation>;
  /** Endless repeats, and the step each plays (its `loop` says until when it runs). */
  readonly loops: Map<Animation, ScheduledStep>;
  /** The groups played in the state last applied. */
  played: number;
  /** Text split for a word or character build, restored with the state. */
  readonly splits: Split[];
  readonly report: SanitizeReport;
}

/** What playing a slide's builds needs from the player. */
export interface SlideEnv {
  /** The slide's pixel size. */
  readonly size: { readonly width: number; readonly height: number };
  readonly reduced: boolean;
  readonly media: MediaPolicy;
  /** Whether the registered custom properties are usable. */
  readonly properties: boolean;
  /** The dissolve cell budget. */
  readonly cells: number;
  /** A step played a declared default (its fallback, or a default timing). */
  readonly onIssue?: (step: ScheduledStep, issue: Issue) => void;
}

const sheets = new WeakMap<Document, CSSStyleSheet | null>();

function sharedSheet(document: Document): CSSStyleSheet | null {
  if (!sheets.has(document)) {
    let sheet: CSSStyleSheet | null = null;
    const view = document.defaultView as (Window & typeof globalThis) | null;
    try {
      if (view !== null && typeof view.CSSStyleSheet === 'function' && 'adoptedStyleSheets' in document) {
        sheet = new view.CSSStyleSheet();
        sheet.replaceSync(SLIDE_CSS);
      }
    } catch {
      sheet = null;
    }
    sheets.set(document, sheet);
  }
  return sheets.get(document) ?? null;
}

/** Gives a new shadow root the slide rules. */
export function adoptSlideCss(document: Document, root: ShadowRoot): void {
  const sheet = sharedSheet(document);
  if (sheet !== null) {
    root.adoptedStyleSheets = [sheet];
  } else {
    const style = document.createElement('style');
    style.textContent = SLIDE_CSS;
    root.append(style);
  }
}

/** Mounts slide `index` into a new layer: clone, sanitize, resolve media, then adopt into a shadow root. */
export function mountSlide(
  document: Document,
  source: DeckSource,
  slide: DeckSlide,
  index: number,
  size: { readonly width: number; readonly height: number },
): MountedSlide {
  const layer = document.createElement('div');
  layer.className = 'sd-layer';
  layer.setAttribute('data-sd-layer', slide.id);
  layer.style.width = `${size.width}px`;
  layer.style.height = `${size.height}px`;
  const root = layer.attachShadow({ mode: 'open' });
  adoptSlideCss(document, root);
  const fragment = source.slide(slide.id);
  let report: SanitizeReport = { elements: [], attributes: [] };
  if (fragment !== null) {
    report = sanitize(fragment);
    if (source.mediaUrl !== undefined) {
      resolveMedia(fragment, source.mediaUrl);
    }
    root.append(fragment);
  }
  return { index, id: slide.id, layer, root, animations: new Set(), loops: new Map(), played: 0, splits: [], report };
}

/** Finds the elements of units in one slide root, indexing its `data-el` elements and their paragraphs once. */
export function unitFinder(root: ParentNode): (unit: Unit) => Element[] {
  let elements: Map<string, Element> | null = null;
  const paragraphs = new Map<Element, Map<string, Element>>();
  const find = (id: string): Element | undefined => {
    if (elements === null) {
      elements = new Map();
      for (const candidate of Array.from(root.querySelectorAll('[data-el]'))) {
        const key = candidate.getAttribute('data-el') ?? '';
        if (!elements.has(key)) {
          elements.set(key, candidate);
        }
      }
    }
    return elements.get(id);
  };
  const paragraphOf = (element: Element, index: number): Element | undefined => {
    let byIndex = paragraphs.get(element);
    if (byIndex === undefined) {
      byIndex = new Map();
      for (const candidate of Array.from(element.querySelectorAll('[data-p]'))) {
        const key = candidate.getAttribute('data-p') ?? '';
        if (!byIndex.has(key)) {
          byIndex.set(key, candidate);
        }
      }
      paragraphs.set(element, byIndex);
    }
    return byIndex.get(String(index));
  };
  return (unit) => {
    const element = find(unit.element);
    if (element === undefined) {
      return [];
    }
    if (unit.background === true) {
      return nonTextChildren(element);
    }
    if (unit.paragraph === null) {
      return [element];
    }
    const paragraph = paragraphOf(element, unit.paragraph);
    return paragraph === undefined ? [] : [paragraph];
  };
}

function canAnimate(element: Element): boolean {
  return typeof (element as Partial<Element>).animate === 'function';
}

/** Finishes animations that can end; endless repeats keep running. */
export function finishAll(animations: Iterable<Animation>): void {
  for (const animation of animations) {
    const end = animation.effect?.getComputedTiming?.().endTime;
    if (end === undefined || Number.isFinite(Number(end))) {
      try {
        animation.finish();
      } catch {
        // An endless animation, or one this engine cannot finish.
      }
    }
  }
}

export function cancelAnimations(slide: MountedSlide, keepLoops = false): void {
  for (const animation of Array.from(slide.animations)) {
    if (!(keepLoops && slide.loops.has(animation))) {
      animation.cancel();
      slide.animations.delete(animation);
      slide.loops.delete(animation);
    }
  }
}

/** Puts back the text a word or character build split. */
export function restoreSplits(slide: MountedSlide): void {
  for (const split of slide.splits.splice(0)) {
    split.restore();
  }
}

/** The opacity an element shows with no animation on it, read once while none runs. */
const baseOpacity = new WeakMap<Element, number>();

function opacityOf(element: Element): number {
  const known = baseOpacity.get(element);
  if (known !== undefined) {
    return known;
  }
  const view = element.ownerDocument.defaultView;
  if (view === null || (element.getAnimations?.().length ?? 0) > 0) {
    return 1;
  }
  const read = Number.parseFloat(view.getComputedStyle(element).opacity);
  const value = Number.isFinite(read) ? Math.min(1, Math.max(0, read)) : 1;
  baseOpacity.set(element, value);
  return value;
}

function parseAngle(value: string): number {
  const match = /^(-?[\d.]+)deg$/.exec(value.trim());
  return match === null ? 0 : Number(match[1]);
}

/**
 * A slide-space vector in the element's own transform space: its static `rotate` and `scale` undone, then the turns
 * and scales of the first `count` transforms the hold an earlier step left (a live transform composes after them).
 */
function localVector(
  element: Element,
  hold: Hold = NO_HOLD,
  count = hold.transforms.length,
): (dx: number, dy: number) => readonly [number, number] {
  const style = element.ownerDocument.defaultView?.getComputedStyle(element);
  const deg = parseAngle(style?.rotate ?? '');
  const scales = (style?.scale || 'none').trim().split(/\s+/).map(Number);
  const first = scales[0];
  const second = scales[1];
  const sx = first !== undefined && Number.isFinite(first) && first !== 0 ? first : 1;
  const sy = second !== undefined && Number.isFinite(second) && second !== 0 ? second : sx;
  const undo = (x: number, y: number, rotate: number, scaleX: number, scaleY: number): [number, number] => {
    const rad = (-rotate * Math.PI) / 180;
    const cos = Math.cos(rad);
    const sin = Math.sin(rad);
    return [(x * cos - y * sin) / (scaleX || 1), (x * sin + y * cos) / (scaleY || 1)];
  };
  return (dx, dy) => {
    let [x, y] = undo(dx, dy, deg, sx, sy);
    for (const op of hold.transforms.slice(0, count)) {
      if (op.op === 'rotate') {
        [x, y] = undo(x, y, op.deg, 1, 1);
      } else if (op.op === 'scale') {
        [x, y] = undo(x, y, 0, op.x, op.y);
      }
    }
    return [x, y];
  };
}

/** The element's box in slide pixels. */
function measure(element: Element, layer: HTMLElement, size: SlideEnv['size']) {
  const box = element.getBoundingClientRect();
  const frame = layer.getBoundingClientRect();
  const scale = frame.width > 0 ? frame.width / size.width : 1;
  return {
    x: (box.left - frame.left) / scale,
    y: (box.top - frame.top) / scale,
    w: box.width / scale,
    h: box.height / scale,
  };
}

/**
 * The transform a hold leaves on `element`, composed as the steps that left it played. It ends with a turn of 0deg:
 * after a group's paused frames, Chromium paints a held frame whose transform is translations alone differently from
 * the still render of the same state (overlapping shapes come out changed), and a transform that is not a pure
 * translation paints the same in both.
 */
function transformOf(hold: Hold, element: Element): string {
  return hold.transforms
    .map((op) => {
      if (op.op === 'translate') {
        const [x, y] = localVector(element, hold, op.under)(op.x, op.y);
        return `translate(${x}px, ${y}px)`;
      }
      return op.op === 'rotate' ? `rotate(${op.deg}deg)` : `scale(${op.x}, ${op.y})`;
    })
    .concat('rotate(0deg)')
    .join(' ');
}

/** The element an endless repeat animates, and the paint it changes there (none: not a paint). */
const loopPaints = new WeakMap<Animation, readonly [Element, Paint | undefined]>();

/** The steps (by index) of the endless repeats still running on the slide. */
function running(slide: MountedSlide): Set<number> {
  return new Set(Array.from(slide.loops.values(), (step) => step.index));
}

/** Sets the slide to its state after `played` groups, with no motion; `keepLoops` lets endless repeats run on. */
export function applyState(
  slide: MountedSlide,
  timeline: readonly DeckStep[],
  played: number,
  options: { keepLoops?: boolean } = {},
): void {
  cancelAnimations(slide, options.keepLoops === true);
  slide.played = played;
  if (slide.loops.size === 0) {
    restoreSplits(slide);
  }
  for (const layer of Array.from(slide.root.querySelectorAll('[data-sd-boxlayer]'))) {
    layer.remove();
  }
  // A repeat still running shows through its link until it ends.
  unlinkPaints(
    slide.root,
    Array.from(slide.loops.keys(), (animation) => loopPaints.get(animation) ?? []),
  );
  for (const element of Array.from(slide.root.querySelectorAll('[data-sd-state],[data-sd-mask],[data-sd-box]'))) {
    element.removeAttribute('data-sd-state');
    element.removeAttribute('data-sd-mask');
    element.removeAttribute('data-sd-box');
  }
  // A repeat still running holds nothing yet: a held frame would cover it.
  const state = slideState(timeline, played, running(slide));
  const unitElements = unitFinder(slide.root);
  for (const unit of state.hidden.values()) {
    for (const element of unitElements(unit)) {
      element.setAttribute('data-sd-state', 'hidden');
    }
    // A hidden background hides the shape's own paint too.
    if (unit.background === true) {
      for (const element of unitElements({ element: unit.element, paragraph: null })) {
        if (hasBox(element)) {
          element.setAttribute('data-sd-box', 'hidden');
        }
      }
    }
  }
  // Colours and font switches a change left, then dims (text colours), held on each unit's element.
  const hold = (unit: Unit, paint: Partial<Record<Paint, string>>): void => {
    for (const element of unitElements(unit)) {
      for (const [name, value] of Object.entries(paint) as [Paint, string][]) {
        holdPaint(element, name, value);
      }
    }
  };
  for (const { unit, hold: change } of state.holds.values()) {
    hold(unit, change.paint);
  }
  for (const { unit, color } of state.dimmed.values()) {
    hold(unit, { text: color });
  }
  const held = new Map<Element, Keyframe>();
  for (const { unit, hold } of state.holds.values()) {
    if (isIdentity({ ...hold, paint: {} })) {
      continue;
    }
    for (const element of unitElements(unit)) {
      const frame: Keyframe = { ...held.get(element) };
      if (hold.transforms.length > 0) {
        frame.transform = transformOf(hold, element);
      }
      if (hold.opacity !== 1) {
        frame.opacity = opacityOf(element) * hold.opacity;
      }
      if (hold.blur !== 0) {
        // After the element's own filter (its printed shadow), which the held frame replaces.
        frame.filter = `${(element as HTMLElement).style.filter}blur(${hold.blur}px)`;
      }
      held.set(element, frame);
    }
  }
  // Read every held element's style before any held frame applies: without a style read between the cancelled
  // animations and the held frames, Chromium keeps a cancelled transform's compositing layer, rasterised at the scale
  // it was paused at, and the held state's edges differ from a fresh render's.
  for (const element of held.keys()) {
    void element.ownerDocument.defaultView?.getComputedStyle(element).transform;
  }
  for (const [element, frame] of held) {
    if (!canAnimate(element)) {
      continue;
    }
    opacityOf(element);
    const animation = element.animate([frame], { duration: 0, fill: 'forwards' });
    animation.finish();
    slide.animations.add(animation);
  }
}

/**
 * Cuts the slide's endless repeats, as the click that ends them does (one that leaves the slide ends them all): each
 * holds where its one round ends, as the still render shows it.
 */
export function cutLoops(slide: MountedSlide, timeline: readonly DeckStep[]): void {
  if (slide.loops.size > 0) {
    applyState(slide, timeline, slide.played);
  }
}

/** The computed properties that paint a shape's own box: its backgrounds, borders and shadow. */
const BOX_PROPERTY = /^(background|border|box-shadow)/;

/** Whether a computed colour paints nothing: `transparent`, or an alpha of 0 (`rgba(r, g, b, 0)`, `… / 0)`). */
function transparent(color: string): boolean {
  return /^(transparent)?$|^rgba\(([^,]*,){3}\s*0(\.0*)?\)$|\/\s*0(\.0*)?%?\)$/.test(color.trim());
}

/** Whether a computed image or shadow list paints anything; a lone transparent shadow (an empty box's extent) does not. */
function paints(value: string): boolean {
  return value !== '' && value !== 'none' && !/^rgba\(([^,]*,){3} 0\)[^,]*$/.test(value);
}

function borderWidths(style: CSSStyleDeclaration): [number, number, number, number] {
  const width = (side: string) => Number.parseFloat(style.getPropertyValue(`border-${side}-width`)) || 0;
  return [width('top'), width('right'), width('bottom'), width('left')];
}

/**
 * Whether a shape paints a box of its own that can build apart from its content: a background, a border or a shadow,
 * on a positioned element whose background is not its text's paint and whose border its overflow does not clip.
 */
function hasBox(element: Element): boolean {
  const style = element.ownerDocument.defaultView?.getComputedStyle(element);
  if (style === undefined || style.position === 'static' || style.backgroundClip.includes('text')) {
    return false;
  }
  const bordered = borderWidths(style).some((w) => w > 0);
  if (bordered && style.overflow !== 'visible') {
    return false;
  }
  return bordered || !transparent(style.backgroundColor) || paints(style.backgroundImage) || paints(style.boxShadow);
}

/**
 * Moves a shape's own box (background, border, shadow) onto a layer of its own beneath its content, so it can build
 * apart from its text; the state puts it back. `null` when it paints none.
 */
function liftBox(element: Element): HTMLElement | null {
  const view = element.ownerDocument.defaultView;
  if (view === null || !(element instanceof view.HTMLElement)) {
    return null;
  }
  // A background the state hid paints again once lifted.
  element.removeAttribute('data-sd-box');
  if (!hasBox(element)) {
    return null;
  }
  element.querySelector(':scope>[data-sd-boxlayer]')?.remove();
  const style = view.getComputedStyle(element);
  const [top, right, bottom, left] = borderWidths(style);
  const layer = element.ownerDocument.createElement('div');
  layer.setAttribute('data-sd-boxlayer', '');
  layer.setAttribute('aria-hidden', 'true');
  for (const name of Array.from(style)) {
    if (BOX_PROPERTY.test(name)) {
      layer.style.setProperty(name, style.getPropertyValue(name));
    }
  }
  // Over the border box: the padding box (its containing block) widened by the border.
  layer.style.inset = `${-top}px ${-right}px ${-bottom}px ${-left}px`;
  element.setAttribute('data-sd-box', 'lifted');
  element.prepend(layer);
  return layer;
}

/** The children of an element that hold no text: a shape's geometry and pictures. */
function nonTextChildren(element: Element): Element[] {
  return Array.from(element.children).filter(
    (child) =>
      !child.hasAttribute('data-p') &&
      child.querySelector('[data-p]') === null &&
      !child.hasAttribute('data-sd-boxlayer'),
  );
}

/**
 * The box parts of an entrance or exit that builds a whole element's text unit by unit: each element's own box and its
 * children without text, which build with the text (the source apps build a shape with its text).
 */
function boxParts(roots: readonly Element[], units: readonly Element[]): Element[] {
  if (units.every((unit) => roots.includes(unit))) {
    return [];
  }
  return roots.flatMap((root) => {
    const layer = liftBox(root);
    return [...(layer === null ? [] : [layer]), ...nonTextChildren(root)];
  });
}

function isStop(step: DeckStep): boolean {
  return /(stop|pause)/i.test(step.native);
}

/** The roots a step's units come from: its paragraphs, its shape, or its element. */
function targetRoots(slide: MountedSlide, step: DeckStep): Element[] {
  const find = unitFinder(slide.root);
  return stepUnits(step).flatMap((unit) => find(unit));
}

/** Effects that animate only opacity or colour, which run on inline text units. */
const INLINE_EFFECTS: ReadonlySet<string> = new Set([
  'appear',
  'fade',
  'typewriter',
  'blink',
  'flash',
  'transparency',
  'colorChange',
]);

/** Splits or collects the units a step delivers, in order. */
function deliver(slide: MountedSlide, step: DeckStep, effectName: string | null): Element[] {
  const roots = targetRoots(slide, step);
  const delivery = effectName === 'typewriter' ? 'byChar' : step.delivery;
  if (step.target.background === true || delivery === 'all') {
    return roots;
  }
  if (delivery === 'byParagraph') {
    if (step.target.paragraphs !== null) {
      return roots;
    }
    const paragraphs = roots.flatMap((root) => Array.from(root.querySelectorAll('[data-p]')));
    return paragraphs.length > 0 ? paragraphs : roots;
  }
  if (delivery === 'byWord' || delivery === 'byChar') {
    const split = splitText(roots, delivery === 'byWord' ? 'word' : 'grapheme', {
      inlineBlock: effectName !== null && !INLINE_EFFECTS.has(effectName),
    });
    if (split === null || split.units.length === 0) {
      split?.restore();
      return roots;
    }
    slide.splits.push(split);
    return [...split.units];
  }
  return roots;
}

function contextFor(slide: MountedSlide, element: Element, env: SlideEnv, seed: number, hold: Hold): BuildContext {
  return {
    box: measure(element, slide.layer, env.size),
    slide: env.size,
    local: localVector(element, hold),
    opacity: opacityOf(element) * hold.opacity,
    seed,
    properties: env.properties,
    cells: env.cells,
  };
}

function runMedia(
  slide: MountedSlide,
  scheduled: ScheduledStep,
  units: readonly Element[],
  media: MediaPolicy,
): Animation[] {
  const { step, effect, startMs } = scheduled;
  const command = effect?.effect ?? (isStop(step) ? 'pause' : 'play');
  const started: Animation[] = [];
  for (const element of units) {
    if (!canAnimate(element)) {
      continue;
    }
    const targets = (
      element.matches('video, audio') ? [element] : Array.from(element.querySelectorAll('video, audio'))
    ) as HTMLMediaElement[];
    const cue = element.animate([], { delay: startMs, duration: 0 });
    slide.animations.add(cue);
    cue.finished.then(
      () => {
        for (const medium of targets) {
          if (command === 'play') {
            playMedium(medium, effect?.effect === 'play' ? effect.params.from : 0, media);
          } else if (command === 'pause') {
            medium.pause();
          } else if (command === 'stop') {
            stopMedium(medium);
          } else if (command === 'toggle') {
            if (medium.paused) {
              play(medium, media);
            } else {
              medium.pause();
            }
          }
        }
      },
      () => undefined,
    );
    started.push(cue);
  }
  return started;
}

/** Plays one step's units; returns the animations its group waits for. */
function playStep(
  slide: MountedSlide,
  scheduled: ScheduledStep,
  units: readonly Element[],
  box: readonly Element[],
  hold: Hold,
  env: SlideEnv,
): Animation[] {
  const { step } = scheduled;
  const started: Animation[] = [];
  const seed = hashSeed(`${slide.id}#${scheduled.index}`);
  const play = (element: Element, unitIndex: number, report: boolean): void => {
    if (!canAnimate(element)) {
      return;
    }
    const ctx = contextFor(slide, element, env, seed, hold);
    let recipe: Recipe | null = null;
    const runs = new Map<Paint, Run[]>();
    if (env.reduced) {
      recipe = fallbackRecipe(step.kind === 'entrance' || step.kind === 'exit' ? 'appear' : 'none', step.kind, ctx);
    } else if (scheduled.effect !== null) {
      recipe = buildRecipe(scheduled.effect, step.kind, ctx);
      // A colour or font change plays on each element that paints it, from the value each shows now: read them all
      // before any is linked. What nothing paints here is left out (reported); a recipe left with nothing falls back.
      for (const { paint } of recipe?.frames ?? []) {
        if (paint !== undefined && !runs.has(paint)) {
          runs.set(paint, env.properties ? paintRuns(element, paint) : []);
        }
      }
      const out = [...runs].filter(([, list]) => list.length === 0).map(([paint]) => paint);
      if (recipe?.frames.every((frames) => frames.paint !== undefined && out.includes(frames.paint))) {
        recipe = null;
      }
      if (report) {
        for (const property of recipe === null ? [] : out) {
          env.onIssue?.(scheduled, { kind: 'track', property });
        }
        if (recipe === null) {
          env.onIssue?.(scheduled, { kind: 'cannotPlay', name: scheduled.effect.effect });
        }
      }
    }
    const native = recipe !== null && !env.reduced;
    if (recipe === null) {
      recipe = fallbackRecipe(step.fallback, step.kind, ctx);
    }
    const effect = native ? scheduled.effect : null;
    const delay = scheduled.startMs + (scheduled.unitDelays[unitIndex] ?? scheduled.unitDelays.at(-1) ?? 0);
    const instant =
      env.reduced ||
      effect?.effect === 'appear' ||
      effect?.effect === 'typewriter' ||
      (!native && step.fallback === 'appear');
    const duration = instant ? 0 : scheduled.durationMs;
    const holdsEnd = effect !== null && holdsAfter(effect);
    // An emphasis or a path that does not hold returns to the start when it ends.
    const returns = effect !== null && (family(effect) === 'emphasis' || family(effect) === 'motion') && !holdsEnd;
    const fill: FillMode = returns && (step.kind === 'emphasis' || step.kind === 'motion') ? 'backwards' : 'both';
    for (const [paint, list] of runs) {
      for (const { run, from } of list) {
        linkPaint(run, paint, from);
      }
    }
    const span = scheduled.durationMs;
    const rounds = scheduled.iterations;
    for (const frames of recipe.frames) {
      if (frames.mask !== undefined) {
        element.setAttribute('data-sd-mask', frames.mask);
      }
      const { paint, durationMs: own } = frames;
      // A step's own repeat, finite or endless, repeats the whole step: frames on their own time play their part of
      // each round, shown before it as they start (a switch: as the target showed) and held after it.
      const whole = own !== undefined && !instant && span > 0 && rounds !== 1;
      const starts = frames.keyframes[0]?.offset === 0;
      const targets: [Element, Keyframe[], Keyframe][] =
        paint === undefined
          ? [[element, frames.keyframes, { opacity: ctx.opacity }]]
          : (runs.get(paint) ?? []).map(({ run, from }) => {
              const shown = { [paintProperty(paint)]: from };
              return [run, starts ? frames.keyframes : [{ offset: 0, ...shown }, ...frames.keyframes], shown];
            });
      const curve = frames.curve ?? scheduled.curve;
      const begin = frames.beginMs ?? 0;
      for (const [target, keyframes, shown] of targets) {
        let eased = easeKeyframes(keyframes, curve);
        if (whole) {
          const list = eased ?? keyframes;
          const value = ({ offset: _offset, easing: _easing, ...rest }: Keyframe = {}) => rest;
          eased = [
            { ...(frames.fill === undefined ? value(list[0]) : shown), offset: 0, easing: 'steps(1, end)' },
            ...list.map((frame) => ({
              ...frame,
              offset: Math.min(1, Math.max(0, (begin + (frame.offset ?? 0) * own) / span)),
            })),
            { ...value(list.at(-1)), offset: 1 },
          ];
        }
        const animation = target.animate(eased ?? keyframes, {
          delay: delay + (whole ? 0 : begin),
          duration: instant ? 0 : whole ? span : (own ?? duration),
          iterations: (own !== undefined && !whole) || instant ? 1 : rounds,
          direction: scheduled.direction,
          easing: eased === null ? cssEasing(curve) : 'linear',
          fill: frames.fill ?? fill,
          ...(frames.composite === undefined ? {} : { composite: frames.composite }),
        });
        slide.animations.add(animation);
        if (scheduled.loop !== null && (own === undefined || whole) && !instant) {
          slide.loops.set(animation, scheduled);
          loopPaints.set(animation, [target, paint]);
        } else {
          started.push(animation);
        }
      }
    }
    // An exit, or a step that hides its target after, leaves it hidden until the state takes over: at the end of the
    // step's duration, which an instant effect (an appear shown for a while) counts too.
    if (step.kind === 'exit' || scheduled.after === 'hide') {
      const rounds = Number.isFinite(scheduled.iterations) ? scheduled.iterations : 1;
      const hide = element.animate([{ opacity: 0 }], {
        delay: delay + (env.reduced ? 0 : scheduled.durationMs) * rounds,
        duration: 0,
        fill: 'forwards',
      });
      slide.animations.add(hide);
      started.push(hide);
    }
  };
  units.forEach((element, unitIndex) => play(element, unitIndex, unitIndex === 0));
  // The shape's own box builds with its text: in with the unit that starts first, out with the one that starts last.
  const delays = units.map((_, i) => scheduled.unitDelays[i] ?? scheduled.unitDelays.at(-1) ?? 0);
  const boxAt = delays.indexOf((step.kind === 'exit' ? Math.max : Math.min)(...delays));
  box.forEach((element) => play(element, boxAt, false));
  return started;
}

/**
 * Plays group `index` of the slide's timeline from its current state (the state after `index` groups). Returns the
 * animations the group waits for; endless repeats run beside them (`slide.loops`). With `reduced`, entrances and
 * exits appear and emphases jump to their end state when the group settles.
 */
export function playGroup(
  slide: MountedSlide,
  timeline: readonly DeckStep[],
  index: number,
  env: SlideEnv,
): Animation[] {
  // An endless repeat "until the next click" ends when the next group starts, and leaves its hold from then on.
  const loops = slide.loops.size;
  for (const [animation, { loop }] of Array.from(slide.loops)) {
    if (loop === 'click') {
      animation.cancel();
      slide.loops.delete(animation);
      slide.animations.delete(animation);
    }
  }
  if (slide.loops.size < loops) {
    applyState(slide, timeline, index, { keepLoops: true });
  }
  const steps = bounded(timeline);
  const raw = groups(steps)[index];
  if (raw === undefined) {
    return [];
  }
  // Deliver first: the stagger depends on how many units each step split into.
  const delivered = new Map<number, Element[]>();
  for (const scheduled of raw.steps) {
    delivered.set(scheduled.index, deliver(slide, scheduled.step, scheduled.effect?.effect ?? null));
  }
  // Read the units' own opacity before any of this group's animations runs on them.
  for (const units of delivered.values()) {
    units.forEach(opacityOf);
  }
  const group = groups(steps, (_step, i) => delivered.get(i)?.length ?? 1)[index] ?? raw;
  // Each step plays from the state shown: a repeat still running holds nothing yet.
  const state = slideState(steps, index, running(slide));
  const started: Animation[] = [];
  for (const scheduled of group.steps) {
    const { step } = scheduled;
    for (const issue of scheduled.issues) {
      env.onIssue?.(scheduled, issue);
    }
    let units = delivered.get(scheduled.index) ?? [];
    if (step.kind === 'entrance') {
      for (const root of targetRoots(slide, step)) {
        root.removeAttribute('data-sd-state');
      }
    }
    if (step.kind === 'media') {
      started.push(...runMedia(slide, scheduled, units, env.media));
      continue;
    }
    let box: Element[] = [];
    if (step.kind === 'entrance' || step.kind === 'exit') {
      if (step.target.background === true) {
        // A background build builds the shape's own box with its geometry.
        const find = unitFinder(slide.root);
        const layers = find({ element: step.target.element, paragraph: null }).flatMap((e) => liftBox(e) ?? []);
        units = [...layers, ...units];
      } else if (step.target.paragraphs === null) {
        box = boxParts(targetRoots(slide, step), units);
      }
    }
    const first = stepUnits(step)[0];
    const hold = (first === undefined ? undefined : state.holds.get(unitKey(first))?.hold) ?? NO_HOLD;
    started.push(...playStep(slide, scheduled, units, box, hold, env));
  }
  return started;
}
