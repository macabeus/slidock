// Builds apps/web as the Pages workflow does, checks that dist is a build for /slidock/, then serves apps/web/dist under
// /slidock/ on a free loopback port, as GitHub Pages serves a project site: every other path answers 404. The tests
// read the site's URL from `SLIDOCK_PAGES_URL`.
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { readFile, stat } from 'node:fs/promises';
import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { extname, join, normalize, sep } from 'node:path';

import { envPort } from '../../../../packages/player/e2e/support/browserServer';
import { APP, REPO } from '../support/decks';
import { expectBuiltFor } from '../support/dist';

export const BASE = '/slidock/';
const DIST = join(APP, 'dist');

const TYPES: Record<string, string> = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.wasm': 'application/wasm',
  '.json': 'application/json',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.webp': 'image/webp',
  '.woff2': 'font/woff2',
  '.txt': 'text/plain; charset=utf-8',
};

/** The file a request path names under the site, or null (outside /slidock/, a traversal, or no such file). */
async function siteFile(pathname: string): Promise<string | null> {
  if (!pathname.startsWith(BASE)) {
    return null;
  }
  const relative = decodeURIComponent(pathname.slice(BASE.length)) || 'index.html';
  const path = normalize(join(DIST, relative));
  if (!path.startsWith(DIST + sep)) {
    return null;
  }
  const info = await stat(path).catch(() => null);
  if (info?.isDirectory()) {
    return siteFile(`${pathname.replace(/\/?$/, '/')}index.html`);
  }
  return info?.isFile() ? path : null;
}

export default async function globalSetup() {
  if (process.env.SLIDOCK_PAGES_SKIP_BUILD !== '1') {
    execFileSync('pnpm', ['wasm:build'], { cwd: REPO, stdio: 'inherit' });
    execFileSync('pnpm', ['--filter', '@slidock/web', 'build'], {
      cwd: REPO,
      stdio: 'inherit',
      env: { ...process.env, VITE_BASE_URL: BASE },
    });
  }
  expectBuiltFor(
    readFileSync(join(DIST, 'index.html'), 'utf8'),
    BASE,
    'apps/web/dist',
    `the dry run serves the Pages build: run it without SLIDOCK_PAGES_SKIP_BUILD=1 to build with VITE_BASE_URL=${BASE}`,
  );
  const server = createServer((request, response) => {
    void (async () => {
      const { pathname } = new URL(request.url ?? '/', 'http://localhost');
      const file = request.method === 'GET' || request.method === 'HEAD' ? await siteFile(pathname) : null;
      if (!file) {
        response.writeHead(404, { 'content-type': 'text/plain' }).end('not found');
        return;
      }
      const body = await readFile(file);
      response.writeHead(200, { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream' });
      response.end(request.method === 'HEAD' ? undefined : body);
    })().catch(() => response.writeHead(500).end());
  });
  await new Promise<void>((resolve) => server.listen(envPort('SLIDOCK_PAGES_PORT', 0), '127.0.0.1', resolve));
  const { port } = server.address() as AddressInfo;
  process.env.SLIDOCK_PAGES_URL = `http://127.0.0.1:${port}${BASE}`;
  return () => new Promise<void>((resolve) => server.close(() => resolve()));
}
