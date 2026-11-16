import { defineConfig, devices } from '@playwright/test';
import { join } from 'node:path';

import { FIREFOX_IN_DOCKER, dockerProjects } from '../../packages/player/e2e/support/browserServer';
import { BROWSER_SERVER_PORT, DEMO_PORT, OUT, APP_PORT as PORT } from './e2e/support/decks';

// The web server serves the production build with `vite preview`. Before it, the player, the wasm module and the
// app are built (each tool rebuilds only what changed); `SLIDOCK_E2E_SKIP_BUILD=1` serves the last build as it is.
// A second server, the dev server, serves the demo page (`demo.html`, the app over a fake worker that tells a font
// story) for the fonts suite.
const build = 'pnpm --filter @slidock/player build && pnpm -w run wasm:build && pnpm run build';
// A server already on a port is an error, unless `SLIDOCK_E2E_REUSE=1` asks to reuse it: it may be left over from
// another run and serve another build. The global setup checks that the app's server serves a build for `/`.
const REUSE = process.env.SLIDOCK_E2E_REUSE === '1';
const serve = `pnpm exec vite preview --port ${PORT} --strictPort`;

export default defineConfig({
  testDir: './e2e',
  outputDir: join(OUT, 'results'),
  globalSetup: './e2e/global-setup.ts',
  timeout: 120_000,
  expect: { timeout: 15_000 },
  fullyParallel: true,
  workers: process.env.CI ? 2 : 3,
  forbidOnly: !!process.env.CI,
  reporter: [['list'], ['json', { outputFile: join(OUT, 'report.json') }]],
  use: {
    baseURL: `http://localhost:${PORT}`,
    acceptDownloads: true,
    trace: 'retain-on-failure',
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
    ...(FIREFOX_IN_DOCKER ? [] : [{ name: 'firefox', use: { ...devices['Desktop Firefox'] } }]),
    { name: 'webkit', use: { ...devices['Desktop Safari'] } },
    // Google Chrome (`SLIDOCK_E2E_CHROME=1`, where it is installed), for what Playwright's Chromium lacks: an HEVC
    // decoder, which the HEIC spec decodes photos with.
    ...(process.env.SLIDOCK_E2E_CHROME === '1'
      ? [{ name: 'chrome', testMatch: /heic\.spec\.ts/, use: { ...devices['Desktop Chrome'], channel: 'chrome' } }]
      : []),
    // `firefox` and `webkit-docker` in the pinned Playwright image (`SLIDOCK_E2E_FIREFOX=docker`,
    // `SLIDOCK_E2E_WEBKIT=docker`; packages/player/e2e/support/browserServer.ts).
    ...dockerProjects(BROWSER_SERVER_PORT),
  ],
  webServer: [
    {
      command: process.env.SLIDOCK_E2E_SKIP_BUILD === '1' ? serve : `${build} && ${serve}`,
      url: `http://localhost:${PORT}`,
      timeout: 15 * 60_000,
      reuseExistingServer: REUSE,
      stdout: 'ignore',
      stderr: 'pipe',
    },
    {
      command: `pnpm exec vite --port ${DEMO_PORT} --strictPort`,
      url: `http://localhost:${DEMO_PORT}/demo.html`,
      timeout: 2 * 60_000,
      reuseExistingServer: REUSE,
      stdout: 'ignore',
      stderr: 'pipe',
    },
  ],
});
