// Morph and Magic Move: a FLIP between the elements of two slides that share a match key. The
// incoming slide plays beneath the outgoing one. Each matched pair moves in step from the outgoing element's box onto
// the incoming one's (translate, scale, rotate) on the curve's progress, while the outgoing slide fades out as one
// picture over the move and the incoming match fades in beneath it ([`beneath`]), so the pair weighs as a linear mix
// and covers as one element whose opacity goes from the outgoing element's to the incoming one's (Keynote 14.5's
// movies: a translucent object moves without doubling, and 0.5 opacity reaches 1 on the curve). The first frame is
// the outgoing slide and the last the incoming one, each in its own stacking order, and a match never shows another
// through it. The outgoing background is drawn beneath the incoming one, which fades in over it across the move.
// Unmatched objects fade when the deck says so (Keynote's `custom_magic_move_fade_unmatched_objects`, always for a
// Morph), else go and come at once. With Keynote's Magic Move the outgoing slide's unmatched objects fade out over the
// first third of the time and the incoming slide's fade in over the last third, the curve whole on each third
// (measured on Keynote 14.5's movies); with a Morph they fade over the whole move (no PowerPoint measurement). An
// element on both slides by id (a layout's) shows from the incoming slide. Matching is 1:1: a key on two elements of
// one slide matches nothing. There is no text-level morph: matched text boxes morph as whole objects.
import type { TransitionPart, TransitionPlan } from './transitions';

export interface MorphScene {
  readonly out: HTMLElement;
  readonly into: HTMLElement;
  readonly size: { readonly width: number; readonly height: number };
  /** Element id → match key, per slide (the deck's `matchKeys`); the markup's `data-sd-match` when absent. */
  readonly outKeys?: Readonly<Record<string, string>> | undefined;
  readonly inKeys?: Readonly<Record<string, string>> | undefined;
  /** Fade unmatched elements; otherwise they go and come at once. */
  readonly fadeUnmatched: boolean;
  /** Keynote's Magic Move: unmatched objects fade out over the first third ([`EXIT`]) and in over the last ([`ENTER`]). */
  readonly thirds?: boolean;
}

/** The time Keynote's Magic Move fades the outgoing slide's unmatched objects out, the curve whole on it. */
export const EXIT: readonly [number, number] = [0, 1 / 3];
/** The time Keynote's Magic Move fades the incoming slide's unmatched objects in, the curve whole on it. */
export const ENTER: readonly [number, number] = [2 / 3, 1];
/** Where unmatched objects go and come at once (curve progress). */
const AT_ONCE = 0.05;

/** Elements matched at most per slide; more are left to crossfade. */
const MATCHES = 2_000;

function slideMain(layer: HTMLElement): Element | null {
  return layer.shadowRoot?.querySelector('[data-slide]') ?? null;
}

/**
 * A slide's `matchKeys` from the deck JSON: an object of strings, or `undefined` (absent or malformed), in which case
 * the markup's `data-sd-match` keys the slide.
 */
export function readMatchKeys(value: unknown): {
  keys: Readonly<Record<string, string>> | undefined;
  malformed: boolean;
} {
  if (value === undefined || value === null) {
    return { keys: undefined, malformed: false };
  }
  if (typeof value !== 'object' || Array.isArray(value)) {
    return { keys: undefined, malformed: true };
  }
  return { keys: value as Readonly<Record<string, string>>, malformed: false };
}

/** Each key's single element in a slide (keys on more than one element are dropped as ambiguous). */
export function keyed(main: Element, keys: Readonly<Record<string, string>> | undefined): Map<string, Element> {
  const found = new Map<string, Element | null>();
  for (const element of Array.from(main.querySelectorAll('[data-el]'))) {
    const id = element.getAttribute('data-el') ?? '';
    const key =
      keys === undefined ? element.getAttribute('data-sd-match') : Object.hasOwn(keys, id) ? keys[id] : undefined;
    if (typeof key !== 'string' || key === '') {
      continue;
    }
    found.set(key, found.has(key) ? null : element);
    if (found.size > MATCHES) {
      break;
    }
  }
  const out = new Map<string, Element>();
  for (const [key, element] of found) {
    if (element !== null) {
      out.set(key, element);
    }
  }
  return out;
}

function box(element: Element, layer: HTMLElement, width: number) {
  const r = element.getBoundingClientRect();
  const f = layer.getBoundingClientRect();
  const scale = f.width > 0 ? f.width / width : 1;
  return { x: (r.left - f.left) / scale, y: (r.top - f.top) / scale, w: r.width / scale, h: r.height / scale };
}

function rotation(element: Element): number {
  const value = element.ownerDocument.defaultView?.getComputedStyle(element).rotate ?? '';
  const match = /^(-?[\d.]+)deg$/.exec(value.trim());
  return match === null ? 0 : Number(match[1]);
}

/** An element's own opacity (1 where it reads none). */
function opacity(element: Element): number {
  const own = Number.parseFloat(element.ownerDocument.defaultView?.getComputedStyle(element).opacity ?? '1');
  return Number.isFinite(own) ? own : 1;
}

/**
 * The move of `element` by (dx, dy) slide px, turning by `turn` degrees and scaling by (sx, sy), added to its own
 * transform: it ends there when `to`, else it starts there and ends on its own box. The translation is in the
 * element's own frame (its static rotation undone).
 */
function flip(element: Element, dx: number, dy: number, turn: number, sx: number, sy: number, to: boolean) {
  const rad = (-rotation(element) * Math.PI) / 180;
  const keyframes = [
    { transform: 'translate(0px, 0px) rotate(0deg) scale(1, 1)' },
    {
      transform: `translate(${dx * Math.cos(rad) - dy * Math.sin(rad)}px, ${dx * Math.sin(rad) + dy * Math.cos(rad)}px) rotate(${turn}deg) scale(${sx}, ${sy})`,
    },
  ];
  return { target: element, keyframes: to ? keyframes : keyframes.reverse(), composite: 'add' as const };
}

/** Samples of an incoming match's opacity. */
const SAMPLES = 8;

/**
 * The opacity of an incoming match of opacity `b` beneath the outgoing slide, which fades out as one picture over the
 * curve's progress w and holds its match at opacity `a`: w·b / (1 − (1 − w)·a), so the two weigh (1 − w)·a and w·b
 * and cover as one element of opacity (1 − w)·a + w·b where nothing else of either slide lies under them.
 */
export function beneath(a: number, b: number): Keyframe[] {
  const keyframes: Keyframe[] = [];
  for (let i = 0; i <= SAMPLES; i++) {
    const w = i / SAMPLES;
    const rest = 1 - (1 - w) * a;
    keyframes.push({ offset: w, opacity: rest > 1e-6 ? (w * b) / rest : b });
  }
  return keyframes;
}

/** The FLIP plan between two slides. */
export function morphPlan(scene: MorphScene): TransitionPlan {
  const outMain = slideMain(scene.out);
  const inMain = slideMain(scene.into);
  if (outMain === null || inMain === null) {
    return { parts: [{ target: scene.into, keyframes: [{ opacity: 0 }, { opacity: 1 }] }] };
  }
  const outs = keyed(outMain, scene.outKeys);
  const ins = keyed(inMain, scene.inKeys);
  const pairs: [Element, Element][] = [];
  for (const [key, a] of outs) {
    const b = ins.get(key);
    if (b !== undefined) {
      pairs.push([a, b]);
    }
  }
  const parts: TransitionPart[] = [];
  const hide = (target: Element) => parts.push({ target, keyframes: [{ opacity: 0 }, { opacity: 0 }] });
  for (const [a, b] of pairs) {
    const from = box(a, scene.out, scene.size.width);
    const to = box(b, scene.into, scene.size.width);
    const dx = to.x + to.w / 2 - (from.x + from.w / 2);
    const dy = to.y + to.h / 2 - (from.y + from.h / 2);
    const turn = rotation(b) - rotation(a);
    // The two move in step: the outgoing element onto the incoming box, the incoming one from the outgoing box.
    parts.push(flip(a, dx, dy, turn, from.w > 0 ? to.w / from.w : 1, from.h > 0 ? to.h / from.h : 1, true));
    parts.push(flip(b, -dx, -dy, -turn, to.w > 0 ? from.w / to.w : 1, to.h > 0 ? from.h / to.h : 1, false));
    parts.push({ target: b, keyframes: beneath(opacity(a), opacity(b)) });
  }
  // The outgoing slide fades out as one picture over the move, so its matches never show one another through.
  parts.push({ target: scene.out, keyframes: [{ opacity: 1 }, { opacity: 0 }] });
  const background = (main: Element) => {
    const first = main.firstElementChild;
    return first?.hasAttribute('data-sd-bg') === true ? first : null;
  };
  const backgrounds = [background(outMain), background(inMain)] as const;
  const ids = (main: Element) =>
    new Set(Array.from(main.querySelectorAll('[data-el]'), (e) => e.getAttribute('data-el')));
  // Each slide's unmatched objects, walking into a holder of matches: the outgoing slide's fade out, the incoming
  // slide's in; an element on both slides by id shows from the incoming slide; the outgoing background goes beneath
  // the incoming one, which fades in over the whole move. A fade plays on the given third of the time with Keynote's
  // Magic Move, else on the curve's progress over the whole move, or at once.
  for (const side of [0, 1] as const) {
    const [main, other] = side === 0 ? [outMain, inMain] : [inMain, outMain];
    const matched = pairs.map((pair) => pair[side]);
    const shared = ids(other);
    const visit = (parent: Element) => {
      for (const child of Array.from(parent.children)) {
        if (matched.includes(child)) {
          continue;
        }
        if (matched.some((element) => child.contains(element))) {
          visit(child);
          continue;
        }
        const own = opacity(child);
        const [start, end] = side === 0 ? [own, 0] : [0, own];
        if (child.hasAttribute('data-el') && shared.has(child.getAttribute('data-el'))) {
          if (side === 0) {
            hide(child);
          }
        } else if (child === backgrounds[side] && backgrounds[1] !== null) {
          if (side === 0) {
            hide(child);
          } else {
            parts.push({ target: child, keyframes: [{ opacity: 0 }, { opacity: own }] });
          }
        } else if (scene.fadeUnmatched) {
          const keyframes = [{ opacity: start }, { opacity: end }];
          parts.push(
            scene.thirds === true
              ? { target: child, keyframes, window: side === 0 ? EXIT : ENTER }
              : { target: child, keyframes },
          );
        } else {
          parts.push({
            target: child,
            keyframes: [
              { offset: 0, opacity: start },
              { offset: AT_ONCE, opacity: end },
              { offset: 1, opacity: end },
            ],
          });
        }
      }
    };
    visit(main);
  }
  const temporary: Element[] = [];
  if (backgrounds[0] !== null && backgrounds[1] !== null) {
    const beneath = backgrounds[0].cloneNode(true) as Element;
    inMain.insertBefore(beneath, backgrounds[1]);
    temporary.push(beneath);
  }
  return { parts, under: true, temporary };
}
