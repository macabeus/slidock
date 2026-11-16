// Starts the browser server in the pinned image when a project runs there (support/browserServer.ts), and stops it
// after. It mounts the repository (the server's playwright-core) and the compiled decks' folder.
import { mkdirSync } from 'node:fs';

import { BROWSER_SERVER_PORT, startBrowserServer } from './support/browserServer';
import { OUT, REPO } from './support/cli';

export default function globalSetup() {
  mkdirSync(OUT, { recursive: true });
  return startBrowserServer({ port: BROWSER_SERVER_PORT, mounts: [REPO, ...(OUT.startsWith(REPO) ? [] : [OUT])] });
}
