// The app's compile progress as the suite reads it: the conversion panel's active step, until the deck's toolbar
// replaces the panel.
import { type Page, expect } from '@playwright/test';

/**
 * Waits on the app's own progress until it has compiled the deck it was given: the conversion panel steps through its
 * phases until the deck's toolbar replaces it, within `timeout`. Resolves with the phases it saw.
 *
 * Each read is one call that answers at once: the panel may be gone by the next call, and a call that waits for an
 * element (`textContent`) runs with no timeout inside `expect.poll`, so it would wait for a panel that never returns.
 */
export async function untilCompiled(page: Page, timeout: number): Promise<string[]> {
  const seen: string[] = [];
  await expect
    .poll(
      async () => {
        if (await page.getByRole('toolbar', { name: 'Deck' }).isVisible()) {
          return 'compiled';
        }
        const [phase = ''] = await page.getByLabel('Conversion').locator('li[aria-current="step"]').allTextContents();
        if (phase !== '' && !seen.includes(phase)) {
          seen.push(phase);
        }
        return `converting (${phase || 'no phase shown'})`;
      },
      { message: `the app compiles the deck within ${timeout} ms`, timeout, intervals: [500] },
    )
    .toBe('compiled');
  return seen;
}
