import { loadFixture } from '../mock/fixtures';
import { answer } from '../mock/plugin';
import { apiSource } from '../src/data/source';

export const fixture = loadFixture();

/** The API source over the dev mock: the same paths `slidock-bench ui` serves. */
export function mockSource() {
  const requests: string[] = [];
  const source = apiSource(async (path) => {
    requests.push(path);
    const result = answer(fixture, path.replace(/^api/, ''));
    const body = typeof result.body === 'string' ? result.body : new Uint8Array(result.body);
    return new Response(body, { status: result.status, headers: { 'content-type': result.type } });
  });
  return { source, requests };
}

export function latest() {
  const results = fixture.results.get(fixture.latest);
  if (!results) {
    throw new Error('no latest results');
  }
  return results;
}

export function baseline() {
  const results = fixture.results.get(fixture.baseline);
  if (!results) {
    throw new Error('no baseline results');
  }
  return results;
}
