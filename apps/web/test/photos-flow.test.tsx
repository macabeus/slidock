import { act, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { decodedFile, fakePhotoDecoder, gridPlan, mediaNeed } from '../src/testing/fake-photos';
import { pick, renderApp, sampleScript } from './harness';

const A = 'a'.repeat(64);

describe('HEIC photos in the app', () => {
  it('shows the decoding step after reading the slides, then the deck', async () => {
    const plan = gridPlan(2, 1);
    const decoder = fakePhotoDecoder(() => decodedFile(plan));
    const app = renderApp(
      sampleScript({ hold: ['finish'], mediaNeeds: [mediaNeed(A, plan), mediaNeed('b'.repeat(64), plan)] }),
      undefined,
      () => decoder,
    );
    await waitFor(() => expect(screen.getByTestId('wasm-status').textContent).toBe('slidock-wasm 9.9.9'));
    await pick(app.user, new File(['key bytes'], 'Photos.key'));
    const conversion = await screen.findByRole('region', { name: 'Conversion' });
    const step = await within(conversion).findByText(/^Decoding 2 HEIC photos/);
    await waitFor(() =>
      expect(app.worker.sent.map(({ type }) => type)).toEqual(['open', 'addMedia', 'addMedia', 'finish']),
    );
    // Done once both are added; reading the slides is done too.
    const done = (item: HTMLElement) => within(item.closest('li')!).queryByRole('img', { name: 'done' });
    await waitFor(() => expect(done(step)).toBeTruthy());
    expect(step.closest('li')?.getAttribute('aria-current')).toBeNull();
    expect(done(within(conversion).getByText('Reading and lowering the slides'))).toBeTruthy();
    act(() => app.worker.release('finish'));
    await screen.findByRole('toolbar', { name: 'Deck' });
    expect(decoder.decoded).toEqual([A, 'b'.repeat(64)]);
  });

  it('says in the diagnostics what to do about a photo this browser cannot decode', async () => {
    const plan = gridPlan(1, 1);
    const omitted = {
      code: 'MID5601',
      severity: 'decline' as const,
      message: `Media ${A} is image/heic, which the output cannot show`,
      feature: null,
      container: 's1',
      node: null,
    };
    const app = renderApp(sampleScript({ mediaNeeds: [mediaNeed(A, plan)], diagnostics: [omitted] }), undefined, () =>
      fakePhotoDecoder(() => ['no HEVC decoder']),
    );
    await waitFor(() => expect(screen.getByTestId('wasm-status').textContent).toBe('slidock-wasm 9.9.9'));
    await pick(app.user, new File(['key bytes'], 'Photos.key'));
    await screen.findByRole('toolbar', { name: 'Deck' });
    await app.user.click(screen.getByRole('tab', { name: /^Diagnostics/ }));
    expect(
      screen.getByText(
        `Media ${A} is a HEIC photo, which this browser cannot decode: open the deck in Chrome or Safari, or export it to PPTX from Keynote, which converts its photos to JPEG`,
      ),
    ).toBeTruthy();
  });
});
