// The published fidelity bench (apps/web/public/bench, written by `slidock-bench report --public`) on the Pages build:
// the landing's link opens it under /slidock/bench/, where it loads under its own CSP with no console error, page
// error or policy violation and no request outside the site, lists rows of the presentations of bench/public.json
// only, and shows a mismatch row's oracle, render and heatmap from the site.
import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { REPO } from '../support/decks';
import { watch } from '../support/player';

const ALLOWED = new Set(
  (JSON.parse(readFileSync(join(REPO, 'bench/public.json'), 'utf8')) as { presentations: string[] }).presentations,
);

function site(): string {
  const url = process.env.SLIDOCK_PAGES_URL;
  if (!url) {
    throw new Error('SLIDOCK_PAGES_URL is unset: run with --config e2e/pages/pages.config.ts');
  }
  return url;
}

test('the landing links to the published bench, which lists the cleared decks only and shows their images', async ({
  page,
  context,
}) => {
  const requests: string[] = [];
  context.on('request', (request) => requests.push(request.url()));
  const problems = watch(page);

  await page.goto(site());
  await page.getByRole('link', { name: 'fidelity benchmark' }).click();
  await expect(page).toHaveURL(`${site()}bench/`);
  const tabs = page.getByRole('tablist', { name: 'Bench views' });
  await expect(tabs.getByRole('tab')).toHaveText(['Overview', 'Explorer', 'Methodology']);
  await expect(page.getByText("A published snapshot of slidock's fidelity bench")).toBeVisible();

  await page.goto(`${site()}bench/#tab=explorer&ref=1`);
  const open = page.getByRole('button', { name: /^Open / });
  await expect(open.first()).toBeVisible();
  const ids = await open.evaluateAll((buttons) =>
    buttons.map((button) => (button.getAttribute('aria-label') ?? '').replace(/^Open /, '')),
  );
  expect(ids.length).toBeGreaterThan(0);
  const decks = new Set(ids.map((id) => id.split(':')[0] ?? ''));
  expect([...decks].filter((deck) => !ALLOWED.has(deck))).toEqual([]);

  await page.goto(`${site()}bench/#tab=explorer&oc=mismatch`);
  await page
    .getByRole('button', { name: /^Open / })
    .first()
    .click();
  const dialog = page.getByRole('dialog');
  for (const name of ['Oracle', 'Render', 'Heatmap']) {
    const image = dialog.getByRole('img', { name, exact: true });
    await expect(image).toBeVisible();
    await expect(image).toHaveAttribute('src', /^images\/[0-9a-f]{12}\/[^/]+\/(oracle|render|heatmap)\.webp$/);
    await expect
      .poll(() => image.evaluate((node: HTMLImageElement) => node.complete && node.naturalWidth))
      .toBeGreaterThan(0);
  }
  await expect(dialog.getByRole('heading', { name: 'Reproduce' })).toHaveCount(0);

  expect(requests.filter((url) => !url.startsWith(site()) && !url.startsWith('data:'))).toEqual([]);
  expect(
    requests.some((url) => url.startsWith(`${site()}bench/images/`) && url.endsWith('.webp')),
    'the images load from the site',
  ).toBe(true);
  expect(problems, 'console errors, page errors and CSP violations').toEqual([]);
});
