// The transition runner: plays the incoming slide's transition with the Web Animations API on
// the outgoing and incoming layers. It plays the transition's effect in the common vocabulary when this player knows
// it, else the IR's canonical fallback (none, dissolve, push, wipe). With reduced motion every transition but a cut
// is a 150 ms fade.
import type { DeckTransition, TransitionEffect, TransitionFallback } from '@slidock/types';

import { type Curve, EASE_IN_OUT, LINEAR, cssEasing, easeKeyframes, readEasing } from './effects/easing';
import { morphPlan } from './effects/morph';
import { type TransitionPart, type TransitionPlan, transitionRecipe } from './effects/transitions';
import { LIMITS, readTransition } from './effects/vocab';
import { adoptSlideCss } from './slide';

export const REDUCED_MOTION_MS = 150;

/** What plays for a transition: its effect, its fallback, or the reduced-motion fade. */
export interface PlannedTransition {
  /** The effect in the vocabulary, or `null` for the fallback. */
  readonly effect: TransitionEffect | null;
  readonly fallback: TransitionFallback;
  readonly durationMs: number;
  readonly curve: Curve | null;
  /** Why the effect did not play, when the deck names one this player cannot play. */
  readonly issue: string | null;
}

const FALLBACKS: readonly TransitionFallback[] = ['none', 'dissolve', 'push', 'wipe'];

/** A transition's canonical fallback; one outside the contract is a cut. */
function readFallback(value: unknown): TransitionFallback | null {
  return typeof value === 'string' && (FALLBACKS as readonly string[]).includes(value)
    ? (value as TransitionFallback)
    : null;
}

/** Decides what a transition plays. The deck is untrusted: a transition that is not an object plays none. */
export function planTransition(transition: DeckTransition | null, reduced: boolean): PlannedTransition {
  const none: PlannedTransition = { effect: null, fallback: 'none', durationMs: 0, curve: null, issue: null };
  if (transition === null) {
    return none;
  }
  if (typeof transition !== 'object' || Array.isArray(transition)) {
    return { ...none, issue: 'malformed transition' };
  }
  const duration = transition.durationMs;
  const validDuration =
    typeof duration === 'number' && Number.isFinite(duration) && duration > 0 && duration <= LIMITS.ms;
  const read = readTransition(transition);
  const effect = 'effect' in read ? read.effect : null;
  const fallback = readFallback(transition.fallback);
  const issues: string[] = [];
  if ('unread' in read && read.unread !== 'absent') {
    issues.push(`${read.unread} transition ${read.name ?? '?'}`);
  }
  if (fallback === null) {
    issues.push('unknown fallback');
  }
  const issue = issues.length === 0 ? null : issues.join(', ');
  const cut =
    effect !== null
      ? effect.effect === 'none' || (effect.effect === 'cut' && !effect.params.throughBlack)
      : fallback === null || fallback === 'none';
  if (cut || !validDuration) {
    return { ...none, issue: validDuration || cut ? issue : 'invalid duration' };
  }
  if (reduced) {
    return {
      effect: { effect: 'fade', params: { through: 'none' } },
      fallback: 'dissolve',
      durationMs: REDUCED_MOTION_MS,
      curve: LINEAR,
      issue,
    };
  }
  const curve = transition.easing === undefined ? null : readEasing(transition.easing);
  return {
    effect,
    fallback: fallback ?? 'none',
    durationMs: duration,
    curve,
    issue,
  };
}

/** The keyframes of the canonical fallbacks: the incoming layer on top, the outgoing one under it. */
export function fallbackKeyframes(effect: TransitionFallback): { in: Keyframe[]; out: Keyframe[] | null } {
  switch (effect) {
    case 'dissolve':
      return { in: [{ opacity: 0 }, { opacity: 1 }], out: null };
    case 'push':
      return {
        in: [{ transform: 'translateX(100%)' }, { transform: 'translateX(0)' }],
        out: [{ transform: 'translateX(0)' }, { transform: 'translateX(-100%)' }],
      };
    case 'wipe':
      return {
        in: [
          { offset: 0, clipPath: 'inset(0 100% 0 0)' },
          { offset: 1, clipPath: 'inset(0 0 0 0)' },
          { offset: 1, clipPath: 'none' },
        ],
        out: null,
      };
    case 'none':
      return { in: [], out: null };
  }
}

export interface TransitionStage {
  readonly out: HTMLElement | null;
  readonly into: HTMLElement;
  readonly size: { readonly width: number; readonly height: number };
  readonly seed: number;
  readonly properties: boolean;
  readonly cells: number;
  readonly outKeys?: Readonly<Record<string, string>> | undefined;
  readonly inKeys?: Readonly<Record<string, string>> | undefined;
}

export interface TransitionRun {
  readonly animations: Animation[];
  /** Removes the temporary layers and the stacking the transition set. */
  cleanup(): void;
  /** The effect could not play here (it then played its fallback). */
  readonly unplayable: string | null;
}

function cloneLayer(layer: HTMLElement, side: 'in' | 'out'): HTMLElement {
  const document = layer.ownerDocument;
  const clone = document.createElement('div');
  clone.className = 'sd-layer';
  clone.setAttribute('data-sd-clone', side);
  clone.style.width = layer.style.width;
  clone.style.height = layer.style.height;
  const root = clone.attachShadow({ mode: 'open' });
  adoptSlideCss(document, root);
  for (const child of Array.from(layer.shadowRoot?.childNodes ?? [])) {
    if ((child as Element).localName !== 'style') {
      root.append(child.cloneNode(true));
    }
  }
  layer.parentNode?.appendChild(clone);
  return clone;
}

function overlayLayer(stage: TransitionStage, color: string): HTMLElement {
  const overlay = stage.into.ownerDocument.createElement('div');
  overlay.className = 'sd-overlay';
  overlay.style.background = color;
  overlay.style.opacity = '0';
  stage.into.parentNode?.appendChild(overlay);
  return overlay;
}

/** Starts the transition from `out` to `into`; returns its animations (none for a cut). */
export function runTransition(stage: TransitionStage, plan: PlannedTransition): TransitionRun {
  const temporary: Element[] = [];
  const masked: Element[] = [];
  const cleanup = (): void => {
    for (const element of temporary.splice(0)) {
      element.remove();
    }
    stage.into.removeAttribute('data-sd-under');
    for (const element of masked.splice(0)) {
      element.removeAttribute('data-sd-mask');
    }
  };
  const { out, into } = stage;
  if (plan.durationMs <= 0 || typeof into.animate !== 'function') {
    return { animations: [], cleanup, unplayable: null };
  }
  let recipe: TransitionPlan | null = null;
  let unplayable: string | null = null;
  if (plan.effect !== null) {
    if (plan.effect.effect === 'morph' || plan.effect.effect === 'magicMove') {
      recipe =
        out === null
          ? null
          : morphPlan({
              out,
              into,
              size: stage.size,
              outKeys: stage.outKeys,
              inKeys: stage.inKeys,
              fadeUnmatched: plan.effect.effect === 'morph' || plan.effect.params.fadeUnmatched,
              thirds: plan.effect.effect === 'magicMove',
            });
    } else {
      recipe = transitionRecipe(plan.effect, {
        out,
        into,
        size: stage.size,
        seed: stage.seed,
        properties: stage.properties,
        cells: stage.cells,
        clone: (layer) => {
          const clone = cloneLayer(layer, layer === into ? 'in' : 'out');
          temporary.push(clone);
          return clone;
        },
        overlay: (color) => {
          const overlay = overlayLayer(stage, color);
          temporary.push(overlay);
          return overlay;
        },
      });
    }
    if (recipe === null) {
      unplayable = plan.effect.effect;
    }
  }
  if (recipe === null) {
    const keyframes = fallbackKeyframes(plan.fallback);
    const parts: TransitionPart[] = [{ target: into, keyframes: keyframes.in }];
    if (out !== null && keyframes.out !== null) {
      parts.push({ target: out, keyframes: keyframes.out });
    }
    recipe = { parts: keyframes.in.length === 0 ? [] : parts };
  }
  temporary.push(...(recipe.temporary ?? []));
  if (recipe.under === true) {
    into.setAttribute('data-sd-under', '');
  }
  // Composited while moving; the player clears it when the transition ends, so text re-rasterizes at the final scale.
  into.style.willChange = 'transform, opacity';
  if (out !== null) {
    out.style.willChange = 'transform, opacity';
  }
  const curve = plan.curve ?? (recipe.easing === 'linear' ? LINEAR : EASE_IN_OUT);
  const animations: Animation[] = [];
  for (const part of recipe.parts) {
    if (part.mask !== undefined) {
      part.target.setAttribute('data-sd-mask', part.mask);
      masked.push(part.target);
    }
    if (typeof part.target.animate === 'function') {
      const eased = easeKeyframes(part.keyframes, curve);
      // A part on a window starts late and holds to the transition's end, so every part ends together.
      const [start, end] = part.window ?? [0, 1];
      animations.push(
        part.target.animate(eased ?? part.keyframes, {
          delay: start * plan.durationMs,
          duration: (end - start) * plan.durationMs,
          endDelay: (1 - end) * plan.durationMs,
          easing: eased === null ? cssEasing(curve) : 'linear',
          fill: 'both',
          ...(part.composite === undefined ? {} : { composite: part.composite }),
        }),
      );
    }
  }
  return { animations, cleanup, unplayable };
}
