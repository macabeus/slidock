// The dev server's demo page (`/demo.html`, not part of the build): the app over a fake compile worker that
// "compiles" the sample deck and tells its font story (src/testing/sample-fonts.ts), with @slidock/player and the
// browser's own fetch and Local Font Access. `?view=landing` stays on the start screen; `?theme=light` or
// `?theme=dark` forces a theme; `?panel=fonts|download|help` opens a panel. `?worker=` scripts the worker for the
// other screens: `load-error` (the compiler fails to load), `open-error` (the deck is refused), `hold-finish` (the
// conversion stays on its second phase) and `bundle-error` (the zip fails).
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { App } from './App';
import { browserFontsEnvironment } from './fonts/environment';
import { slidockPlayer } from './player/slidock-player';
import './styles.css';
import { FakeWorker, type FakeWorkerScript } from './testing/fake-worker';
import { sampleFontsScript } from './testing/sample-fonts';

const params = new URLSearchParams(location.search);
const theme = params.get('theme');
try {
  if (theme === 'light' || theme === 'dark') {
    localStorage.setItem('slidock.theme', theme);
  } else {
    localStorage.removeItem('slidock.theme');
  }
} catch {
  // Storage unavailable: the system theme applies.
}

const WORKER_ERROR = {
  'open-error': {
    errors: {
      open: {
        kind: 'refused',
        message: 'the file is not a zip archive',
        diagnostics: [
          {
            code: 'IO0101',
            severity: 'error',
            message: 'The input is not a ZIP archive (no end of central directory).',
            feature: null,
            container: null,
            node: null,
          },
        ],
      },
    },
  },
  'bundle-error': {
    errors: { bundle: { kind: 'bundle', message: 'the media file media/a.svg was not found', diagnostics: [] } },
  },
  'hold-finish': { hold: ['finish'] },
  'load-error': { loadError: 'WebAssembly.instantiate(): expected magic word 00 61 73 6d' },
} as const satisfies Record<string, FakeWorkerScript>;

const workerParam = params.get('worker');
const workerScript: FakeWorkerScript =
  workerParam !== null && Object.hasOwn(WORKER_ERROR, workerParam)
    ? WORKER_ERROR[workerParam as keyof typeof WORKER_ERROR]
    : {};

const startWorker = () =>
  new FakeWorker({
    ...sampleFontsScript(),
    ...workerScript,
    version: 'demo',
    zipChunks: [new TextEncoder().encode('not a real zip')],
    report: {
      files: [
        { path: 'index.html', bytes: 182_400 },
        { path: 'media/a.svg', bytes: 214 },
        { path: 'LICENSES.txt', bytes: 4_100 },
      ],
      bytes: 186_714,
      singleFile: false,
      fontsLeftOut: [],
    },
  });

const root = document.getElementById('root');
if (root) {
  createRoot(root).render(
    <StrictMode>
      <App services={{ startWorker, player: slidockPlayer, fonts: browserFontsEnvironment() }} />
    </StrictMode>,
  );
}

/** Runs `step` every 50 ms until it says it is done: the app mounts, listens and loads at its own pace. */
function until(step: () => boolean): void {
  const tick = () => {
    if (!step()) {
      setTimeout(tick, 50);
    }
  };
  tick();
}

// A presenter window (opened from the demo's deck) is the app's own presenter view: nothing is dropped there.
if (params.get('view') !== 'landing' && !location.hash.includes('presenter=')) {
  // The sample deck is dropped until the app answers it (its drop listener is attached in an effect, after the first
  // render): the deck shows, converts, or is refused with an alert. Then the panel opens once the deck shows.
  const showing = () =>
    document.querySelector('[role="toolbar"][aria-label="Deck"], [aria-label="Conversion"], [role="alert"]') !== null;
  let lastDrop = -Infinity;
  until(() => {
    if (showing()) {
      return true;
    }
    if (document.querySelector('main') !== null && performance.now() - lastDrop > 500) {
      lastDrop = performance.now();
      const data = new DataTransfer();
      data.items.add(new File(['demo'], 'Quarterly review.pptx'));
      window.dispatchEvent(new DragEvent('drop', { dataTransfer: data, bubbles: true, cancelable: true }));
    }
    return false;
  });
  const panel = params.get('panel');
  const labels: Record<string, string> = { fonts: 'Fonts', download: 'Download zip', help: 'Keyboard shortcuts' };
  const label = panel ? labels[panel] : undefined;
  if (label) {
    until(() => {
      const toolbar = document.querySelector('[role="toolbar"][aria-label="Deck"]');
      const button = [...(toolbar?.querySelectorAll('button') ?? [])].find(
        (candidate) =>
          candidate.getAttribute('aria-label') === label || candidate.textContent?.trim().startsWith(label),
      );
      button?.click();
      return button !== undefined;
    });
  }
}
