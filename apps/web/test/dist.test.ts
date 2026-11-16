// @vitest-environment node
import { describe, expect, it } from 'vitest';

import { builtBase, expectBuiltFor } from '../e2e/support/dist';

const built = (src: string) =>
  `<!doctype html><html><head><meta charset="utf-8" /><link rel="icon" href="/favicon.svg" />
  <script type="module" crossorigin src="${src}"></script></head><body><div id="root"></div></body></html>`;

describe('builtBase', () => {
  it("reads the base from the entry script's folder", () => {
    expect(builtBase(built('/assets/index-CBZY1R3_.js'))).toBe('/');
    expect(builtBase(built('/slidock/assets/index-CBZY1R3_.js'))).toBe('/slidock/');
  });

  it('is null for a page that is not a build: the source index.html, or no module entry', () => {
    expect(builtBase(built('/src/main.tsx'))).toBeNull();
    expect(builtBase('<!doctype html><html><body>not found</body></html>')).toBeNull();
  });
});

describe('expectBuiltFor', () => {
  it('passes a build for the base and refuses any other, saying which it is', () => {
    expect(() => expectBuiltFor(built('/assets/a.js'), '/', 'dist', 'rebuild')).not.toThrow();
    expect(() => expectBuiltFor(built('/slidock/assets/a.js'), '/', 'dist', 'rebuild')).toThrow(
      'dist is a build for the base /slidock/, not for /: rebuild',
    );
    expect(() => expectBuiltFor(built('/src/main.tsx'), '/slidock/', 'dist', 'rebuild')).toThrow(
      'dist is not a build of the app, not for /slidock/: rebuild',
    );
  });
});
