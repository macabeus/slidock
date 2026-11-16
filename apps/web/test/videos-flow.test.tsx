import type { PlannedVideo } from '@slidock/types';
import { screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { ImageCompressor, VideoCompressor } from '../src/media/compressor';
import type { VideoJob, VideoOutcome } from '../src/media/video';
import { SAMPLE_IMAGE_PATH, sampleDeck } from '../src/testing/sample-deck';
import { writeImageChoice } from '../src/views/ImageCompression';
import {
  DEFAULT_VIDEO_CHOICE,
  VIDEO_CHOICE_KEY,
  progressText,
  readVideoChoice,
  writeVideoChoice,
} from '../src/views/VideoCompression';
import { openSample, renderApp, sampleScript } from './harness';

afterEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
});

const VIDEO_PATH = `media/${'b'.repeat(64)}.mov`;
const ALPHA_PATH = `media/${'c'.repeat(64)}.mov`;

function planned(path: string, bytes: number, keep: string | null = null): PlannedVideo {
  return {
    path,
    mime: 'video/quicktime',
    bytes,
    codec: 'avc1',
    width: 1280,
    height: 720,
    rotation: 0,
    durationMs: 2000,
    frames: 60,
    videoBytes: bytes,
    audio: null,
    colour: { primaries: 1, transfer: 1, matrix: 1, fullRange: false, wideGamut: false },
    targetWidth: 640,
    targetHeight: 360,
    keep,
    silent: false,
    uncopyable: null,
  };
}

function stubWebCodecs() {
  for (const name of ['VideoEncoder', 'VideoDecoder', 'EncodedVideoChunk', 'VideoFrame', 'OffscreenCanvas']) {
    vi.stubGlobal(name, function stub() {});
  }
}

/** The sample deck with a video on its second slide. */
function deckWithVideo() {
  const compiled = sampleDeck();
  const blob = new Blob([new Uint8Array(4000)], { type: 'video/quicktime' });
  return {
    ...compiled,
    deck: {
      ...compiled.deck,
      media: [
        ...compiled.deck.media,
        { path: VIDEO_PATH, mime: 'video/quicktime', sha256: 'b'.repeat(64), bytes: 4000 },
      ],
    },
    media: [...compiled.media, { path: VIDEO_PATH, mime: 'video/quicktime', blob }],
  };
}

/** A compressor that writes `size` bytes for each video, reporting half its frames first; or one that waits. */
function fakeCompressor(size: number | null) {
  const jobs: VideoJob[] = [];
  let disposed = 0;
  const waiting: ((outcome: VideoOutcome) => void)[] = [];
  const compressor: VideoCompressor = {
    compress(job, onProgress): Promise<VideoOutcome> {
      jobs.push(job);
      onProgress?.({ frames: job.plan.frames / 2 });
      if (size === null) {
        return new Promise((resolve) => waiting.push(resolve));
      }
      return Promise.resolve({
        kind: 'compressed',
        file: new Blob([new Uint8Array(size)], { type: 'video/mp4' }),
        audio: 'none',
        video: 'encoded',
        codec: 'avc1.64001f',
        elapsed: 1,
      });
    },
    dispose() {
      disposed += 1;
      for (const resolve of waiting.splice(0)) {
        resolve({ kind: 'kept', reason: 'the video workers were stopped', elapsed: 0 });
      }
    },
  };
  return { compressor, jobs, disposed: () => disposed };
}

describe('the video choice', () => {
  it('is off at High by default, and survives storage that throws', () => {
    expect(readVideoChoice(null)).toEqual(DEFAULT_VIDEO_CHOICE);
    writeVideoChoice({ compress: true, quality: 'small' }, localStorage);
    expect(readVideoChoice(localStorage)).toEqual({ compress: true, quality: 'small' });
    localStorage.setItem(VIDEO_CHOICE_KEY, JSON.stringify({ compress: true, quality: 'ultra' }));
    expect(readVideoChoice(localStorage)).toEqual({ compress: true, quality: 'high' });
  });

  it('says how far the videos are and, after a few seconds, how long is left', () => {
    const progress = { done: 1, total: 4, frames: 300, totalFrames: 1200, started: 0, updated: 2000 };
    expect(progressText(progress)).toBe('1 of 4 videos · 25%');
    expect(progressText({ ...progress, updated: 30_000 })).toBe('1 of 4 videos · 25% · about 2 min left');
    expect(progressText({ ...progress, updated: 10_000, frames: 1100 })).toBe('1 of 4 videos · 91% · about 1 s left');
    expect(progressText({ ...progress, total: 0 })).toBe('Planning the videos');
  });
});

describe('a download with compressed videos', () => {
  it('cancelled while the compiler plans, starts no re-encode when the plans arrive', async () => {
    stubWebCodecs();
    writeVideoChoice({ compress: true, quality: 'high' }, localStorage);
    writeImageChoice({ compress: true, quality: 'high' }, localStorage);
    const videos = fakeCompressor(1000);
    let pictures = 0;
    const images: ImageCompressor = {
      compress: () => {
        pictures += 1;
        return Promise.resolve({ kind: 'kept', reason: 'not smaller', elapsed: 0 });
      },
      dispose: () => {},
    };
    const app = renderApp(
      sampleScript({
        deck: deckWithVideo(),
        videos: [planned(VIDEO_PATH, 4000)],
        images: [
          {
            path: SAMPLE_IMAGE_PATH,
            mime: 'image/png',
            bytes: 9,
            width: 4,
            height: 4,
            orientation: 1,
            targetWidth: 2,
            targetHeight: 2,
          },
        ],
        hold: ['videoPlan', 'imagePlan'],
      }),
      undefined,
      undefined,
      () => images,
      () => videos.compressor,
    );
    await openSample(app);
    await app.user.click(screen.getByRole('button', { name: 'Download zip' }));
    const dialog = screen.getByRole('dialog', { name: 'Download zip' });
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    await within(dialog).findByText('Planning the videos');
    await app.user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    app.worker.release('videoPlan');
    app.worker.release('imagePlan');
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(videos.jobs).toEqual([]);
    expect(pictures).toBe(0);
    expect(app.worker.sent.map((message) => message.type)).not.toContain('bundle');
    vi.unstubAllGlobals();
  });

  it('offers no video switch for a deck without videos', async () => {
    const app = renderApp(sampleScript());
    await openSample(app);
    await app.user.click(screen.getByRole('button', { name: 'Download zip' }));
    const dialog = screen.getByRole('dialog', { name: 'Download zip' });
    expect(within(dialog).queryByRole('switch', { name: 'Compress videos' })).toBeNull();
  });

  it('re-encodes the planned videos, hands them over, bundles them and says what it saved and kept', async () => {
    vi.stubGlobal('VideoEncoder', function VideoEncoder() {});
    vi.stubGlobal('VideoDecoder', function VideoDecoder() {});
    vi.stubGlobal('EncodedVideoChunk', function EncodedVideoChunk() {});
    vi.stubGlobal('VideoFrame', function VideoFrame() {});
    vi.stubGlobal('OffscreenCanvas', function OffscreenCanvas() {});
    const videos = fakeCompressor(1000);
    const app = renderApp(
      sampleScript({
        deck: deckWithVideo(),
        videos: [
          planned(VIDEO_PATH, 4000),
          planned(ALPHA_PATH, 9000, 'it has an alpha channel, which H.264 cannot carry'),
        ],
        report: { files: [], bytes: 20_000, singleFile: false, fontsLeftOut: [] },
      }),
      undefined,
      undefined,
      undefined,
      () => videos.compressor,
    );
    await openSample(app);
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    await app.user.click(screen.getByRole('button', { name: 'Download zip' }));
    const dialog = screen.getByRole('dialog', { name: 'Download zip' });
    const toggle = within(dialog).getByRole('switch', { name: 'Compress videos' });
    expect((toggle as HTMLInputElement).checked).toBe(false);
    await app.user.click(toggle);
    await app.user.click(within(dialog).getByRole('radio', { name: /Small/ }));
    expect(readVideoChoice(localStorage)).toEqual({ compress: true, quality: 'small' });
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    await within(dialog).findByText('Saved Quarterly review.slidock.zip');

    const sent = app.worker.sent.map((message) => message.type);
    expect(sent.slice(-3)).toEqual(['videoPlan', 'addVideo', 'bundle']);
    expect(app.worker.sent.at(-1)).toMatchObject({ type: 'bundle', settings: { compressVideos: true } });
    expect(app.worker.videos.map(({ path, file }) => [path, file.size])).toEqual([[VIDEO_PATH, 1000]]);
    // Only the video the plan does not keep is re-encoded, at Small.
    expect(videos.jobs.map(({ plan, quality }) => [plan.path, quality])).toEqual([[VIDEO_PATH, 'small']]);
    expect(within(dialog).getByRole('heading', { name: 'Compressed videos' })).toBeTruthy();
    expect(within(dialog).getByText(/1 of 2 videos re-encoded/)).toBeTruthy();
    expect(within(dialog).getByText('13 KB → 10 KB')).toBeTruthy();
    expect(within(dialog).getByText(/alpha channel/)).toBeTruthy();
    // 20,000 bytes, and the 3000 saved.
    expect(within(dialog).getByText('23 KB')).toBeTruthy();

    // Again: the finished re-encode is reused.
    await app.user.click(within(dialog).getByRole('checkbox', { name: /Single HTML file/ }));
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    await waitFor(() => expect(app.worker.sent.filter((message) => message.type === 'videoPlan')).toHaveLength(2));
    await within(dialog).findByText('Saved Quarterly review.slidock.zip');
    expect(videos.jobs).toHaveLength(1);

    // Another quality re-encodes it, and forgets the first: back to Small, it is encoded again.
    for (const quality of [/High/, /Small/]) {
      await app.user.click(within(dialog).getByRole('radio', { name: quality }));
      await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
      await within(dialog).findByText('Saved Quarterly review.slidock.zip');
    }
    expect(videos.jobs.map(({ quality }) => quality)).toEqual(['small', 'high', 'small']);
    vi.unstubAllGlobals();
  });

  it('says how many videos had their sound, never heard, removed, and what that saved', async () => {
    stubWebCodecs();
    writeVideoChoice({ compress: true, quality: 'high' }, localStorage);
    const sound = { codec: 'mp4a', objectType: 64, aac: true, channels: 2, sampleRate: 48_000, sampleSize: 16 };
    const compressor: VideoCompressor = {
      // The muted one copied as it is, without its sound.
      compress: ({ plan }) =>
        Promise.resolve({
          kind: 'compressed',
          file: new Blob([new Uint8Array(plan.silent ? 2500 : 1000)], { type: 'video/mp4' }),
          audio: plan.silent ? 'removed' : 'copied',
          video: plan.silent ? 'copied' : 'encoded',
          codec: 'avc1',
          elapsed: 1,
        }),
      dispose: () => {},
    };
    const app = renderApp(
      sampleScript({
        deck: deckWithVideo(),
        videos: [
          { ...planned(VIDEO_PATH, 4000), audio: { ...sound, bytes: 1500 }, silent: true },
          { ...planned(ALPHA_PATH, 9000), audio: { ...sound, bytes: 2000 } },
        ],
        report: { files: [], bytes: 20_000, singleFile: false, fontsLeftOut: [] },
      }),
      undefined,
      undefined,
      undefined,
      () => compressor,
    );
    await openSample(app);
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    await app.user.click(screen.getByRole('button', { name: 'Download zip' }));
    const dialog = screen.getByRole('dialog', { name: 'Download zip' });
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    await within(dialog).findByText('Saved Quarterly review.slidock.zip');
    // The other has no file here: kept.
    expect(within(dialog).getByText(/0 of 2 videos re-encoded, 1 copied as it is\./)).toBeTruthy();
    expect(within(dialog).getByText(/Sound removed from 1 video muted in every use/).textContent).toBe(
      'Sound removed from 1 video muted in every use: 1.5 KB saved.',
    );
    vi.unstubAllGlobals();
  });

  it('cancels a running re-encode: the workers stop, nothing is handed over, and the next download starts again', async () => {
    vi.stubGlobal('VideoEncoder', function VideoEncoder() {});
    vi.stubGlobal('VideoDecoder', function VideoDecoder() {});
    vi.stubGlobal('EncodedVideoChunk', function EncodedVideoChunk() {});
    vi.stubGlobal('VideoFrame', function VideoFrame() {});
    vi.stubGlobal('OffscreenCanvas', function OffscreenCanvas() {});
    writeVideoChoice({ compress: true, quality: 'high' }, localStorage);
    const videos = fakeCompressor(null);
    const app = renderApp(
      sampleScript({ deck: deckWithVideo(), videos: [planned(VIDEO_PATH, 4000)] }),
      undefined,
      undefined,
      undefined,
      () => videos.compressor,
    );
    await openSample(app);
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    await app.user.click(screen.getByRole('button', { name: 'Download zip' }));
    const dialog = screen.getByRole('dialog', { name: 'Download zip' });
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    await within(dialog).findByText(/0 of 1 video · 50%/);
    expect(within(dialog).getByRole('progressbar', { name: 'Compressing videos' })).toBeTruthy();

    await app.user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() =>
      expect((within(dialog).getByRole('button', { name: 'Build and save' }) as HTMLButtonElement).disabled).toBe(
        false,
      ),
    );
    expect(videos.disposed()).toBe(1);
    expect(app.worker.videos).toEqual([]);
    expect(app.worker.sent.map((message) => message.type)).not.toContain('bundle');

    // The stopped video is not reused: the next download re-encodes it.
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    await waitFor(() => expect(videos.jobs).toHaveLength(2));
    vi.unstubAllGlobals();
  });
});
