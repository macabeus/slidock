// The bench UI in real browsers: the design rules (packages/ui/src/audit.ts) on every view, over the dev server's mock
// API, in Chromium and macOS WebKit, and in Firefox and WebKit from the pinned Playwright image with
// `SLIDOCK_E2E_FIREFOX=docker` and `SLIDOCK_E2E_WEBKIT=docker` (packages/player/e2e/support/browserServer.ts):
//
//   pnpm --filter @slidock/bench-web exec playwright test --config scripts/e2e/e2e.config.ts
//
// Ports: `SLIDOCK_BENCH_E2E_PORT` (the dev server, default 4320) and `SLIDOCK_BENCH_E2E_BROWSER_PORT` (the browser
// server in Docker, default 3557). A server already on the port is an error unless `SLIDOCK_E2E_REUSE=1` asks to reuse
// it (one left over from another run may serve another tree). Results go to `<cargo target>/e2e/bench-web/`.
import { defineConfig, devices } from '@playwright/test';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { FIREFOX_IN_DOCKER, dockerProjects, envPort } from '../../../../packages/player/e2e/support/browserServer';

export const REPO = fileURLToPath(new URL('../../../..', import.meta.url));
const APP = join(REPO, 'apps/bench-web');
const TARGET = resolve(REPO, process.env.CARGO_TARGET_DIR ?? 'target');
export const PORT = envPort('SLIDOCK_BENCH_E2E_PORT', 4320);
export const BROWSER_SERVER_PORT = envPort('SLIDOCK_BENCH_E2E_BROWSER_PORT', 3557);
const REUSE = process.env.SLIDOCK_E2E_REUSE === '1';

export default defineConfig({
  testDir: '.',
  testMatch: '*.e2e.ts',
  outputDir: join(TARGET, 'e2e', 'bench-web', 'results'),
  globalSetup: './global-setup.ts',
  timeout: 300_000,
  expect: { timeout: 15_000 },
  fullyParallel: true,
  workers: process.env.CI ? 2 : 4,
  forbidOnly: !!process.env.CI,
  reporter: [['list']],
  use: { baseURL: `http://localhost:${PORT}`, trace: 'retain-on-failure' },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
    ...(FIREFOX_IN_DOCKER ? [] : [{ name: 'firefox', use: { ...devices['Desktop Firefox'] } }]),
    { name: 'webkit', use: { ...devices['Desktop Safari'] } },
    ...dockerProjects(BROWSER_SERVER_PORT),
  ],
  webServer: {
    command: `pnpm exec vite --port ${PORT} --strictPort`,
    cwd: APP,
    url: `http://localhost:${PORT}/api/runs`,
    timeout: 2 * 60_000,
    reuseExistingServer: REUSE,
    stdout: 'ignore',
    stderr: 'pipe',
  },
});
