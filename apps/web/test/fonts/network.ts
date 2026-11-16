import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

import type { Fetch } from '../../src/fonts/google';
import { ROBOTO_PLAN } from '../../src/testing/sample-fonts';
import { substitutePath } from '../../substitutes';
import { answer, recorded } from './recorded';

const PUBLIC = join(import.meta.dirname, '../../public');

/** WOFF2-looking bytes for a gstatic file. */
export function chunkBytes(url: string): Uint8Array<ArrayBuffer> {
  return new Uint8Array(new TextEncoder().encode(`wOF2 ${url}`));
}

/**
 * The network of a test: Google answers with the recorded Roboto response and small chunk bodies, the app's own
 * origin serves `public/` and the bundled substitutes under `/`. Every request is logged.
 */
export function testNetwork() {
  const requests: { url: string; init: RequestInit }[] = [];
  const fetch: Fetch = async (url, init) => {
    requests.push({ url, init });
    if (url === ROBOTO_PLAN.url) {
      return answer(recorded('roboto-browser'));
    }
    if (url.startsWith('https://fonts.gstatic.com/')) {
      return answer(chunkBytes(url));
    }
    const own = url.startsWith('/') ? (substitutePath(url.slice(1)) ?? join(PUBLIC, url)) : undefined;
    if (own !== undefined && existsSync(own)) {
      return answer(readFileSync(own));
    }
    return answer('not found', { status: 404 });
  };
  return {
    fetch,
    requests,
    google: () => requests.filter((request) => new URL(request.url, 'http://app.test').hostname !== 'app.test'),
  };
}
