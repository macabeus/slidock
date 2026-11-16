// The presenter view: the same deck in a second window, its own player in sync with the deck window.
// It shows the current slide, a still preview of what the next click shows, the speaker notes, the elapsed and
// per-slide timers, "slide n of N", go to slide, and blank. It fills its host.
import type { DeckSource } from '../deck';
import { el } from '../dom';
import type { MachineModel, Position } from '../machine';
import type { PresenterOptions, PresenterSupport, PresenterView } from '../player';
import { clickGroups, createPlayer, numbering, reduce, slideForNumber, stepCount } from './player';
import { ensurePresenterStyle } from './style';

/** `m:ss`, or `h:mm:ss` from an hour. */
export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}

/** What the next click shows: the slide's next build step, the next slide, or nothing (the end of the show). */
export type NextClick =
  | { readonly kind: 'build' | 'slide'; readonly slide: number; readonly step: number; readonly label: string }
  | { readonly kind: 'end'; readonly label: string };

/** The next click from `position`, as the presenter view previews and labels it. */
export function nextClick(model: MachineModel, numbers: readonly (number | null)[], position: Position): NextClick {
  const upcoming = reduce(model, { ...position, blank: null }, { type: 'next' });
  if (upcoming.motion === 'none') {
    return { kind: 'end', label: 'Next' };
  }
  const { slide, step } = upcoming.position;
  if (upcoming.motion === 'build') {
    return { kind: 'build', slide, step, label: `Next: build ${step} of ${clickGroups(model, slide)}` };
  }
  const number = numbers[slide] ?? null;
  return { kind: 'slide', slide, step, label: number === null ? 'Next: slide' : `Next: slide ${number}` };
}

/** The presenter view of `source` in `host`, its player joined to the deck window through `support`. */
export function mountView(
  host: HTMLElement,
  source: DeckSource,
  options: PresenterOptions,
  support: PresenterSupport,
): PresenterView {
  const document = host.ownerDocument;
  const { deck } = source;
  ensurePresenterStyle(document);
  document.title = `Presenter · ${deck.title}`;
  const root = el(document, 'div', { class: 'sd-presenter', role: 'application', 'aria-label': 'Presenter view' });
  const header = el(document, 'header');
  const counter = el(document, 'span', { class: 'sd-time', 'aria-live': 'polite' });
  const elapsed = el(document, 'span', { class: 'sd-time', title: 'Elapsed' });
  const onSlide = el(document, 'span', { class: 'sd-time', title: 'On this slide' });
  const button = (label: string, title: string) => el(document, 'button', { type: 'button', title }, label);
  const previousButton = button('←', 'Previous');
  const nextButton = button('→', 'Next');
  const blackButton = button('Black', 'Black screen (B)');
  const whiteButton = button('White', 'White screen (W)');
  const resetButton = button('Reset', 'Reset the timers');
  const goToForm = el(document, 'form');
  const goToInput = el(document, 'input', {
    type: 'number',
    min: '1',
    inputmode: 'numeric',
    'aria-label': 'Go to slide',
    placeholder: '#',
  });
  goToForm.append(goToInput, el(document, 'button', { type: 'submit' }, 'Go'));
  const label = (text: string) => el(document, 'span', { class: 'sd-label' }, text);
  header.append(
    counter,
    label('Elapsed'),
    elapsed,
    label('Slide'),
    onSlide,
    resetButton,
    previousButton,
    nextButton,
    blackButton,
    whiteButton,
    goToForm,
  );
  const current = el(document, 'div', { class: 'sd-current' });
  const next = el(document, 'section', { class: 'sd-next', 'aria-label': 'Next' });
  const preview = el(document, 'div');
  const nextLabel = label('Next');
  next.append(nextLabel, preview);
  const notes = el(document, 'section', { class: 'sd-notes', 'aria-label': 'Speaker notes' });
  root.append(header, current, next, notes);
  host.append(root);

  const player = createPlayer(current, source, {
    mode: 'present',
    keyboard: 'document',
    pointer: 'present',
    chrome: false,
    muted: true,
    sync: { role: 'presenter', session: options.session },
    presenter: support,
    ...(options.reducedMotion === undefined ? {} : { reducedMotion: options.reducedMotion }),
    ...(options.readyTimeoutMs === undefined ? {} : { readyTimeoutMs: options.readyTimeoutMs }),
    ...(options.loop === undefined ? {} : { loop: options.loop }),
    ...(options.startAt === undefined ? {} : { startAt: options.startAt }),
  });
  const model: MachineModel = {
    slides: deck.slides.map((slide) => ({ hidden: slide.hidden, clickGroups: stepCount(slide) })),
    builds: false,
    loop: options.loop ?? false,
  };
  const numbers = numbering(deck.slides);
  const shownTotal = numbers.filter((n) => n !== null).length;

  let startedAt = Date.now();
  let slideStartedAt = startedAt;
  let disposePreview: () => void = () => undefined;
  let shownSlide = -1;
  const tick = (): void => {
    const now = Date.now();
    elapsed.textContent = formatDuration(now - startedAt);
    onSlide.textContent = formatDuration(now - slideStartedAt);
  };
  const update = (position: Position): void => {
    const slide = deck.slides[position.slide];
    const number = numbers[position.slide] ?? null;
    counter.textContent = `Slide ${number ?? '(hidden)'} of ${shownTotal}`;
    blackButton.setAttribute('aria-pressed', String(position.blank === 'black'));
    whiteButton.setAttribute('aria-pressed', String(position.blank === 'white'));
    if (position.slide !== shownSlide) {
      shownSlide = position.slide;
      slideStartedAt = Date.now();
      notes.replaceChildren(...(slide?.notes ?? []).map((paragraph) => el(document, 'p', {}, paragraph)));
      if (slide !== undefined && slide.notes.length === 0) {
        notes.append(el(document, 'p', { class: 'sd-label' }, 'No notes'));
      }
    }
    disposePreview();
    preview.replaceChildren();
    const upcoming = nextClick(model, numbers, position);
    next.setAttribute('data-sd-next', upcoming.kind);
    nextLabel.textContent = upcoming.label;
    if (upcoming.kind === 'end') {
      preview.append(el(document, 'p', { class: 'sd-label' }, 'End of presentation'));
      disposePreview = () => undefined;
    } else {
      disposePreview = player.renderStatic(upcoming.slide, upcoming.step, preview);
    }
    tick();
  };
  const offPosition = player.on('position', ({ position }) => update(position));
  update(player.position);
  const timer = setInterval(tick, 1000);

  previousButton.addEventListener('click', () => player.previous());
  nextButton.addEventListener('click', () => player.next());
  blackButton.addEventListener('click', () => player.blank(player.position.blank === 'black' ? null : 'black'));
  whiteButton.addEventListener('click', () => player.blank(player.position.blank === 'white' ? null : 'white'));
  resetButton.addEventListener('click', () => {
    startedAt = Date.now();
    slideStartedAt = startedAt;
    tick();
  });
  goToForm.addEventListener('submit', (event) => {
    event.preventDefault();
    const index = slideForNumber(deck.slides, Number(goToInput.value));
    if (index !== null) {
      player.goTo(index);
      goToInput.value = '';
    }
  });

  return {
    player,
    destroy: () => {
      clearInterval(timer);
      offPosition();
      disposePreview();
      player.destroy();
      root.remove();
    },
  };
}
