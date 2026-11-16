// Every screen and state of the app (support/screens.ts), shot for a design review (review.config.ts) as
// `<screen>.<theme>.<width>.png`, the project being `<theme>-<width>`.
import { test } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';

import { SCREENS, demoDefaults } from '../support/screens';
import { REVIEW_OUT } from './review.config';

mkdirSync(REVIEW_OUT, { recursive: true });

test.beforeEach(async ({ page }) => demoDefaults(page));

for (const screen of SCREENS) {
  test(screen.name, async ({ page }, info) => {
    const shown = await screen.open(page, '/demo.html');
    const [theme, width] = info.project.name.split('-');
    // A popup (the presenter window) opens at its own size; it is shot at the project's.
    const viewport = page.viewportSize();
    if (shown !== page && viewport) {
      await shown.setViewportSize(viewport);
    }
    // Let transitions of the chrome (150 ms) end.
    await shown.waitForTimeout(300);
    await shown.screenshot({
      path: join(REVIEW_OUT, `${screen.name}.${theme}.${width}.png`),
      fullPage: screen.fullPage,
    });
  });
}
