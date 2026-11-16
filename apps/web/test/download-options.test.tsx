import { screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { notesPlacement } from '../src/views/DownloadDialog';
import { openSample, renderApp, sampleScript } from './harness';

describe('notes placement', () => {
  it.each([
    // presenter, notes, on the page, text version → what ships (the CLI's `--no-presenter`, `--no-notes`, `--notes-on-page`)
    [true, true, false, true, 'presenter'],
    [true, true, true, true, 'page'],
    [true, true, true, false, 'presenter'],
    [true, false, true, true, 'none'],
    [false, true, false, true, 'none'],
    [false, true, true, true, 'page'],
    [false, true, true, false, 'none'],
    [false, false, true, true, 'none'],
  ] as const)(
    'presenter %s, notes %s, on the page %s, text %s → %s',
    (presenter, notes, notesOnPage, textView, out) => {
      expect(notesPlacement({ presenter, notes, notesOnPage, textView })).toBe(out);
    },
  );
});

describe('download: presenter view and speaker notes', () => {
  it('ships both by default, and sends each choice as the CLI says it', async () => {
    const app = renderApp(
      sampleScript({
        zipChunks: [new Uint8Array([80, 75])],
        report: { files: [{ path: 'index.html', bytes: 3 }], bytes: 2, singleFile: false, fontsLeftOut: [] },
      }),
    );
    await openSample(app);
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    await app.user.click(screen.getByRole('button', { name: 'Download zip' }));
    const dialog = screen.getByRole('dialog', { name: 'Download zip' });
    const checkbox = (name: RegExp) => within(dialog).getByRole<HTMLInputElement>('checkbox', { name });
    const build = async () => {
      await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));
      await within(dialog).findByText(/^Saved /);
      const sent = app.worker.sent.at(-1);
      if (sent?.type !== 'bundle') {
        throw new Error('no bundle sent');
      }
      const { presenter, notes, textView } = sent.settings;
      return { presenter, notes, textView };
    };

    expect(checkbox(/^Presenter view/).checked).toBe(true);
    expect(checkbox(/^Speaker notes/).checked).toBe(true);
    expect(checkbox(/Also on the page/).checked).toBe(false);
    expect(within(dialog).getByText(/with its presenter view/)).toBeTruthy();
    expect(await build()).toEqual({ presenter: undefined, notes: undefined, textView: undefined });

    await app.user.click(checkbox(/^Speaker notes/));
    expect(within(dialog).queryByRole('checkbox', { name: /Also on the page/ })).toBeNull();
    expect(within(dialog).getByText('Left out: the zip carries none.')).toBeTruthy();
    expect(await build()).toEqual({ presenter: undefined, notes: 'none', textView: undefined });

    await app.user.click(checkbox(/^Speaker notes/));
    await app.user.click(checkbox(/Also on the page/));
    expect(await build()).toEqual({ presenter: undefined, notes: 'page', textView: undefined });

    // Without the presenter view, the page is the notes' only place: the one checkbox is it.
    await app.user.click(checkbox(/^Presenter view/));
    expect(within(dialog).queryByText(/with its presenter view/)).toBeNull();
    expect(within(dialog).queryByRole('checkbox', { name: /^Speaker notes$|Also on the page/ })).toBeNull();
    const onPage = checkbox(/^Speaker notes on the page/);
    expect(onPage.checked).toBe(true);
    expect(await build()).toEqual({ presenter: false, notes: 'page', textView: undefined });
    await app.user.click(onPage);
    expect(within(dialog).getByText(/unchecked, the zip carries none/)).toBeTruthy();
    expect(await build()).toEqual({ presenter: false, notes: 'none', textView: undefined });

    // The notes on the page are in the text version: without it, the option is off and says why.
    await app.user.click(checkbox(/^Speaker notes on the page/));
    await app.user.click(checkbox(/Text version below the slides/));
    expect(checkbox(/^Speaker notes on the page/).disabled).toBe(true);
    expect(checkbox(/^Speaker notes on the page/).checked).toBe(false);
    expect(within(dialog).getByText('Needs the text version below the slides.')).toBeTruthy();
    expect(await build()).toEqual({ presenter: false, notes: 'none', textView: false });
    await app.user.click(checkbox(/^Presenter view/));
    expect(checkbox(/Also on the page/).disabled).toBe(true);
    expect(await build()).toEqual({ presenter: undefined, notes: undefined, textView: false });
  });
});
