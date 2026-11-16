// The player: one stage, each slide in its own shadow root, fitted to the host; the current slide and its
// neighbours stay mounted. Navigation runs through the position machine; moving forward animates (transitions,
// builds), moving back cuts. A slide shows once the readiness gate opens.
import type { Deck, DeckSlide } from '@slidock/types';

import { type Advance, autoNext, readAdvance } from './advance';
import { TEXT_VIEW_ID, ensureChrome, helpOverlay } from './chrome';
import type { DeckSource } from './deck';
import { el } from './dom';
import { cellBudget } from './effects/clip';
import { readMatchKeys } from './effects/morph';
import { ensureProperties } from './effects/properties';
import { hashSeed } from './effects/random';
import { Emitter, type Listener, type PlayerEvents } from './events';
import { LINK_ACTIONS, activates, clickZone, isActivatable, isEditable, keyCommand, linkTarget } from './input';
import {
  type Action,
  type Blank,
  type MachineModel,
  type Move,
  type Position,
  initialPosition,
  navigable,
  numbering,
  reduce,
  slideForNumber,
} from './machine';
import {
  type Carry,
  type MediaPolicy,
  carriedFrames,
  carryMedia,
  handOver,
  hideMedia,
  playAlong,
  prepareMedia,
  releaseCarry,
  showMedia,
} from './media';
import type { Sync, SyncRole, SyncState } from './presenter/sync';
import { ready } from './ready';
import { isSafeLink } from './sanitize';
import { type Issue, STEPS_PER_SLIDE, type ScheduledStep, groups } from './schedule';
import {
  type MountedSlide,
  type SlideEnv,
  applyState,
  cancelAnimations,
  cutLoops,
  finishAll,
  mountSlide,
  playGroup,
  restoreSplits,
} from './slide';
import { type TimelineField, type TimelineIssue, UNITS_PER_SLIDE, slideTimeline, stepCount } from './timeline';
import { planTransition, runTransition } from './transition';

/** Runtime diagnostic codes. */
export const RT = {
  readyTimeout: 'RT0001',
  sanitized: 'RT0002',
  everyHidden: 'RT0003',
  presenterBlocked: 'RT0004',
  missingSlide: 'RT0005',
  /** A step or a transition names an effect this player cannot play: it plays its canonical fallback. */
  effectFallback: 'RT0006',
  /** A step's timing value or field is out of the contract: it plays the declared default. */
  timingDefault: 'RT0007',
  /** A step's structure is out of the contract (its target, kind or trigger, or a slide bound): it is dropped. */
  stepDropped: 'RT0008',
  /** A move failed; the player stays at the position it reached and goes on from there. */
  moveFailed: 'RT0009',
} as const;

const TIMELINE_ISSUES: Readonly<Record<TimelineField, string>> = {
  timeline: 'the timeline is not a list; no step plays',
  step: 'not a step; dropped',
  target: 'its target names no element; dropped',
  kind: 'its kind is not in the contract; dropped',
  trigger: 'its trigger is not in the contract (an interactive trigger); dropped',
  steps: `past the ${STEPS_PER_SLIDE} steps a slide plays; dropped`,
  units: `past the ${UNITS_PER_SLIDE} build units a slide addresses; dropped`,
  delivery: 'its delivery is not in the contract; built whole',
  fallback: 'its fallback is not in the contract; played appear',
  paragraphs: 'its paragraph range is malformed; built the whole element',
  native: 'its native id is not a string',
};

/** One message per kind of timeline issue on a slide: the first step it hit, and how many. */
export function timelineMessages(issues: readonly TimelineIssue[]): { code: string; message: string }[] {
  const byField = new Map<string, TimelineIssue[]>();
  for (const issue of issues) {
    const key = `${issue.field}\u0000${issue.dropped}`;
    byField.set(key, [...(byField.get(key) ?? []), issue]);
  }
  return Array.from(byField.values(), (list) => {
    const first = list[0] as TimelineIssue;
    const where =
      first.index < 0
        ? 'timeline'
        : list.length === 1
          ? `step ${first.index}`
          : `${list.length} steps from step ${first.index}`;
    return {
      code: first.dropped ? RT.stepDropped : RT.timingDefault,
      message: `${where}: ${TIMELINE_ISSUES[first.field]}`,
    };
  });
}

export interface PlayerOptions {
  /** `view` shows slides fully built and every slide; `present` skips hidden slides and plays builds. */
  readonly mode?: 'view' | 'present';
  /** Play builds while viewing. */
  readonly builds?: boolean;
  /** Play transitions while viewing. */
  readonly transitions?: boolean;
  readonly reducedMotion?: 'auto' | 'always' | 'never';
  readonly startAt?: { readonly slide: number; readonly step?: number };
  readonly loop?: boolean;
  /**
   * Where keys are read: the whole document, the player's own element (focusable; the whole document while it
   * presents), or nowhere.
   */
  readonly keyboard?: 'document' | 'host' | 'off';
  /** Click zones and swipes: while presenting, always, or never. Links in slides work in every case. */
  readonly pointer?: 'present' | 'always' | 'off';
  /** The slide counter and the help overlay. */
  readonly chrome?: boolean;
  /** Plays every medium muted (the presenter view: the sound is the deck window's). */
  readonly muted?: boolean;
  /**
   * Joins a presenter-view sync session at once, with `presenter` (the deck page does, so a reloaded presenter finds
   * it). Only moves made while presenting are sent; the presenter view's player always presents.
   */
  readonly sync?: { readonly role: SyncRole; readonly session?: string };
  /** The presenter window's URL for a session; the same page with `#presenter=<session>` by default. */
  readonly presenterUrl?: (session: string) => string;
  /**
   * The presenter view (`@slidock/player/presenter`'s `presenter`; a deck page's `SlidockPresenter`). Without it the
   * player offers none: no P key or help row, no sync session, and `openPresenter` opens nothing.
   */
  readonly presenter?: PresenterSupport;
  readonly readyTimeoutMs?: number;
}

/** What a player joining a presenter-view sync session tells the presenter view. */
export interface JoinOptions {
  /** The deck's `id`, which keys the channel. */
  readonly deck: string;
  readonly role: SyncRole;
  /** The session to join; a new one when absent. */
  readonly session?: string;
  /** The other window, as this one knows it: the window it opened, or its opener. */
  readonly peer: () => Window | null;
  /** The window that answered from the other end. */
  readonly onPeer: (peer: Window) => void;
  /** The local position, sent to a window that says hello. */
  readonly current: () => SyncState;
  /** Follows the other window. */
  readonly apply: (state: SyncState) => void;
}

export interface PresenterOptions extends Pick<PlayerOptions, 'reducedMotion' | 'readyTimeoutMs' | 'loop' | 'startAt'> {
  /** The deck window's session (from the URL); an empty one is adopted from the deck window. */
  readonly session: string;
}

export interface PresenterView {
  readonly player: Player;
  destroy(): void;
}

/**
 * The presenter view, which the deck player leaves out (`@slidock/player/presenter` holds it, and a deck page carries
 * it as a script of its own only when it ships a presenter view): the sync between the deck window and the presenter
 * window, the opening of that window, and the presenter window's page.
 */
export interface PresenterSupport {
  /** The help overlay's row for the P key. */
  readonly help: readonly [string, string];
  /** The session a `#presenter[=<session>]` hash names, or `null` when the page is not a presenter window. */
  session(hash: string): string | null;
  /** Joins a sync session from `view`. */
  join(view: Window, options: JoinOptions): Sync;
  /**
   * Opens the presenter window at `url` for the deck `deck` (one window per deck) and places it beside the deck's
   * `stage`; `null` when the browser blocks it.
   */
  open(view: Window, url: string, deck: string, stage: HTMLElement): Window | null;
  /** The presenter window's page in `host`: the window's header and the presenter view of `source`. */
  mount(host: HTMLElement, source: DeckSource, options: PresenterOptions): PresenterView;
}

export interface Player {
  readonly deck: Deck;
  readonly position: Position;
  next(): void;
  previous(): void;
  first(): void;
  last(): void;
  /** Cuts to a slide (index, or id) at a step. */
  goTo(slide: number | string, step?: number): void;
  /**
   * Starts presenting, full screen when the browser allows it, else filling the window; resolves once the show fills
   * the screen. A `stop` while the browser has yet to answer cancels it: the page is then left as it was.
   */
  present(): Promise<void>;
  stop(): void;
  blank(kind: Blank | null): void;
  /** Renders slide `slide` at `step` into `target`, still and scaled; returns its disposer. */
  renderStatic(slide: number | string, step: number, target: HTMLElement, fit?: 'contain' | 'width'): () => void;
  /** Opens the presenter view in a second window, presenting from here if this player is not yet. */
  openPresenter(): Window | null;
  /** Follows another window's position: animated when it is this window's next move and `animate` is set. */
  follow(state: SyncState, animate: boolean): void;
  on<K extends keyof PlayerEvents>(type: K, listener: Listener<K>): () => void;
  destroy(): void;
}

interface Run {
  readonly token: number;
  skip: boolean;
  /** Removes a transition's temporary layers. */
  cleanup: (() => void) | null;
  readonly abort: AbortController;
  readonly animations: Set<Animation>;
  /** The `next`s that came once what the move plays had played to its end: the move goes on to them when it settles. */
  queued: number;
  /** The automatic group still to play once the transition ends: a press after the transition's end finishes it. */
  follows?: boolean;
  /** The clips this move carries into the incoming slide, sought but not yet handed over. */
  carry: Carry[];
}

interface FullscreenRequest {
  /** A show waits on it: false once the show stops. */
  wanted: boolean;
  presenterOpened: boolean;
  /** Resolves once the browser answered and the show (or the page) was left as that answer says. */
  settled: Promise<void>;
}

function settled(animations: Iterable<Animation>): Promise<void> {
  return Promise.all(Array.from(animations, (a) => a.finished.catch(() => undefined))).then(() => undefined);
}

export function pixelSize(deck: Deck): { width: number; height: number } {
  return { width: (deck.size.w * 4) / 3, height: (deck.size.h * 4) / 3 };
}

export function machineModel(deck: Deck, options: Pick<PlayerOptions, 'builds' | 'loop'>): MachineModel {
  return {
    slides: deck.slides.map((slide) => ({
      hidden: slide.hidden,
      clickGroups: stepCount(slide),
    })),
    builds: options.builds ?? false,
    loop: options.loop ?? false,
  };
}

/** Scales `frame` (the deck's pixel size) into `box`: letterboxed, or to the box's width. */
function fit(box: HTMLElement, frame: HTMLElement, size: { width: number; height: number }, mode: 'contain' | 'width') {
  const width = box.clientWidth;
  if (mode === 'width') {
    const scale = width / size.width;
    box.style.height = `${size.height * scale}px`;
    frame.style.transform = `scale(${scale})`;
    return;
  }
  const height = box.clientHeight;
  const scale = Math.min(width / size.width, height / size.height) || 0;
  const x = (width - size.width * scale) / 2;
  const y = (height - size.height * scale) / 2;
  frame.style.transform = `translate(${x}px,${y}px) scale(${scale})`;
}

function observeSize(element: HTMLElement, onResize: () => void): () => void {
  const view = element.ownerDocument.defaultView;
  if (view === null) {
    return () => undefined;
  }
  const observer = new view.ResizeObserver(onResize);
  observer.observe(element);
  return () => observer.disconnect();
}

export function createPlayer(host: HTMLElement, source: DeckSource, options: PlayerOptions = {}): Player {
  const document = host.ownerDocument;
  const view = document.defaultView;
  const { deck } = source;
  const size = pixelSize(deck);
  const model = machineModel(deck, options);
  const numbers = numbering(deck.slides);
  const shownTotal = numbers.filter((n) => n !== null).length;
  const emitter = new Emitter();
  const cleanups: (() => void)[] = [];
  const mounted = new Map<number, MountedSlide>();
  const reported = new Set<string>();

  ensureChrome(host);
  const wrapper = el(document, 'div', {
    class: 'sd-player',
    role: 'region',
    'aria-roledescription': 'slide deck',
    'aria-label': deck.title,
  });
  // The deck page: the stage is the first screen, and the page's text view (if any) follows it.
  const page = host === document.body;
  if (page) {
    wrapper.classList.add('sd-page');
  }
  const textView = page && document.getElementById(TEXT_VIEW_ID) !== null;
  const stage = el(document, 'div', { class: 'sd-frame' });
  stage.style.width = `${size.width}px`;
  stage.style.height = `${size.height}px`;
  const blankLayer = el(document, 'div', { class: 'sd-blank' });
  const live = el(document, 'div', { class: 'sd-sr', 'aria-live': 'polite' });
  wrapper.append(stage, blankLayer, live);
  const chrome = options.chrome ?? true;
  const counter = el(document, 'div', { class: 'sd-counter', 'aria-hidden': 'true' });
  const support = options.presenter;
  const help = helpOverlay(document, support?.help ?? null, textView);
  const toText = el(document, 'a', { class: 'sd-to-text', href: `#${TEXT_VIEW_ID}` }, 'Text');
  if (chrome) {
    wrapper.append(counter);
    if (textView) {
      wrapper.append(toText);
    }
    wrapper.append(help);
  }
  if (page) {
    host.prepend(wrapper);
  } else {
    host.append(wrapper);
  }
  cleanups.push(observeSize(wrapper, () => fit(wrapper, stage, size, 'contain')));
  fit(wrapper, stage, size, 'contain');

  let position = initialPosition(model, options.mode === 'present');
  if (options.startAt !== undefined) {
    position = reduce(model, position, {
      type: 'goTo',
      slide: options.startAt.slide,
      step: options.startAt.step ?? 0,
    }).position;
  }
  let shown: number | null = null;
  let token = 0;
  let busy: Run | null = null;
  let digits = '';
  let fullscreenStops = false;
  /**
   * The full screen request `present` waits on: whether a show still wants it (a stop withdraws it, a later `present`
   * takes it over instead of asking again), and whether a presenter window opened meanwhile.
   */
  let fullscreenRequest: FullscreenRequest | null = null;
  let destroyed = false;
  let sync: Sync | null = null;
  const role: SyncRole = options.sync?.role ?? 'deck';
  let presenterWindow: Window | null = null;
  let applyingRemote = false;
  let rejoin: { session: string } | null = null;
  /** The shown slide's auto-advance: its rule, and whether its timer has run out. */
  let advancing: {
    slide: number;
    rule: Advance;
    due: boolean;
    timer: ReturnType<typeof setTimeout> | undefined;
  } | null = null;

  const diagnostic = (code: string, message: string, slide: string | null): void => {
    emitter.emit('diagnostic', { code, message, slide });
  };
  const issues = new Set<string>();
  const reportIssue = (slide: DeckSlide, step: ScheduledStep, issue: Issue): void => {
    const key = `${slide.id}\u0000${step.index}\u0000${JSON.stringify(issue)}`;
    if (issues.has(key)) {
      return;
    }
    issues.add(key);
    if (issue.kind === 'timing') {
      diagnostic(RT.timingDefault, `slide ${slide.id} step ${step.index}: ${issue.field} is out of range`, slide.id);
    } else if (issue.kind === 'track') {
      diagnostic(RT.effectFallback, `slide ${slide.id} step ${step.index}: ${issue.property} track left out`, slide.id);
    } else {
      const name = issue.kind === 'effect' ? (issue.name ?? '?') : issue.name;
      const why = issue.kind === 'effect' ? `${issue.unread} effect` : 'effect that cannot play here';
      diagnostic(
        RT.effectFallback,
        `slide ${slide.id} step ${step.index}: ${why} ${name}, played ${step.step.fallback}`,
        slide.id,
      );
    }
  };
  /** A slide's match keys; malformed ones read as absent (the markup's `data-sd-match`), reported once. */
  const matchKeys = (slide: DeckSlide | undefined) => {
    if (slide === undefined) {
      return undefined;
    }
    const read = readMatchKeys(slide.matchKeys);
    if (read.malformed && !issues.has(`${slide.id}\u0000matchKeys`)) {
      issues.add(`${slide.id}\u0000matchKeys`);
      diagnostic(RT.timingDefault, `slide ${slide.id}: matchKeys is malformed; matched by the markup`, slide.id);
    }
    return read.keys;
  };
  /** The steps slide `index` plays. */
  const stepsAt = (index: number) => {
    const slide = deck.slides[index];
    return slide === undefined ? [] : slideTimeline(slide).steps;
  };
  const properties = ensureProperties(view as (Window & typeof globalThis) | null);
  const cells = cellBudget(view?.navigator?.hardwareConcurrency);
  const slideEnv = (slide: DeckSlide, reduced: boolean): SlideEnv => ({
    size,
    reduced,
    media: mediaPolicy(slide),
    properties,
    cells,
    onIssue: (step, issue) => reportIssue(slide, step, issue),
  });
  const reducedMotion = (): boolean => {
    if (options.reducedMotion === 'always') {
      return true;
    }
    if (options.reducedMotion === 'never' || view === null || typeof view.matchMedia !== 'function') {
      return false;
    }
    return view.matchMedia('(prefers-reduced-motion: reduce)').matches;
  };
  const mediaPolicy = (slide: DeckSlide): MediaPolicy => ({
    muted: options.muted ?? false,
    onError: (media) =>
      emitter.emit('mediaerror', { slide: slide.id, src: media.currentSrc || media.getAttribute('src') || '' }),
  });

  const mount = (index: number): MountedSlide | null => {
    const existing = mounted.get(index);
    if (existing !== undefined) {
      return existing;
    }
    const slide = deck.slides[index];
    if (slide === undefined) {
      return null;
    }
    const entry = mountSlide(document, source, slide, index, size);
    if (!reported.has(slide.id)) {
      reported.add(slide.id);
      if (entry.root.querySelector('[data-slide]') === null) {
        diagnostic(RT.missingSlide, `slide ${slide.id} has no markup`, slide.id);
      }
      const removed = [...entry.report.elements, ...entry.report.attributes];
      if (removed.length > 0) {
        diagnostic(RT.sanitized, `removed from slide ${slide.id}: ${removed.join(', ')}`, slide.id);
      }
      for (const { code, message } of timelineMessages(slideTimeline(slide).issues)) {
        diagnostic(code, `slide ${slide.id} ${message}`, slide.id);
      }
    }
    stage.append(entry.layer);
    mounted.set(index, entry);
    return entry;
  };
  const unmount = (index: number): void => {
    const entry = mounted.get(index);
    if (entry !== undefined) {
      hideMedia(entry.root);
      cancelAnimations(entry);
      restoreSplits(entry);
      entry.layer.remove();
      mounted.delete(index);
    }
  };
  const faces = (slide: DeckSlide) =>
    (Array.isArray(slide.fonts) ? slide.fonts : []).flatMap((index) => {
      const face = deck.fonts[index];
      return face === undefined ? [] : [face];
    });
  const gate = (entry: MountedSlide, signal?: AbortSignal) => {
    const slide = deck.slides[entry.index];
    return ready(entry.root, slide === undefined ? [] : faces(slide), {
      fonts: document.fonts as FontFaceSet | undefined,
      ...(options.readyTimeoutMs === undefined ? {} : { timeoutMs: options.readyTimeoutMs }),
      ...(signal === undefined ? {} : { signal }),
    });
  };

  /** Keeps the shown slide and its neighbours mounted, the incoming one ready for its transition. */
  const keepNeighbours = (): void => {
    const keep = new Set<number>([position.slide]);
    for (const delta of [-1, 1]) {
      for (let i = position.slide + delta; i >= 0 && i < deck.slides.length; i += delta) {
        if (navigable(model, i, position.presenting)) {
          keep.add(i);
          break;
        }
      }
    }
    for (const index of Array.from(mounted.keys())) {
      if (!keep.has(index)) {
        unmount(index);
      }
    }
    for (const index of keep) {
      if (index !== position.slide) {
        const entry = mount(index);
        if (entry !== null) {
          applyState(entry, stepsAt(index), 0);
          void gate(entry);
          const current = mounted.get(position.slide);
          if (index > position.slide && position.presenting && current !== undefined) {
            prepareMedia(current.root, entry.root);
          }
        }
      }
    }
  };

  const setCurrent = (entry: MountedSlide): void => {
    for (const other of mounted.values()) {
      other.layer.removeAttribute('data-sd-incoming');
      if (other !== entry) {
        if (other.layer.hasAttribute('data-sd-current')) {
          hideMedia(other.root);
        }
        other.layer.removeAttribute('data-sd-current');
      }
    }
    const entering = shown !== entry.index;
    entry.layer.setAttribute('data-sd-current', '');
    shown = entry.index;
    if (entering && position.presenting) {
      const slide = deck.slides[entry.index];
      if (slide !== undefined) {
        showMedia(entry.root, mediaPolicy(slide));
      }
    }
  };

  let counterTimer: ReturnType<typeof setTimeout> | undefined;
  const updateChrome = (): void => {
    blankLayer.setAttribute('data-sd-blank', position.blank ?? '');
    const slide = deck.slides[position.slide];
    // Numbers count shown slides only, viewing or presenting; a hidden slide has none.
    const number = numbers[position.slide] ?? null;
    const label = number === null ? 'hidden' : `${number} / ${shownTotal}`;
    counter.textContent = digits === '' ? label : `→ ${digits}`;
    counter.setAttribute('data-sd-active', '');
    toText.setAttribute('data-sd-active', '');
    clearTimeout(counterTimer);
    counterTimer = setTimeout(() => {
      counter.removeAttribute('data-sd-active');
      toText.removeAttribute('data-sd-active');
    }, 2000);
    const text =
      slide === undefined
        ? ''
        : `Slide ${number ?? '–'} of ${shownTotal}${slide.title === '' ? '' : `: ${slide.title}`}`;
    if (live.textContent !== text) {
      live.textContent = text;
    }
  };

  const newRun = (): Run => {
    const run: Run = {
      token: ++token,
      skip: false,
      cleanup: null,
      abort: new AbortController(),
      animations: new Set(),
      queued: 0,
      carry: [],
    };
    busy = run;
    return run;
  };
  const stale = (run: Run): boolean => destroyed || run.token !== token;
  /** The move is over: the player is idle, and goes on to the `next`s that came while it settled, in turn. */
  const done = (run: Run): void => {
    busy = null;
    if (!run.queued) {
      idle();
    }
    for (; run.queued > 0; run.queued--) {
      dispatch({ type: 'next' });
    }
  };
  /** Fast-forwards the running move to its end. */
  const finishRun = (run: Run): void => {
    run.skip = true;
    run.abort.abort();
    finishAll(run.animations);
    const entry = mounted.get(position.slide);
    if (entry !== undefined) {
      finishAll(entry.animations);
    }
  };
  /** Drops the running move, leaving the shown slide in the state of its position; the next render goes on from it. */
  const abandonRun = (run: Run): void => {
    run.abort.abort();
    for (const animation of run.animations) {
      animation.cancel();
    }
    run.cleanup?.();
    releaseCarry(run.carry);
    run.carry = [];
    busy = null;
    token++;
    for (const entry of mounted.values()) {
      entry.layer.removeAttribute('data-sd-incoming');
      entry.layer.style.removeProperty('will-change');
    }
    const entry = shown === null ? undefined : mounted.get(shown);
    if (entry !== undefined && shown === position.slide) {
      applyState(entry, stepsAt(shown), position.step + 1);
    }
  };

  const clearAdvance = (): void => {
    if (advancing !== null) {
      clearTimeout(advancing.timer);
      advancing = null;
    }
  };
  /**
   * Starts the shown slide's auto-advance as it arrives (its transition over, or a cut): only while presenting, and
   * only in the deck window, which the presenter view follows.
   */
  const armAdvance = (index: number): void => {
    clearAdvance();
    const slide = deck.slides[index];
    if (slide === undefined || !position.presenting || role === 'presenter') {
      return;
    }
    const rule = readAdvance(slide.transition);
    if (rule.issue !== null && !issues.has(`${slide.id}\u0000advance`)) {
      issues.add(`${slide.id}\u0000advance`);
      diagnostic(RT.timingDefault, `slide ${slide.id} transition: advance ${rule.issue} is out of range`, slide.id);
    }
    if (rule.onClick && rule.afterMs === null) {
      return;
    }
    const entry: NonNullable<typeof advancing> = { slide: index, rule, due: false, timer: undefined };
    advancing = entry;
    if (rule.afterMs !== null) {
      entry.timer = setTimeout(() => {
        entry.due = true;
        idle();
      }, rule.afterMs);
    }
  };
  /** An idle player goes on by itself when the shown slide's auto-advance says so. */
  const idle = (): void => {
    const entry = advancing;
    if (
      destroyed ||
      busy !== null ||
      entry === null ||
      entry.slide !== position.slide ||
      shown !== position.slide ||
      !position.presenting ||
      position.blank !== null
    ) {
      return;
    }
    const groupsLeft = position.step < (model.slides[position.slide]?.clickGroups ?? 0);
    if (!autoNext(entry.rule, entry.due, groupsLeft)) {
      return;
    }
    const before = position;
    dispatch({ type: 'next' });
    // The end of the show: nothing more to advance to.
    if (position === before && advancing === entry) {
      clearAdvance();
    }
  };

  const render = async (move: Move): Promise<void> => {
    const target = move.position;
    const slide = deck.slides[target.slide];
    const entry = mount(target.slide);
    updateChrome();
    if (slide === undefined || entry === null) {
      return;
    }
    if (move.motion === 'none' && shown === target.slide) {
      idle();
      return;
    }
    const run = newRun();
    const timeline = slideTimeline(slide).steps;
    const all = groups(timeline);
    const reduced = reducedMotion();
    const env = slideEnv(slide, reduced);
    const playsTransition = target.presenting || (options.transitions ?? false);

    if (move.motion === 'build' && shown === target.slide) {
      if (all[target.step] !== undefined) {
        for (const animation of playGroup(entry, timeline, target.step, env)) {
          run.animations.add(animation);
        }
        await settled(run.animations);
      }
      if (stale(run)) {
        return;
      }
      applyState(entry, timeline, target.step + 1, { keepLoops: true });
      done(run);
      return;
    }

    const animated = move.motion === 'transition' && shown !== target.slide;
    const automatic = (all[0]?.steps.length ?? 0) > 0;
    const outgoing = shown === null ? null : (mounted.get(shown) ?? null);
    if (outgoing !== null && outgoing !== entry) {
      cutLoops(outgoing, stepsAt(outgoing.index));
    }
    applyState(entry, timeline, animated ? 0 : target.step + 1);
    const readiness = await gate(entry, run.abort.signal);
    if (stale(run)) {
      return;
    }
    if (readiness.timedOut) {
      diagnostic(RT.readyTimeout, `slide ${slide.id} showed before its fonts and images loaded`, slide.id);
    }
    emitter.emit('ready', { slide: target.slide, timedOut: readiness.timedOut });
    // A clip marked to play across slides goes on from the outgoing slide into the incoming one's same clip.
    const carry: Carry[] =
      animated && outgoing !== null && outgoing !== entry ? carryMedia(outgoing.root, entry.root) : [];
    run.carry = carry;
    if (carry.length > 0) {
      // The incoming clip shows the outgoing one's frame from the move's first frame, not its own first frame.
      await carriedFrames(carry, { signal: run.abort.signal });
      if (stale(run)) {
        return;
      }
      playAlong(carry);
    }
    if (animated && playsTransition && !run.skip && outgoing !== entry) {
      entry.layer.setAttribute('data-sd-incoming', '');
      const plan = planTransition(slide.transition, reduced);
      const transition = runTransition(
        {
          out: outgoing?.layer ?? null,
          into: entry.layer,
          size,
          seed: hashSeed(slide.id),
          properties,
          cells,
          outKeys: matchKeys(outgoing === null ? undefined : deck.slides[outgoing.index]),
          inKeys: matchKeys(slide),
        },
        plan,
      );
      run.cleanup = transition.cleanup;
      const why = plan.issue ?? (transition.unplayable === null ? null : `${transition.unplayable} cannot play here`);
      if (why !== null && !issues.has(`${slide.id}\u0000transition`)) {
        issues.add(`${slide.id}\u0000transition`);
        diagnostic(RT.effectFallback, `slide ${slide.id} transition: ${why}, played ${plan.fallback}`, slide.id);
      }
      run.follows = automatic;
      for (const animation of transition.animations) {
        run.animations.add(animation);
      }
      await settled(run.animations);
      if (stale(run)) {
        return;
      }
    }
    handOver(carry, env.media);
    run.carry = [];
    setCurrent(entry);
    armAdvance(target.slide);
    for (const animation of run.animations) {
      animation.cancel();
    }
    run.animations.clear();
    run.follows = false;
    run.cleanup?.();
    run.cleanup = null;
    for (const layer of [entry.layer, outgoing?.layer]) {
      layer?.style.removeProperty('will-change');
    }
    if (animated) {
      if (automatic) {
        for (const animation of playGroup(entry, timeline, 0, env)) {
          run.animations.add(animation);
        }
        // A fast-forwarded move still reaches the group's end, its media started.
        if (run.skip) {
          finishRun(run);
        }
        await settled(run.animations);
        if (stale(run)) {
          return;
        }
      }
      applyState(entry, timeline, target.step + 1, { keepLoops: true });
    }
    busy = null;
    keepNeighbours();
    done(run);
  };

  /**
   * Renders a move. `render` starts its run before its first await: a move that throws drops that run and reports
   * it, so the next move starts from the position this one reached instead of waiting on it for ever.
   */
  const play = (move: Move): void => {
    const rendering = render(move);
    const run = busy;
    rendering.catch((error: unknown) => {
      if (run !== null && busy === run) {
        try {
          abandonRun(run);
        } catch {
          // The state could not be applied either; the run is dropped all the same.
        }
      }
      const slide = deck.slides[move.position.slide];
      const why = error instanceof Error ? error.message : String(error);
      diagnostic(
        RT.moveFailed,
        `slide ${slide?.id ?? move.position.slide}: the move failed (${why})`,
        slide?.id ?? null,
      );
    });
  };

  const commit = (move: Move, previous: Position): void => {
    position = move.position;
    if (advancing !== null && (position.slide !== advancing.slide || !position.presenting)) {
      clearAdvance();
    }
    if (move.position !== previous) {
      emitter.emit('position', { position, previous, motion: move.motion });
      // The other window follows the show: a move made while not presenting (viewing after Escape) stays here.
      if (sync !== null && !applyingRemote && position.presenting) {
        sync.publish(syncState());
      }
    }
    play(move);
  };

  const dispatch = (action: Action): void => {
    if (destroyed) {
      return;
    }
    if (busy !== null) {
      if (action.type === 'next' && position.blank === null) {
        // What still plays finishes. Once all of it has played to its end or was cancelled (the engine has yet to settle
        // it), the press counts as one made just after: at a transition's end that is a press during its automatic
        // group, which it finishes; otherwise the move goes on to it when it settles, one group per press.
        if (
          busy.animations.size > 0 &&
          [...busy.animations].every((a) => a.playState === 'finished' || a.playState === 'idle') &&
          !(busy.follows && !busy.skip)
        ) {
          busy.queued++;
        }
        finishRun(busy);
        return;
      }
      abandonRun(busy);
    }
    const previous = position;
    const move = reduce(model, position, action);
    if (action.type === 'next' && move.position === previous) {
      emitter.emit('ended', {});
    }
    if (move.position === previous && shown === position.slide) {
      return;
    }
    commit(move, previous);
  };

  const syncState = (): SyncState => ({ slide: position.slide, step: position.step, blank: position.blank });

  const ensureSync = (role: SyncRole, session?: string): Sync | null => {
    if (sync !== null || view === null || support === undefined) {
      return sync;
    }
    sync = support.join(view, {
      deck: deck.id,
      role,
      ...(session === undefined ? {} : { session }),
      peer: () => presenterWindow ?? view.opener,
      onPeer: (peer) => {
        if (role === 'deck') {
          presenterWindow = peer;
        }
      },
      current: syncState,
      apply: (state) => player.follow(state, role === 'deck'),
    });
    return sync;
  };

  // Filling the window covers the deck page, whose text view then must not scroll beneath the show. Another host's
  // page is left as it is.
  let rootOverflow: string | null = null;
  const fill = (on: boolean): void => {
    const root = document.documentElement;
    if (on) {
      wrapper.classList.add('sd-fill');
      if (page && rootOverflow === null) {
        rootOverflow = root.style.overflow;
        root.style.overflow = 'hidden';
      }
    } else {
      wrapper.classList.remove('sd-fill');
      if (rootOverflow !== null) {
        root.style.overflow = rootOverflow;
        rootOverflow = null;
      }
    }
  };
  cleanups.push(() => fill(false));

  /**
   * Ends a full screen request. The show that still wants it plays full screen, or fills the window when the browser
   * refused (no user gesture, or an iframe without permission) or a presenter window opened meanwhile (it takes the
   * browser out of full screen again). A request no show wants (stopped, destroyed) leaves the full screen it entered.
   */
  const settleFullscreen = (request: FullscreenRequest, entered: boolean): void => {
    if (fullscreenRequest === request) {
      fullscreenRequest = null;
    }
    if (!request.wanted || destroyed || !position.presenting) {
      if (entered && document.fullscreenElement === wrapper) {
        void document.exitFullscreen?.().catch(() => undefined);
      }
      return;
    }
    if (entered && !request.presenterOpened) {
      fullscreenStops = true;
    } else {
      fill(true);
    }
  };

  const toggleFullscreen = (): void => {
    if (document.fullscreenElement !== null && document.fullscreenElement !== undefined) {
      void document.exitFullscreen?.().catch(() => undefined);
    } else {
      fullscreenStops = false;
      void wrapper.requestFullscreen?.().catch(() => undefined);
    }
  };

  const keyboard = options.keyboard ?? 'host';
  // The help overlay is modal: it takes focus while open (keys other than its own then do nothing) and gives it
  // back when it closes.
  let focusBeforeHelp: HTMLElement | null = null;
  const toggleHelp = (open?: boolean): void => {
    const opening = open ?? help.hidden;
    if (opening === !help.hidden) {
      return;
    }
    help.hidden = !opening;
    if (opening) {
      const active = document.activeElement as HTMLElement | null;
      focusBeforeHelp = active !== null && active !== document.body ? active : null;
      help.focus({ preventScroll: true });
    } else {
      const restore = focusBeforeHelp ?? (keyboard === 'host' ? wrapper : null);
      focusBeforeHelp = null;
      if (help.contains(document.activeElement)) {
        if (restore?.isConnected === true) {
          restore.focus({ preventScroll: true });
        } else {
          (document.activeElement as HTMLElement | null)?.blur();
        }
      }
    }
  };

  const onKey = (event: KeyboardEvent): void => {
    const path = event.composedPath?.() ?? [];
    const origin = path[0] ?? event.target;
    // Enter and Space on the player's own links (the Text link, a slide's link) follow them; a typed slide number
    // still takes Enter, and a host's buttons outside the player leave the keys to the deck.
    const ownLink = activates(event.key) && digits === '' && path.includes(wrapper) && isActivatable(origin);
    if (isEditable(origin) || ownLink) {
      return;
    }
    const result = keyCommand(event, {
      presenting: position.presenting,
      digits,
      helpOpen: chrome && !help.hidden,
      presenter: support !== undefined && role === 'deck',
    });
    const digitsChanged = result.digits !== digits;
    digits = result.digits;
    if (result.handled) {
      event.preventDefault();
    }
    if (digitsChanged) {
      updateChrome();
    }
    const command = result.command;
    if (command === null) {
      return;
    }
    switch (command.type) {
      case 'next':
      case 'previous':
      case 'first':
      case 'last':
        dispatch({ type: command.type });
        break;
      case 'goToNumber': {
        const index = slideForNumber(deck.slides, command.n);
        if (index !== null) {
          dispatch({ type: 'goTo', slide: index });
        } else {
          updateChrome();
        }
        break;
      }
      case 'blank':
        dispatch({ type: 'toggleBlank', kind: command.kind });
        break;
      case 'fullscreen':
        toggleFullscreen();
        break;
      case 'help':
        if (chrome) {
          toggleHelp();
        }
        break;
      case 'presenter':
        player.openPresenter();
        break;
      case 'present':
        void player.present();
        break;
      case 'stop':
        player.stop();
        break;
    }
  };
  // With `document`, the page's keys are the player's while it fills the screen, or while at least half of it is in
  // view and the key is not aimed at the page's text view: otherwise Space and the arrows scroll the page. In view is
  // inside the window and above the text view, which scrolls up over the deck page's stage. The share is read at the
  // key, so a key right after a scroll sees where the page is.
  const halfInView = (): boolean => {
    const box = wrapper.getBoundingClientRect();
    const height = view?.innerHeight ?? 0;
    const cover = textView ? (document.getElementById(TEXT_VIEW_ID)?.getBoundingClientRect().top ?? height) : height;
    const shown = Math.min(box.bottom, height, cover) - Math.max(box.top, 0);
    // A stage with no layout (no box) cannot be scrolled away.
    return box.height === 0 || shown >= box.height / 2;
  };
  const filling = (): boolean => wrapper.classList.contains('sd-fill') || document.fullscreenElement === wrapper;
  const pageKeys = (event: KeyboardEvent): boolean => {
    if (filling()) {
      return true;
    }
    const origin = event.composedPath?.()[0] ?? event.target;
    const inText =
      typeof (origin as Partial<Element> | null)?.closest === 'function' &&
      (origin as Element).closest(`#${TEXT_VIEW_ID}`) !== null;
    return !inText && halfInView();
  };
  // `host` reads the keys aimed at the player, and the page's other keys while it presents (it fills the screen); a
  // key the player handled is default-prevented, so the page listener leaves it.
  const onPageKey = (event: KeyboardEvent): void => {
    if (keyboard === 'document' ? pageKeys(event) : position.presenting) {
      onKey(event);
    }
  };
  if (keyboard !== 'off') {
    if (keyboard === 'host') {
      wrapper.tabIndex = 0;
      wrapper.addEventListener('keydown', onKey);
      cleanups.push(() => wrapper.removeEventListener('keydown', onKey));
    }
    document.addEventListener('keydown', onPageKey);
    cleanups.push(() => document.removeEventListener('keydown', onPageKey));
  }

  const pointer = options.pointer ?? 'present';
  let swiped = false;
  /** Follows a slide element's `data-link` (`url:`, `slide:`, `action:`); false for one it cannot follow. */
  const followDataLink = (value: string): boolean => {
    if (value.startsWith('url:')) {
      const url = value.slice('url:'.length);
      if (!isSafeLink(url)) {
        return false;
      }
      view?.open(url, '_blank', 'noopener,noreferrer');
      return true;
    }
    if (value.startsWith('slide:')) {
      const id = value.slice('slide:'.length);
      const index = deck.slides.findIndex((s) => s.id === id);
      if (index < 0 || !navigable(model, index, position.presenting)) {
        return false;
      }
      dispatch({ type: 'goTo', slide: index });
      return true;
    }
    const action = LINK_ACTIONS.get(value);
    if (action === undefined) {
      return false;
    }
    dispatch({ type: action });
    return true;
  };

  const onClick = (event: MouseEvent): void => {
    const path = event.composedPath?.() ?? [];
    const nearest = path.find((node): node is Element => {
      const element = node as Partial<Element>;
      return (
        typeof element.hasAttribute === 'function' &&
        ((element.localName === 'a' && (node as Element).hasAttribute('href')) ||
          (node as Element).hasAttribute('data-link'))
      );
    });
    if (nearest === toText) {
      return;
    }
    const link = nearest?.localName === 'a' && nearest.hasAttribute('href') ? nearest : undefined;
    const dataLink = link === undefined ? nearest?.getAttribute('data-link') : null;
    if (dataLink !== null && dataLink !== undefined && followDataLink(dataLink)) {
      event.preventDefault();
      return;
    }
    if (link !== undefined) {
      event.preventDefault();
      const target = linkTarget(link.getAttribute('href') ?? '');
      if (target?.type === 'slide') {
        const index = deck.slides.findIndex((s) => s.id === target.id);
        if (index >= 0) {
          dispatch({ type: 'goTo', slide: index });
        }
      } else if (target?.type === 'number') {
        const index = slideForNumber(deck.slides, target.n);
        if (index !== null) {
          dispatch({ type: 'goTo', slide: index });
        }
      } else if (target?.type === 'external') {
        view?.open(target.url, '_blank', 'noopener,noreferrer');
      }
      return;
    }
    if (chrome && !help.hidden) {
      toggleHelp(false);
      return;
    }
    if (swiped) {
      swiped = false;
      return;
    }
    if (pointer === 'off' || (pointer === 'present' && !position.presenting) || event.button !== 0) {
      return;
    }
    const box = wrapper.getBoundingClientRect();
    const zone = clickZone(event.clientX - box.left, box.width);
    // A slide that does not advance on a click ignores one forward; keys and clickers still advance it.
    if (
      zone === 'next' &&
      position.presenting &&
      !readAdvance(deck.slides[position.slide]?.transition ?? null).onClick
    ) {
      return;
    }
    dispatch({ type: zone });
  };
  wrapper.addEventListener('click', onClick);
  let touch: { x: number; y: number } | null = null;
  const onTouchStart = (event: TouchEvent): void => {
    const t = event.touches[0];
    touch = event.touches.length === 1 && t !== undefined ? { x: t.clientX, y: t.clientY } : null;
  };
  const onTouchEnd = (event: TouchEvent): void => {
    const t = event.changedTouches[0];
    if (touch === null || t === undefined || pointer === 'off' || (pointer === 'present' && !position.presenting)) {
      return;
    }
    const dx = t.clientX - touch.x;
    const dy = t.clientY - touch.y;
    touch = null;
    if (Math.abs(dx) > 40 && Math.abs(dx) > Math.abs(dy)) {
      swiped = true;
      setTimeout(() => (swiped = false), 500);
      dispatch({ type: dx < 0 ? 'next' : 'previous' });
    }
  };
  wrapper.addEventListener('touchstart', onTouchStart, { passive: true });
  wrapper.addEventListener('touchend', onTouchEnd);
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  const onMove = (): void => {
    wrapper.classList.remove('sd-idle');
    clearTimeout(idleTimer);
    if (position.presenting) {
      idleTimer = setTimeout(() => wrapper.classList.add('sd-idle'), 2000);
    }
  };
  wrapper.addEventListener('pointermove', onMove);
  const onFullscreenChange = (): void => {
    if (!document.fullscreenElement && fullscreenStops && position.presenting) {
      fullscreenStops = false;
      player.stop();
    }
  };
  document.addEventListener('fullscreenchange', onFullscreenChange);
  cleanups.push(() => {
    wrapper.removeEventListener('click', onClick);
    wrapper.removeEventListener('touchstart', onTouchStart);
    wrapper.removeEventListener('touchend', onTouchEnd);
    wrapper.removeEventListener('pointermove', onMove);
    document.removeEventListener('fullscreenchange', onFullscreenChange);
    clearTimeout(idleTimer);
    clearTimeout(counterTimer);
  });

  const indexOf = (slide: number | string): number =>
    typeof slide === 'number' ? slide : deck.slides.findIndex((s) => s.id === slide);

  const player: Player = {
    deck,
    get position() {
      return position;
    },
    next: () => dispatch({ type: 'next' }),
    previous: () => dispatch({ type: 'previous' }),
    first: () => dispatch({ type: 'first' }),
    last: () => dispatch({ type: 'last' }),
    goTo: (slide, step) => {
      const index = indexOf(slide);
      if (index >= 0) {
        dispatch({ type: 'goTo', slide: index, ...(step === undefined ? {} : { step }) });
      }
    },
    present: async () => {
      if (!position.presenting) {
        dispatch({ type: 'present' });
        if (!position.presenting) {
          diagnostic(RT.everyHidden, 'every slide is hidden', null);
          return;
        }
      }
      if (keyboard === 'host') {
        wrapper.focus({ preventScroll: true });
      }
      if (typeof wrapper.requestFullscreen === 'function') {
        if (fullscreenRequest === null) {
          const request: FullscreenRequest = { wanted: true, presenterOpened: false, settled: Promise.resolve() };
          fullscreenRequest = request;
          request.settled = new Promise<void>((resolve) => resolve(wrapper.requestFullscreen())).then(
            () => settleFullscreen(request, true),
            () => settleFullscreen(request, false),
          );
        }
        fullscreenRequest.wanted = true;
        await fullscreenRequest.settled;
        return;
      }
      fill(true);
    },
    stop: () => {
      // The presenter view's show is the deck window's: it stops when that window does, never on its own.
      if (role === 'presenter') {
        return;
      }
      fill(false);
      wrapper.classList.remove('sd-idle');
      if (fullscreenRequest !== null) {
        fullscreenRequest.wanted = false;
      }
      if (fullscreenStops && document.fullscreenElement) {
        fullscreenStops = false;
        void document.exitFullscreen?.().catch(() => undefined);
      }
      dispatch({ type: 'stop' });
    },
    blank: (kind) => dispatch({ type: 'blank', kind }),
    renderStatic: (slide, step, target, mode = 'contain') => renderStatic(source, indexOf(slide), step, target, mode),
    openPresenter: () => {
      if (view === null || support === undefined || role !== 'deck') {
        return null;
      }
      if (!position.presenting) {
        dispatch({ type: 'present' });
      }
      const session = ensureSync('deck')?.session ?? '';
      const url =
        options.presenterUrl?.(session) ??
        `${view.location.href.split('#')[0] ?? ''}#presenter=${encodeURIComponent(session)}`;
      // Browsers leave full screen when a window opens (WebKit while `open` runs): the show then fills the window
      // instead of ending.
      const stops = fullscreenStops;
      fullscreenStops = false;
      const opened = support.open(view, url, deck.id, wrapper);
      presenterWindow = opened;
      emitter.emit('presenter', { window: opened });
      if (opened === null) {
        fullscreenStops = stops;
        diagnostic(RT.presenterBlocked, 'the browser blocked the presenter window', null);
      } else {
        if (stops) {
          fill(true);
        }
        if (fullscreenRequest !== null) {
          fullscreenRequest.presenterOpened = true;
        }
      }
      return opened;
    },
    follow: (state, animate) => {
      if (destroyed) {
        return;
      }
      const next = reduce(model, position, { type: 'next' });
      let move: Move;
      if (
        animate &&
        state.blank === null &&
        next.motion !== 'none' &&
        next.position.slide === state.slide &&
        next.position.step === state.step &&
        next.position.blank === null
      ) {
        move = next;
      } else {
        const moved = reduce(model, position, { type: 'goTo', slide: state.slide, step: state.step }).position;
        const target = reduce(model, moved, { type: 'blank', kind: state.blank }).position;
        if (target.slide === position.slide && target.step === position.step && target.blank === position.blank) {
          return;
        }
        const cut = target.slide !== position.slide || target.step !== position.step;
        move = { position: target, motion: cut ? 'cut' : 'none' };
      }
      if (busy !== null) {
        abandonRun(busy);
      }
      applyingRemote = true;
      try {
        commit(move, position);
      } finally {
        applyingRemote = false;
      }
    },
    on: (type, listener) => emitter.on(type, listener),
    destroy: () => {
      if (destroyed) {
        return;
      }
      destroyed = true;
      clearAdvance();
      if (busy !== null) {
        abandonRun(busy);
      }
      sync?.leave();
      for (const index of Array.from(mounted.keys())) {
        unmount(index);
      }
      for (const cleanup of cleanups) {
        cleanup();
      }
      wrapper.remove();
      emitter.clear();
    },
  };

  if (options.sync !== undefined) {
    ensureSync(options.sync.role, options.sync.session);
  }
  if (view !== null) {
    // A page leaving says bye, so the other window can follow its reload; one restored from the back-forward cache
    // rejoins with its session.
    const onPageHide = (): void => {
      const left = sync;
      if (left !== null) {
        rejoin = { session: left.session };
        left.leave();
        sync = null;
      }
    };
    const onPageShow = (event: PageTransitionEvent): void => {
      if (event.persisted && rejoin !== null) {
        ensureSync(role, rejoin.session);
        rejoin = null;
      }
    };
    view.addEventListener('pagehide', onPageHide);
    view.addEventListener('pageshow', onPageShow);
    cleanups.push(() => {
      view.removeEventListener('pagehide', onPageHide);
      view.removeEventListener('pageshow', onPageShow);
    });
  }
  updateChrome();
  // The first render waits a microtask, so listeners added right after creation hear its events.
  queueMicrotask(() => {
    if (!destroyed && shown === null && busy === null) {
      play({ position, motion: 'cut' });
    }
  });
  return player;
}

/** A still render of slide `index` at `step` in `target`, scaled to it; returns its disposer. */
export function renderStatic(
  source: DeckSource,
  index: number,
  step: number,
  target: HTMLElement,
  mode: 'contain' | 'width' = 'contain',
): () => void {
  const document = target.ownerDocument;
  const { deck } = source;
  const slide = deck.slides[index];
  if (slide === undefined) {
    return () => undefined;
  }
  ensureChrome(target);
  const size = pixelSize(deck);
  const box = el(document, 'div', { class: 'sd-static', 'aria-label': slide.title });
  if (mode === 'contain') {
    box.style.width = '100%';
    box.style.height = '100%';
  }
  const frame = el(document, 'div', { class: 'sd-frame' });
  frame.style.width = `${size.width}px`;
  frame.style.height = `${size.height}px`;
  const entry = mountSlide(document, source, slide, index, size);
  const timeline = slideTimeline(slide).steps;
  const groups = stepCount(slide);
  entry.layer.setAttribute('data-sd-current', '');
  frame.append(entry.layer);
  box.append(frame);
  target.append(box);
  // Applied once connected: held transforms read the elements' own rotation and flips.
  applyState(entry, timeline, Math.min(Math.max(0, Math.trunc(step)), groups) + 1);
  const stop = observeSize(box, () => fit(box, frame, size, mode));
  fit(box, frame, size, mode);
  return () => {
    stop();
    hideMedia(entry.root);
    box.remove();
  };
}
