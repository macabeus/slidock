// Reads a slide's timeline from the deck JSON, which is untrusted input. Effects, params and timing values are read
// where they play (vocab.ts, schedule.ts); this reads the structure every later stage relies on — each step's target,
// trigger, kind, delivery and fallback — and bounds the units a slide builds, so a slide's state stays cheap to
// compute. A field outside the contract fails closed, with an issue:
// - a step that is not an object, or whose target, kind or trigger does not read, is dropped (an unknown trigger is
//   an interactive one, which the compilers drop too);
// - an unknown delivery builds the target whole (`all`), a malformed paragraph range builds the whole element, an
//   unknown fallback is `appear`, a native id that is not a string reads as empty;
// - steps past `STEPS_PER_SLIDE`, and steps that would take the slide past `UNITS_PER_SLIDE` units, are dropped.
import type { DeckStep, Delivery, StepFallback, StepKind, Trigger } from '@slidock/types';

import { STEPS_PER_SLIDE, clickGroupCount } from './schedule';

/** Build units (whole elements, backgrounds, paragraphs of a range) one slide's steps address at most, together. */
export const UNITS_PER_SLIDE = 10_000;

export type TimelineField =
  | 'timeline'
  | 'step'
  | 'target'
  | 'kind'
  | 'trigger'
  | 'delivery'
  | 'fallback'
  | 'paragraphs'
  | 'native'
  | 'steps'
  | 'units';

/** A step field outside the contract: the step was dropped, or the field read as its declared default. */
export interface TimelineIssue {
  /** The step's index in the deck's timeline (`-1` for the timeline itself). */
  readonly index: number;
  readonly field: TimelineField;
  readonly dropped: boolean;
}

export interface Timeline {
  /** The steps that read, in order; every structural field holds a contract value. */
  readonly steps: readonly DeckStep[];
  readonly issues: readonly TimelineIssue[];
}

export const EMPTY_TIMELINE: Timeline = { steps: [], issues: [] };

const TRIGGERS: readonly Trigger[] = ['click', 'withPrevious', 'afterPrevious'];
const KINDS: readonly StepKind[] = ['entrance', 'exit', 'emphasis', 'motion', 'media'];
const DELIVERIES: readonly Delivery[] = ['all', 'byParagraph', 'byWord', 'byChar'];
const FALLBACKS: readonly StepFallback[] = ['appear', 'dissolve', 'none'];

function oneOf<T extends string>(values: readonly T[], value: unknown): T | null {
  return typeof value === 'string' && (values as readonly string[]).includes(value) ? (value as T) : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** A paragraph range `[first, last]`, or `null` when the value is not one. */
function readRange(value: unknown): [number, number] | null {
  if (!Array.isArray(value) || value.length !== 2) {
    return null;
  }
  const [first, last] = value as unknown[];
  return Number.isSafeInteger(first) &&
    Number.isSafeInteger(last) &&
    (first as number) >= 0 &&
    (last as number) >= (first as number)
    ? [first as number, last as number]
    : null;
}

/** Reads a step's target; `null` when it names no element. */
function readTarget(value: unknown, issue: (field: TimelineField) => void): DeckStep['target'] | null {
  if (!isRecord(value) || typeof value.element !== 'string' || value.element === '') {
    return null;
  }
  const background = value.background;
  if (background !== undefined && typeof background !== 'boolean') {
    return null;
  }
  let paragraphs: [number, number] | null = null;
  if (value.paragraphs !== null && value.paragraphs !== undefined) {
    paragraphs = readRange(value.paragraphs);
    if (paragraphs === null) {
      issue('paragraphs');
    }
  }
  return background === true
    ? { element: value.element, paragraphs, background: true }
    : { element: value.element, paragraphs };
}

/** How many build units a target addresses. */
function unitCount(target: DeckStep['target']): number {
  if (target.background === true || target.paragraphs === null) {
    return 1;
  }
  return target.paragraphs[1] - target.paragraphs[0] + 1;
}

/** Reads a slide's timeline; never throws. */
export function readTimeline(value: unknown): Timeline {
  if (value === undefined || value === null) {
    return EMPTY_TIMELINE;
  }
  if (!Array.isArray(value)) {
    return { steps: [], issues: [{ index: -1, field: 'timeline', dropped: true }] };
  }
  const steps: DeckStep[] = [];
  const issues: TimelineIssue[] = [];
  let units = 0;
  const raw = value as readonly unknown[];
  for (let index = 0; index < raw.length; index++) {
    if (index >= STEPS_PER_SLIDE) {
      issues.push({ index, field: 'steps', dropped: true });
      break;
    }
    const item = raw[index];
    if (!isRecord(item)) {
      issues.push({ index, field: 'step', dropped: true });
      continue;
    }
    const defaults: TimelineField[] = [];
    const target = readTarget(item.target, (field) => defaults.push(field));
    if (target === null) {
      issues.push({ index, field: 'target', dropped: true });
      continue;
    }
    const kind = oneOf(KINDS, item.kind);
    if (kind === null) {
      issues.push({ index, field: 'kind', dropped: true });
      continue;
    }
    const trigger = oneOf(TRIGGERS, item.trigger);
    if (trigger === null) {
      issues.push({ index, field: 'trigger', dropped: true });
      continue;
    }
    const count = unitCount(target);
    if (count > UNITS_PER_SLIDE - units) {
      issues.push({ index, field: 'units', dropped: true });
      continue;
    }
    units += count;
    let delivery = oneOf(DELIVERIES, item.delivery);
    if (delivery === null) {
      defaults.push('delivery');
      delivery = 'all';
    }
    let fallback = oneOf(FALLBACKS, item.fallback);
    if (fallback === null) {
      defaults.push('fallback');
      fallback = 'appear';
    }
    let native = item.native;
    if (typeof native !== 'string') {
      defaults.push('native');
      native = '';
    }
    for (const field of defaults) {
      issues.push({ index, field, dropped: false });
    }
    steps.push({ ...(item as DeckStep), target, kind, trigger, delivery, fallback, native: native as string });
  }
  return { steps, issues };
}

const read = new WeakMap<object, Timeline>();

/** A slide's timeline, read once per slide object. */
export function slideTimeline(slide: { readonly timeline?: unknown }): Timeline {
  if (typeof slide !== 'object' || slide === null) {
    return EMPTY_TIMELINE;
  }
  let timeline = read.get(slide);
  if (timeline === undefined) {
    timeline = readTimeline(slide.timeline);
    read.set(slide, timeline);
  }
  return timeline;
}

/**
 * The click groups the player plays on a slide: its `click` steps among the steps that read (a malformed step is
 * dropped, as the player drops it). A position's `step` counts up to it.
 */
export function stepCount(slide: { readonly timeline?: unknown }): number {
  return clickGroupCount(slideTimeline(slide).steps);
}
