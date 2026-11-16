import type { BundleSettings, Deck, DeckDiagnostic, PlannedVideo } from '@slidock/types';
import { Button, Icon, intParam, openOverlayCount, readHash, writeHash } from '@slidock/ui';
import { type CSSProperties, useCallback, useEffect, useMemo, useRef, useState } from 'react';

import type { BundleState, LoadedDeck } from '../compile/session';
import type { FontsController } from '../fonts/controller';
import { DocumentFonts } from '../fonts/documentFonts';
import { familyRows, missingFamilies } from '../fonts/families';
import { useFontsState } from '../fonts/useFonts';
import type { ImageQuality } from '../media/compress';
import type { VideoQuality } from '../media/video';
import { type DeckInput, type Player, type PlayerFactory, type Position, clickGroups } from '../player/types';
import type { PresenterLink } from '../presenter/bridge';
import { BottomBar } from './BottomBar';
import { DownloadDialog } from './DownloadDialog';
import { FontsPanel } from './FontsPanel';
import { FontsPrompt } from './FontsPrompt';
import { KeyboardHelp } from './KeyboardHelp';
import { Rail } from './Rail';

export interface ViewerProps {
  loaded: LoadedDeck;
  playerFactory: PlayerFactory;
  /** The presenter window's link, which outlives this deck. */
  presenter: PresenterLink;
  /** The deck's fonts flow; `null` in a deck no session opened (tests). */
  fonts: FontsController | null;
  bundle: BundleState;
  onBundle(settings: BundleSettings, quality?: ImageQuality, videoQuality?: VideoQuality): void;
  onResetBundle(): void;
  onCancelBundle?(): void;
  /** The deck's videos that get a copy other browsers play, for the download dialog. */
  planAlternates?: () => Promise<PlannedVideo[]>;
}

type Overlay = 'fonts' | 'download' | 'help' | null;

/** Where the fragment says to start (`#slide=3&step=1`, 1-based slide), within the deck and the slide's steps. */
function startFromHash(deck: Deck): { slide: number; step?: number } {
  const params = readHash();
  const slide = Math.min(deck.slides.length - 1, Math.max(0, (intParam(params, 'slide') ?? 1) - 1));
  const step = intParam(params, 'step');
  return step === null ? { slide } : { slide, step: Math.min(clickGroups(deck, slide), Math.max(0, step)) };
}

function isInteractive(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLElement &&
    (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT|BUTTON|A)$/.test(target.tagName))
  );
}

export function Viewer({
  loaded,
  playerFactory,
  presenter,
  fonts,
  bundle,
  onBundle,
  onResetBundle,
  onCancelBundle,
  planAlternates,
}: ViewerProps) {
  // A re-finish (the fonts changed) swaps the deck in place at the current slide; while presenting it waits until
  // the presentation ends, so full screen is never left for it.
  const [shown, setShown] = useState(loaded);
  const { deck } = shown;
  const stageHost = useRef<HTMLDivElement>(null);
  const [player, setPlayer] = useState<Player | null>(null);
  const [position, setPosition] = useState<Position>(() => {
    const start = startFromHash(deck);
    return { slide: start.slide, step: start.step ?? clickGroups(deck, start.slide), presenting: false, blank: null };
  });
  const [runtimeDiagnostics, setRuntimeDiagnostics] = useState<readonly DeckDiagnostic[]>([]);
  const [overlay, setOverlay] = useState<Overlay>(null);
  const fontsState = useFontsState(fonts);
  // While the fonts flow may still swap the deck (and the player with it), the stage says so.
  const swapping =
    fontsState !== null && (fontsState.settling || fontsState.refreshing || fontsState.substitutes.state === 'loading');
  const positionRef = useRef(position);
  /** Set once the first player of this viewer exists: later players start where the last one was. */
  const refreshed = useRef(false);
  const [documentFonts] = useState(() => new DocumentFonts());

  if (shown !== loaded && !position.presenting) {
    setShown(loaded);
  }

  const input = useMemo<DeckInput>(
    () => ({ deck, slides: shown.slides, mediaUrl: shown.media.url }),
    [deck, shown.slides, shown.media],
  );

  useEffect(() => documentFonts.replace(shown.compiled.fonts ?? []), [documentFonts, shown.compiled]);
  useEffect(() => () => documentFonts.clear(), [documentFonts]);

  useEffect(() => {
    const host = stageHost.current;
    if (!host) {
      return;
    }
    const follow = (next: Position) => {
      positionRef.current = next;
      setPosition(next);
    };
    setRuntimeDiagnostics([]);
    const at = positionRef.current;
    const created = playerFactory.createPlayer(host, input, {
      mode: 'view',
      startAt: refreshed.current ? { slide: at.slide, step: at.step } : startFromHash(input.deck),
      onEvent(event) {
        if (event.type === 'diagnostic') {
          setRuntimeDiagnostics((list) => [...list, event.diagnostic]);
          return;
        }
        // However it was opened (the button, or the player's own P key), the presenter window gets the deck.
        if (event.type === 'presenter') {
          if (event.window) {
            presenter.attach(event.window);
          }
          return;
        }
        if (event.type !== 'position') {
          return;
        }
        const next = event.position;
        follow(next);
        writeHash({
          slide: String(next.slide + 1),
          step: next.step === clickGroups(input.deck, next.slide) ? null : String(next.step),
          present: next.presenting ? '1' : null,
        });
      },
    });
    // The counter shows where the player is, which may differ from where the fragment asked it to start.
    follow(created.position);
    refreshed.current = true;
    setPlayer(created);
    return () => {
      created.destroy();
      setPlayer(null);
    };
  }, [playerFactory, input, presenter]);

  useEffect(
    () =>
      presenter.serve(() => ({
        // Titled as a downloaded deck's page is, so both presenter windows name the deck alike.
        deck: { ...deck, title: shown.compiled.page?.title ?? deck.title },
        slides: [...shown.slides],
        media: [...shown.compiled.media],
        fonts: [...(shown.compiled.fonts ?? [])],
        position: { slide: positionRef.current.slide, step: positionRef.current.step },
      })),
    [presenter, deck, shown.slides, shown.compiled],
  );

  const present = useCallback(() => {
    void player?.present();
  }, [player]);

  const openPresenter = useCallback(() => {
    player?.openPresenter();
  }, [player]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (
        !player ||
        player.position.presenting ||
        event.defaultPrevented ||
        event.metaKey ||
        event.ctrlKey ||
        event.altKey ||
        openOverlayCount() > 0
      ) {
        return;
      }
      const interactive = isInteractive(event.target);
      const key = event.key;
      if (key === 'ArrowRight' || key === 'PageDown' || (key === ' ' && !interactive)) {
        player.next();
      } else if (key === 'ArrowLeft' || key === 'PageUp') {
        player.previous();
      } else if (key === 'Home' && !interactive) {
        player.first();
      } else if (key === 'End' && !interactive) {
        player.last();
      } else if (key === '?') {
        setOverlay('help');
      } else if ((key === 'p' || key === 'P') && !interactive) {
        present();
      } else {
        return;
      }
      event.preventDefault();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [player, present]);

  const current = deck.slides[position.slide];
  const shownSlides = deck.slides.filter((slide) => !slide.hidden).length;
  const groups = clickGroups(deck, position.slide);
  const diagnostics = useMemo(
    () => (runtimeDiagnostics.length === 0 ? shown.diagnostics : [...shown.diagnostics, ...runtimeDiagnostics]),
    [shown.diagnostics, runtimeDiagnostics],
  );
  const faces = shown.compiled.faces;
  const missingFonts = useMemo(
    () => missingFamilies(familyRows(deck, faces ?? [], shown.needs, new Set())).length,
    [deck, faces, shown.needs],
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      <div role="toolbar" aria-label="Deck" className="flex flex-wrap items-center gap-2">
        <div className="flex items-center gap-1">
          <Button variant="ghost" size="sm" aria-label="Previous slide" onClick={() => player?.previous()}>
            <Icon name="chevron-left" className="size-4" />
          </Button>
          <p className="min-w-28 text-center font-mono text-sm tabular-nums text-fg-secondary">
            {current?.hidden ? 'Hidden slide' : `Slide ${current?.number ?? position.slide + 1} of ${shownSlides}`}
            {groups > 0 && (
              <span className="text-muted">
                {' '}
                · step {position.step} / {groups}
              </span>
            )}
          </p>
          <Button variant="ghost" size="sm" aria-label="Next slide" onClick={() => player?.next()}>
            <Icon name="chevron-right" className="size-4" />
          </Button>
        </div>
        <div className="ml-auto flex flex-wrap items-center gap-2">
          <Button variant="ghost" size="sm" onClick={() => setOverlay('fonts')} aria-haspopup="dialog">
            <Icon name="fonts" className="size-4" />
            <span className="max-sm:sr-only">Fonts</span>
            {missingFonts > 0 && (
              <span
                className="rounded bg-warning/15 px-1.5 font-mono text-[11px] tabular-nums text-warning"
                aria-label={`${missingFonts} missing`}
              >
                {missingFonts}
              </span>
            )}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            aria-label="Keyboard shortcuts"
            aria-haspopup="dialog"
            onClick={() => setOverlay('help')}
          >
            <Icon name="keyboard" className="size-4" />
          </Button>
          <Button variant="ghost" size="sm" onClick={openPresenter} disabled={!player}>
            <Icon name="external-link" className="size-4" />
            <span className="max-sm:sr-only">Presenter view</span>
          </Button>
          <Button variant="wash" size="sm" onClick={() => setOverlay('download')} aria-haspopup="dialog">
            <Icon name="download" className="size-4" />
            <span className="max-sm:sr-only">Download zip</span>
          </Button>
          <Button variant="primary" size="sm" onClick={present} disabled={!player}>
            <Icon name="play" className="size-4" />
            Present
          </Button>
        </div>
      </div>

      <FontsPrompt deck={deck} faces={faces ?? []} fonts={fonts} onOpenPanel={() => setOverlay('fonts')} />

      <div className="flex flex-col-reverse gap-3 lg:grid lg:min-h-0 lg:flex-1 lg:grid-cols-[11rem_minmax(0,1fr)]">
        <Rail deck={deck} player={player} current={position.slide} onSelect={(index) => player?.goTo(index)} />
        <div
          data-stage-well
          className="relative aspect-(--deck-aspect) overflow-hidden rounded-xl border border-line bg-sunken shadow-inner lg:aspect-auto lg:min-h-0"
          style={{ '--deck-aspect': `${deck.size.w} / ${deck.size.h}` } as CSSProperties}
        >
          <div ref={stageHost} className="absolute inset-0" data-testid="stage" aria-busy={swapping} />
        </div>
      </div>

      <BottomBar
        deck={deck}
        diagnostics={diagnostics}
        current={position.slide}
        onGoTo={(slide) => player?.goTo(slide)}
      />

      <FontsPanel
        open={overlay === 'fonts'}
        onClose={() => setOverlay(null)}
        deck={deck}
        faces={faces ?? []}
        needs={shown.needs}
        fonts={fonts}
      />
      <DownloadDialog
        open={overlay === 'download'}
        onClose={() => setOverlay(null)}
        deck={deck}
        faces={faces ?? []}
        page={shown.compiled.page}
        fonts={fonts}
        bundle={bundle}
        onBundle={onBundle}
        onReset={onResetBundle}
        {...(onCancelBundle ? { onCancel: onCancelBundle } : {})}
        {...(planAlternates ? { planAlternates } : {})}
      />
      <KeyboardHelp open={overlay === 'help'} onClose={() => setOverlay(null)} />
    </div>
  );
}
