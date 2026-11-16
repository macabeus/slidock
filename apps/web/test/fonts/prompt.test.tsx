import { screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { LocalFontsApi } from '../../src/fonts/localFonts';
import {
  TRUETYPE_HEAD,
  fakeLocalFonts,
  localFont,
  memoryStorage,
  testFontsEnvironment,
} from '../../src/testing/fake-fonts';
import { sampleFontsScript } from '../../src/testing/sample-fonts';
import { openSample, renderApp, sampleScript } from '../harness';
import { testNetwork } from './network';

const INSTALLED = [localFont('HelveticaNeue', 'Helvetica Neue'), localFont('Optima-Regular', 'Optima')];

afterEach(() => {
  vi.restoreAllMocks();
});

function setup(localFonts: LocalFontsApi | null) {
  const network = testNetwork();
  const env = testFontsEnvironment({ fetch: network.fetch, localFonts, storage: memoryStorage() });
  return renderApp(sampleFontsScript(), env);
}

/** The prompt, once the sources that need no click have settled. */
async function prompt(): Promise<HTMLElement> {
  return waitFor(() => {
    const banner = screen
      .getAllByRole('alert')
      .find((alert) => /fonts? (isn't|aren't) in the file/.test(alert.textContent ?? ''));
    if (!banner) {
      throw new Error('no fonts prompt');
    }
    return banner;
  });
}

const title = (banner: HTMLElement) => banner.querySelector('p')?.textContent ?? '';

describe('fonts prompt: asks for the missing fonts once a deck opens', () => {
  it('names the families no source had, after Google settled, and reads nothing before the click', async () => {
    const local = fakeLocalFonts(INSTALLED, { state: 'prompt', answer: 'granted' });
    const app = setup(local);
    await openSample(app);
    const banner = await prompt();
    // Roboto came from Google on its own; the families still missing are named.
    expect(title(banner)).toBe("3 fonts aren't in the file: Brand Sans, Helvetica Neue, Optima");
    expect(within(banner).getByText(/Use the fonts installed on this computer, or add the font files/)).toBeTruthy();
    expect(local.queries).toBe(0);

    await app.user.click(within(banner).getByRole('button', { name: 'Use fonts installed on this computer' }));
    expect(local.queries).toBe(1);
    // Helvetica Neue and Optima arrive from this computer (Optima's licence only keeps it out of the download, as the
    // fonts panel says); Brand Sans is no one's.
    await waitFor(async () => expect(title(await prompt())).toBe("1 font isn't in the file: Brand Sans"));
    const after = await prompt();
    expect(within(after).queryByRole('button', { name: 'Use fonts installed on this computer' })).toBeNull();
    expect(
      within(after).getByText(/The fonts this browser lists for this computer don't include it: add the font files/),
    ).toBeTruthy();

    // The last family arrives from the user's file: nothing is missing, and the prompt goes.
    await app.user.upload(within(after).getByLabelText('Missing font files'), [
      new File([TRUETYPE_HEAD], 'BrandSans.ttf'),
    ]);
    await waitFor(() => expect(screen.queryByText(/fonts? (isn't|aren't) in the file/)).toBeNull());
  });

  it('without Local Font Access: a picked file fills its family, and the prompt names only the rest', async () => {
    const app = setup(null);
    await openSample(app);
    const banner = await prompt();
    expect(within(banner).queryByRole('button', { name: 'Use fonts installed on this computer' })).toBeNull();
    expect(within(banner).getByText(/This browser can't share the fonts installed on this computer/)).toBeTruthy();

    await app.user.upload(within(banner).getByLabelText('Missing font files'), [
      new File([TRUETYPE_HEAD], 'BrandSans.ttf'),
    ]);
    await waitFor(async () => expect(title(await prompt())).toBe("2 fonts aren't in the file: Helvetica Neue, Optima"));
    await waitFor(() =>
      expect(app.player.last.input.deck.fonts.find((font) => font.family === 'Brand Sans')?.source).toBe('user'),
    );
  });

  it('denied before: no button that cannot work, and how to allow it', async () => {
    const local = fakeLocalFonts(INSTALLED, { state: 'denied' });
    const app = setup(local);
    await openSample(app);
    const banner = await prompt();
    expect(within(banner).queryByRole('button', { name: 'Use fonts installed on this computer' })).toBeNull();
    expect(within(banner).getByText(/blocked for this site/)).toBeTruthy();
    expect(local.queries).toBe(0);
  });

  it('keeping the substitutes hides it for this deck, and Details opens the fonts panel', async () => {
    const app = setup(null);
    await openSample(app);
    await app.user.click(within(await prompt()).getByRole('button', { name: 'Details' }));
    expect(screen.getByRole('dialog', { name: 'Fonts' })).toBeTruthy();
    await app.user.keyboard('{Escape}');

    await app.user.click(within(await prompt()).getByRole('button', { name: 'Keep the substitutes' }));
    expect(screen.queryByText(/fonts? (isn't|aren't) in the file/)).toBeNull();
  });

  it('asks nothing when every font is available', async () => {
    const app = renderApp(sampleScript(), testFontsEnvironment());
    await openSample(app);
    // Once the fonts flow settled (the stage stops saying it may swap), nothing is missing and nothing asks.
    await waitFor(() => expect(screen.getByTestId('stage').getAttribute('aria-busy')).toBe('false'));
    expect(screen.queryByText(/fonts? (isn't|aren't) in the file/)).toBeNull();
  });
});
