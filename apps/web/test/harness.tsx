import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect } from 'vitest';

import { App } from '../src/App';
import type { FontsEnvironment } from '../src/fonts/controller';
import type { AlphaCopier } from '../src/media/alpha';
import type { ImageCompressor, VideoCompressor } from '../src/media/compressor';
import type { PhotoDecoder } from '../src/media/photos';
import { testFontsEnvironment } from '../src/testing/fake-fonts';
import { FakePlayerFactory } from '../src/testing/fake-player';
import { FakeWorker, type FakeWorkerScript } from '../src/testing/fake-worker';
import { SAMPLE_DIAGNOSTICS, sampleDeck } from '../src/testing/sample-deck';

/**
 * Renders the app over fake workers (one per start, each from `script`), a fake player, an offline fonts
 * environment (or `fonts`), no photo decoder (or `photos`), no picture or video compressor (or `images`, `videos`)
 * and no copier of transparent videos (or `alternates`).
 */
export function renderApp(
  script: FakeWorkerScript | ((index: number) => FakeWorkerScript) = {},
  fonts: FontsEnvironment = testFontsEnvironment(),
  photos?: () => PhotoDecoder,
  images?: () => ImageCompressor,
  videos?: () => VideoCompressor,
  alternates?: () => AlphaCopier,
) {
  const workers: FakeWorker[] = [];
  const player = new FakePlayerFactory();
  const startWorker = () => {
    const worker = new FakeWorker(typeof script === 'function' ? script(workers.length) : script);
    workers.push(worker);
    return worker;
  };
  // A person can pick any file with the picker's “All files” option.
  const user = userEvent.setup({ applyAccept: false });
  const utils = render(
    <App
      services={{
        startWorker,
        player,
        fonts,
        ...(photos ? { photos } : {}),
        ...(images ? { images } : {}),
        ...(videos ? { videos } : {}),
        ...(alternates ? { alternates } : {}),
      }}
    />,
  );
  return {
    ...utils,
    user,
    player,
    workers,
    fonts,
    get worker(): FakeWorker {
      const worker = workers.at(-1);
      if (!worker) {
        throw new Error('no worker started');
      }
      return worker;
    },
  };
}

/** A script that compiles the sample deck. */
export function sampleScript(extra: FakeWorkerScript = {}): FakeWorkerScript {
  return { version: '9.9.9', deck: sampleDeck(), diagnostics: SAMPLE_DIAGNOSTICS, ...extra };
}

export function fileInput(): HTMLInputElement {
  const input = document.querySelector<HTMLInputElement>('input[type="file"]');
  if (!input) {
    throw new Error('no file input');
  }
  return input;
}

/** Picks `file` with the landing's file input. */
export async function pick(user: ReturnType<typeof userEvent.setup>, file: File) {
  await user.upload(fileInput(), file);
}

/** Picks the sample deck and waits for the viewer. */
export async function openSample(app: ReturnType<typeof renderApp>) {
  await waitFor(() => expect(screen.getByTestId('wasm-status').textContent).toBe('slidock-wasm 9.9.9'));
  await pick(app.user, new File(['pptx bytes'], 'Quarterly review.pptx'));
  await screen.findByRole('toolbar', { name: 'Deck' });
}

/** Fires a window-wide drop of `items` (as `DataTransferItem`-like objects). */
export function dropFiles(files: File[]) {
  const items = files.map((file) => ({ kind: 'file', webkitGetAsEntry: () => null, getAsFile: () => file }));
  const dataTransfer = { types: ['Files'], items, files, dropEffect: 'none' };
  const event = new Event('drop', { bubbles: true, cancelable: true });
  Object.defineProperty(event, 'dataTransfer', { value: dataTransfer });
  window.dispatchEvent(event);
}
