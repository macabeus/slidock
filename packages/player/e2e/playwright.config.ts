import { defineConfig, devices } from '@playwright/test';
import { fileURLToPath } from 'node:url';

import { BROWSER_SERVER_PORT, dockerProjects } from './support/browserServer';

// The player's effect suite: each test loads the built IIFE (`pnpm --filter @slidock/player e2e` builds it first)
// into a blank page, so it needs no server. `SLIDOCK_E2E_FIREFOX=docker` and `SLIDOCK_E2E_WEBKIT=docker` add the
// `firefox` and `webkit-docker` projects, whose browsers run in the pinned Playwright image (support/browserServer.ts;
// the global setup starts it); without them those projects are left out.
const OUT = fileURLToPath(new URL('../../../target/player-e2e', import.meta.url));

export default defineConfig({
  testDir: '.',
  outputDir: OUT,
  globalSetup: './global-setup.ts',
  timeout: 60_000,
  fullyParallel: true,
  workers: process.env.CI ? 2 : 4,
  forbidOnly: !!process.env.CI,
  reporter: [['list']],
  use: { deviceScaleFactor: 1 },
  projects: [
    // LCD text antialiasing changes when a sibling animates on its own compositing layer; grayscale text keeps a
    // still frame comparable with the static render.
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'], deviceScaleFactor: 1, launchOptions: { args: ['--disable-lcd-text'] } },
    },
    { name: 'webkit', use: { ...devices['Desktop Safari'], deviceScaleFactor: 1 } },
    ...dockerProjects(BROWSER_SERVER_PORT, { deviceScaleFactor: 1 }),
  ],
});
