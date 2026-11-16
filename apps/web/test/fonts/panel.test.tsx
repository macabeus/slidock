import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { FontsController, GOOGLE_PREFERENCE } from '../../src/fonts/controller';
import type { LocalFontData, LocalFontsApi } from '../../src/fonts/localFonts';
import {
  type FakeLocalFonts,
  TRUETYPE_HEAD,
  fakeLocalFonts,
  localFont,
  memoryStorage,
  testFontsEnvironment,
} from '../../src/testing/fake-fonts';
import type { FakeWorkerScript } from '../../src/testing/fake-worker';
import { SAMPLE_IMAGE_PATH, sampleDeck } from '../../src/testing/sample-deck';
import { sampleFontsScript } from '../../src/testing/sample-fonts';
import { FontsPanel } from '../../src/views/FontsPanel';
import { dropFiles, openSample, renderApp } from '../harness';
import { objectUrls } from '../setup';
import { testNetwork } from './network';

const INSTALLED = [
  localFont('Arial-BoldMT', 'Arial'),
  localFont('HelveticaNeue', 'Helvetica Neue'),
  localFont('Optima-Regular', 'Optima'),
];

afterEach(() => {
  vi.restoreAllMocks();
});

function setup(
  options: {
    localFonts?: LocalFontsApi | null;
    storage?: ReturnType<typeof memoryStorage>;
    script?: FakeWorkerScript;
  } = {},
) {
  const network = testNetwork();
  const env = testFontsEnvironment({
    fetch: network.fetch,
    localFonts: options.localFonts ?? null,
    storage: options.storage ?? memoryStorage(),
  });
  const app = renderApp(sampleFontsScript(options.script), env);
  return { app, network };
}

async function openPanel(app: ReturnType<typeof renderApp>) {
  await openSample(app);
  await app.user.click(screen.getByRole('button', { name: /^Fonts/ }));
  return screen.getByRole('dialog', { name: 'Fonts' });
}

/** The face pills of a family's row. */
function faces(drawer: HTMLElement, family: string): string[] {
  const row = within(drawer).getByRole('listitem', { name: family });
  return within(row)
    .getAllByRole('listitem')
    .map((item) => (item.textContent ?? '').replace(/\s*·\s*/g, ' · '));
}

const localSection = (drawer: HTMLElement) =>
  within(drawer).getByRole('heading', { name: 'This computer' }).closest('section') as HTMLElement;

/** Settles the sources that start on their own (Google, the substitutes) and their re-finishes. */
async function settled(drawer: HTMLElement) {
  await waitFor(() => expect(within(drawer).getByText(/draws the faces no source has/)).toBeTruthy());
  await waitFor(() => expect(within(drawer).queryByText('Updating the slides with the new fonts…')).toBeNull());
}

describe('fonts panel: Local Font Access', () => {
  it('unsupported: a hint to drop the files instead, and no button', async () => {
    const { app } = setup({ localFonts: null });
    const drawer = await openPanel(app);
    const section = localSection(drawer);
    expect(section.dataset.lfaState).toBe('unsupported');
    expect(within(section).getByText(/This browser can't share the fonts installed on this computer/)).toBeTruthy();
    expect(within(section).queryByRole('button')).toBeNull();
  });

  it('prompt: nothing is read until the click, then only the picked files, and their faces draw', async () => {
    const local = fakeLocalFonts(INSTALLED, { state: 'prompt', answer: 'granted' });
    const { app } = setup({ localFonts: local });
    const drawer = await openPanel(app);
    await settled(drawer);
    const section = localSection(drawer);
    expect(section.dataset.lfaState).toBe('prompt');
    expect(local.queries).toBe(0);
    expect(faces(drawer, 'Helvetica Neue')).toEqual(['400 · Substitute']);

    await app.user.click(within(section).getByRole('button', { name: 'Use fonts installed on this computer' }));
    await waitFor(() => expect(section.dataset.lfaState).toBe('done'));
    expect(local.queries).toBe(1);
    expect(within(section).getByText('Added 2 font files from this computer.')).toBeTruthy();
    await waitFor(() => expect(faces(drawer, 'Helvetica Neue')).toEqual(['400 · This computer']));
    expect(faces(drawer, 'Optima')).toEqual(['400 · This computer · restricted licence']);
    // The first selectLocal names no font: it asks whether the worker picks installed faces at all.
    const selects = app.worker.sent.flatMap((message) => (message.type === 'selectLocal' ? [message.fonts] : []));
    expect(selects).toHaveLength(2);
    expect(selects[0]).toEqual([]);
    expect(selects[1]?.map((font) => font.postscriptName)).toEqual(['Arial-BoldMT', 'HelveticaNeue', 'Optima-Regular']);
    const localAdds = app.worker.fonts.filter((font) => font.source === 'local');
    expect(localAdds).toHaveLength(2);
  });

  it('names each installed file it does not use, and why', async () => {
    const huge: LocalFontData = {
      ...localFont('Optima-Regular', 'Optima'),
      blob: () =>
        Promise.resolve({ size: 192_120_460, arrayBuffer: () => Promise.resolve(new ArrayBuffer(4)) } as Blob),
    };
    const local = fakeLocalFonts([INSTALLED[1] as LocalFontData, huge], { state: 'granted' });
    const { app } = setup({ localFonts: local });
    const drawer = await openPanel(app);
    const section = localSection(drawer);
    await waitFor(() => expect(section.dataset.lfaState).toBe('done'));
    expect(within(section).getByRole('status').textContent).toBe(
      'Added 1 font file from this computer. 1 file not used:',
    );
    const list = within(section).getByRole('list', { name: 'Installed files not used' });
    expect(list.textContent).toBe(
      'Optima Regular: its file is 183 MB, over the 96 MB left of the 96 MB of installed fonts one presentation may read',
    );
  });

  it('granted: reads the installed fonts on open, with no click', async () => {
    const local = fakeLocalFonts(INSTALLED, { state: 'granted' });
    const { app } = setup({ localFonts: local });
    const drawer = await openPanel(app);
    await waitFor(() => expect(localSection(drawer).dataset.lfaState).toBe('done'));
    expect(local.queries).toBe(1);
    await waitFor(() => expect(faces(drawer, 'Helvetica Neue')).toEqual(['400 · This computer']));
    // Every picked file was used: no list of files not used.
    expect(within(localSection(drawer)).queryByRole('list', { name: 'Installed files not used' })).toBeNull();
  });

  it('denied before: says how to allow it, and never asks', async () => {
    const local = fakeLocalFonts(INSTALLED, { state: 'denied' });
    const { app } = setup({ localFonts: local });
    const drawer = await openPanel(app);
    const section = localSection(drawer);
    await waitFor(() => expect(section.dataset.lfaState).toBe('denied'));
    expect(within(section).getByText(/blocked for this site/)).toBeTruthy();
    expect(within(section).queryByRole('button')).toBeNull();
    expect(local.queries).toBe(0);
  });

  it('denied at the prompt: the same hint, and the deck stays as it was', async () => {
    const local = fakeLocalFonts(INSTALLED, { state: 'prompt', answer: 'denied' });
    const { app } = setup({ localFonts: local });
    const drawer = await openPanel(app);
    await settled(drawer);
    const section = localSection(drawer);
    await app.user.click(within(section).getByRole('button', { name: 'Use fonts installed on this computer' }));
    await waitFor(() => expect(section.dataset.lfaState).toBe('denied'));
    expect(within(section).getByText(/blocked for this site/)).toBeTruthy();
    expect(app.worker.fonts.filter((font) => font.source === 'local')).toEqual([]);
  });

  it('activation: when the browser wants a click even with the permission granted, the button comes back', async () => {
    let first = true;
    const local: FakeLocalFonts = fakeLocalFonts(INSTALLED, { state: 'granted' });
    const query = local.query;
    local.query = () => {
      if (first) {
        first = false;
        return Promise.reject(new DOMException('needs a user gesture', 'SecurityError'));
      }
      return query();
    };
    const { app } = setup({ localFonts: local });
    const drawer = await openPanel(app);
    const section = localSection(drawer);
    await waitFor(() => expect(section.dataset.lfaState).toBe('activation'));
    expect(within(section).getByText(/wants a click/)).toBeTruthy();
    await app.user.click(within(section).getByRole('button', { name: 'Use fonts installed on this computer' }));
    await waitFor(() => expect(section.dataset.lfaState).toBe('done'));
  });

  it('a worker that cannot match installed fonts: a hint to drop the files, no button, and no prompt', async () => {
    const local = fakeLocalFonts(INSTALLED, { state: 'granted' });
    const { app } = setup({ localFonts: local, script: { matchesLocal: false } });
    const drawer = await openPanel(app);
    const section = localSection(drawer);
    await waitFor(() => expect(section.dataset.lfaState).toBe('no-matching'));
    expect(within(section).getByText(/can't pick the fonts installed on this computer/)).toBeTruthy();
    expect(within(section).queryByRole('button')).toBeNull();
    expect(local.queries).toBe(0);
  });

  it('error: shows the browser’s message and a retry', async () => {
    const local = fakeLocalFonts(INSTALLED, { state: 'prompt', failure: () => new Error('font service crashed') });
    const { app } = setup({ localFonts: local });
    const drawer = await openPanel(app);
    await settled(drawer);
    const section = localSection(drawer);
    await app.user.click(within(section).getByRole('button', { name: 'Use fonts installed on this computer' }));
    await waitFor(() => expect(section.dataset.lfaState).toBe('error'));
    expect(within(section).getByText('font service crashed')).toBeTruthy();
    expect(within(section).getByRole('button', { name: 'Try again' })).toBeTruthy();
  });
});

describe('fonts panel: Google Fonts', () => {
  it('is on by default: the catalog family arrives from its chunks, and no request carries the text', async () => {
    const { app, network } = setup();
    const drawer = await openPanel(app);
    await settled(drawer);
    expect(within(drawer).getByRole('switch', { name: /Fetch open fonts from Google Fonts/ })).toHaveProperty(
      'checked',
      true,
    );
    expect(within(drawer).getByText('2 files for Roboto.')).toBeTruthy();
    expect(faces(drawer, 'Roboto')).toEqual(['400 · Google Fonts']);
    const google = network.google();
    expect(google.length).toBe(3);
    for (const { url } of google) {
      expect(new URL(url).searchParams.has('text')).toBe(false);
      expect(url).not.toMatch(/Quarterly|review|Highlights/);
    }
    expect(app.worker.fonts.filter((font) => font.source === 'google').map((font) => font.unicodeRange)).toHaveLength(
      2,
    );
  });

  it('turned off: the deck opens again without its faces and keeps the user’s; the choice is remembered', async () => {
    const storage = memoryStorage();
    const { app, network } = setup({ storage });
    const drawer = await openPanel(app);
    await settled(drawer);
    dropFiles([new File([TRUETYPE_HEAD], 'BrandSans.ttf')]);
    await waitFor(() => expect(faces(drawer, 'Brand Sans')).toEqual(['400 · Your file', '700 · Synthesized from 400']));

    await app.user.click(within(drawer).getByRole('switch', { name: /Fetch open fonts from Google Fonts/ }));
    await waitFor(() => expect(faces(drawer, 'Roboto')).toEqual(['400 · Substitute']));
    expect(faces(drawer, 'Brand Sans')).toEqual(['400 · Your file', '700 · Synthesized from 400']);
    expect(storage.values.get(GOOGLE_PREFERENCE)).toBe('off');
    expect(app.worker.sent.filter((message) => message.type === 'open')).toHaveLength(2);
    expect(app.worker.fonts.map((font) => font.source)).toEqual(['bundled', 'bundled', 'user']);
    const before = network.google().length;

    await app.user.click(within(drawer).getByRole('switch', { name: /Fetch open fonts from Google Fonts/ }));
    await waitFor(() => expect(faces(drawer, 'Roboto')).toEqual(['400 · Google Fonts']));
    expect(network.google().length).toBe(before + 3);
  });

  it('stays off when the viewer turned it off before: nothing is fetched from Google', async () => {
    const storage = memoryStorage();
    storage.setItem(GOOGLE_PREFERENCE, 'off');
    const { app, network } = setup({ storage });
    const drawer = await openPanel(app);
    await settled(drawer);
    expect(within(drawer).getByRole('switch', { name: /Fetch open fonts from Google Fonts/ })).toHaveProperty(
      'checked',
      false,
    );
    expect(network.google()).toEqual([]);
  });
});

describe('fonts panel: dropped and picked files', () => {
  it('a font file dropped anywhere fills the missing family, and the deck swaps in place at the current slide', async () => {
    const { app } = setup();
    await openSample(app);
    act(() => app.player.last.player.goTo(1));
    const players = app.player.players.length;
    dropFiles([new File([TRUETYPE_HEAD], 'BrandSans.ttf')]);
    await waitFor(() => expect(app.worker.fonts.some((font) => font.source === 'user')).toBe(true));
    await waitFor(() =>
      expect(app.player.last.input.deck.fonts.find((font) => font.family === 'Brand Sans')?.source).toBe('user'),
    );
    expect(app.player.players.length).toBeGreaterThan(players);
    expect(app.player.last.options.startAt).toEqual({ slide: 1, step: 1 });
    expect(screen.getByRole('toolbar', { name: 'Deck' }).textContent).toContain('Slide 2 of 3');
    // A dropped font never replaces the open deck.
    expect(screen.queryByRole('dialog', { name: /Open another/ })).toBeNull();
  });

  it('a font arriving while presenting keeps the shown deck’s media URLs alive, and the swap after reuses them', async () => {
    const { app } = setup();
    await openSample(app);
    const url = app.player.last.input.mediaUrl(SAMPLE_IMAGE_PATH);
    await app.user.click(screen.getByRole('button', { name: 'Present' }));
    const presenting = app.player.last;
    const finishes = () => app.worker.sent.filter((message) => message.type === 'finish').length;
    const before = finishes();
    dropFiles([new File([TRUETYPE_HEAD], 'BrandSans.ttf')]);
    await waitFor(() => expect(finishes()).toBeGreaterThan(before));
    await waitFor(() => expect(screen.queryByText('Updating the slides with the new fonts…')).toBeNull());
    // The presentation goes on with the deck it started with; its images still load.
    expect(app.player.last).toBe(presenting);
    expect(presenting.input.mediaUrl(SAMPLE_IMAGE_PATH)).toBe(url);
    expect(objectUrls.has(url)).toBe(true);
    act(() => presenting.player.stop());
    await waitFor(() =>
      expect(app.player.last.input.deck.fonts.find((font) => font.family === 'Brand Sans')?.source).toBe('user'),
    );
    expect(app.player.last.input.mediaUrl(SAMPLE_IMAGE_PATH)).toBe(url);
    expect(objectUrls.has(url)).toBe(true);
  });

  it('picked files are listed with what became of each', async () => {
    const { app } = setup();
    const drawer = await openPanel(app);
    await app.user.upload(within(drawer).getByLabelText('Font files'), [
      new File([TRUETYPE_HEAD], 'BrandSans.ttf'),
      new File(['plain text'], 'Broken.woff2'),
    ]);
    const list = await within(drawer).findByRole('list', { name: 'Added files' });
    await waitFor(() => expect(within(list).getByText('Added')).toBeTruthy());
    expect(within(list).getByText('not a font file')).toBeTruthy();
    await waitFor(() => expect(faces(drawer, 'Brand Sans')).toEqual(['400 · Your file', '700 · Synthesized from 400']));
  });

  it('a family from the user’s files can be left out of the download', async () => {
    const { app } = setup();
    const drawer = await openPanel(app);
    dropFiles([new File([TRUETYPE_HEAD], 'BrandSans.ttf')]);
    const toggle = await within(within(drawer).getByRole('listitem', { name: 'Brand Sans' })).findByRole('checkbox', {
      name: /Include in the download/,
    });
    await app.user.click(toggle);
    await app.user.keyboard('{Escape}');
    await app.user.click(screen.getByRole('button', { name: /Download zip/ }));
    const dialog = screen.getByRole('dialog', { name: 'Download zip' });
    expect(within(dialog).getByRole('checkbox', { name: 'Include Brand Sans' })).toHaveProperty('checked', false);
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    const bundle = app.worker.sent.find((message) => message.type === 'bundle');
    expect(bundle?.type === 'bundle' && bundle.settings.leaveOut).toEqual(['Brand Sans']);
  });

  it('a restricted family draws here, says why the download leaves it out, and the toggle includes it', async () => {
    const local = fakeLocalFonts(INSTALLED, { state: 'granted' });
    const { app } = setup({ localFonts: local });
    const drawer = await openPanel(app);
    await waitFor(() => expect(faces(drawer, 'Optima')).toEqual(['400 · This computer · restricted licence']));
    const row = within(drawer).getByRole('listitem', { name: 'Optima' });
    expect(
      within(row).getByText(
        /Found on this computer, but its licence restricts embedding it: the slides here draw with it, and the download leaves it out/,
      ),
    ).toBeTruthy();
    const toggle = within(row).getByRole('checkbox', { name: /Include in the download/ });
    expect(toggle).toHaveProperty('checked', false);
    expect(within(row).getByText(/include it only if you may share it/)).toBeTruthy();
    await app.user.click(toggle);
    expect(toggle).toHaveProperty('checked', true);
    expect(within(row).getByText(/you included it in the download/)).toBeTruthy();
    await app.user.keyboard('{Escape}');
    await app.user.click(screen.getByRole('button', { name: /Download zip/ }));
    const dialog = screen.getByRole('dialog', { name: 'Download zip' });
    expect(within(dialog).getByRole('checkbox', { name: /Include Optima/ })).toHaveProperty('checked', true);
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    const bundle = app.worker.sent.find((message) => message.type === 'bundle');
    expect(bundle?.type === 'bundle' && bundle.settings).toMatchObject({ leaveOut: [], include: ['Optima'] });
  });

  it('leaving a family out in the panel after a zip was built forgets that zip', async () => {
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    const { app } = setup();
    await openSample(app);
    dropFiles([new File([TRUETYPE_HEAD], 'BrandSans.ttf')]);
    await waitFor(() =>
      expect(app.player.last.input.deck.fonts.find((font) => font.family === 'Brand Sans')?.source).toBe('user'),
    );
    await app.user.click(screen.getByRole('button', { name: /Download zip/ }));
    let dialog = screen.getByRole('dialog', { name: 'Download zip' });
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    await within(dialog).findByRole('button', { name: 'Save again' });
    await app.user.keyboard('{Escape}');

    await app.user.click(screen.getByRole('button', { name: /^Fonts/ }));
    const drawer = screen.getByRole('dialog', { name: 'Fonts' });
    await app.user.click(
      within(within(drawer).getByRole('listitem', { name: 'Brand Sans' })).getByRole('checkbox', {
        name: /Include in the download/,
      }),
    );
    await app.user.keyboard('{Escape}');
    await app.user.click(screen.getByRole('button', { name: /Download zip/ }));
    dialog = screen.getByRole('dialog', { name: 'Download zip' });
    expect(within(dialog).queryByRole('button', { name: 'Save again' })).toBeNull();
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
    await within(dialog).findByRole('button', { name: 'Save again' });
    const leaveOuts = app.worker.sent.flatMap((message) =>
      message.type === 'bundle' ? [message.settings.leaveOut] : [],
    );
    expect(leaveOuts).toEqual([[], ['Brand Sans']]);
    expect(click).toHaveBeenCalledTimes(2);
  });

  it('a zip whose build ends after the families to leave out changed is not saved', async () => {
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    const { app } = setup({ script: { hold: ['bundle'] } });
    await openSample(app);
    dropFiles([new File([TRUETYPE_HEAD], 'BrandSans.ttf')]);
    await waitFor(() =>
      expect(app.player.last.input.deck.fonts.find((font) => font.family === 'Brand Sans')?.source).toBe('user'),
    );
    await app.user.click(screen.getByRole('button', { name: /Download zip/ }));
    await app.user.click(
      within(screen.getByRole('dialog', { name: 'Download zip' })).getByRole('button', { name: 'Build and save' }),
    );
    await app.user.keyboard('{Escape}');
    await app.user.click(screen.getByRole('button', { name: /^Fonts/ }));
    const drawer = screen.getByRole('dialog', { name: 'Fonts' });
    await app.user.click(
      within(within(drawer).getByRole('listitem', { name: 'Brand Sans' })).getByRole('checkbox', {
        name: /Include in the download/,
      }),
    );
    await app.user.keyboard('{Escape}');
    act(() => app.worker.release('bundle'));
    await app.user.click(screen.getByRole('button', { name: /Download zip/ }));
    const dialog = screen.getByRole('dialog', { name: 'Download zip' });
    await within(dialog).findByRole('button', { name: 'Build and save' });
    expect(within(dialog).queryByText(/^Saved /)).toBeNull();
    expect(click).not.toHaveBeenCalled();
  });
});

describe('fonts panel: families', () => {
  it('lists each family with its sources, the demoted embedded face and the substitutes', async () => {
    const { app } = setup();
    const drawer = await openPanel(app);
    await settled(drawer);
    expect(faces(drawer, 'Barlow Condensed')).toEqual([
      '400 · Embedded in the file',
      '700 · Embedded in the file · mislabeled',
    ]);
    expect(within(drawer).getByText(/whose own name says another weight or family/)).toBeTruthy();
    expect(faces(drawer, 'Brand Sans')).toEqual(['400 · Substitute', '700 · Substitute']);
    expect(within(drawer).getByText(/fonts aren't in the file: Brand Sans, Helvetica Neue, Optima/)).toBeTruthy();
    expect(app.worker.fonts.filter((font) => font.source === 'bundled')).toHaveLength(2);
  });
});

describe('fonts panel: leave-out toggles', () => {
  it('each family has its own toggle, whatever letters its name uses', async () => {
    const user = userEvent.setup();
    const base = sampleDeck();
    const family = (name: string) => ({
      family: name,
      weight: 400,
      style: 'normal' as const,
      source: 'local' as const,
      licence: null,
    });
    const deck = {
      ...base.deck,
      fonts: [family('游ゴシック'), family('メイリオ'), family('Brand-Sans'), family('Brand Sans')],
    };
    const fonts = new FontsController(
      {
        addFont: () => Promise.resolve([]),
        selectLocal: () => Promise.resolve([]),
        refinish: () => Promise.resolve(),
        reopen: () => Promise.resolve([]),
        slides: () => [],
        substitutes: () => [],
      },
      testFontsEnvironment(),
      [],
    );
    render(<FontsPanel open onClose={() => {}} deck={deck} faces={[]} needs={[]} fonts={fonts} />);
    const toggles = screen.getAllByRole('checkbox', { name: /Include in the download/ });
    expect(toggles).toHaveLength(4);
    expect(new Set(toggles.map((toggle) => toggle.id)).size).toBe(4);
    const labels = screen.getAllByText('Include in the download');
    await user.click(labels[1] as HTMLElement);
    expect(fonts.getSnapshot().leaveOut).toEqual(['メイリオ']);
    await user.click(labels[3] as HTMLElement);
    expect(fonts.getSnapshot().leaveOut).toEqual(['メイリオ', 'Brand Sans']);
  });
});
