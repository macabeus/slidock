import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import {
  CORE_MANIFEST,
  type CoreFace,
  loadFace,
  loadManifest,
  parseManifest,
  substituteFile,
} from '../../src/fonts/bundled';
import type { Fetch } from '../../src/fonts/google';
import { SUBSTITUTES_DIR, substitutePath } from '../../substitutes';
import { answer } from './recorded';

const committed = parseManifest(JSON.parse(readFileSync(substitutePath(CORE_MANIFEST) ?? '', 'utf8')));

/** Serves the bundled substitutes under `base`, as the app does. */
function publicFolder(base: string): Fetch & { urls: string[] } {
  const urls: string[] = [];
  const fetch = async (url: string) => {
    urls.push(url);
    const path = url.startsWith(base) ? substitutePath(url.slice(base.length)) : undefined;
    return path !== undefined && existsSync(path) ? answer(readFileSync(path)) : answer('', { status: 404 });
  };
  return Object.assign(fetch, { urls });
}

describe('the committed substitutes', () => {
  it('are the 24 faces of Liberation, DejaVu Sans, Carlito and Caladea, each file matching its SHA-256', () => {
    expect(committed.faces).toHaveLength(24);
    expect([...new Set(committed.faces.map((face) => face.family))]).toEqual([
      'Liberation Sans',
      'Liberation Serif',
      'Liberation Mono',
      'DejaVu Sans',
      'Carlito',
      'Caladea',
    ]);
    for (const face of committed.faces) {
      const bytes = readFileSync(join(SUBSTITUTES_DIR, face.file));
      expect(createHash('sha256').update(bytes).digest('hex'), face.file).toBe(face.sha256);
      expect(bytes.subarray(0, 4).toString('latin1'), face.file).toBe('wOF2');
    }
  });

  it('ship each licence and nothing else', () => {
    const raw = JSON.parse(readFileSync(join(SUBSTITUTES_DIR, 'core.json'), 'utf8')) as {
      faces: { licenceFile: string }[];
    };
    const licences = new Set(raw.faces.map((face) => face.licenceFile));
    expect([...licences].sort()).toEqual([
      'Caladea-OFL.txt',
      'Carlito-OFL.txt',
      'DejaVu-LICENSE.txt',
      'Liberation-LICENSE.txt',
    ]);
    const files = readdirSync(SUBSTITUTES_DIR).sort();
    expect(files).toEqual([...committed.faces.map((face) => face.file), ...licences, 'core.json'].sort());
    expect(readFileSync(join(SUBSTITUTES_DIR, 'Carlito-OFL.txt'), 'utf8')).toContain('SIL OPEN FONT LICENSE');
    expect(readFileSync(join(SUBSTITUTES_DIR, 'DejaVu-LICENSE.txt'), 'utf8')).toContain('Bitstream Vera');
  });
});

describe('substituteFile', () => {
  it('finds the file of the face the worker names, by family, weight and style', () => {
    const file = (family: string, weight: number, style: 'normal' | 'italic') =>
      substituteFile(committed, { family, postscriptName: '', weight, style })?.file;
    expect(file('Liberation Sans', 400, 'normal')).toBe('LiberationSans-Regular.woff2');
    expect(file('liberation sans', 700, 'italic')).toBe('LiberationSans-BoldItalic.woff2');
    expect(file('DejaVu Sans', 400, 'italic')).toBe('DejaVuSans-Oblique.woff2');
    expect(file('Liberation Sans', 500, 'normal')).toBeUndefined();
    expect(file('Nope Sans', 400, 'normal')).toBeUndefined();
  });
});

describe('loading', () => {
  it('fetches the manifest and a face under the base URL, checking the face against its SHA-256', async () => {
    const fetch = publicFolder('/slidock/');
    const manifest = await loadManifest(fetch, '/slidock/');
    const face = manifest.faces.find((entry) => entry.file === 'Carlito-Regular.woff2') as CoreFace;
    const bytes = await loadFace(fetch, '/slidock/', face);
    expect(bytes.byteLength).toBe(readFileSync(join(SUBSTITUTES_DIR, 'Carlito-Regular.woff2')).length);
    expect(fetch.urls).toEqual(['/slidock/fonts/core/core.json', '/slidock/fonts/core/Carlito-Regular.woff2']);
    await expect(loadFace(fetch, '/slidock/', { ...face, sha256: '0'.repeat(64) })).rejects.toThrow(/not the file/);
    await expect(loadFace(fetch, '/slidock/', { ...face, file: 'Missing.woff2' })).rejects.toThrow(/HTTP 404/);
  });

  it('refuses a manifest it cannot read, or one naming a file outside its folder', () => {
    expect(() => parseManifest({})).toThrow(/cannot be read/);
    const face = {
      family: 'X',
      weight: 400,
      style: 'normal',
      file: '../x.woff2',
      sha256: '0'.repeat(64),
      licence: 'OFL-1.1',
    };
    expect(() => parseManifest({ faces: [face] })).toThrow(/cannot be read/);
    expect(() => parseManifest({ faces: [{ ...face, file: 'x.woff2', style: 'oblique' }] })).toThrow(/cannot be read/);
    expect(parseManifest({ faces: [{ ...face, file: 'x.woff2' }] }).faces).toHaveLength(1);
  });
});
