// The package's own code and tests import nothing by path from another package's folder: what they share with the
// web app (the OOXML writers in test/support/ooxml.ts) lives here, and the app imports it by the package's name.
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const PACKAGE = resolve(dirname(fileURLToPath(import.meta.url)), '..');

/** The module specifiers of `source`: static imports and re-exports, and dynamic imports of a string. */
export function specifiers(source: string): string[] {
  const found: string[] = [];
  for (const match of source.matchAll(/(?:\bfrom\s*|\bimport\s*\(\s*|^\s*import\s+)(['"])([^'"]+)\1/gm)) {
    found.push(match[2] ?? '');
  }
  return found;
}

/** Every `.ts` file under `dir`. */
function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true, recursive: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith('.ts'))
    .map((entry) => join(entry.parentPath, entry.name));
}

/** The relative specifiers of `file` that resolve outside the package. */
export function outside(file: string, source: string): string[] {
  return specifiers(source).filter((specifier) => {
    if (!specifier.startsWith('.')) {
      return false;
    }
    const path = relative(PACKAGE, resolve(dirname(file), specifier));
    return path === '..' || path.startsWith(`..${sep}`);
  });
}

describe('the package imports', () => {
  it('reads every kind of specifier', () => {
    const source = [
      "import { a } from '../a';",
      'import type { B } from "./b";',
      "export { c } from '../../c';",
      "import './side-effect';",
      "const d = await import('../../../d');",
      "import { e } from '@slidock/types';",
    ].join('\n');
    expect(specifiers(source)).toEqual(['../a', './b', '../../c', './side-effect', '../../../d', '@slidock/types']);
  });

  it('flags a relative import that leaves the package', () => {
    const test = join(PACKAGE, 'test', 'x.test.ts');
    expect(outside(test, "import { smokePptx } from '../../../apps/web/e2e/fixture/pptx';")).toEqual([
      '../../../apps/web/e2e/fixture/pptx',
    ]);
    expect(outside(test, "import { x } from './support/ooxml';\nimport { y } from '../src/folder';")).toEqual([]);
  });

  it('holds in src and test', () => {
    // This file's own examples above are strings, not imports.
    const self = fileURLToPath(import.meta.url);
    const files = [...sources(join(PACKAGE, 'src')), ...sources(join(PACKAGE, 'test'))].filter((file) => file !== self);
    expect(files.length).toBeGreaterThan(5);
    const leaving = files.flatMap((file) =>
      outside(file, readFileSync(file, 'utf8')).map((specifier) => `${relative(PACKAGE, file)}: ${specifier}`),
    );
    expect(leaving).toEqual([]);
  });
});
