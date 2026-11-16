import type { LocalPick } from '@slidock/types';
import { describe, expect, it, vi } from 'vitest';

import {
  LocalBudget,
  type LocalFontData,
  MAX_LOCAL_BYTES,
  MAX_LOCAL_FILES,
  MAX_LOCAL_FILE_BYTES,
  NoLocalMatching,
  browserLocalFonts,
  lfaFailure,
  readLocalFonts,
} from '../../src/fonts/localFonts';
import { localFont } from '../../src/testing/fake-fonts';

/** A window with what Local Font Access needs, or without it. */
function fakeWindow(options: { query?: () => Promise<LocalFontData[]>; permission?: () => Promise<PermissionStatus> }) {
  return {
    queryLocalFonts: options.query,
    navigator: { permissions: { query: options.permission ?? (() => Promise.reject(new TypeError('unknown'))) } },
  } as unknown as Window;
}

/** A face whose `blob()` has `size` bytes without holding them. */
function sizedFont(postscriptName: string, size: number): LocalFontData {
  return {
    ...localFont(postscriptName, postscriptName),
    blob: () => Promise.resolve({ size, arrayBuffer: () => Promise.resolve(new ArrayBuffer(4)) } as Blob),
  };
}

const pick = (index: number, exactPostscript = false): LocalPick => ({
  index,
  postscriptName: `Face${index}`,
  family: 'Face',
  exactPostscript,
});

describe('browserLocalFonts', () => {
  it('is null where the browser has no queryLocalFonts (Firefox, Safari, an insecure context)', () => {
    expect(browserLocalFonts(fakeWindow({}))).toBeNull();
  });

  it('reads the local-fonts permission, and says prompt when the browser cannot', async () => {
    for (const state of ['granted', 'denied', 'prompt'] as const) {
      const api = browserLocalFonts(
        fakeWindow({ query: async () => [], permission: async () => ({ state }) as PermissionStatus }),
      );
      expect(await api?.permission()).toBe(state);
    }
    const unknown = browserLocalFonts(fakeWindow({ query: async () => [] }));
    expect(await unknown?.permission()).toBe('prompt');
  });

  it('calls queryLocalFonts on the window', async () => {
    const query = vi.fn(async () => [localFont('Arial-BoldMT', 'Arial')]);
    const win = fakeWindow({ query });
    const fonts = await browserLocalFonts(win)?.query();
    expect(fonts?.map((font) => font.postscriptName)).toEqual(['Arial-BoldMT']);
    expect(query.mock.contexts[0]).toBe(win);
  });
});

describe('lfaFailure', () => {
  it('maps the browser errors to what the panel says', () => {
    expect(lfaFailure(new DOMException('denied', 'NotAllowedError'))).toEqual({ kind: 'denied' });
    expect(lfaFailure(new DOMException('no activation', 'SecurityError'))).toEqual({ kind: 'activation' });
    expect(lfaFailure(new Error('boom'))).toEqual({ kind: 'failed', message: 'boom' });
    expect(lfaFailure('odd')).toEqual({ kind: 'failed', message: 'odd' });
  });
});

describe('readLocalFonts', () => {
  it('sends the names only, then reads and adds just the picked files', async () => {
    const fonts = [localFont('Arial-BoldMT', 'Arial'), localFont('HelveticaNeue', 'Helvetica Neue')];
    const blobs = fonts.map((font) => vi.spyOn(font, 'blob'));
    const select = vi.fn(async () => [
      { index: 1, postscriptName: 'HelveticaNeue', family: 'Helvetica Neue', exactPostscript: true },
    ]);
    const added: string[] = [];
    const result = await readLocalFonts(
      { query: async () => fonts, permission: async () => 'granted' },
      { select, add: async (_bytes, chosen) => void added.push(chosen.postscriptName), budget: new LocalBudget() },
    );
    expect(select).toHaveBeenCalledWith([
      { postscriptName: 'Arial-BoldMT', fullName: 'Arial BoldMT', family: 'Arial', style: 'Regular' },
      { postscriptName: 'HelveticaNeue', fullName: 'HelveticaNeue', family: 'Helvetica Neue', style: 'Regular' },
    ]);
    expect(blobs[0]).not.toHaveBeenCalled();
    expect(blobs[1]).toHaveBeenCalledOnce();
    expect(added).toEqual(['HelveticaNeue']);
    expect(result).toEqual({ listed: 2, picked: 1, added: 1, known: 0, skipped: 0, skippedFiles: [] });
  });

  it('asks the worker nothing when nothing is installed', async () => {
    const select = vi.fn(async () => []);
    const result = await readLocalFonts(
      { query: async () => [], permission: async () => 'granted' },
      { select, add: async () => {}, budget: new LocalBudget() },
    );
    expect(select).not.toHaveBeenCalled();
    expect(result).toEqual({ listed: 0, picked: 0, added: 0, known: 0, skipped: 0, skippedFiles: [] });
  });

  it('reads at most 48 files and 96 MB, and a file over 48 MB only for an exact PostScript name', async () => {
    const many = Array.from({ length: MAX_LOCAL_FILES + 2 }, (_, index) => sizedFont(`Face${index}`, 1000));
    const counted = await readLocalFonts(
      { query: async () => many, permission: async () => 'granted' },
      { select: async () => many.map((_, index) => pick(index)), add: async () => {}, budget: new LocalBudget() },
    );
    expect(counted).toMatchObject({ added: MAX_LOCAL_FILES, skipped: 2 });
    expect(counted.skippedFiles.map((file) => file.reason)).toEqual([
      'over the 48 installed files read for one presentation',
      'over the 48 installed files read for one presentation',
    ]);

    const big = [
      sizedFont('Face0', MAX_LOCAL_FILE_BYTES + 1),
      sizedFont('Face1', MAX_LOCAL_FILE_BYTES + 1),
      sizedFont('Face2', MAX_LOCAL_BYTES - MAX_LOCAL_FILE_BYTES),
      sizedFont('Face3', 1),
    ];
    const added: number[] = [];
    const sized = await readLocalFonts(
      { query: async () => big, permission: async () => 'granted' },
      {
        select: async () => [pick(0), pick(1, true), pick(2), pick(3)],
        add: async (_bytes, chosen) => void added.push(chosen.index),
        budget: new LocalBudget(),
      },
    );
    // Face0 is too large for a family match; Face1 is an exact hit; Face2 would pass the session's 96 MB.
    expect(added).toEqual([1, 3]);
    expect(sized).toMatchObject({ picked: 4, added: 2, skipped: 2 });
    // Each file not used says which limit it is over.
    expect(sized.skippedFiles).toEqual([
      { name: sizedFont('Face0', 0).fullName, reason: 'its file is 48 MB, over the 48 MB a font file may have' },
      {
        name: sizedFont('Face2', 0).fullName,
        reason: 'its file is 48 MB, over the 48 MB left of the 96 MB of installed fonts one presentation may read',
      },
    ]);
  });

  it('counts a pick the worker refuses, an out-of-range index and a repeated pick as skipped', async () => {
    const fonts = [localFont('A', 'A'), localFont('B', 'B')];
    const result = await readLocalFonts(
      { query: async () => fonts, permission: async () => 'granted' },
      {
        select: async () => [pick(0), pick(0), pick(7), pick(1)],
        add: async (_bytes, chosen) => {
          if (chosen.index === 1) {
            throw new Error('not a font');
          }
        },
        budget: new LocalBudget(),
      },
    );
    expect(result).toMatchObject({ listed: 2, picked: 4, added: 1, known: 0, skipped: 3 });
    // The refused file is named with the worker's reason; the bad picks name no file.
    expect(result.skippedFiles).toEqual([{ name: fonts[1]?.fullName, reason: 'not a font' }]);
  });

  it('holds the caps for the whole session, and reads a file added before only once', async () => {
    const MB = 1 << 20;
    const fonts = [
      sizedFont('Face0', 30 * MB),
      sizedFont('Face1', 30 * MB),
      sizedFont('Face2', 30 * MB),
      sizedFont('Face3', 30 * MB),
    ];
    const budget = new LocalBudget();
    const added: number[] = [];
    const read = () =>
      readLocalFonts(
        { query: async () => fonts, permission: async () => 'granted' },
        {
          select: async () => fonts.map((_, index) => pick(index)),
          add: async (_bytes, chosen) => void added.push(chosen.index),
          budget,
        },
      );
    expect(await read()).toMatchObject({ added: 3, known: 0, skipped: 1 });
    // Look again: the three files are known, the fourth still passes the session's 96 MB.
    expect(await read()).toMatchObject({ added: 0, known: 3, skipped: 1 });
    expect(await read()).toMatchObject({ added: 0, known: 3, skipped: 1 });
    expect(added).toEqual([0, 1, 2]);
    expect(budget.bytes).toBe(90 * MB);

    const many = Array.from({ length: MAX_LOCAL_FILES + 2 }, (_, index) => sizedFont(`Face${index}`, 1000));
    const shared = new LocalBudget();
    const half = (from: number) =>
      readLocalFonts(
        { query: async () => many, permission: async () => 'granted' },
        {
          select: async () => many.slice(from, from + 30).map((_, index) => pick(from + index)),
          add: async () => {},
          budget: shared,
        },
      );
    expect(await half(0)).toMatchObject({ added: 30 });
    expect(await half(30)).toMatchObject({ added: MAX_LOCAL_FILES - 30, skipped: 2 });
    expect(shared.files).toBe(MAX_LOCAL_FILES);
  });

  it('stops when the worker answers with no picks at all (it cannot match installed fonts)', async () => {
    const add = vi.fn(async () => {});
    await expect(
      readLocalFonts(
        { query: async () => [localFont('A', 'A')], permission: async () => 'granted' },
        { select: async () => null, add, budget: new LocalBudget() },
      ),
    ).rejects.toBeInstanceOf(NoLocalMatching);
    expect(add).not.toHaveBeenCalled();
  });

  it('lets the browser error through, for the panel to show', async () => {
    const denied = new DOMException('denied', 'NotAllowedError');
    await expect(
      readLocalFonts(
        { query: () => Promise.reject(denied), permission: async () => 'prompt' },
        { select: async () => [], add: async () => {}, budget: new LocalBudget() },
      ),
    ).rejects.toBe(denied);
  });
});
