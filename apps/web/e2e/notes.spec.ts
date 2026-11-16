// The notes panel keeps one height whatever the slide's notes hold, so moving between slides never resizes the
// stage (a panel sized by its content made the slide jump on every change).
import { expect, test } from '@playwright/test';

import { smokeCase } from './support/decks';

const smoke = smokeCase();

test('the notes panel and the stage keep their height across slides with different notes', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await page.locator('input[type="file"]').first().setInputFiles(smoke.input);
  await expect(page.getByRole('toolbar', { name: 'Deck' })).toBeVisible({ timeout: 60_000 });
  // The missing-font banner over the stage comes once the font sources settle, which can be after the first
  // measurement: keeping the substitutes first leaves the stage at its final height.
  const fontsPrompt = page.getByRole('alert').filter({ hasText: "isn't in the file" });
  await fontsPrompt.getByRole('button', { name: 'Keep the substitutes' }).click();
  await expect(fontsPrompt).toHaveCount(0);
  await page.getByRole('tab', { name: /^Notes/ }).click();
  const panel = page.locator('#bottom-panel-notes');
  await expect(panel).toContainText('Welcome everyone.');

  const sizes = () =>
    page.evaluate(() => ({
      stage: document.querySelector('[data-stage-well]')!.getBoundingClientRect().height,
      panel: document.querySelector('#bottom-panel-notes')!.getBoundingClientRect().height,
    }));
  const seen = [await sizes()];
  await page.getByRole('button', { name: 'Next slide' }).click();
  await expect(panel).toContainText('The picture is a PNG drawn in code.');
  seen.push(await sizes());
  for (let step = 0; step < 3 && !(await panel.innerText()).includes('no speaker notes'); step++) {
    await page.getByRole('button', { name: 'Next slide' }).click();
  }
  await expect(panel).toContainText('This slide has no speaker notes.');
  seen.push(await sizes());

  expect(new Set(seen.map((size) => size.panel)).size, JSON.stringify(seen)).toBe(1);
  expect(new Set(seen.map((size) => size.stage)).size, JSON.stringify(seen)).toBe(1);
});
