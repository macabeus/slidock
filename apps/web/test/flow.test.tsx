import { act, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { SAMPLE_IMAGE_PATH, sampleDeck } from '../src/testing/sample-deck';
import { dropFiles, openSample, pick, renderApp, sampleScript } from './harness';
import { objectUrls } from './setup';

afterEach(() => {
  vi.restoreAllMocks();
});

describe('upload → compile → view', () => {
  it('opens a picked file, shows the phases, then the viewer with rail, stage and diagnostics', async () => {
    const app = renderApp(sampleScript({ hold: ['finish'] }));
    expect(screen.getByRole('button', { name: 'Choose a presentation' })).toBeTruthy();
    expect(screen.getByText(/Your file never leaves this tab/)).toBeTruthy();
    await waitFor(() => expect(screen.getByTestId('wasm-status').textContent).toBe('slidock-wasm 9.9.9'));

    const file = new File(['pptx bytes'], 'Quarterly review.pptx');
    await pick(app.user, file);
    const conversion = await screen.findByRole('region', { name: 'Conversion' });
    expect(within(conversion).getByText('Quarterly review.pptx')).toBeTruthy();
    expect(within(conversion).getByRole('progressbar', { name: 'Converting Quarterly review.pptx' })).toBeTruthy();
    await waitFor(() =>
      expect(
        within(conversion).getByText('Fonts, text metrics and layout').closest('li')?.getAttribute('aria-current'),
      ).toBe('step'),
    );
    expect(within(conversion).getByText(/1 error · 1 warning/)).toBeTruthy();
    expect(app.worker.sent.map((message) => message.type)).toEqual(['open', 'finish']);
    expect(app.worker.sent[0]).toEqual({ type: 'open', id: 1, file });

    act(() => app.worker.release('finish'));
    const toolbar = await screen.findByRole('toolbar', { name: 'Deck' });
    expect(within(toolbar).getByText('Slide 1 of 3')).toBeTruthy();
    expect(document.title).toBe('Quarterly review · slidock');

    const player = app.player.last;
    expect(player.options.mode).toBe('view');
    expect(player.host).toBe(screen.getByTestId('stage'));
    expect(player.input.slides.map((slide) => slide.id)).toEqual(['s1', 's2', 's3', 's4']);
    await waitFor(() => expect([...player.rendered].sort()).toEqual(['s1', 's2', 's3', 's4']));

    const rail = screen.getByRole('navigation', { name: 'Slides' });
    const thumbs = within(rail).getAllByRole('button');
    expect(thumbs.map((thumb) => thumb.getAttribute('aria-label'))).toEqual([
      'Slide 1: Quarterly review',
      'Slide 2: Highlights',
      'Hidden slide: Backup numbers',
      'Slide 3: Thank you',
    ]);
    expect(thumbs[0]?.getAttribute('aria-current')).toBe('true');

    await app.user.click(screen.getByRole('tab', { name: /^Diagnostics\s*2$/ }));
    expect(screen.getByText('PX3101')).toBeTruthy();
    expect(screen.getByText('A SmartArt graphic shows as its fallback picture')).toBeTruthy();
    await app.user.click(screen.getByRole('button', { name: 'Go to slide 2' }));
    expect(player.calls).toContain('goTo:1');
  });

  it('navigates with the toolbar, the rail and the keyboard, and keeps the position in the fragment', async () => {
    const app = renderApp(sampleScript());
    await openSample(app);
    const player = app.player.last;

    await app.user.click(screen.getByRole('button', { name: 'Next slide' }));
    expect(screen.getByText('Slide 2 of 3')).toBeTruthy();
    expect(screen.getByText(/step 1 \/ 1/)).toBeTruthy();
    expect(location.hash).toBe('#slide=2');

    await app.user.click(screen.getByRole('button', { name: 'Slide 3: Thank you' }));
    expect(player.calls.at(-1)).toBe('goTo:3');
    expect(location.hash).toBe('#slide=4');

    await app.user.click(screen.getByRole('button', { name: 'Hidden slide: Backup numbers' }));
    expect(screen.getByText('Hidden slide')).toBeTruthy();

    act(() => (document.activeElement as HTMLElement | null)?.blur());
    await app.user.keyboard('{ArrowLeft}');
    expect(player.calls.at(-1)).toBe('previous');
    await app.user.keyboard('{Home}');
    expect(player.calls.at(-1)).toBe('first');
    await app.user.keyboard('{End}');
    expect(player.calls.at(-1)).toBe('last');

    await app.user.click(screen.getByRole('tab', { name: 'Notes' }));
    await app.user.click(screen.getByRole('button', { name: 'Slide 1: Quarterly review' }));
    expect(screen.getByText('Welcome everyone.')).toBeTruthy();
  });

  it('starts at the slide the fragment names', async () => {
    window.history.replaceState(null, '', '/#slide=4');
    const app = renderApp(sampleScript());
    await openSample(app);
    expect(app.player.last.options.startAt).toEqual({ slide: 3 });
    expect(screen.getByText('Slide 3 of 3')).toBeTruthy();
  });

  it('keeps a fragment step within the slide, and the counter shows where the player is', async () => {
    window.history.replaceState(null, '', '/#slide=2&step=99');
    const app = renderApp(sampleScript());
    await openSample(app);
    expect(app.player.last.options.startAt).toEqual({ slide: 1, step: 1 });

    app.unmount();
    // The fake player starts every slide fully built, whatever step it is asked for.
    window.history.replaceState(null, '', '/#slide=2&step=0');
    const again = renderApp(sampleScript());
    await openSample(again);
    expect(again.player.last.options.startAt).toEqual({ slide: 1, step: 0 });
    expect(again.player.last.player.position.step).toBe(1);
    expect(screen.getByText(/step 1 \/ 1/)).toBeTruthy();
  });

  it('only the first deck of a page load starts where the fragment says', async () => {
    window.history.replaceState(null, '', '/#slide=2');
    const app = renderApp(sampleScript());
    await openSample(app);
    expect(app.player.last.options.startAt).toEqual({ slide: 1 });
    await app.user.keyboard('{End}');
    expect(location.hash).toBe('#slide=4');

    act(() => dropFiles([new File(['y'], 'Other.odp')]));
    await app.user.click(screen.getByRole('button', { name: 'Open the new one' }));
    await waitFor(() => expect(app.player.players).toHaveLength(2));
    expect(app.player.last.options.startAt).toEqual({ slide: 0 });
    expect(screen.getByText('Slide 1 of 3')).toBeTruthy();
  });

  it('a failed open followed by another file starts that file at its beginning', async () => {
    window.history.replaceState(null, '', '/#slide=4');
    const app = renderApp(sampleScript());
    await waitFor(() => expect(screen.getByTestId('wasm-status').textContent).toBe('slidock-wasm 9.9.9'));
    await pick(app.user, new File(['%PDF'], 'slides.pdf'));
    await app.user.click(within(await screen.findByRole('alert')).getByRole('button', { name: 'Dismiss' }));
    expect(location.hash).toBe('');
    await pick(app.user, new File(['pptx bytes'], 'Quarterly review.pptx'));
    await screen.findByRole('toolbar', { name: 'Deck' });
    expect(app.player.last.options.startAt).toEqual({ slide: 0 });
  });

  it('merges the diagnostics the player reports into the diagnostics panel and its report', async () => {
    const app = renderApp(sampleScript());
    await openSample(app);
    const writeText = vi.fn((_text: string) => Promise.resolve());
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    act(() =>
      app.player.last.options.onEvent?.({
        type: 'diagnostic',
        diagnostic: {
          code: 'RT0001',
          severity: 'warning',
          message: 'the slide showed before its media was ready',
          feature: null,
          container: 's2',
          node: null,
        },
      }),
    );
    await app.user.click(screen.getByRole('tab', { name: /^Diagnostics\s*3$/ }));
    expect(screen.getByText('RT0001')).toBeTruthy();
    expect(screen.getByText('the slide showed before its media was ready')).toBeTruthy();
    await app.user.click(screen.getByRole('button', { name: 'Copy report' }));
    const report = JSON.parse(writeText.mock.calls[0]?.[0] ?? '[]') as { code: string }[];
    expect(report.map((diagnostic) => diagnostic.code)).toContain('RT0001');
  });

  it('presents through the player and marks the fragment', async () => {
    const app = renderApp(sampleScript());
    await openSample(app);
    await app.user.click(screen.getByRole('button', { name: 'Present' }));
    expect(app.player.last.calls).toContain('present');
    expect(location.hash).toBe('#slide=1&present=1');
    act(() => app.player.last.player.stop());
    expect(location.hash).toBe('#slide=1');
  });

  it('serves the deck to the presenter window it opens', async () => {
    const app = renderApp(sampleScript());
    await openSample(app);
    const popup = { postMessage: vi.fn(), focus: vi.fn() };
    const open = vi.spyOn(window, 'open').mockReturnValue(popup as unknown as Window);
    await app.user.click(screen.getByRole('button', { name: 'Presenter view' }));
    expect(open).toHaveBeenCalledTimes(1);
    expect(app.player.last.calls).toContain('openPresenter');
    expect(popup.focus).toHaveBeenCalled();
    // Attaching sends the deck at once; the window's own `ready` (after it loads) gets it again.
    expect(popup.postMessage).toHaveBeenCalledTimes(1);
    popup.postMessage.mockClear();

    const ready = new MessageEvent('message', { data: { type: 'slidock:presenter-ready' }, origin: location.origin });
    Object.defineProperty(ready, 'source', { value: popup });
    act(() => {
      window.dispatchEvent(ready);
    });
    expect(popup.postMessage).toHaveBeenCalledTimes(1);
    const [payload, origin] = popup.postMessage.mock.calls[0] as [Record<string, unknown>, string];
    expect(origin).toBe(location.origin);
    expect(payload.type).toBe('slidock:presenter-deck');
    expect(payload.position).toEqual({ slide: 0, step: 0 });
    expect((payload.media as { path: string }[]).map((medium) => medium.path)).toEqual([SAMPLE_IMAGE_PATH]);

    const stranger = new MessageEvent('message', {
      data: { type: 'slidock:presenter-ready' },
      origin: location.origin,
    });
    Object.defineProperty(stranger, 'source', { value: {} });
    act(() => {
      window.dispatchEvent(stranger);
    });
    expect(popup.postMessage).toHaveBeenCalledTimes(1);
  });

  it('serves the deck to a presenter window the player opens by itself', async () => {
    const app = renderApp(sampleScript());
    await openSample(app);
    const popup = { postMessage: vi.fn(), focus: vi.fn(), closed: false };
    act(() => app.player.last.options.onEvent?.({ type: 'presenter', window: popup as unknown as Window }));
    expect(popup.focus).toHaveBeenCalled();
    expect(popup.postMessage.mock.calls.map(([message]) => (message as { type: string }).type)).toEqual([
      'slidock:presenter-deck',
    ]);
    // A blocked window is no presenter window.
    act(() => app.player.last.options.onEvent?.({ type: 'presenter', window: null }));
    expect(popup.postMessage).toHaveBeenCalledTimes(1);
  });

  it("keeps the presenter window on the deck window's deck as decks are closed and opened", async () => {
    const app = renderApp(sampleScript());
    await openSample(app);
    const popup = { postMessage: vi.fn(), focus: vi.fn(), closed: false };
    vi.spyOn(window, 'open').mockReturnValue(popup as unknown as Window);
    await app.user.click(screen.getByRole('button', { name: 'Presenter view' }));
    popup.postMessage.mockClear();
    const ready = new MessageEvent('message', { data: { type: 'slidock:presenter-ready' }, origin: location.origin });
    Object.defineProperty(ready, 'source', { value: popup });
    act(() => {
      window.dispatchEvent(ready);
    });
    const sent = () => popup.postMessage.mock.calls.map(([message]) => (message as { type: string }).type);
    expect(sent()).toEqual(['slidock:presenter-deck']);

    await app.user.click(screen.getByRole('button', { name: 'Open another' }));
    expect(sent()).toEqual(['slidock:presenter-deck', 'slidock:presenter-closed']);
    await pick(app.user, new File(['odp bytes'], 'Second.odp'));
    await screen.findByRole('toolbar', { name: 'Deck' });
    expect(sent()).toEqual(['slidock:presenter-deck', 'slidock:presenter-closed', 'slidock:presenter-deck']);

    act(() => dropFiles([new File(['y'], 'Third.pptx')]));
    await app.user.click(screen.getByRole('button', { name: 'Open the new one' }));
    await waitFor(() => expect(app.player.players).toHaveLength(3));
    expect(sent().slice(3)).toEqual(['slidock:presenter-closed', 'slidock:presenter-deck']);
    const last = popup.postMessage.mock.calls.at(-1)?.[0] as { media: { path: string }[] };
    expect(last.media.map((medium) => medium.path)).toEqual([SAMPLE_IMAGE_PATH]);

    popup.closed = true;
    await app.user.click(screen.getByRole('button', { name: 'Open another' }));
    expect(popup.postMessage).toHaveBeenCalledTimes(5);
  });

  it('tells a previous presenter window the deck closed when the player opens another', async () => {
    const app = renderApp(sampleScript());
    await openSample(app);
    const first = { postMessage: vi.fn(), focus: vi.fn(), closed: false };
    const second = { postMessage: vi.fn(), focus: vi.fn(), closed: false };
    const open = vi.spyOn(window, 'open').mockReturnValue(first as unknown as Window);
    await app.user.click(screen.getByRole('button', { name: 'Presenter view' }));
    open.mockReturnValue(second as unknown as Window);
    await app.user.click(screen.getByRole('button', { name: 'Presenter view' }));
    const types = (popup: typeof first) =>
      popup.postMessage.mock.calls.map(([message]) => (message as { type: string }).type);
    expect(types(first)).toEqual(['slidock:presenter-deck', 'slidock:presenter-closed']);
    expect(types(second)).toEqual(['slidock:presenter-deck']);
    open.mockReturnValue(second as unknown as Window);
    await app.user.click(screen.getByRole('button', { name: 'Presenter view' }));
    expect(types(second)).toEqual(['slidock:presenter-deck', 'slidock:presenter-deck']);
  });

  it('serves media as blob: URLs and revokes them when the deck closes', async () => {
    const app = renderApp(sampleScript());
    await openSample(app);
    const url = app.player.last.input.mediaUrl(SAMPLE_IMAGE_PATH);
    expect(url).toMatch(/^blob:/);
    expect(objectUrls.has(url)).toBe(true);
    await app.user.click(screen.getByRole('button', { name: 'Open another' }));
    expect(objectUrls.has(url)).toBe(false);
    expect(app.player.last.destroyed).toBe(true);
    expect(screen.getByRole('button', { name: 'Choose a presentation' })).toBeTruthy();
  });

  it('revokes media URLs when the app unmounts', async () => {
    const app = renderApp(sampleScript());
    await openSample(app);
    const url = app.player.last.input.mediaUrl(SAMPLE_IMAGE_PATH);
    app.unmount();
    expect(objectUrls.has(url)).toBe(false);
    expect(app.worker.terminated).toBe(true);
  });
});

describe('download', () => {
  it('bundles in the worker, saves the zip and lists what it holds and the fonts left out', async () => {
    const app = renderApp(
      sampleScript({
        zipChunks: [new Uint8Array([80, 75, 3, 4]), new Uint8Array([1, 2])],
        report: {
          files: [
            { path: 'index.html', bytes: 1200 },
            { path: 'media/x.svg', bytes: 300 },
            { path: 'LICENSES.txt', bytes: 90 },
          ],
          bytes: 6,
          singleFile: false,
          fontsLeftOut: ['Gotham'],
        },
      }),
    );
    await openSample(app);
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      saved.push({ name: this.download, href: this.href });
    });
    const saved: { name: string; href: string }[] = [];

    await app.user.click(screen.getByRole('button', { name: 'Download zip' }));
    const dialog = screen.getByRole('dialog', { name: 'Download zip' });
    expect(within(dialog).getByText('index.html')).toBeTruthy();
    expect(within(dialog).getByText(/A font whose licence forbids it is left out/)).toBeTruthy();
    await app.user.click(within(dialog).getByRole('checkbox', { name: /Single HTML file/ }));
    await app.user.click(within(dialog).getByRole('button', { name: 'Build and save' }));

    await within(dialog).findByText('Saved Quarterly review.slidock.zip');
    // After open, finish and the dialog's alternatePlan.
    expect(app.worker.sent.at(-1)).toEqual({
      type: 'bundle',
      id: 4,
      settings: { singleFile: true, singleFileThreshold: null, leaveOut: [], include: [] },
    });
    expect(click).toHaveBeenCalledTimes(1);
    expect(saved[0]?.name).toBe('Quarterly review.slidock.zip');
    const blob = [...objectUrls.entries()].find(([url]) => url === saved[0]?.href)?.[1] as Blob | undefined;
    if (blob) {
      expect([...new Uint8Array(await blob.arrayBuffer())]).toEqual([80, 75, 3, 4, 1, 2]);
    }
    expect(within(dialog).getByText('Gotham')).toBeTruthy();
    expect(within(dialog).getAllByText('LICENSES.txt')).toHaveLength(2);

    await app.user.click(within(dialog).getByRole('button', { name: 'Save again' }));
    expect(click).toHaveBeenCalledTimes(2);
  });

  it("prefills the page's title and description, checks its public URL and sends only what was changed", async () => {
    const app = renderApp(
      sampleScript({
        deck: { ...sampleDeck(), page: { title: 'Opening', description: 'Agenda · Numbers' } },
        zipChunks: [new Uint8Array([80, 75])],
        report: {
          files: [{ path: 'index.html', bytes: 3 * 1024 * 1024 }],
          bytes: 2,
          singleFile: false,
          fontsLeftOut: [],
        },
      }),
    );
    await openSample(app);
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    await app.user.click(screen.getByRole('button', { name: 'Download zip' }));
    const dialog = screen.getByRole('dialog', { name: 'Download zip' });
    const title = within(dialog).getByRole('textbox', { name: 'Title' });
    expect((title as HTMLInputElement).value).toBe('Opening');
    const description = within(dialog).getByRole('textbox', { name: /Description/ });
    expect((description as HTMLTextAreaElement).value).toBe('Agenda · Numbers');
    const build = within(dialog).getByRole('button', { name: 'Build and save' });

    const url = within(dialog).getByRole('textbox', { name: /Public URL/ });
    await app.user.type(url, 'ftp://example.com/talk/');
    expect(within(dialog).getByRole('alert').textContent).toMatch(/absolute http/);
    expect((build as HTMLButtonElement).disabled).toBe(true);
    await app.user.clear(url);
    await app.user.type(url, 'https://example.com/talk/');
    expect(within(dialog).queryByRole('alert')).toBeNull();

    await app.user.clear(description);
    await app.user.click(within(dialog).getByRole('checkbox', { name: /Also on the page/ }));
    await app.user.click(within(dialog).getByRole('checkbox', { name: /Text version below the slides/ }));
    await app.user.click(build);
    await within(dialog).findByText(/Saved/);
    // After open, finish and the dialog's alternatePlan.
    expect(app.worker.sent.at(-1)).toEqual({
      type: 'bundle',
      id: 4,
      settings: {
        singleFile: false,
        singleFileThreshold: null,
        leaveOut: [],
        include: [],
        description: '',
        siteUrl: 'https://example.com/talk/',
        textView: false,
      },
    });
    expect(within(dialog).getByText(/index.html is 3.0 MiB. Google reads the first 2 MiB/)).toBeTruthy();
  });

  it('drops a bundle that finishes after its deck was replaced', async () => {
    const app = renderApp(sampleScript({ hold: ['bundle'], zipChunks: [new Uint8Array([80, 75])] }));
    await openSample(app);
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    await app.user.click(screen.getByRole('button', { name: 'Download zip' }));
    await app.user.click(screen.getByRole('button', { name: 'Build and save' }));
    await waitFor(() => expect(app.worker.sent.at(-1)?.type).toBe('bundle'));
    await app.user.keyboard('{Escape}');

    act(() => dropFiles([new File(['y'], 'Other.odp')]));
    await app.user.click(screen.getByRole('button', { name: 'Open the new one' }));
    await waitFor(() => expect(app.player.players).toHaveLength(2));
    act(() => app.worker.release('bundle'));
    await new Promise((resolve) => setTimeout(resolve, 10));

    await app.user.click(screen.getByRole('button', { name: 'Download zip' }));
    const dialog = screen.getByRole('dialog', { name: 'Download zip' });
    expect(within(dialog).getByRole('button', { name: 'Build and save' })).toBeTruthy();
    expect(click).not.toHaveBeenCalled();
  });

  it('says when every font ships, and shows a bundle failure', async () => {
    const app = renderApp(
      sampleScript({ errors: { bundle: { kind: 'bundle', message: 'disk full', diagnostics: [] } } }),
    );
    await openSample(app);
    await app.user.click(screen.getByRole('button', { name: 'Download zip' }));
    await app.user.click(screen.getByRole('button', { name: 'Build and save' }));
    expect(await screen.findByText("The zip couldn't be built")).toBeTruthy();
    expect(screen.getByText('bundle: disk full')).toBeTruthy();
  });
});

describe('errors', () => {
  it('refuses an unsupported file without asking the worker', async () => {
    const app = renderApp(sampleScript());
    await waitFor(() => expect(screen.getByTestId('wasm-status').textContent).toBe('slidock-wasm 9.9.9'));
    await pick(app.user, new File(['%PDF'], 'slides.pdf'));
    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain("Can't open “slides.pdf”");
    expect(alert.textContent).toContain("PDF isn't supported yet");
    expect(app.worker.sent).toEqual([]);
    await app.user.click(within(alert).getByRole('button', { name: 'Dismiss' }));
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('shows a refusal with its diagnostic codes', async () => {
    const app = renderApp(
      sampleScript({
        errors: {
          open: {
            kind: 'refused',
            message: 'KN1002: password-protected',
            diagnostics: [
              {
                code: 'KN1002',
                severity: 'error',
                message: 'The presentation is password-protected',
                feature: null,
                container: null,
                node: null,
              },
            ],
          },
        },
      }),
    );
    await waitFor(() => expect(screen.getByTestId('wasm-status').textContent).toBe('slidock-wasm 9.9.9'));
    await pick(app.user, new File(['x'], 'Locked.key'));
    const alert = await screen.findByRole('alert');
    expect(within(alert).getByText("slidock couldn't read this presentation")).toBeTruthy();
    expect(within(alert).getByText('KN1002')).toBeTruthy();
    expect(within(alert).getByText('The presentation is password-protected')).toBeTruthy();
    expect(within(alert).getByText('Locked.key')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Choose a presentation' })).toBeTruthy();
  });

  it('reports a compiler that fails to load, and starts a new worker on retry', async () => {
    const app = renderApp((index) =>
      index === 0 ? { loadError: 'WebAssembly.instantiate(): bad magic' } : sampleScript(),
    );
    const alert = await screen.findByRole('alert');
    expect(within(alert).getByText("The compiler couldn't load")).toBeTruthy();
    expect(within(alert).getByText('WebAssembly.instantiate(): bad magic')).toBeTruthy();
    await app.user.click(within(alert).getByRole('button', { name: 'Try again' }));
    expect(app.workers).toHaveLength(2);
    expect(app.workers[0]?.terminated).toBe(true);
    await waitFor(() => expect(screen.getByTestId('wasm-status').textContent).toBe('slidock-wasm 9.9.9'));
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('a file picked while the compiler is down shows the compiler error', async () => {
    const app = renderApp({ loadError: 'no wasm' });
    await screen.findByRole('alert');
    await pick(app.user, new File(['x'], 'Deck.odp'));
    expect(await screen.findByText("The compiler couldn't load")).toBeTruthy();
  });
});

describe('cancel and replace', () => {
  it('cancel stops the compile worker and starts a fresh one', async () => {
    const app = renderApp(sampleScript({ hold: ['open'] }));
    await waitFor(() => expect(screen.getByTestId('wasm-status').textContent).toBe('slidock-wasm 9.9.9'));
    await pick(app.user, new File(['x'], 'Deck.pptx'));
    await app.user.click(await screen.findByRole('button', { name: 'Cancel' }));
    expect(app.workers[0]?.terminated).toBe(true);
    expect(app.workers).toHaveLength(2);
    expect(screen.getByRole('button', { name: 'Choose a presentation' })).toBeTruthy();
  });

  it('a drop during a compile replaces it', async () => {
    const app = renderApp((index) => (index === 0 ? sampleScript({ hold: ['open'] }) : sampleScript()));
    await waitFor(() => expect(screen.getByTestId('wasm-status').textContent).toBe('slidock-wasm 9.9.9'));
    await pick(app.user, new File(['x'], 'First.pptx'));
    await screen.findByRole('region', { name: 'Conversion' });
    act(() => dropFiles([new File(['y'], 'Second.odp')]));
    await screen.findByRole('toolbar', { name: 'Deck' });
    expect(app.workers[0]?.terminated).toBe(true);
    expect(app.workers[1]?.sent[0]).toMatchObject({ type: 'open' });
  });

  it('a drop while viewing asks before replacing the deck', async () => {
    const app = renderApp(sampleScript());
    await openSample(app);
    act(() => dropFiles([new File(['y'], 'Other.odp')]));
    const dialog = screen.getByRole('dialog', { name: 'Replace the open deck?' });
    expect(within(dialog).getByText(/“Other.odp” closes “Quarterly review”/)).toBeTruthy();
    await app.user.click(within(dialog).getByRole('button', { name: 'Keep the current deck' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(app.worker.sent.filter((message) => message.type === 'open')).toHaveLength(1);

    act(() => dropFiles([new File(['y'], 'Other.odp')]));
    await app.user.click(screen.getByRole('button', { name: 'Open the new one' }));
    await waitFor(() => expect(app.worker.sent.filter((message) => message.type === 'open')).toHaveLength(2));
  });
});

describe('panels', () => {
  it('opens the fonts panel with each family, its faces and their sources', async () => {
    const app = renderApp(
      sampleScript({
        needs: [
          {
            family: 'Gotham',
            key: 'gotham',
            faces: [{ weight: 400, style: 'normal' }],
            postscriptNames: [],
            embedded: false,
          },
        ],
      }),
    );
    await openSample(app);
    await app.user.click(screen.getByRole('button', { name: /^Fonts/ }));
    const drawer = screen.getByRole('dialog', { name: 'Fonts' });
    expect(within(drawer).getByText("1 font isn't in the file: Gotham")).toBeTruthy();
    expect(within(drawer).getByText('Liberation Sans')).toBeTruthy();
    expect(within(drawer).getAllByText('· Substitute')).toHaveLength(2);
    expect(within(within(drawer).getByRole('listitem', { name: 'Gotham' })).getByText('· Missing')).toBeTruthy();
    await app.user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('? opens the keyboard help, and Escape closes it', async () => {
    const app = renderApp(sampleScript());
    await openSample(app);
    act(() => (document.activeElement as HTMLElement | null)?.blur());
    await app.user.keyboard('?');
    expect(screen.getByRole('dialog', { name: 'Keyboard shortcuts' })).toBeTruthy();
    await app.user.keyboard('{ArrowRight}');
    expect(app.player.last.calls).not.toContain('next');
    await app.user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).toBeNull();
    await app.user.keyboard('{ArrowRight}');
    expect(app.player.last.calls).toContain('next');
  });

  it('the theme toggle switches the document theme', async () => {
    const app = renderApp(sampleScript());
    await app.user.click(screen.getByRole('button', { name: /^System theme/ }));
    expect(document.documentElement.getAttribute('data-theme')).toBe('light');
  });
});
