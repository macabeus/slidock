import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/** The Google Fonts `css2` responses recorded for slidock-fonts (crates/slidock-fonts/tests/data/google/README.md). */
export type Recorded = 'roboto-browser' | 'roboto-cli' | 'oswald-browser' | 'oswald-cli' | 'two-families-cli';

const DIR = join(import.meta.dirname, '../../../../crates/slidock-fonts/tests/data/google/');

export function recorded(name: Recorded): string {
  return readFileSync(`${DIR}${name}.css`, 'utf8');
}

/** A `Response` as `fetch` would give it. */
export function answer(body: BodyInit | null, init: ResponseInit = {}): Response {
  return new Response(body, { status: 200, ...init });
}

/** The code points of `text`, sorted. */
export function points(text: string): number[] {
  return [...new Set([...text].map((char) => char.codePointAt(0) ?? 0))].sort((a, b) => a - b);
}
