// The position machine: a pure reducer over `(slide, step)`. Viewing steps through every slide,
// presenting skips hidden ones; `step` counts the click groups played, and is pinned to the slide's last group
// while viewing without builds. An action that moves nothing returns the same position object.

/** A blanked screen: black or white. */
export type Blank = 'black' | 'white';

export interface Position {
  /** The slide's index in show order, hidden slides included. */
  readonly slide: number;
  /** The click groups played, from 0 to the slide's click-group count. */
  readonly step: number;
  readonly presenting: boolean;
  readonly blank: Blank | null;
}

/** What the machine knows of a slide. */
export interface SlideModel {
  readonly hidden: boolean;
  /** The slide's click groups: its `click` steps. */
  readonly clickGroups: number;
}

export interface MachineModel {
  readonly slides: readonly SlideModel[];
  /** Whether viewing plays builds; when not, a viewed slide shows fully built. */
  readonly builds: boolean;
  /** Whether presenting wraps from the last shown slide to the first. */
  readonly loop: boolean;
}

export type Action =
  | { readonly type: 'next' }
  | { readonly type: 'previous' }
  | { readonly type: 'first' }
  | { readonly type: 'last' }
  | { readonly type: 'goTo'; readonly slide: number; readonly step?: number }
  | { readonly type: 'present' }
  | { readonly type: 'stop' }
  | { readonly type: 'blank'; readonly kind: Blank | null }
  | { readonly type: 'toggleBlank'; readonly kind: Blank };

/**
 * How the player reaches the new position: `none` (nothing to draw but the blank), `cut` (no motion), `build`
 * (play the next click group) or `transition` (the next slide's transition, then its automatic group).
 */
export type Motion = 'none' | 'cut' | 'build' | 'transition';

export interface Move {
  readonly position: Position;
  readonly motion: Motion;
}

export function initialPosition(model: MachineModel, presenting: boolean): Position {
  const start: Position = { slide: 0, step: 0, presenting: false, blank: null };
  const viewing = { ...start, step: landingStep(model, start, 0, false) };
  return presenting ? reduce(model, viewing, { type: 'present' }).position : viewing;
}

/** Whether `position` plays builds: presenting, or viewing with builds on. */
export function playsBuilds(model: MachineModel, position: Position): boolean {
  return position.presenting || model.builds;
}

export function clickGroups(model: MachineModel, slide: number): number {
  return model.slides[slide]?.clickGroups ?? 0;
}

/** Whether `slide` can be landed on: every slide while viewing, shown ones while presenting. */
export function navigable(model: MachineModel, slide: number, presenting: boolean): boolean {
  const entry = model.slides[slide];
  return entry !== undefined && !(presenting && entry.hidden);
}

function landingStep(model: MachineModel, position: Position, slide: number, atEnd: boolean): number {
  return atEnd || !playsBuilds(model, position) ? clickGroups(model, slide) : 0;
}

function seek(model: MachineModel, from: number, delta: 1 | -1, presenting: boolean): number | null {
  for (let i = from + delta; i >= 0 && i < model.slides.length; i += delta) {
    if (navigable(model, i, presenting)) {
      return i;
    }
  }
  return null;
}

function same(a: Position, b: Position): boolean {
  return a.slide === b.slide && a.step === b.step && a.presenting === b.presenting && a.blank === b.blank;
}

function settle(from: Position, to: Position, motion: Motion): Move {
  return same(from, to) ? { position: from, motion: 'none' } : { position: to, motion };
}

export function reduce(model: MachineModel, position: Position, action: Action): Move {
  if (model.slides.length === 0) {
    return { position, motion: 'none' };
  }
  const { presenting } = position;
  const plays = playsBuilds(model, position);
  switch (action.type) {
    case 'next': {
      if (position.blank !== null) {
        return settle(position, { ...position, blank: null }, 'none');
      }
      if (plays && position.step < clickGroups(model, position.slide)) {
        return settle(position, { ...position, step: position.step + 1 }, 'build');
      }
      let next = seek(model, position.slide, 1, presenting);
      if (next === null && presenting && model.loop) {
        next = seek(model, -1, 1, presenting);
      }
      if (next === null) {
        return { position, motion: 'none' };
      }
      return settle(
        position,
        { ...position, slide: next, step: landingStep(model, position, next, false) },
        plays ? 'transition' : 'cut',
      );
    }
    case 'previous': {
      if (position.blank !== null) {
        return settle(position, { ...position, blank: null }, 'none');
      }
      if (plays && position.step > 0) {
        return settle(position, { ...position, step: position.step - 1 }, 'cut');
      }
      const previous = seek(model, position.slide, -1, presenting);
      if (previous === null) {
        return { position, motion: 'none' };
      }
      return settle(position, { ...position, slide: previous, step: clickGroups(model, previous) }, 'cut');
    }
    case 'first':
    case 'last': {
      const target =
        action.type === 'first' ? seek(model, -1, 1, presenting) : seek(model, model.slides.length, -1, presenting);
      if (target === null) {
        return { position, motion: 'none' };
      }
      return settle(
        position,
        { ...position, slide: target, step: landingStep(model, position, target, false), blank: null },
        'cut',
      );
    }
    case 'goTo': {
      const slide = clamp(Math.trunc(action.slide), 0, model.slides.length - 1);
      const groups = clickGroups(model, slide);
      const step = plays ? clamp(Math.trunc(action.step ?? 0), 0, groups) : groups;
      return settle(position, { ...position, slide, step, blank: null }, 'cut');
    }
    case 'present': {
      if (presenting) {
        return { position, motion: 'none' };
      }
      const start = navigable(model, position.slide, true)
        ? position.slide
        : (seek(model, position.slide, 1, true) ?? seek(model, position.slide, -1, true));
      if (start === null) {
        return { position, motion: 'none' };
      }
      return settle(position, { slide: start, step: 0, presenting: true, blank: null }, 'cut');
    }
    case 'stop': {
      if (!presenting) {
        return { position, motion: 'none' };
      }
      const viewing: Position = { ...position, presenting: false, blank: null };
      const step = model.builds ? position.step : clickGroups(model, position.slide);
      return settle(position, { ...viewing, step }, 'cut');
    }
    case 'blank':
      return settle(position, { ...position, blank: action.kind }, 'none');
    case 'toggleBlank':
      return settle(position, { ...position, blank: position.blank === action.kind ? null : action.kind }, 'none');
  }
}

function clamp(value: number, min: number, max: number): number {
  return Number.isNaN(value) ? min : Math.min(Math.max(value, min), max);
}

/** Whether every slide is hidden, so presenting has nothing to show. */
export function everyHidden(model: MachineModel): boolean {
  return model.slides.every((slide) => slide.hidden);
}

/** The 1-based numbers of the shown slides; `null` for a hidden one. */
export function numbering(slides: readonly { readonly hidden: boolean }[]): (number | null)[] {
  let shown = 0;
  return slides.map((slide) => (slide.hidden ? null : ++shown));
}

/** The index of the slide with show number `n`, viewing or presenting: the number the counter shows. */
export function slideForNumber(slides: readonly { readonly hidden: boolean }[], n: number): number | null {
  if (!Number.isInteger(n) || n < 1) {
    return null;
  }
  const index = numbering(slides).indexOf(n);
  return index < 0 ? null : index;
}
