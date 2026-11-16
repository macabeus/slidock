import type {
  DeckFont,
  FaceRequest,
  FontFile,
  FontNeed,
  FontReport,
  GooglePlan,
  LocalPick,
  SubstituteFace,
} from '@slidock/types';

import type { CompiledDeck } from '../compile/client';
import type { FakeFont, FakeWorkerScript } from './fake-worker';
import { SAMPLE_DIAGNOSTICS, sampleDeck } from './sample-deck';

// The sample deck with a font story, as the worker would tell it: each family's faces come from a different source
// once that source has delivered. The fake worker reads no font file, so it decides by source alone:
//
// | Family           | Before                  | After                                               |
// | ---------------- | ----------------------- | --------------------------------------------------- |
// | Brand Sans       | missing, substitute     | a `user` file of 400: it draws 400, and 700 emboldened |
// | Roboto           | missing, Google plan    | any `google` file: the chunks draw the subtitle     |
// | Helvetica Neue   | missing, LFA candidate  | the first `local` file                              |
// | Optima           | missing, LFA candidate  | the second `local` file, restricted: not in the zip |
// | Barlow Condensed | embedded (700 demoted)  | unchanged                                           |

export const ROBOTO_PLAN: GooglePlan = {
  family: 'Roboto',
  licence: 'OFL-1.1',
  faces: [
    { weight: 400, style: 'normal' },
    { weight: 700, style: 'normal' },
    { weight: 400, style: 'italic' },
  ],
  url: 'https://fonts.googleapis.com/css2?family=Roboto:ital,wght@0,400;0,700;1,400&display=block',
};

const REGULAR: FaceRequest = { weight: 400, style: 'normal' };
const BOLD: FaceRequest = { weight: 700, style: 'normal' };

/** The installed faces the fake worker picks, by PostScript name, in reading order. */
export const LOCAL_CANDIDATES = ['HelveticaNeue', 'Optima-Regular'] as const;

function bySource(fonts: readonly FakeFont[], source: FakeFont['source']): FakeFont[] {
  return fonts.filter((font) => font.source === source);
}

/** Puts `stack` in place of the font stack of element `id`'s text. */
function setFamily(html: string, id: string, stack: string): string {
  const at = html.indexOf(`data-el="${id}"`);
  const family = 'font-family:&quot;Liberation Sans&quot;,sans-serif';
  const start = html.indexOf(family, at);
  if (at < 0 || start < 0) {
    return html;
  }
  return `${html.slice(0, start)}font-family:${stack}${html.slice(start + family.length)}`;
}

const quoted = (...families: string[]) =>
  [...families.map((family) => `&quot;${family}&quot;`), 'sans-serif'].join(',');

/** What is still missing once `fonts` were added. */
export function sampleFontNeeds(fonts: readonly FakeFont[]): FontNeed[] {
  const user = bySource(fonts, 'user').length > 0;
  const google = bySource(fonts, 'google').length > 0;
  const local = bySource(fonts, 'local').length;
  const need = (family: string, faces: FaceRequest[], extra: Partial<FontNeed> = {}): FontNeed => ({
    family,
    key: family.toLowerCase().replace(/[\s-]/g, ''),
    faces,
    postscriptNames: [],
    embedded: false,
    ...(faces.length > 0 ? { substitute: 'Liberation Sans' } : {}),
    ...extra,
  });
  return [
    // The user's file is a regular face: no source has the bold one, which the browser draws from it emboldened.
    need('Brand Sans', user ? [BOLD] : [REGULAR, BOLD]),
    need('Roboto', google ? [] : [REGULAR], { google: ROBOTO_PLAN }),
    need('Helvetica Neue', local > 0 ? [] : [REGULAR], { postscriptNames: ['HelveticaNeue'] }),
    need('Optima', local > 1 ? [] : [REGULAR], { postscriptNames: ['Optima-Regular'] }),
    need('Barlow Condensed', [], { embedded: true }),
  ];
}

/** The deck a finish prints once `fonts` were added. */
export function sampleFontsDeck(fonts: readonly FakeFont[]): CompiledDeck {
  const base = sampleDeck();
  const users = bySource(fonts, 'user');
  const googles = bySource(fonts, 'google');
  const locals = bySource(fonts, 'local');
  const restricted = locals.length > 1;
  const face = (family: string, request: FaceRequest, source: DeckFont['source']): DeckFont => ({
    family,
    weight: request.weight,
    style: request.style,
    source,
    licence: source === 'system' ? null : `${family} licence`,
  });
  const deckFonts: DeckFont[] = [
    face('Brand Sans', REGULAR, users.length > 0 ? 'user' : 'bundled'),
    face('Brand Sans', BOLD, users.length > 0 ? 'user' : 'bundled'),
    face('Roboto', REGULAR, googles.length > 0 ? 'google' : 'bundled'),
    face('Helvetica Neue', REGULAR, locals.length > 0 ? 'local' : 'bundled'),
    // A restricted face draws here; the download leaves it out unless the user includes it.
    face('Optima', REGULAR, restricted ? 'local' : 'bundled'),
    face('Barlow Condensed', REGULAR, 'embedded'),
    face('Barlow Condensed', BOLD, 'embedded'),
  ];
  const shipped: FontFile[] = [
    ...users.slice(0, 1).map((font) => ({
      family: 'Brand Sans',
      weight: '400',
      style: 'normal' as const,
      unicodeRange: null,
      format: 'woff2',
      bytes: font.bytes,
    })),
    ...googles.map((font) => ({
      family: 'Roboto',
      weight: '400',
      style: 'normal' as const,
      unicodeRange: font.unicodeRange,
      format: 'woff2',
      bytes: font.bytes,
    })),
    ...locals.slice(0, 2).map((font, index) => ({
      family: index === 0 ? 'Helvetica Neue' : 'Optima',
      weight: '400',
      style: 'normal' as const,
      unicodeRange: null,
      format: 'woff2',
      bytes: font.bytes,
    })),
  ];
  const report = (
    family: string,
    request: FaceRequest,
    source: FontReport['source'],
    extra: Partial<FontReport> = {},
  ): FontReport => ({
    family,
    postscriptName: family.replace(/\s/g, ''),
    weight: request.weight,
    style: request.style,
    source,
    shipped: true,
    bytes: 1024,
    subset: true,
    demoted: false,
    restricted: false,
    unstated: false,
    repaired: null,
    refused: null,
    notice: source === 'local' || source === 'user',
    serves: [{ family, weight: request.weight, style: request.style }],
    licence: `${family} licence`,
    ...extra,
  });
  const faces: FontReport[] = [
    report('Barlow Condensed', REGULAR, 'embedded'),
    // The 400 file serves the 700 the text asks for: its own tables name a regular face.
    report('Barlow Condensed', REGULAR, 'embedded', {
      postscriptName: 'BarlowCondensed-Bold',
      demoted: true,
      serves: [{ family: 'Barlow Condensed', weight: 700, style: 'normal' }],
    }),
    ...(users.length > 0
      ? [
          report('Brand Sans', REGULAR, 'user', {
            serves: [
              { family: 'Brand Sans', weight: 400, style: 'normal' },
              { family: 'Brand Sans', weight: 700, style: 'normal' },
            ],
          }),
        ]
      : []),
    ...(googles.length > 0 ? [report('Roboto', REGULAR, 'google')] : []),
    ...(locals.length > 0 ? [report('Helvetica Neue', REGULAR, 'local')] : []),
    ...(restricted ? [report('Optima', REGULAR, 'local', { restricted: true })] : []),
  ];
  // Liberation Sans draws the faces the deck lists as `bundled`; the fake takes its files in this order.
  const drawn = deckFonts.filter((font) => font.source === 'bundled');
  const substitutes: SubstituteFace[] = [
    { family: 'Liberation Sans', postscriptName: 'LiberationSans', weight: 400, style: 'normal' as const },
    { family: 'Liberation Sans', postscriptName: 'LiberationSans-Bold', weight: 700, style: 'normal' as const },
  ]
    .filter((face) => drawn.some((font) => font.weight === face.weight && font.style === face.style))
    .slice(bySource(fonts, 'bundled').length);
  const slides = base.slides.map((slide) => {
    let html = slide.html;
    html = setFamily(html, 's1.title', quoted('Brand Sans', 'Liberation Sans'));
    html = setFamily(html, 's1.sub', quoted('Roboto', 'Liberation Sans'));
    html = setFamily(html, 's2.title', quoted('Barlow Condensed', 'Liberation Sans'));
    html = setFamily(html, 's2.list', quoted('Helvetica Neue', 'Liberation Sans'));
    html = setFamily(html, 's4.title', quoted('Optima', 'Liberation Sans'));
    return { ...slide, html };
  });
  return { ...base, deck: { ...base.deck, fonts: deckFonts }, slides, fonts: shipped, faces, substitutes };
}

/** The installed faces among `fonts` the fake worker picks: the candidates, in their order. */
export function pickSampleLocal(fonts: readonly { postscriptName: string }[]): LocalPick[] {
  return LOCAL_CANDIDATES.flatMap((name) => {
    const index = fonts.findIndex((font) => font.postscriptName === name);
    return index < 0
      ? []
      : [
          {
            index,
            postscriptName: name,
            family: name === 'HelveticaNeue' ? 'Helvetica Neue' : 'Optima',
            exactPostscript: true,
          },
        ];
  });
}

const FONT_TAGS = new Set(['\u0000\u0001\u0000\u0000', 'OTTO', 'true', 'ttcf', 'wOFF', 'wOF2']);

/** Refuses a file that does not start like a font file, as the worker refuses what is no font. */
export function refuseNonFont(font: FakeFont): string | null {
  const tag = String.fromCharCode(...new Uint8Array(font.bytes.slice(0, 4)));
  return FONT_TAGS.has(tag) ? null : 'not a font file';
}

/** A fake worker script that compiles the sample deck with its font story. */
export function sampleFontsScript(extra: FakeWorkerScript = {}): FakeWorkerScript {
  return {
    version: '9.9.9',
    deck: sampleFontsDeck,
    needs: sampleFontNeeds,
    pickLocal: pickSampleLocal,
    refuseFont: refuseNonFont,
    diagnostics: SAMPLE_DIAGNOSTICS,
    ...extra,
  };
}
