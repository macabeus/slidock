// The dev server's stand-in for `slidock-bench ui`: the same GET-only API (see `src/data/source.ts`) over the
// synthetic bench of `fixtures.ts`, reloaded when the schema crate's files change.
import { watch } from 'node:fs';
import type { IncomingMessage, ServerResponse } from 'node:http';
import type { Plugin } from 'vite';

import { DATA_DIR, type Fixture, loadFixture, resolveKey, rowDetail, rowImage } from './fixtures.ts';

type Answer = { status: number; type: string; body: string | Buffer };

const json = (value: unknown): Answer => ({ status: 200, type: 'application/json', body: JSON.stringify(value) });
const NOT_FOUND: Answer = { status: 404, type: 'application/json', body: '{"error":"not found"}' };

/** Answers one API path (without the `/api` prefix and the query). */
export function answer(fixture: Fixture, path: string): Answer {
  const parts = path
    .split('/')
    .filter((part) => part !== '')
    .map(decodeURIComponent);
  const [head, run = '', what, row = ''] = parts;
  if (head === 'status' && parts.length === 1) {
    return json(fixture.status);
  }
  if (head === 'runs' && parts.length === 1) {
    return json(fixture.runs);
  }
  if (head === 'runs' && parts.length === 3 && what === 'results') {
    const results = fixture.results.get(resolveKey(fixture, run));
    return results ? json(results) : NOT_FOUND;
  }
  if (head === 'runs' && parts.length === 3 && what === 'parity') {
    const view = fixture.parity.get(resolveKey(fixture, run));
    return view ? json(view) : NOT_FOUND;
  }
  if (head === 'runs' && parts.length === 4 && what === 'rows') {
    const detail = rowDetail(fixture, resolveKey(fixture, run), row);
    return detail ? json(detail) : NOT_FOUND;
  }
  if (head === 'png' && parts.length === 4) {
    const [, key = '', id = '', image = ''] = parts;
    const png = rowImage(fixture, resolveKey(fixture, key), id, image);
    return png ? { status: 200, type: 'image/png', body: png } : NOT_FOUND;
  }
  return NOT_FOUND;
}

export function benchMock(): Plugin {
  let fixture: Fixture | null = null;
  return {
    name: 'slidock-bench-mock',
    apply: 'serve',
    configureServer(server) {
      const watcher = watch(DATA_DIR, () => {
        fixture = null;
        server.ws.send({ type: 'full-reload' });
      });
      server.httpServer?.on('close', () => watcher.close());
      server.middlewares.use('/api', (request: IncomingMessage, response: ServerResponse) => {
        if (request.method !== 'GET' && request.method !== 'HEAD') {
          response.writeHead(405, { allow: 'GET, HEAD' }).end();
          return;
        }
        fixture ??= loadFixture();
        const path = (request.url ?? '/').split('?')[0] ?? '/';
        const { status, type, body } = answer(fixture, path);
        response.writeHead(status, {
          'content-type': type,
          'x-content-type-options': 'nosniff',
          'cache-control': 'no-store',
        });
        response.end(request.method === 'HEAD' ? undefined : body);
      });
    },
  };
}
