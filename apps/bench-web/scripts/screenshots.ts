// Screenshots of every bench view in both themes at desktop and phone width, for review:
// `pnpm screenshots <out dir> [report.html]`. It starts the dev server (the mock API over the schema crate's synthetic
// bench), opens each view in headless Chromium with the system theme emulated dark and light, at 1440 px and 375 px,
// and writes `<view>.<theme>.<width>.png`. With a built report, it also shoots the report opened from file://.
// Nothing is written outside `<out dir>`. It exits 1 on a page error or a page wider than the viewport.
import { chromium } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { createServer } from 'vite';

import { loadFixture } from '../mock/fixtures.ts';
import { ROW, benchViews, settle } from './views.ts';

const [outArg, reportArg] = process.argv.slice(2);
if (!outArg) {
  console.error('usage: node scripts/screenshots.ts <out dir> [report.html]');
  process.exit(2);
}
const out = resolve(outArg);
mkdirSync(out, { recursive: true });

const server = await createServer({ root: resolve(import.meta.dirname, '..'), server: { port: 0 }, logLevel: 'warn' });
await server.listen();
const address = server.httpServer?.address();
const port = typeof address === 'object' && address ? address.port : 0;
const origin = `http://localhost:${port}`;

const shots = benchViews(loadFixture().runs);

const VIEWPORTS = [
  { width: 1440, height: 900 },
  { width: 375, height: 812 },
] as const;

const browser = await chromium.launch();
const written: string[] = [];
try {
  for (const theme of ['dark', 'light'] as const) {
    for (const viewport of VIEWPORTS) {
      const context = await browser.newContext({ viewport, colorScheme: theme });
      const page = await context.newPage();
      const errors: string[] = [];
      page.on('pageerror', (error) => errors.push(error.message));
      page.on('response', (response) => {
        // The API answers 404 for what the bench does not hold (a run without a parity report): expected.
        const expected = response.status() === 404 && new URL(response.url()).pathname.startsWith('/api/');
        if (response.status() >= 400 && !expected) {
          errors.push(`${response.status()} ${response.url()}`);
        }
      });
      const shoot = async (name: string, url: string, drawer: boolean) => {
        // A fresh document per shot: a fragment change alone would keep the previous view's scroll and state.
        await page.goto('about:blank');
        await page.goto(url);
        await settle(page);
        // The page never scrolls sideways (tables scroll inside their frames).
        const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
        if (overflow > 0) {
          errors.push(`${name} at ${viewport.width} px: the page is ${overflow}px wider than the viewport`);
        }
        const file = `${out}/${name}.${theme}.${viewport.width}.png`;
        if (drawer) {
          await page.screenshot({ path: file });
          await page
            .locator('[role="dialog"] > div.overflow-y-auto')
            .evaluate((node) => node.scrollTo(0, node.scrollHeight));
          await page.waitForTimeout(150);
          const end = `${out}/${name}-end.${theme}.${viewport.width}.png`;
          await page.screenshot({ path: end });
          written.push(file, end);
        } else {
          await page.screenshot({ path: file, fullPage: true });
          written.push(file);
        }
      };
      for (const shot of shots) {
        await shoot(shot.name, `${origin}/${shot.hash}`, shot.drawer === true);
      }
      if (reportArg) {
        const url = pathToFileURL(resolve(reportArg)).href;
        for (const [name, hash] of [
          ['report-overview', ''],
          ['report-drawer', `#tab=explorer&row=${ROW}`],
          ['report-parity', '#tab=parity'],
        ] as const) {
          await shoot(name, `${url}${hash}`, hash.includes('row='));
        }
      }
      if (errors.length > 0) {
        console.error(`${theme} at ${viewport.width} px: page errors:\n${errors.join('\n')}`);
        process.exitCode = 1;
      }
      await context.close();
    }
  }
} finally {
  await browser.close();
  await server.close();
}
console.log(`${written.length} screenshots in ${out}`);
