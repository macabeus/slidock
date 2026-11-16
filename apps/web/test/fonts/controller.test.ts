import type { AddedFontSource, FontNeed, SubstituteFace } from '@slidock/types';
import { describe, expect, it, vi } from 'vitest';

import { FontsController, type FontsHost, GOOGLE_PREFERENCE, googlePlans } from '../../src/fonts/controller';
import { MAX_USER_FILES } from '../../src/fonts/dropped';
import type { Fetch } from '../../src/fonts/google';
import { fakeLocalFonts, localFont, memoryStorage, testFontsEnvironment } from '../../src/testing/fake-fonts';
import type { FakeFont } from '../../src/testing/fake-worker';
import {
  ROBOTO_PLAN,
  pickSampleLocal,
  refuseNonFont,
  sampleFontNeeds,
  sampleFontsDeck,
} from '../../src/testing/sample-fonts';
import { testNetwork } from './network';

const TTF = new Uint8Array([0, 1, 0, 0, 1, 2, 3, 4]);

/** A host over the sample font story, logging what the controller asks. */
function storyHost() {
  let fonts: FakeFont[] = [];
  const log: string[] = [];
  let finishes = 0;
  const host: FontsHost = {
    async addFont(bytes: ArrayBuffer, source: AddedFontSource, unicodeRange: string | null) {
      const font = { source, bytes, unicodeRange };
      const refused = refuseNonFont(font);
      log.push(`add ${source}${refused ? ' refused' : ''}`);
      if (refused) {
        throw new Error(refused);
      }
      fonts.push(font);
      return sampleFontNeeds(fonts);
    },
    async selectLocal(metas) {
      log.push(metas.length === 0 ? 'probe' : 'select');
      return pickSampleLocal(metas);
    },
    async refinish() {
      finishes += 1;
      log.push('finish');
    },
    async reopen() {
      log.push('reopen');
      fonts = [];
      return sampleFontNeeds(fonts);
    },
    slides: () => sampleFontsDeck(fonts).slides,
    substitutes: () => sampleFontsDeck(fonts).substitutes ?? [],
  };
  return {
    host,
    log,
    get fonts() {
      return fonts;
    },
    get finishes() {
      return finishes;
    },
  };
}

describe('googlePlans', () => {
  it('takes each plan once, only for families with open faces', () => {
    const needs = sampleFontNeeds([]);
    const twice: FontNeed[] = [...needs, { ...needs[1], family: 'Roboto 2' } as FontNeed];
    expect(googlePlans(twice)).toEqual([ROBOTO_PLAN]);
    expect(googlePlans(needs.map((need) => ({ ...need, faces: [] })))).toEqual([]);
  });
});

describe('FontsController', () => {
  it('on start: the substitutes and Google, each with its re-finish, never Local Font Access unasked', async () => {
    const story = storyHost();
    const network = testNetwork();
    const local = fakeLocalFonts([localFont('HelveticaNeue', 'Helvetica Neue')], { state: 'prompt' });
    const controller = new FontsController(
      story.host,
      testFontsEnvironment({ fetch: network.fetch, localFonts: local }),
      sampleFontNeeds([]),
    );
    await controller.start();
    expect(local.queries).toBe(0);
    const state = controller.getSnapshot();
    expect(state.permission).toBe('prompt');
    expect(state.google).toMatchObject({ state: 'done', result: { families: ['Roboto'], files: 2, problems: [] } });
    expect(state.substitutes).toMatchObject({ state: 'done', families: ['Liberation Sans'], problems: [] });
    expect(state.needs.find((need) => need.family === 'Roboto')?.faces).toEqual([]);
    expect(story.log[0]).toBe('probe');
    expect(story.log.filter((entry) => entry.startsWith('add')).sort()).toEqual([
      'add bundled',
      'add bundled',
      'add google',
      'add google',
    ]);
    expect(story.finishes).toBeGreaterThanOrEqual(1);
    expect(story.log.at(-1)).toBe('finish');
    expect(network.google().every(({ url }) => !new URL(url).searchParams.has('text'))).toBe(true);
  });

  it('reads the installed fonts on start when the permission is granted', async () => {
    const story = storyHost();
    const local = fakeLocalFonts(
      [localFont('HelveticaNeue', 'Helvetica Neue'), localFont('Optima-Regular', 'Optima')],
      {
        state: 'granted',
      },
    );
    const controller = new FontsController(
      story.host,
      testFontsEnvironment({ localFonts: local, storage: memoryStorage() }),
      sampleFontNeeds([]),
    );
    await controller.start();
    expect(local.queries).toBe(1);
    expect(controller.getSnapshot().local).toEqual({
      state: 'done',
      result: { listed: 2, picked: 2, added: 2, known: 0, skipped: 0, skippedFiles: [] },
    });
    expect(story.log.filter((entry) => entry !== 'add bundled').slice(0, 4)).toEqual([
      'probe',
      'select',
      'add local',
      'add local',
    ]);
  });

  it('keeps Google off when the viewer turned it off, and fetches nothing', async () => {
    const story = storyHost();
    const network = testNetwork();
    const storage = memoryStorage();
    storage.setItem(GOOGLE_PREFERENCE, 'off');
    const controller = new FontsController(
      story.host,
      testFontsEnvironment({ fetch: network.fetch, storage }),
      sampleFontNeeds([]),
    );
    await controller.start();
    expect(controller.getSnapshot().googleEnabled).toBe(false);
    expect(network.google()).toEqual([]);
  });

  it('turning Google off opens the deck again and adds back every other file; on fetches again', async () => {
    const story = storyHost();
    const network = testNetwork();
    const storage = memoryStorage();
    const controller = new FontsController(
      story.host,
      testFontsEnvironment({ fetch: network.fetch, storage }),
      sampleFontNeeds([]),
    );
    await controller.start();
    await controller.addFiles([new File([TTF], 'Brand.ttf')]);
    story.log.length = 0;

    await controller.setGoogle(false);
    expect(storage.values.get(GOOGLE_PREFERENCE)).toBe('off');
    expect(story.log).toEqual(['reopen', 'add bundled', 'add bundled', 'add user', 'finish']);
    expect(story.fonts.map((font) => font.source)).toEqual(['bundled', 'bundled', 'user']);
    const state = controller.getSnapshot();
    expect(state.google).toEqual({ state: 'idle' });
    expect(state.needs.find((need) => need.family === 'Roboto')?.faces).toHaveLength(1);

    story.log.length = 0;
    await controller.setGoogle(true);
    expect(storage.values.has(GOOGLE_PREFERENCE)).toBe(false);
    expect(story.log).toEqual(['add google', 'add google', 'finish']);
  });

  it('adds dropped files: a file the worker refuses is listed with its reason, the others re-finish once', async () => {
    const story = storyHost();
    const controller = new FontsController(story.host, testFontsEnvironment(), sampleFontNeeds([]));
    await controller.addFiles([new File([TTF], 'Brand.ttf'), new File(['nope'], 'Broken.otf')]);
    expect(controller.getSnapshot().files).toEqual([
      { name: 'Brand.ttf', state: 'added' },
      { name: 'Broken.otf', state: 'rejected', reason: 'not a font file' },
    ]);
    expect(story.finishes).toBe(1);
    await controller.addFiles([new File(['x'], 'Nothing.woff')]);
    expect(story.finishes).toBe(1);
  });

  it('takes at most 64 of the user’s files and 256 MB per deck, reading each only when its turn comes', async () => {
    const story = storyHost();
    let reads = 0;
    let readsBeforeFirstAdd: number | null = null;
    const addFont = story.host.addFont;
    story.host.addFont = async (...args) => {
      readsBeforeFirstAdd ??= reads;
      return addFont(...args);
    };
    /** A font file that says it has `size` bytes and holds only a few. */
    const sized = (name: string, size: number) => {
      const file = new File([TTF], name);
      Object.defineProperty(file, 'size', { value: size });
      const read = file.arrayBuffer.bind(file);
      file.arrayBuffer = () => {
        reads += 1;
        return read();
      };
      return file;
    };
    const controller = new FontsController(story.host, testFontsEnvironment(), sampleFontNeeds([]));
    await controller.addFiles(Array.from({ length: MAX_USER_FILES + 6 }, (_, index) => sized(`F${index}.ttf`, 1000)));
    expect(readsBeforeFirstAdd).toBe(1);
    expect(reads).toBe(MAX_USER_FILES);
    const files = controller.getSnapshot().files;
    expect(files.filter((file) => file.state === 'added')).toHaveLength(MAX_USER_FILES);
    expect(files.at(-1)).toEqual({
      name: `F${MAX_USER_FILES + 5}.ttf`,
      state: 'rejected',
      reason: 'over the 64 files a deck takes',
    });

    const bytes = new FontsController(story.host, testFontsEnvironment(), sampleFontNeeds([]));
    const big = 60 << 20;
    await bytes.addFiles([
      sized('A.ttf', big),
      sized('B.ttf', big),
      sized('C.ttf', big),
      sized('D.ttf', big),
      sized('E.ttf', big),
    ]);
    expect(bytes.getSnapshot().files.map((file) => file.state)).toEqual([
      'added',
      'added',
      'added',
      'added',
      'rejected',
    ]);
    expect(bytes.getSnapshot().files[4]).toMatchObject({ reason: 'over the 256 MB of files a deck takes' });
  });

  it('gives back the share of a file the worker refuses', async () => {
    const story = storyHost();
    const controller = new FontsController(story.host, testFontsEnvironment(), sampleFontNeeds([]));
    await controller.addFiles(
      Array.from({ length: MAX_USER_FILES }, (_, index) => new File(['nope'], `N${index}.otf`)),
    );
    await controller.addFiles([new File([TTF], 'Brand.ttf')]);
    expect(controller.getSnapshot().files.at(-1)).toEqual({ name: 'Brand.ttf', state: 'added' });
  });

  it('keeps no copy of a file it added: reopening reads the user’s file again', async () => {
    const story = storyHost();
    const network = testNetwork();
    const controller = new FontsController(
      story.host,
      testFontsEnvironment({ fetch: network.fetch }),
      sampleFontNeeds([]),
    );
    await controller.start();
    const file = new File([TTF], 'Brand.ttf');
    const read = vi.spyOn(file, 'arrayBuffer');
    await controller.addFiles([file]);
    expect(read).toHaveBeenCalledTimes(1);
    await controller.setGoogle(false);
    expect(read).toHaveBeenCalledTimes(2);
    expect(story.fonts.filter((font) => font.source === 'user')).toHaveLength(1);
  });

  it('adds the substitute faces each deck names, also those a re-finished deck names', async () => {
    const network = testNetwork();
    // The substitutes the deck shown names; the first deck draws a generic fallback, which no need names.
    let named: SubstituteFace[] = [
      { family: 'Liberation Sans', postscriptName: 'LiberationSans', weight: 400, style: 'normal' },
    ];
    const added: string[] = [];
    const host: FontsHost = {
      async addFont(_bytes, source) {
        added.push(source);
        if (source === 'bundled') {
          named = [];
        }
        return [];
      },
      selectLocal: async () => [],
      refinish: async () => {},
      reopen: async () => [],
      slides: () => [],
      substitutes: () => named,
    };
    const controller = new FontsController(host, testFontsEnvironment({ fetch: network.fetch }), []);
    await controller.start();
    expect(added).toEqual(['bundled']);
    expect(controller.getSnapshot().substitutes).toEqual({
      state: 'done',
      families: ['Liberation Sans'],
      problems: [],
    });

    named = [{ family: 'Liberation Mono', postscriptName: 'LiberationMono', weight: 400, style: 'normal' }];
    await controller.addFiles([new File([TTF], 'Brand.ttf')]);
    await vi.waitFor(() => expect(added).toEqual(['bundled', 'user', 'bundled']));
    const own = network.requests.map((request) => request.url).filter((url) => url.startsWith('/fonts/core/'));
    expect(own).toEqual([
      '/fonts/core/core.json',
      '/fonts/core/LiberationSans-Regular.woff2',
      '/fonts/core/LiberationMono-Regular.woff2',
    ]);
  });

  it('loads the substitutes without waiting on a Google request that never answers', async () => {
    const story = storyHost();
    const network = testNetwork();
    const urls: string[] = [];
    const fetch: Fetch = (url, init) => {
      urls.push(url);
      return url.startsWith('https://fonts.googleapis.com/')
        ? new Promise<Response>(() => {})
        : network.fetch(url, init);
    };
    const controller = new FontsController(story.host, testFontsEnvironment({ fetch }), sampleFontNeeds([]));
    void controller.start();
    await vi.waitFor(() => expect(controller.getSnapshot().substitutes).toMatchObject({ state: 'done', problems: [] }));
    expect(urls).toContain('/fonts/core/core.json');
    expect(controller.getSnapshot().google.state).toBe('fetching');
    await vi.waitFor(() => expect(story.log).toContain('finish'));
  });

  it('settles once the sources that need no click and their re-finishes are done', async () => {
    const story = storyHost();
    const network = testNetwork();
    let answer: () => void = () => {};
    const google = new Promise<void>((resolve) => (answer = resolve));
    const fetch: Fetch = async (url, init) => {
      if (url.startsWith('https://fonts.googleapis.com/')) {
        await google;
      }
      return network.fetch(url, init);
    };
    const controller = new FontsController(story.host, testFontsEnvironment({ fetch }), sampleFontNeeds([]));
    expect(controller.getSnapshot().settling).toBe(true);
    const started = controller.start();
    await vi.waitFor(() => expect(controller.getSnapshot().substitutes).toMatchObject({ state: 'done' }));
    expect(controller.getSnapshot().settling).toBe(true);
    answer();
    await started;
    expect(controller.getSnapshot()).toMatchObject({ settling: false, refreshing: false });
    expect(story.log.at(-1)).toBe('finish');
  });

  it('reads the installed fonts once per deck: looking again adds nothing new', async () => {
    const story = storyHost();
    const local = fakeLocalFonts([localFont('HelveticaNeue', 'Helvetica Neue'), localFont('Optima-Regular', 'Optima')]);
    const controller = new FontsController(
      story.host,
      testFontsEnvironment({ localFonts: local }),
      sampleFontNeeds([]),
    );
    await controller.useLocalFonts();
    await controller.useLocalFonts();
    await controller.useLocalFonts();
    expect(local.queries).toBe(3);
    expect(story.fonts.filter((font) => font.source === 'local')).toHaveLength(2);
    expect(controller.getSnapshot().local).toEqual({
      state: 'done',
      result: { listed: 2, picked: 2, added: 0, known: 2, skipped: 0, skippedFiles: [] },
    });
    expect(story.finishes).toBe(1);
  });

  it('never asks the browser for installed fonts when the worker cannot match them', async () => {
    const story = storyHost();
    story.host.selectLocal = async () => null;
    const local = fakeLocalFonts([localFont('HelveticaNeue', 'Helvetica Neue')], { state: 'granted' });
    const controller = new FontsController(
      story.host,
      testFontsEnvironment({ localFonts: local }),
      sampleFontNeeds([]),
    );
    await controller.start();
    expect(controller.getSnapshot().localMatching).toBe(false);
    await controller.useLocalFonts();
    expect(local.queries).toBe(0);
    expect(controller.getSnapshot().local).toEqual({ state: 'idle' });
  });

  it('runs one re-finish for changes that arrive while one runs', async () => {
    const story = storyHost();
    let release: () => void = () => {};
    const first = new Promise<void>((resolve) => (release = resolve));
    const refinish = vi.fn(async () => {
      if (refinish.mock.calls.length === 1) {
        await first;
      }
    });
    const controller = new FontsController({ ...story.host, refinish }, testFontsEnvironment(), sampleFontNeeds([]));
    const one = controller.addFiles([new File([TTF], 'a.ttf')]);
    await vi.waitFor(() => expect(refinish).toHaveBeenCalledTimes(1));
    const two = controller.addFiles([new File([TTF], 'b.ttf')]);
    const three = controller.addFiles([new File([TTF], 'c.ttf')]);
    // Both files reach the worker while the first re-finish runs: they share the next one.
    await vi.waitFor(() => expect(story.fonts).toHaveLength(3));
    expect(controller.getSnapshot().refreshing).toBe(true);
    release();
    await Promise.all([one, two, three]);
    expect(refinish).toHaveBeenCalledTimes(2);
    expect(controller.getSnapshot().refreshing).toBe(false);
  });

  it('reports a re-finish that fails', async () => {
    const story = storyHost();
    const controller = new FontsController(
      { ...story.host, refinish: () => Promise.reject(new Error('worker gone')) },
      testFontsEnvironment(),
      sampleFontNeeds([]),
    );
    await controller.addFiles([new File([TTF], 'a.ttf')]);
    expect(controller.getSnapshot()).toMatchObject({ refreshing: false, refreshError: 'worker gone' });
  });

  it('leaves families out of the download and puts them back', () => {
    const controller = new FontsController(storyHost().host, testFontsEnvironment(), []);
    controller.setLeaveOut('Brand Sans', true);
    controller.setLeaveOut('Helvetica Neue', true);
    controller.setLeaveOut('Brand Sans', false);
    expect(controller.getSnapshot().leaveOut).toEqual(['Helvetica Neue']);
  });

  it('changes nothing after dispose', async () => {
    const story = storyHost();
    const controller = new FontsController(story.host, testFontsEnvironment(), sampleFontNeeds([]));
    const listener = vi.fn();
    controller.subscribe(listener);
    controller.dispose();
    await controller.addFiles([new File([TTF], 'a.ttf')]);
    expect(listener).not.toHaveBeenCalled();
    expect(story.log).toEqual([]);
  });
});
