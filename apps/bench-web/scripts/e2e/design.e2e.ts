// The design rules (packages/ui/src/audit.ts) on every bench view (../views.ts), in both themes at phone (375 px) and
// desktop width: text contrast at least WCAG AA, no horizontal page scroll, a visible focus on every control the Tab
// key reaches (at least 3:1), and a name on every control. The dev server's HMR socket is held open
// (`holdDevServerSocket`), so no view reloads under a check.
import { expect, test } from '@playwright/test';

import { FIREFOX_IN_DOCKER } from '../../../../packages/player/e2e/support/browserServer';
import { type AuditFinding, auditPage, holdDevServerSocket, tabKeyFor } from '../../../../packages/ui/src/audit';
import { loadFixture } from '../../mock/fixtures';
import { benchViews, settle } from '../views';

const VIEWPORTS = [
  { width: 375, height: 812 },
  { width: 1280, height: 800 },
] as const;

const summary = (findings: readonly AuditFinding[]) => findings.map((f) => `${f.rule}: ${f.element}: ${f.detail}`);

const views = benchViews(loadFixture().runs);

test.beforeEach(async ({ context }) => {
  await holdDevServerSocket(context);
});

for (const view of views) {
  test(`${view.name} keeps the design rules`, async ({ page, browserName }, info) => {
    const docker = info.project.name === 'webkit-docker' || (info.project.name === 'firefox' && FIREFOX_IN_DOCKER);
    const tabKey = tabKeyFor(browserName, process.platform, docker);
    for (const colorScheme of ['dark', 'light'] as const) {
      for (const viewport of VIEWPORTS) {
        await test.step(`${colorScheme} at ${viewport.width} px`, async () => {
          await page.emulateMedia({ colorScheme });
          await page.setViewportSize(viewport);
          await page.goto('about:blank');
          await page.goto(`/${view.hash}`);
          await settle(page);
          expect.soft(summary(await auditPage(page, { skip: [], tabKey }))).toEqual([]);
        });
      }
    }
  });
}
