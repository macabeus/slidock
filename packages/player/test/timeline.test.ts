import type { Deck, DeckSlide, DeckStep } from '@slidock/types';
import { describe, expect, it } from 'vitest';

import { stepCount as exported } from '../src/index';
import { machineModel } from '../src/player';
import { STEPS_PER_SLIDE, slideState } from '../src/schedule';
import { UNITS_PER_SLIDE, readTimeline, slideTimeline, stepCount } from '../src/timeline';
import { step } from './fixtures';

const raw = (overrides: Record<string, unknown>): unknown => ({ ...step('s1.title'), ...overrides });

describe('timeline: untrusted structure fails closed', () => {
  it('reads a step in the contract as it is, with no issue', () => {
    const good = step('s1.title', { target: { element: 's1.title', paragraphs: [0, 1] }, effect: 'fade' });
    expect(readTimeline([good])).toEqual({ steps: [good], issues: [] });
    expect(readTimeline(undefined)).toEqual({ steps: [], issues: [] });
  });

  it('drops a step whose target names no element', () => {
    for (const target of [null, undefined, 3, 's1.title', {}, { element: '' }, { element: 7, paragraphs: null }]) {
      const read = readTimeline([raw({ target }), raw({})]);
      expect(read.steps.length, JSON.stringify(target)).toBe(1);
      expect(read.issues).toEqual([{ index: 0, field: 'target', dropped: true }]);
    }
    const background = readTimeline([raw({ target: { element: 's1.title', paragraphs: null, background: 'yes' } })]);
    expect(background.issues).toEqual([{ index: 0, field: 'target', dropped: true }]);
  });

  it('drops a step whose kind or trigger is not in the contract', () => {
    const read = readTimeline([raw({ kind: 'teleport' }), raw({ trigger: 'onMediaEnd' }), raw({ trigger: 'click' })]);
    expect(read.steps.map((s) => s.trigger)).toEqual(['click']);
    expect(read.issues).toEqual([
      { index: 0, field: 'kind', dropped: true },
      { index: 1, field: 'trigger', dropped: true },
    ]);
  });

  it('drops what is not a step, and a timeline that is not a list', () => {
    expect(readTimeline([null, 'x', [], raw({})]).issues.map((i) => i.index)).toEqual([0, 1, 2]);
    expect(readTimeline({ length: 3 })).toEqual({
      steps: [],
      issues: [{ index: -1, field: 'timeline', dropped: true }],
    });
  });

  it('reads an unknown delivery, fallback or paragraph range as its declared default', () => {
    const read = readTimeline([
      raw({ delivery: 'byLine', fallback: 'cover', native: 42 }),
      raw({ target: { element: 's1.title', paragraphs: [3, 1] } }),
      raw({ target: { element: 's1.title', paragraphs: [0.5, 2] } }),
    ]);
    expect(read.steps.map((s) => [s.delivery, s.fallback, s.native, s.target.paragraphs])).toEqual([
      ['all', 'appear', '', null],
      ['all', 'dissolve', 'pptx:fade', null],
      ['all', 'dissolve', 'pptx:fade', null],
    ]);
    expect(read.issues).toEqual([
      { index: 0, field: 'delivery', dropped: false },
      { index: 0, field: 'fallback', dropped: false },
      { index: 0, field: 'native', dropped: false },
      { index: 1, field: 'paragraphs', dropped: false },
      { index: 2, field: 'paragraphs', dropped: false },
    ]);
  });

  it('plays at most STEPS_PER_SLIDE steps', () => {
    const read = readTimeline(Array.from({ length: STEPS_PER_SLIDE + 5 }, () => raw({})));
    expect(read.steps.length).toBe(STEPS_PER_SLIDE);
    expect(read.issues).toEqual([{ index: STEPS_PER_SLIDE, field: 'steps', dropped: true }]);
  });

  it('bounds the units a slide addresses together, not per step', () => {
    const wide = (i: number) => raw({ target: { element: `s1.el${i}`, paragraphs: [0, 9_999] } });
    const read = readTimeline(Array.from({ length: 2_000 }, (_, i) => wide(i)));
    expect(read.steps.length).toBe(1);
    expect(read.issues.length).toBe(1_999);
    expect(read.issues.every((issue) => issue.field === 'units' && issue.dropped)).toBe(true);
    // The state before any group holds every unit the slide addresses, and no more.
    expect(slideState(read.steps, 0).hidden.size).toBe(UNITS_PER_SLIDE);
    // Exactly at the bound reads; one more unit is dropped.
    const half = (first: number) => raw({ target: { element: 's1.title', paragraphs: [first, first + 4_999] } });
    expect(readTimeline([half(0), half(5_000)]).issues).toEqual([]);
    expect(readTimeline([half(0), half(5_000), raw({})]).issues).toEqual([{ index: 2, field: 'units', dropped: true }]);
    // A range wider than the slide's bound is dropped whole.
    expect(readTimeline([raw({ target: { element: 's1.title', paragraphs: [0, UNITS_PER_SLIDE] } })]).steps).toEqual(
      [],
    );
  });

  it('reads a slide once, and a slide that is not an object as empty', () => {
    const slide = { timeline: [raw({})] };
    expect(slideTimeline(slide)).toBe(slideTimeline(slide));
    expect(slideTimeline(null as unknown as { timeline: unknown }).steps).toEqual([]);
    const kept: DeckStep | undefined = slideTimeline(slide).steps[0];
    expect(kept?.target.element).toBe('s1.title');
  });
});

describe('stepCount: the click groups the player plays', () => {
  it('counts the click steps that read, malformed ones dropped, and is exported for the app', () => {
    const timeline = [
      raw({ trigger: 'click' }),
      raw({ trigger: 'withPrevious' }),
      // Dropped: an interactive trigger, a target with no element, a kind outside the contract, not a step.
      raw({ trigger: 'onShapeClick' }),
      raw({ trigger: 'click', target: { element: '' } }),
      raw({ trigger: 'click', kind: 'teleport' }),
      'step',
      raw({ trigger: 'afterPrevious' }),
      raw({ trigger: 'click' }),
    ];
    const slide = { timeline };
    expect(stepCount(slide)).toBe(2);
    // A naive count of the deck's click steps disagrees.
    expect(timeline.filter((s) => (s as { trigger?: unknown }).trigger === 'click').length).toBe(4);
    expect(exported).toBe(stepCount);
    expect(stepCount({})).toBe(0);
    expect(stepCount({ timeline: 'nope' })).toBe(0);
    expect(stepCount({ timeline: Array.from({ length: STEPS_PER_SLIDE + 5 }, () => raw({})) })).toBe(STEPS_PER_SLIDE);
  });

  it('is the count the player navigates: the last step of a slide is stepCount', () => {
    const deck = minimalDeck([
      raw({ trigger: 'click' }),
      raw({ trigger: 'click', kind: 'teleport' }),
      raw({ trigger: 'click' }),
    ]);
    expect(machineModel(deck, {}).slides[0]?.clickGroups).toBe(stepCount(deck.slides[0] as DeckSlide));
  });
});

function minimalDeck(timeline: unknown[]): Deck {
  return {
    slides: [{ id: 's1', hidden: false, timeline } as unknown as DeckSlide],
  } as unknown as Deck;
}
