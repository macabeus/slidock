// Starts the browser server in the pinned image when a project runs there, and returns its teardown.
import { startBrowserServer } from '../../../../packages/player/e2e/support/browserServer';
import { BROWSER_SERVER_PORT, REPO } from './e2e.config';

export default async function globalSetup() {
  return startBrowserServer({ port: BROWSER_SERVER_PORT, mounts: [REPO] });
}
