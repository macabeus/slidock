import {
  AppHeader,
  Button,
  Dialog,
  DropOverlay,
  Icon,
  ThemeProvider,
  ThemeToggle,
  readHash,
  useWindowDrop,
  writeHash,
} from '@slidock/ui';
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react';

import { type DroppedItem, captureDrop } from './compile/input';
import { DeckSession } from './compile/session';
import { droppedFonts } from './fonts/dropped';
import { PresenterApp } from './presenter/PresenterApp';
import { PRESENTER_HASH, PresenterLink } from './presenter/bridge';
import type { AppServices } from './services';
import { Converting } from './views/Converting';
import { Landing } from './views/Landing';
import { Logo } from './views/Logo';
import { Viewer } from './views/Viewer';

function droppedName(items: readonly DroppedItem[]): string {
  const first = items[0];
  return first ? (first.kind === 'file' ? first.file.name : first.entry.name) : 'this drop';
}

const NO_POSITION = { slide: null, step: null, present: null };

/** slidock's licence and notices and those of what the app is built from (`cargo xtask notices`). */
const NOTICES_URL = `${import.meta.env.BASE_URL}THIRD-PARTY-NOTICES.txt`;

const TRADEMARKS =
  'Keynote is a trademark of Apple Inc. and PowerPoint of Microsoft Corporation; slidock is not affiliated with either.';

function DeckApp({ services }: { services: AppServices }) {
  const [session] = useState(
    () =>
      new DeckSession(
        services.startWorker,
        services.fonts,
        services.photos,
        services.images,
        services.videos,
        services.alternates,
      ),
  );
  const state = useSyncExternalStore(session.subscribe, session.getSnapshot);
  const [replacement, setReplacement] = useState<DroppedItem[] | null>(null);
  const [presenter] = useState(() => new PresenterLink());
  const planAlternates = useCallback(() => session.alternatePlan(), [session]);
  const { view, compiler, bundle, fonts } = state;

  useEffect(() => {
    session.connect();
    const onPageHide = () => session.dispose();
    const onPageShow = () => session.connect();
    window.addEventListener('pagehide', onPageHide);
    window.addEventListener('pageshow', onPageShow);
    return () => {
      window.removeEventListener('pagehide', onPageHide);
      window.removeEventListener('pageshow', onPageShow);
      session.dispose();
    };
  }, [session]);

  useEffect(() => presenter.listen(), [presenter]);

  useEffect(() => {
    document.title = view.kind === 'deck' ? `${view.deck.title} · slidock` : 'slidock';
  }, [view]);

  // The fragment's position (`#slide`, `#step`) belongs to one deck. The first open of a page load starts there (a
  // reload, then the same deck opened again); every later open starts at the beginning.
  const opened = useRef(false);
  const open = useCallback((start: () => Promise<void>) => {
    if (opened.current) {
      writeHash(NO_POSITION);
    }
    opened.current = true;
    void start();
  }, []);

  const onDrop = useCallback(
    (data: DataTransfer) => {
      // Font files dropped anywhere on an open deck's page are added to it.
      const current = session.getSnapshot();
      const fontFiles = droppedFonts(data);
      if (fontFiles && current.view.kind === 'deck' && current.fonts) {
        void current.fonts.addFiles(fontFiles);
        return;
      }
      const items = captureDrop(data);
      if (current.view.kind === 'deck') {
        setReplacement(items);
      } else {
        open(() => session.openDrop(items));
      }
    },
    [session, open],
  );
  const dragging = useWindowDrop(onDrop);

  const close = () => {
    session.close();
    writeHash(NO_POSITION, 'push');
  };

  const loaded = view.kind === 'deck';
  return (
    <div
      className={
        loaded
          ? 'flex min-h-dvh flex-col gap-3 p-4 lg:h-dvh'
          : 'mx-auto flex min-h-dvh max-w-[1400px] flex-col gap-6 px-4 py-6'
      }
    >
      <AppHeader
        compact={loaded}
        logo={<Logo />}
        title="slidock"
        subtitle={loaded ? view.deck.title : 'Presentations to standalone HTML, converted in your browser'}
        right={
          <>
            {loaded && (
              <Button variant="ghost" size="sm" onClick={close}>
                <Icon name="folder" className="size-4" />
                Open another
              </Button>
            )}
            <ThemeToggle />
          </>
        }
      />
      <main className="flex min-h-0 flex-1 flex-col" aria-label={loaded ? 'Deck viewer' : 'Open a presentation'}>
        {view.kind === 'deck' ? (
          <Viewer
            loaded={view}
            playerFactory={services.player}
            presenter={presenter}
            fonts={fonts}
            bundle={bundle}
            onBundle={(settings, quality, videoQuality) => void session.bundle(settings, quality, videoQuality)}
            onResetBundle={() => session.resetBundle()}
            onCancelBundle={() => session.cancelBundle()}
            planAlternates={planAlternates}
          />
        ) : view.kind === 'compiling' || view.kind === 'preparing' ? (
          <Converting
            name={view.name}
            phase={view.kind === 'compiling' ? view.phase : null}
            done={view.kind === 'compiling' ? view.done : []}
            warnings={view.kind === 'compiling' ? view.warnings : 0}
            errors={view.kind === 'compiling' ? view.errors : 0}
            preparing={view.kind === 'preparing'}
            photos={view.kind === 'compiling' ? view.photos : null}
            onCancel={() => session.cancel()}
          />
        ) : (
          <Landing
            compiler={compiler}
            error={view.kind === 'error' ? { name: view.name, error: view.error } : null}
            onPick={(file) => open(() => session.openFile(file))}
            onRetryCompiler={() => session.retryCompiler()}
            onDismissError={close}
          />
        )}
      </main>
      {!loaded && (
        <footer className="flex flex-col items-center gap-1 px-4 text-center text-xs text-muted">
          <p>
            <span data-testid="wasm-status">
              {compiler.state === 'ready'
                ? `slidock-wasm ${compiler.version}`
                : compiler.state === 'loading'
                  ? 'Loading the compiler…'
                  : 'The compiler failed to load'}
            </span>
            {' · '}
            <a className="underline hover:text-fg" href={NOTICES_URL}>
              Licences
            </a>
          </p>
          <p>{TRADEMARKS}</p>
        </footer>
      )}
      <DropOverlay
        visible={dragging}
        label={loaded ? 'Drop a deck to open it, or font files to add them' : 'Drop to open'}
      />
      <Dialog
        open={replacement !== null}
        onClose={() => setReplacement(null)}
        title="Replace the open deck?"
        footer={
          <>
            <Button variant="ghost" onClick={() => setReplacement(null)}>
              Keep the current deck
            </Button>
            <Button
              variant="primary"
              data-autofocus
              onClick={() => {
                const items = replacement;
                setReplacement(null);
                if (items) {
                  open(() => session.openDrop(items));
                }
              }}
            >
              Open the new one
            </Button>
          </>
        }
      >
        <p className="text-sm text-fg-secondary">
          Opening {replacement ? `“${droppedName(replacement)}”` : 'it'} closes{' '}
          {loaded ? `“${view.deck.title}”` : 'the deck you are viewing'}.
        </p>
      </Dialog>
    </div>
  );
}

export function App({ services }: { services: AppServices }) {
  const [presenter] = useState(() => readHash().has(PRESENTER_HASH));
  return (
    <ThemeProvider>
      {presenter ? <PresenterApp player={services.player} /> : <DeckApp services={services} />}
    </ThemeProvider>
  );
}
