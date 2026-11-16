import type { PlannedImage } from '@slidock/types';
import { screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { CompressJob, CompressOutcome } from '../src/media/compress';
import type { ImageCompressor } from '../src/media/compressor';
import { SAMPLE_IMAGE_PATH } from '../src/testing/sample-deck';
import {
  DEFAULT_IMAGE_CHOICE,
  IMAGE_CHOICE_KEY,
  readImageChoice,
  writeImageChoice,
  zipWithout,
} from '../src/views/ImageCompression';
import { openSample, renderApp, sampleScript } from './harness';

afterEach(() => {
  vi.restoreAllMocks();
});

function planned(path: string, bytes: number): PlannedImage {
  return { path, mime: 'image/png', bytes, width: 40, height: 30, orientation: 1, targetWidth: 20, targetHeight: 15 };
}

/** A compressor that writes `sizes[path]` bytes for a picture, or keeps it when it names none. */
function fakeCompressor(sizes: Record<string, number>) {
  const jobs: CompressJob[] = [];
  let disposed = 0;
  const compressor: ImageCompressor = {
    compress(job): Promise<CompressOutcome> {
      jobs.push(job);
      const size = sizes[job.plan.path];
      return Promise.resolve(
        size === undefined
          ? { kind: 'kept', reason: 'not smaller', elapsed: 0 }
          : { kind: 'compressed', bytes: new ArrayBuffer(size), mime: 'image/webp', elapsed: 1 },
      );
    },
    dispose: () => void (disposed += 1),
  };
  return { compressor, jobs, disposed: () => disposed };
}

/** A storage whose every call throws, as a browser with blocked site data has. */
const BLOCKED = {
  getItem: () => {
    throw new Error('blocked');
  },
  setItem: () => {
    throw new Error('blocked');
  },
} as unknown as Storage;

describe('the image choice', () => {
  it('is off at High by default, and reads back what was written', () => {
    expect(readImageChoice(null)).toEqual(DEFAULT_IMAGE_CHOICE);
    expect(DEFAULT_IMAGE_CHOICE).toEqual({ compress: false, quality: 'high' });
    writeImageChoice({ compress: true, quality: 'small' }, localStorage);
    expect(readImageChoice(localStorage)).toEqual({ compress: true, quality: 'small' });
  });

  it('falls back to the default on storage that throws or holds something else', () => {
    expect(() => writeImageChoice({ compress: true, quality: 'small' }, BLOCKED)).not.toThrow();
    expect(readImageChoice(BLOCKED)).toEqual(DEFAULT_IMAGE_CHOICE);
    localStorage.setItem(IMAGE_CHOICE_KEY, '{not json');
    expect(readImageChoice(localStorage)).toEqual(DEFAULT_IMAGE_CHOICE);
    localStorage.setItem(IMAGE_CHOICE_KEY, JSON.stringify({ compress: 'yes', quality: 'ultra' }));
    expect(readImageChoice(localStorage)).toEqual(DEFAULT_IMAGE_CHOICE);
  });

  it('estimates the zip without compression: the bytes saved back, base64 in a single file', () => {
    const images = { planned: 2, compressed: 1, before: 900, after: 300, kept: [], elapsed: 0 };
    expect(zipWithout(images, 1000, false)).toBe(1600);
    expect(zipWithout(images, 1000, true)).toBe(1800);
  });
});

describe('a download with compressed images', () => {
  it('re-encodes the planned pictures, hands the smaller ones over, bundles them and says what it saved', async () => {
    const images = fakeCompressor({ [SAMPLE_IMAGE_PATH]: 300 });
    const app = renderApp(
      sampleScript({
        images: [planned(SAMPLE_IMAGE_PATH, 1000), planned('media/elsewhere.png', 500)],
        report: { files: [], bytes: 5000, singleFile: false, fontsLeftOut: [] },
      }),
      undefined,
      undefined,
      () => images.compressor,
    );
    await openSample(app);
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});

    await app.user.click(screen.getByRole('button', { name: 'Download zip' }));
    const dialog = screen.getByRole('dialog', { name: 'Download zip' });
    const toggle = within(dialog).getByRole('switch', { name: 'Compress images' });
    expect((toggle as HTMLInputElement).checked).toBe(false);
    expect(within(dialog).queryByRole('radio')).toBeNull();
    await app.user.click(toggle);
    await app.user.click(within(dialog).getByRole('radio', { name: /Medium/ }));
    expect(readImageChoice(localStorage)).toEqual({ compress: true, quality: 'medium' });
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    await within(dialog).findByText('Saved Quarterly review.slidock.zip');

    const sent = app.worker.sent.map((message) => message.type);
    expect(sent.slice(-3)).toEqual(['imagePlan', 'addImage', 'bundle']);
    expect(app.worker.sent.at(-1)).toMatchObject({ type: 'bundle', settings: { compressImages: true } });
    expect(app.worker.images.map(({ path, bytes }) => [path, bytes.byteLength])).toEqual([[SAMPLE_IMAGE_PATH, 300]]);
    // The deck's file at Medium quality; the picture the app holds no file of is kept.
    expect(images.jobs.map(({ plan, quality }) => [plan.path, quality])).toEqual([[SAMPLE_IMAGE_PATH, 0.8]]);
    expect(within(dialog).getByText(/1 of 2 pictures re-encoded/)).toBeTruthy();
    expect(within(dialog).getByText('1.5 KB → 800 B')).toBeTruthy();
    // 5000 bytes, and the 700 saved.
    expect(within(dialog).getByText('5.7 KB')).toBeTruthy();

    // Off again: the next zip asks for no plan and writes the originals.
    await app.user.click(toggle);
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    await waitFor(() => expect(app.worker.sent.at(-1)?.type).toBe('bundle'));
    expect(app.worker.sent.at(-1)).toMatchObject({ settings: { singleFile: false } });
    expect(app.worker.sent.at(-1)).not.toMatchObject({ settings: { compressImages: true } });
    expect(app.worker.sent.at(-2)?.type).not.toBe('imagePlan');
  });

  it('keeps a picture the compiler refuses, and reuses the re-encodes for the next zip', async () => {
    const images = fakeCompressor({ [SAMPLE_IMAGE_PATH]: 300 });
    writeImageChoice({ compress: true, quality: 'high' }, localStorage);
    const app = renderApp(
      sampleScript({
        images: [planned(SAMPLE_IMAGE_PATH, 1000)],
        refuseImage: () => 'media/x.svg: the file is 21 × 15, not the planned 20 × 15',
      }),
      undefined,
      undefined,
      () => images.compressor,
    );
    await openSample(app);
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    await app.user.click(screen.getByRole('button', { name: 'Download zip' }));
    const dialog = screen.getByRole('dialog', { name: 'Download zip' });
    expect((within(dialog).getByRole('switch', { name: 'Compress images' }) as HTMLInputElement).checked).toBe(true);
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    await within(dialog).findByText('Saved Quarterly review.slidock.zip');
    expect(within(dialog).getByText(/0 of 1 picture re-encoded/)).toBeTruthy();
    expect(within(dialog).getByText(/not the planned 20 × 15/)).toBeTruthy();

    await app.user.click(within(dialog).getByRole('checkbox', { name: /Single HTML file/ }));
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    await waitFor(() => expect(app.worker.sent.filter((message) => message.type === 'imagePlan')).toHaveLength(2));
    await within(dialog).findByText('Saved Quarterly review.slidock.zip');
    expect(images.jobs).toHaveLength(1);
  });
});
