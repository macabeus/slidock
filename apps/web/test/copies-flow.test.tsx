import type { PlannedVideo } from '@slidock/types';
import { screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { AlphaCopier, AlphaJob, AlphaOutcome } from '../src/media/alpha';
import { sampleDeck } from '../src/testing/sample-deck';
import { COPIES_CHOICE_KEY, readCopiesChoice, writeCopiesChoice } from '../src/views/VideoCopies';
import { openSample, renderApp, sampleScript } from './harness';

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  localStorage.clear();
});

const ALPHA_PATH = `media/${'c'.repeat(64)}.mov`;

function planned(): PlannedVideo {
  return {
    path: ALPHA_PATH,
    mime: 'video/quicktime',
    bytes: 9000,
    codec: 'hvc1',
    width: 1280,
    height: 720,
    rotation: 0,
    durationMs: 2000,
    frames: 60,
    videoBytes: 9000,
    audio: null,
    colour: { primaries: 1, transfer: 1, matrix: 1, fullRange: false, wideGamut: false },
    targetWidth: 640,
    targetHeight: 360,
    keep: 'it has an alpha channel, which H.264 cannot carry',
    silent: false,
    uncopyable: null,
  };
}

/** The sample deck with a transparent video. */
function deckWithAlpha() {
  const compiled = sampleDeck();
  const blob = new Blob([new Uint8Array(9000)], { type: 'video/quicktime' });
  return {
    ...compiled,
    deck: {
      ...compiled.deck,
      media: [
        ...compiled.deck.media,
        { path: ALPHA_PATH, mime: 'video/quicktime', sha256: 'c'.repeat(64), bytes: 9000 },
      ],
    },
    media: [...compiled.media, { path: ALPHA_PATH, mime: 'video/quicktime', blob }],
  };
}

/** A copier whose copies are `outcome` (a 3000-byte WebM by default), reporting half the frames first. */
function fakeCopier(outcome?: AlphaOutcome) {
  const jobs: AlphaJob[] = [];
  const copier: AlphaCopier = {
    copy(job, onProgress) {
      jobs.push(job);
      onProgress?.({ frames: job.plan.frames / 2 });
      return Promise.resolve(
        outcome ?? {
          kind: 'made',
          file: new Blob([new Uint8Array(3000)], { type: 'video/webm' }),
          codec: 'vp09.00.30.08',
          elapsed: 1,
        },
      );
    },
    dispose: () => {},
  };
  return { copier, jobs };
}

/** The globals a browser that makes copies has. */
function stubCopying() {
  for (const name of ['VideoEncoder', 'VideoFrame', 'OffscreenCanvas', 'Worker']) {
    vi.stubGlobal(name, function stub() {});
  }
}

async function openDialog(app: ReturnType<typeof renderApp>) {
  await openSample(app);
  vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
  await app.user.click(screen.getByRole('button', { name: 'Download zip' }));
  return screen.getByRole('dialog', { name: 'Download zip' });
}

describe('the copies choice', () => {
  it('is on by default, and survives storage that throws', () => {
    expect(readCopiesChoice(null)).toBe(true);
    writeCopiesChoice(false, localStorage);
    expect(localStorage.getItem(COPIES_CHOICE_KEY)).toBe('off');
    expect(readCopiesChoice(localStorage)).toBe(false);
    const throwing = {
      getItem: () => {
        throw new Error('blocked');
      },
    } as unknown as Storage;
    expect(readCopiesChoice(throwing)).toBe(true);
  });
});

describe('a download of a deck with a transparent video', () => {
  it('makes its copy, hands it over and bundles it, whatever the compression switch says', async () => {
    stubCopying();
    const copies = fakeCopier();
    const app = renderApp(
      sampleScript({
        deck: deckWithAlpha(),
        alternates: [planned()],
        report: { files: [], bytes: 20_000, singleFile: false, fontsLeftOut: [] },
      }),
      undefined,
      undefined,
      undefined,
      undefined,
      () => copies.copier,
    );
    const dialog = await openDialog(app);
    const toggle = await within(dialog).findByRole('switch', { name: 'Play transparent videos in every browser' });
    expect((toggle as HTMLInputElement).checked).toBe(true);
    expect(within(dialog).getByText(/1 video with transparency plays only in Safari and Chrome on a Mac/)).toBeTruthy();
    expect((within(dialog).queryByRole('switch', { name: 'Compress videos' }) as HTMLInputElement).checked).toBe(false);
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    await within(dialog).findByText('Saved Quarterly review.slidock.zip');
    expect(copies.jobs.map(({ plan }) => plan.path)).toEqual([ALPHA_PATH]);
    expect(app.worker.alternates.map(({ path, file }) => [path, file.size])).toEqual([[ALPHA_PATH, 3000]]);
    expect(app.worker.sent.at(-1)).toMatchObject({ type: 'bundle', settings: { alternateVideos: true } });
    expect(app.worker.sent.at(-1)).not.toMatchObject({ settings: { compressVideos: true } });
    expect(within(dialog).getByText(/1 of 1 video with a WebM copy for every browser/)).toBeTruthy();
    expect(within(dialog).queryByText('1 video plays only in Safari and Chrome on a Mac')).toBeNull();
    // Again: the finished copy is reused.
    await app.user.click(within(dialog).getByRole('checkbox', { name: /Single HTML file/ }));
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    await waitFor(() => expect(app.worker.sent.filter((message) => message.type === 'addAlternate')).toHaveLength(2));
    expect(copies.jobs).toHaveLength(1);
  });

  it("sends the page's fields beside the copy", async () => {
    stubCopying();
    const copies = fakeCopier();
    const app = renderApp(
      sampleScript({
        deck: deckWithAlpha(),
        alternates: [planned()],
        report: { files: [], bytes: 20_000, singleFile: false, fontsLeftOut: [] },
      }),
      undefined,
      undefined,
      undefined,
      undefined,
      () => copies.copier,
    );
    const dialog = await openDialog(app);
    await within(dialog).findByRole('switch', { name: 'Play transparent videos in every browser' });
    const title = within(dialog).getByRole('textbox', { name: 'Title' });
    await app.user.clear(title);
    await app.user.type(title, 'Talk');
    await app.user.click(within(dialog).getByRole('checkbox', { name: /^Speaker notes/ }));
    await app.user.click(within(dialog).getByRole('checkbox', { name: /Text version below the slides/ }));
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    await within(dialog).findByText(/^Saved /);
    expect(app.worker.sent.at(-1)).toMatchObject({
      type: 'bundle',
      settings: { alternateVideos: true, title: 'Talk', notes: 'none', textView: false },
    });
    expect(within(dialog).getByText(/1 of 1 video with a WebM copy for every browser/)).toBeTruthy();
  });

  it('says the video plays only in Safari and Chrome on a Mac, and how to fix it, when this browser cannot copy it', async () => {
    stubCopying();
    const copies = fakeCopier({ kind: 'kept', reason: 'this browser plays no hvc1 video', elapsed: 0 });
    const app = renderApp(
      sampleScript({ deck: deckWithAlpha(), alternates: [planned()] }),
      undefined,
      undefined,
      undefined,
      undefined,
      () => copies.copier,
    );
    const dialog = await openDialog(app);
    await within(dialog).findByRole('switch', { name: 'Play transparent videos in every browser' });
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    await within(dialog).findByText('Saved Quarterly review.slidock.zip');
    expect(app.worker.alternates).toEqual([]);
    expect(within(dialog).getByText('1 video plays only in Safari and Chrome on a Mac')).toBeTruthy();
    expect(within(dialog).getByText(/this browser plays no hvc1 video/)).toBeTruthy();
    expect(within(dialog).getByText(/download the deck again from Safari or Chrome on a Mac/)).toBeTruthy();
  });

  it('leaves the copy out when switched off, and says so', async () => {
    stubCopying();
    const copies = fakeCopier();
    const app = renderApp(
      sampleScript({ deck: deckWithAlpha(), alternates: [planned()] }),
      undefined,
      undefined,
      undefined,
      undefined,
      () => copies.copier,
    );
    const dialog = await openDialog(app);
    await app.user.click(
      await within(dialog).findByRole('switch', { name: 'Play transparent videos in every browser' }),
    );
    expect(readCopiesChoice(localStorage)).toBe(false);
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    await within(dialog).findByText('Saved Quarterly review.slidock.zip');
    expect(copies.jobs).toEqual([]);
    expect(app.worker.sent.map((message) => message.type)).not.toContain('addAlternate');
    expect(app.worker.sent.at(-1)).not.toMatchObject({ settings: { alternateVideos: true } });
    expect(within(dialog).getByText(/no copy was asked for/)).toBeTruthy();
  });

  it('offers no switch when no video gets a copy, and a disabled one where the browser cannot make them', async () => {
    const plain = renderApp(sampleScript({ deck: deckWithAlpha(), alternates: [] }));
    let dialog = await openDialog(plain);
    await waitFor(() => expect(plain.worker.sent.map((message) => message.type)).toContain('alternatePlan'));
    expect(within(dialog).queryByRole('switch', { name: 'Play transparent videos in every browser' })).toBeNull();
    plain.unmount();

    // No VideoEncoder here (jsdom).
    const app = renderApp(sampleScript({ deck: deckWithAlpha(), alternates: [planned()] }));
    dialog = await openDialog(app);
    const toggle = await within(dialog).findByRole('switch', { name: 'Play transparent videos in every browser' });
    expect((toggle as HTMLInputElement).disabled).toBe(true);
    expect(within(dialog).getByText(/This browser cannot make the copies/)).toBeTruthy();
  });
});
