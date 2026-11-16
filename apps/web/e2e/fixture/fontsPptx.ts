// The font matrix decks: PPTX files written from code whose every text run names a face the compile ships. The fonts
// are the OFL container fixtures of slidock-fonts (crates/slidock-fonts/tests/data/container/README.md), subset to
// about fifty characters, so the text uses only those: A B C G O Q R S a b c g o q r s, digits, `!#&(),.:;?@[]{}` and
// `ÀÉßçñü–—“”€`.
//
// - `MATRIX_DECK`: two faces embedded in the package (a MicroType Express EOT and a plain EOT), the rest from a folder
//   of dropped files (TrueType, a variable font, CFF outlines, a face of a collection).
// - `STACK_DECK`: runs that request a family no file is named after ("Roboto Medium", "Oswald SemiBold"), which bind
//   to a family the compile ships ("Roboto", "Oswald"), so the requested family is not the injected one.
import { CLR_MAP, EMU, NS, XML, escape, rels, storedZip, themeElements, tree, xfrm } from '@slidock/wasm/test/ooxml';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/** The container fixtures of slidock-fonts. */
export const FONT_FIXTURES = 'crates/slidock-fonts/tests/data/container';

interface Run {
  text: string;
  font: string;
  bold?: boolean;
  italic?: boolean;
}

/** A text box: one paragraph per entry, each a list of runs. */
interface Box {
  at: [number, number, number, number];
  size: number;
  paragraphs: Run[][];
}

interface FontSlide {
  boxes: Box[];
}

/** A deck of the matrix: its slides in show order, its embedded fonts and the font files dropped beside it. */
export interface FontDeckSpec {
  slides: readonly FontSlide[];
  /** The embedded fonts: the family the presentation declares, and its regular face's EOT in `FONT_FIXTURES`. */
  embedded: readonly { family: string; file: string }[];
  /** The dropped font files in `FONT_FIXTURES`, and the licence of each family. */
  dropped: readonly { file: string; licence: string }[];
  /** The theme's minor font. */
  theme: string;
}

const MATRIX_SLIDES: readonly FontSlide[] = [
  {
    boxes: [
      { at: [48, 40, 624, 90], size: 54, paragraphs: [[{ text: 'Crabs & Orcas', font: 'Pacifico' }]] },
      {
        at: [48, 150, 624, 200],
        size: 30,
        paragraphs: [
          [{ text: 'SCARAB CARGO: 2026', font: 'Oswald' }],
          [{ text: 'ROSA & BOCA (b)', font: 'Oswald', bold: true }],
          [{ text: 'Boca, Rosa, Gabor; 1984!', font: 'Fira Sans' }],
        ],
      },
    ],
  },
  {
    boxes: [
      {
        at: [48, 40, 624, 320],
        size: 30,
        paragraphs: [
          [{ text: 'Barcos ago — 1984', font: 'Barlow Condensed' }],
          [{ text: 'Cobras: [4, 8, 15]', font: 'Barlow Condensed CFF' }],
          [{ text: '“Gabor” orcas, À ç ñ ü € ß', font: 'Fira Sans' }],
          [{ text: 'Sabor? Garbo!', font: 'Fira Sans', italic: true }],
        ],
      },
    ],
  },
  {
    boxes: [
      {
        at: [48, 40, 624, 320],
        size: 30,
        paragraphs: [
          [{ text: '{ a: 1, b: 2 } @ 3:45', font: 'Roboto Mono' }],
          [
            { text: 'Crabs', font: 'Pacifico' },
            { text: ' & ', font: 'Oswald', bold: true },
            { text: 'Orcas', font: 'Fira Sans' },
            { text: ' #42', font: 'Roboto Mono' },
          ],
          [{ text: 'Cargo (c) 2026', font: 'Barlow Condensed CFF' }],
        ],
      },
      {
        at: [48, 330, 624, 50],
        size: 30,
        paragraphs: [[{ text: 'Orcas: bocas!', font: 'Pacifico' }]],
      },
    ],
  },
];

export const MATRIX_DECK: FontDeckSpec = {
  slides: MATRIX_SLIDES,
  embedded: [
    { family: 'Roboto Mono', file: 'robotomono-mtx.eot' },
    { family: 'Barlow Condensed', file: 'barlow-mkeot.eot' },
  ],
  dropped: [
    { file: 'pacifico.ttf', licence: 'OFL-pacifico.txt' },
    { file: 'oswald.ttf', licence: 'OFL-oswald.txt' },
    { file: 'barlow-cff.otf', licence: 'OFL-barlow.txt' },
    // Barlow Condensed and Fira Sans: the embedded Barlow Condensed comes first, so only Fira Sans draws from here.
    { file: 'pair.ttc', licence: 'OFL-fira.txt' },
  ],
  theme: 'Fira Sans',
};

export const STACK_DECK: FontDeckSpec = {
  slides: [
    {
      boxes: [
        {
          at: [48, 40, 624, 320],
          size: 36,
          paragraphs: [
            [{ text: 'Boca Rosa: 2026', font: 'Roboto Medium' }],
            [{ text: 'SCARAB CARGO', font: 'Oswald SemiBold' }],
            [
              { text: 'Crabs & ', font: 'Roboto' },
              { text: 'Orcas!', font: 'Roboto Medium' },
            ],
          ],
        },
      ],
    },
  ],
  embedded: [],
  dropped: [
    { file: 'roboto.ttf', licence: 'OFL-roboto.txt' },
    { file: 'oswald.ttf', licence: 'OFL-oswald.txt' },
  ],
  theme: 'Roboto',
};

function run({ text, font, bold, italic }: Run, size: number): string {
  const flags = `${bold ? ' b="1"' : ''}${italic ? ' i="1"' : ''}`;
  return (
    `<a:r><a:rPr lang="en-US" sz="${size * 100}"${flags}><a:solidFill><a:srgbClr val="1E1E28"/></a:solidFill>` +
    `<a:latin typeface="${escape(font)}"/></a:rPr><a:t>${escape(text)}</a:t></a:r>`
  );
}

function textBox(id: number, box: Box): string {
  const paragraphs = box.paragraphs.map((runs) => `<a:p>${runs.map((r) => run(r, box.size)).join('')}</a:p>`);
  return (
    `<p:sp><p:nvSpPr><p:cNvPr id="${id}" name="Text ${id}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>` +
    `<p:spPr>${xfrm(...box.at)}<a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr>` +
    `<p:txBody><a:bodyPr wrap="square"/><a:lstStyle/>${paragraphs.join('')}</p:txBody></p:sp>`
  );
}

function slideXml(slide: FontSlide): string {
  const content = slide.boxes.map((box, i) => textBox(i + 2, box)).join('');
  return (
    `${XML}<p:sld ${NS}><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill>` +
    `<a:effectLst/></p:bgPr></p:bg>${tree(content)}</p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>`
  );
}

/** A matrix deck's bytes; `repo` is the repository root the embedded fonts are read from. */
export function fontsPptx(repo: string, spec: FontDeckSpec): Uint8Array {
  const { slides, embedded } = spec;
  const text = (value: string) => new TextEncoder().encode(value);
  const files: [string, Uint8Array][] = [];
  const types =
    `${XML}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">` +
    '<Default Extension="xml" ContentType="application/xml"/>' +
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>' +
    '<Default Extension="fntdata" ContentType="application/x-fontdata"/>' +
    '<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>' +
    '</Types>';
  files.push(['[Content_Types].xml', text(types)]);
  files.push(['_rels/.rels', text(rels([['rId1', 'officeDocument', 'ppt/presentation.xml']]))]);
  const slideIds = slides.map((_, i) => `<p:sldId id="${256 + i}" r:id="rIdS${i + 1}"/>`).join('');
  const fontList = embedded
    .map(
      ({ family }, i) =>
        `<p:embeddedFont><p:font typeface="${escape(family)}"/><p:regular r:id="rIdF${i + 1}"/></p:embeddedFont>`,
    )
    .join('');
  files.push([
    'ppt/presentation.xml',
    text(
      `${XML}<p:presentation ${NS}><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rIdM1"/></p:sldMasterIdLst>` +
        `<p:sldIdLst>${slideIds}</p:sldIdLst><p:sldSz cx="${720 * EMU}" cy="${405 * EMU}"/>` +
        `<p:notesSz cx="${405 * EMU}" cy="${720 * EMU}"/>${fontList === '' ? '' : `<p:embeddedFontLst>${fontList}</p:embeddedFontLst>`}` +
        '</p:presentation>',
    ),
  ]);
  files.push([
    'ppt/_rels/presentation.xml.rels',
    text(
      rels([
        ['rIdM1', 'slideMaster', 'slideMasters/slideMaster1.xml'],
        ...slides.map((_, i) => [`rIdS${i + 1}`, 'slide', `slides/slide${i + 1}.xml`] as const),
        ...embedded.map((_, i) => [`rIdF${i + 1}`, 'font', `fonts/font${i + 1}.fntdata`] as const),
      ]),
    ),
  ]);
  files.push([
    'ppt/theme/theme1.xml',
    text(`${XML}<a:theme ${NS} name="Fonts">${themeElements(spec.theme)}</a:theme>`),
  ]);
  files.push([
    'ppt/slideMasters/slideMaster1.xml',
    text(
      `${XML}<p:sldMaster ${NS}><p:cSld>${tree('')}</p:cSld><p:clrMap ${CLR_MAP}/>` +
        '<p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rIdL1"/></p:sldLayoutIdLst></p:sldMaster>',
    ),
  ]);
  files.push([
    'ppt/slideMasters/_rels/slideMaster1.xml.rels',
    text(
      rels([
        ['rIdTheme', 'theme', '../theme/theme1.xml'],
        ['rIdL1', 'slideLayout', '../slideLayouts/slideLayout1.xml'],
      ]),
    ),
  ]);
  files.push([
    'ppt/slideLayouts/slideLayout1.xml',
    text(
      `${XML}<p:sldLayout ${NS} type="blank"><p:cSld name="Blank">${tree('')}</p:cSld>` +
        '<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>',
    ),
  ]);
  files.push([
    'ppt/slideLayouts/_rels/slideLayout1.xml.rels',
    text(rels([['rIdM', 'slideMaster', '../slideMasters/slideMaster1.xml']])),
  ]);
  slides.forEach((slide, i) => {
    const n = i + 1;
    files.push([`ppt/slides/slide${n}.xml`, text(slideXml(slide))]);
    files.push([
      `ppt/slides/_rels/slide${n}.xml.rels`,
      text(rels([['rIdLayout', 'slideLayout', '../slideLayouts/slideLayout1.xml']])),
    ]);
  });
  embedded.forEach(({ file }, i) => {
    files.push([`ppt/fonts/font${i + 1}.fntdata`, readFileSync(join(repo, FONT_FIXTURES, file))]);
  });
  return storedZip(files);
}
