// The GitHub Pages dry run: apps/web built exactly as .github/workflows/pages.yml builds it (`pnpm wasm:build`, then
// `pnpm --filter @slidock/web build` with `VITE_BASE_URL=/slidock/`), served under /slidock/ on a free loopback port
// (or `SLIDOCK_PAGES_PORT`), and the real flow run against it in Chromium (dry-run.pages.ts), with the published bench
// the app links to (bench.pages.ts):
//
//   pnpm --filter @slidock/web e2e:pages
//
// `SLIDOCK_PAGES_SKIP_BUILD=1` serves the last build of apps/web/dist as it is, if it is a build for /slidock/. The
// build stays in apps/web/dist, which the main suite rebuilds for / (and refuses to serve with
// `SLIDOCK_E2E_SKIP_BUILD=1`). The decks are the repository's probe decks; `SLIDOCK_PAGES_DECK=<path>` adds one.
// Results and downloads go to `<cargo target>/e2e/pages/`.
import { defineConfig, devices } from '@playwright/test';
import { join } from 'node:path';

import { OUT } from '../support/decks';

export default defineConfig({
  testDir: '.',
  testMatch: '*.pages.ts',
  outputDir: join(OUT, 'pages', 'results'),
  globalSetup: './global-setup.ts',
  timeout: 300_000,
  expect: { timeout: 30_000 },
  workers: 1,
  reporter: [['list']],
  use: { acceptDownloads: true, trace: 'retain-on-failure' },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
});
