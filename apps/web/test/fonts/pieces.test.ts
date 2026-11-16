import type { FontFile, FontNeed } from '@slidock/types';
import { describe, expect, it, vi } from 'vitest';

import { deckCodepoints } from '../../src/fonts/codepoints';
import { DocumentFonts, type MakeFontFace } from '../../src/fonts/documentFonts';
import { droppedFonts, isFontFile } from '../../src/fonts/dropped';
import { familyRows, missingFamilies, personalFamilies } from '../../src/fonts/families';
import { sampleFontNeeds, sampleFontsDeck } from '../../src/testing/sample-fonts';

describe('deckCodepoints', () => {
  it('reads the text of the slides, with uppercase forms, digits and the always-drawn marks, sorted', () => {
    const points = deckCodepoints([
      { id: 's1', html: '<main><p>ab <b>ß</b></p><img alt="Not drawn" src="x.png"><script>zz</script></main>' },
    ]);
    const has = (char: string) => points.includes(char.codePointAt(0) ?? -1);
    expect(['a', 'b', 'A', 'B', 'ß', 'S', '0', '9', ' ', '…'].every(has)).toBe(true);
    expect(has('N')).toBe(false);
    expect(points).toEqual([...points].sort((x, y) => x - y));
  });
});

describe('dropped files', () => {
  it('recognizes font files by extension, and takes a drop only when every file is one', () => {
    expect(['a.TTF', 'b.otf', 'c.ttc', 'd.otc', 'e.woff', 'f.woff2'].every((name) => isFontFile({ name }))).toBe(true);
    expect(isFontFile({ name: 'deck.pptx' })).toBe(false);
    const drop = (names: string[]) =>
      droppedFonts({ files: names.map((name) => new File(['x'], name)) } as unknown as DataTransfer);
    expect(drop(['a.ttf', 'b.woff2'])?.map((file) => file.name)).toEqual(['a.ttf', 'b.woff2']);
    expect(drop(['a.ttf', 'deck.key'])).toBeNull();
    expect(drop([])).toBeNull();
  });
});

describe('familyRows', () => {
  it('lists every family with its faces, the missing ones, and which ones the download notice covers', () => {
    const rows = (
      fonts: Parameters<typeof sampleFontsDeck>[0],
      needs: FontNeed[],
      leaveOut: Set<string>,
      include = new Set<string>(),
    ) => {
      const compiled = sampleFontsDeck(fonts);
      return familyRows(compiled.deck, compiled.faces ?? [], needs, leaveOut, include);
    };
    const before = rows([], sampleFontNeeds([]), new Set());
    expect(before.map((row) => row.family)).toEqual([
      'Brand Sans',
      'Roboto',
      'Helvetica Neue',
      'Optima',
      'Barlow Condensed',
    ]);
    // A substitute draws the faces the deck lists; only a face it does not list shows as missing.
    expect(before.find((row) => row.family === 'Brand Sans')?.missing).toEqual([]);
    const gotham = { ...sampleFontNeeds([])[0], family: 'Gotham' } as FontNeed;
    expect(rows([], [gotham], new Set()).at(-1)).toMatchObject({
      family: 'Gotham',
      faces: [],
      missing: gotham.faces,
    });
    expect(before.find((row) => row.family === 'Barlow Condensed')?.faces.map((face) => face.demoted)).toEqual([
      false,
      true,
    ]);
    expect(missingFamilies(before)).toEqual(['Brand Sans', 'Roboto', 'Helvetica Neue', 'Optima']);

    const local = { source: 'local' as const, bytes: new ArrayBuffer(4), unicodeRange: null };
    const user = { source: 'user' as const, bytes: new ArrayBuffer(4), unicodeRange: null };
    const added = [local, local, user];
    const after = rows(added, sampleFontNeeds(added), new Set(['Brand Sans']));
    // Optima is restricted: it draws here, and the download leaves it out until the user includes it.
    expect(after.filter((row) => row.personal).map((row) => row.family)).toEqual([
      'Brand Sans',
      'Helvetica Neue',
      'Optima',
    ]);
    expect(personalFamilies(after)).toEqual(['Helvetica Neue']);
    const optima = after.find((row) => row.family === 'Optima');
    expect(optima).toMatchObject({ restricted: true, leftOut: true });
    expect(optima?.faces[0]).toMatchObject({ source: 'local', restricted: true });
    const included = rows(added, sampleFontNeeds(added), new Set(['Brand Sans']), new Set(['Optima']));
    expect(personalFamilies(included)).toEqual(['Helvetica Neue', 'Optima']);
  });
});

describe('DocumentFonts', () => {
  it('puts the shipped faces in the document with their own descriptors, replacing the last finish’s', () => {
    const added = new Set<FontFace>();
    const set = {
      add: vi.fn((face: FontFace) => added.add(face)),
      delete: vi.fn((face: FontFace) => added.delete(face)),
    };
    const made: { family: string; descriptors: FontFaceDescriptors }[] = [];
    const make: MakeFontFace = (family, _source, descriptors) => {
      made.push({ family, descriptors });
      return { family, loaded: Promise.resolve() } as unknown as FontFace;
    };
    const fonts = new DocumentFonts(set as unknown as FontFaceSet, make);
    const font = (family: string, unicodeRange: string | null): FontFile => ({
      family,
      weight: '200 700',
      style: 'italic',
      unicodeRange,
      format: 'woff2',
      bytes: new ArrayBuffer(4),
    });
    fonts.replace([font('Oswald', 'U+0000-00FF'), font('Brand Sans', null)]);
    expect(made).toEqual([
      {
        family: 'Oswald',
        descriptors: { weight: '200 700', style: 'italic', display: 'block', unicodeRange: 'U+0000-00FF' },
      },
      { family: 'Brand Sans', descriptors: { weight: '200 700', style: 'italic', display: 'block' } },
    ]);
    expect([...added].map((face) => face.family)).toEqual(['Oswald', 'Brand Sans']);
    fonts.replace([font('Roboto', null)]);
    expect([...added].map((face) => face.family)).toEqual(['Roboto']);
    fonts.clear();
    expect(added.size).toBe(0);
  });
});
