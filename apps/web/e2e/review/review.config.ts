// Screenshots of every screen and state of the app for a design review, in both themes at desktop (1440 px) and phone
// (375 px) width, in Chromium:
//
//   pnpm --filter @slidock/web exec playwright test --config e2e/review/review.config.ts
//
// They land in `SLIDOCK_REVIEW_OUT` (default `<repo>/target-review/design/web`) as `<screen>.<theme>.<width>.png`. The
// app is the demo page on the dev server (`src/demo.tsx`: a fake worker scripted per screen, the real player), on
// `SLIDOCK_REVIEW_PORT` (default 4319).
import { defineConfig, devices } from '@playwright/test';
import { join } from 'node:path';

import { envPort } from '../../../../packages/player/e2e/support/browserServer';
import { APP, OUT, REPO } from '../support/decks';

export const REVIEW_PORT = envPort('SLIDOCK_REVIEW_PORT', 4319);
export const REVIEW_OUT = process.env.SLIDOCK_REVIEW_OUT ?? join(REPO, 'target-review', 'design', 'web');

const project = (theme: 'dark' | 'light', width: number, height: number) => ({
  name: `${theme}-${width}`,
  use: { ...devices['Desktop Chrome'], viewport: { width, height }, colorScheme: theme },
});

export default defineConfig({
  testDir: '.',
  testMatch: '*.review.ts',
  outputDir: join(OUT, 'review-results'),
  timeout: 60_000,
  expect: { timeout: 15_000 },
  fullyParallel: true,
  workers: 4,
  reporter: [['list']],
  use: { baseURL: `http://localhost:${REVIEW_PORT}` },
  projects: [
    project('dark', 1440, 900),
    project('light', 1440, 900),
    project('dark', 375, 812),
    project('light', 375, 812),
  ],
  webServer: {
    command: `pnpm exec vite --port ${REVIEW_PORT} --strictPort`,
    cwd: APP,
    url: `http://localhost:${REVIEW_PORT}/demo.html`,
    timeout: 2 * 60_000,
    reuseExistingServer: false,
    stdout: 'ignore',
    stderr: 'pipe',
  },
});
