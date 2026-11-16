// The built module (`pnpm wasm:build`) under Node: the compiler's stages and typed errors, and folders packed as
// zips its reader reads.
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { beforeAll, describe, expect, it } from 'vitest';

import { Compiler as WasmCompiler, initSync, version, zipEntries } from '../pkg/slidock_wasm.js';
import type { ZipEntry } from '../pkg/slidock_wasm.js';
import { Compiler, StageError } from '../src/compiler';
import type { RangeSource } from '../src/compiler';
import { folderName, packFolder } from '../src/folder';
import { CLR_MAP, EMU, NS, XML, escape, rels, storedZip, themeElements, tree, xfrm } from './support/ooxml';

beforeAll(() => {
  initSync({ module: readFileSync(new URL('../pkg/slidock_wasm_bg.wasm', import.meta.url)) });
});

function bytesSource(bytes: Uint8Array): RangeSource {
  return { size: bytes.length, read: (offset, length) => bytes.slice(offset, offset + length) };
}

const encoder = new TextEncoder();

/** Barlow Condensed Regular cut to a few letters (OFL-1.1), from slidock-fonts' fixtures. */
const barlow = new Uint8Array(
  readFileSync(new URL('../../../crates/slidock-fonts/tests/data/container/barlow.ttf', import.meta.url)),
);

/** A one-slide PPTX: `text` in a text box, with `family` as the theme's major and minor font. */
function presentation(family = 'Liberation Sans', text = 'Hello'): Uint8Array {
  const box =
    '<p:sp><p:nvSpPr><p:cNvPr id="2" name="Text 2"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>' +
    `<p:spPr>${xfrm(60, 160, 600, 80)}<a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr>` +
    '<p:txBody><a:bodyPr wrap="square"/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" sz="2400"/>' +
    `<a:t>${escape(text)}</a:t></a:r></a:p></p:txBody></p:sp>`;
  const files: [string, string][] = [
    [
      '[Content_Types].xml',
      `${XML}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">` +
        '<Default Extension="xml" ContentType="application/xml"/>' +
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>' +
        '<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>' +
        '</Types>',
    ],
    ['_rels/.rels', rels([['rId1', 'officeDocument', 'ppt/presentation.xml']])],
    [
      'ppt/presentation.xml',
      `${XML}<p:presentation ${NS}><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rIdM1"/></p:sldMasterIdLst>` +
        `<p:sldIdLst><p:sldId id="256" r:id="rIdS1"/></p:sldIdLst><p:sldSz cx="${720 * EMU}" cy="${405 * EMU}"/>` +
        `<p:notesSz cx="${405 * EMU}" cy="${720 * EMU}"/></p:presentation>`,
    ],
    [
      'ppt/_rels/presentation.xml.rels',
      rels([
        ['rIdM1', 'slideMaster', 'slideMasters/slideMaster1.xml'],
        ['rIdS1', 'slide', 'slides/slide1.xml'],
      ]),
    ],
    ['ppt/theme/theme1.xml', `${XML}<a:theme ${NS} name="Test">${themeElements(family)}</a:theme>`],
    [
      'ppt/slideMasters/slideMaster1.xml',
      `${XML}<p:sldMaster ${NS}><p:cSld>${tree('')}</p:cSld><p:clrMap ${CLR_MAP}/>` +
        '<p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rIdL1"/></p:sldLayoutIdLst></p:sldMaster>',
    ],
    [
      'ppt/slideMasters/_rels/slideMaster1.xml.rels',
      rels([
        ['rIdTheme', 'theme', '../theme/theme1.xml'],
        ['rIdL1', 'slideLayout', '../slideLayouts/slideLayout1.xml'],
      ]),
    ],
    [
      'ppt/slideLayouts/slideLayout1.xml',
      `${XML}<p:sldLayout ${NS} type="blank"><p:cSld name="Blank">${tree('')}</p:cSld>` +
        '<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>',
    ],
    [
      'ppt/slideLayouts/_rels/slideLayout1.xml.rels',
      rels([['rIdM', 'slideMaster', '../slideMasters/slideMaster1.xml']]),
    ],
    [
      'ppt/slides/slide1.xml',
      `${XML}<p:sld ${NS}><p:cSld>${tree(box)}</p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>`,
    ],
    ['ppt/slides/_rels/slide1.xml.rels', rels([['rIdLayout', 'slideLayout', '../slideLayouts/slideLayout1.xml']])],
  ];
  return storedZip(files.map(([path, xml]) => [path, encoder.encode(xml)] as const));
}

/** A document of the IR's JSON format, which only the CLI reads. */
const irDocument = encoder.encode(JSON.stringify({ $schema: 'urn:slidock:presentation-ir:1', slides: [] }));

function stageError(run: () => unknown): StageError {
  try {
    run();
  } catch (error) {
    if (error instanceof StageError) {
      return error;
    }
    throw error;
  }
  throw new Error('the stage succeeded');
}

describe('the wasm compiler', () => {
  it('opens, finishes and prints the document set of a presentation read by range', () => {
    expect(version()).toMatch(/^\d+\.\d+\.\d+/);
    const compiler = new Compiler(new WasmCompiler());
    const reads: number[][] = [];
    const bytes = presentation();
    const source = bytesSource(bytes);
    const opened = compiler.open('hello.pptx', {
      size: source.size,
      read: (offset, length) => {
        reads.push([offset, length]);
        return source.read(offset, length);
      },
    });
    expect(opened.diagnostics).toEqual([]);
    expect(Array.isArray(opened.needs)).toBe(true);
    expect(reads.length).toBeGreaterThan(0);
    compiler.finish();
    try {
      compiler.print();
    } catch (error) {
      // The slide templates come from the bundler; the document set is kept either way.
      expect(error).toBeInstanceOf(StageError);
    }
    const documents = compiler.documents();
    expect(Object.keys(documents.text).sort()).toEqual([
      'layouts/pp-2147483649.html',
      'presentation.json',
      'slides/pp-256.html',
    ]);
    expect(documents.text['slides/pp-256.html']).toContain('>Hello<');
  });

  it('finishes again after a font arrives, shipping it as one finish with the font does', () => {
    const deck = () => bytesSource(presentation('Barlow Condensed', 'Bars cog'));
    const zip = (compiler: Compiler): Uint8Array => {
      const chunks: Uint8Array[] = [];
      compiler.bundle({ singleFile: false, singleFileThreshold: null, leaveOut: [], include: [] }, (chunk) =>
        chunks.push(chunk),
      );
      return Buffer.concat(chunks);
    };
    const late = new Compiler(new WasmCompiler());
    const opened = late.open('barlow.pptx', deck());
    const need = opened.needs.find((n) => n.family === 'Barlow Condensed');
    expect(need?.google?.url).toBe('https://fonts.googleapis.com/css2?family=Barlow+Condensed:wght@400&display=block');
    expect(need?.local?.families).toEqual(['barlowcondensed']);
    late.finish();
    const before = late.print();
    expect(before.fonts).toEqual([]);
    // No source has Barlow Condensed yet: the generic sans-serif substitute draws it, with no file.
    expect(before.substitutes).toContainEqual({
      family: 'Liberation Sans',
      postscriptName: 'LiberationSans',
      weight: 400,
      style: 'normal',
    });
    const needs = late.addFont(barlow, 'user');
    expect(needs.some((n) => n.family === 'Barlow Condensed')).toBe(false);
    late.finish();
    const printed = late.print();
    expect(printed.fonts).toHaveLength(1);
    expect(printed.fonts[0]).toMatchObject({ family: 'Barlow Condensed', weight: '400', format: 'woff2' });
    expect(new TextDecoder().decode(late.fontBytes(0).slice(0, 4))).toBe('wOF2');
    expect(printed.faces).toMatchObject([{ source: 'user', shipped: true, notice: true, restricted: false }]);
    const lateZip = zip(late);

    const early = new Compiler(new WasmCompiler());
    early.open('barlow.pptx', deck());
    early.addFont(barlow, 'user');
    early.finish();
    early.print();
    expect(Buffer.compare(zip(early), lateZip)).toBe(0);
    // Without fonts, nothing ships.
    const parity = new Compiler(new WasmCompiler());
    parity.open('barlow.pptx', deck(), false);
    parity.addFont(barlow, 'user');
    parity.finish();
    expect(parity.print().faces.every((face) => face.source === 'user')).toBe(true);
    // A bundled file is the floor: it ships with no notice.
    const floor = new Compiler(new WasmCompiler());
    floor.open('barlow.pptx', deck());
    floor.addFont(barlow, 'bundled');
    floor.finish();
    expect(floor.print().faces).toMatchObject([{ source: 'bundled', shipped: true, notice: false }]);
  });

  it('answers a refused input, a file that is no font and stages out of order with typed errors', () => {
    const compiler = new Compiler(new WasmCompiler());
    expect(stageError(() => compiler.fontNeeds()).error.kind).toBe('request');
    const refused = stageError(() => compiler.open('notes.txt', bytesSource(encoder.encode('not a deck'))));
    expect(refused.error.kind).toBe('refused');
    expect(refused.error.diagnostics.map((d) => d.code)).toEqual(['IR0902']);
    // The IR's JSON format is the CLI's input only: the wasm build refuses it like any other unread file.
    const ir = stageError(() => compiler.open('hello.presentation-ir.json', bytesSource(irDocument)));
    expect(ir.error.kind).toBe('refused');
    expect(ir.error.diagnostics.map((d) => d.code)).toEqual(['IR0902']);
    compiler.open('hello.pptx', bytesSource(presentation()));
    expect(stageError(() => compiler.print()).error.kind).toBe('request');
    const font = stageError(() => compiler.addFont(encoder.encode('not a font'), 'user'));
    expect(font.error.kind).toBe('request');
    expect(
      stageError(() =>
        compiler.bundle({ singleFile: false, singleFileThreshold: null, leaveOut: [], include: [] }, () => {}),
      ).error.kind,
    ).toBe('request');
  });

  it('turns a source that fails to read into a typed error, not a crash', () => {
    const compiler = new Compiler(new WasmCompiler());
    const failing: RangeSource = {
      size: 100,
      read: () => {
        throw new Error('the file is gone');
      },
    };
    expect(stageError(() => compiler.open('deck.pptx', failing)).error.kind).toMatch(/^(refused|failed)$/);
    const short: RangeSource = { size: 100, read: () => new Uint8Array(1) };
    expect(stageError(() => compiler.open('deck.pptx', short)).error.kind).toMatch(/^(refused|failed)$/);
  });
});

function sha256(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
}

async function entries(file: File): Promise<ZipEntry[]> {
  return zipEntries(bytesSource(new Uint8Array(await file.arrayBuffer())));
}

describe('packFolder', () => {
  const files = [
    { path: 'Deck.key/Index/Slide-1.iwa', file: new Blob([encoder.encode('slide')]) },
    { path: 'Deck.key/Data/é.png', file: new Blob([new Uint8Array([0x89, 0x50, 0x4e, 0x47])]) },
    { path: 'Deck.key/Index/Document.iwa', file: new Blob([encoder.encode('document')]) },
    { path: 'Deck.key/empty', file: new Blob([]) },
  ];
  const expected = [...files]
    .sort((a, b) => (a.path < b.path ? -1 : 1))
    .map(({ path, file }) => ({ name: path, size: file.size, stored: true }));

  it('packs the files as a stored zip, in path order, that the compiler reads', async () => {
    const packed = packFolder('Deck.key', files);
    expect(packed.name).toBe('Deck.key');
    const read = await entries(packed);
    expect(read.map(({ name, size, stored }) => ({ name, size, stored }))).toEqual(expected);
    for (const { path, file } of files) {
      const entry = read.find(({ name }) => name === path);
      expect(entry?.sha256).toBe(sha256(new Uint8Array(await file.arrayBuffer())));
    }
    expect(folderName(files)).toBe('Deck.key');
  });

  it('writes ZIP64 fields the reader reads', async () => {
    const packed = packFolder('Deck.key', files, { zip64: 'always' });
    const read = await entries(packed);
    expect(read.map(({ name, size, stored }) => ({ name, size, stored }))).toEqual(expected);
    expect(packed.size).toBeGreaterThan(packFolder('Deck.key', files).size);
  });

  it('refuses paths that leave the folder or repeat', () => {
    for (const path of ['/abs', 'a/../b', 'a//b', './a', 'a\\b', '']) {
      expect(() => packFolder('x', [{ path, file: new Blob([]) }])).toThrow(TypeError);
    }
    expect(() =>
      packFolder('x', [
        { path: 'a/b', file: new Blob([]) },
        { path: 'a/b', file: new Blob([]) },
      ]),
    ).toThrow(/twice/);
    expect(folderName([{ path: 'loose.txt', file: new Blob([]) }])).toBeUndefined();
    expect(
      folderName([
        { path: 'a/x', file: new Blob([]) },
        { path: 'b/x', file: new Blob([]) },
      ]),
    ).toBeUndefined();
  });
});
