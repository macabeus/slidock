import type {
  Blank,
  DeckInput,
  Player,
  PlayerFactory,
  PlayerOptions,
  Position,
  Presenter,
  PresenterOptions,
} from '../player/types';
import { clickGroups } from '../player/types';

export interface FakePlayerRecord {
  host: HTMLElement;
  input: DeckInput;
  options: PlayerOptions;
  calls: string[];
  /** Slide ids drawn by `renderStatic` and not yet cleaned up. */
  rendered: Set<string>;
  destroyed: boolean;
  player: Player;
}

/** A player that records what the app asks of it and moves by slide, for the app's tests. */
export class FakePlayerFactory implements PlayerFactory {
  readonly players: FakePlayerRecord[] = [];
  readonly presenters: { host: HTMLElement; input: DeckInput; options: PresenterOptions; destroyed: boolean }[] = [];

  get last(): FakePlayerRecord {
    const record = this.players.at(-1);
    if (!record) {
      throw new Error('no player was created');
    }
    return record;
  }

  createPlayer = (host: HTMLElement, input: DeckInput, options: PlayerOptions): Player => {
    const { deck } = input;
    const start = options.startAt?.slide ?? 0;
    let position: Position = { slide: start, step: clickGroups(deck, start), presenting: false, blank: null };
    const rendered = new Set<string>();
    const set = (patch: Partial<Position>) => {
      position = { ...position, ...patch };
      host.dataset.slide = deck.slides[position.slide]?.id ?? '';
      options.onEvent?.({ type: 'position', position });
    };
    const goTo = (slide: number) => {
      const clamped = Math.min(deck.slides.length - 1, Math.max(0, slide));
      set({ slide: clamped, step: clickGroups(deck, clamped) });
    };
    const calls: string[] = [];
    const player: Player = {
      next() {
        calls.push('next');
        goTo(position.slide + 1);
      },
      previous() {
        calls.push('previous');
        goTo(position.slide - 1);
      },
      first() {
        calls.push('first');
        goTo(0);
      },
      last() {
        calls.push('last');
        goTo(deck.slides.length - 1);
      },
      goTo(slide: number) {
        calls.push(`goTo:${slide}`);
        goTo(slide);
      },
      async present() {
        calls.push('present');
        set({ presenting: true });
      },
      stop() {
        calls.push('stop');
        set({ presenting: false, blank: null });
      },
      blank(kind: Blank) {
        calls.push(`blank:${kind}`);
        set({ blank: kind });
      },
      get position() {
        return position;
      },
      renderStatic(slideId: string, target: HTMLElement) {
        rendered.add(slideId);
        target.dataset.rendered = slideId;
        return () => rendered.delete(slideId);
      },
      openPresenter() {
        calls.push('openPresenter');
        set({ presenting: true });
        const opened = window.open('/#presenter', `slidock-presenter-${deck.id.slice(0, 16)}`, 'popup');
        options.onEvent?.({ type: 'presenter', window: opened });
        return opened;
      },
      destroy() {
        calls.push('destroy');
        record.destroyed = true;
      },
    };
    const record: FakePlayerRecord = { host, input, options, calls, rendered, destroyed: false, player };
    host.dataset.slide = deck.slides[position.slide]?.id ?? '';
    this.players.push(record);
    return player;
  };

  createPresenter = (host: HTMLElement, input: DeckInput, options: PresenterOptions): Presenter => {
    const record = { host, input, options, destroyed: false };
    this.presenters.push(record);
    host.dataset.presenter = input.deck.id;
    return {
      destroy() {
        record.destroyed = true;
      },
    };
  };
}
