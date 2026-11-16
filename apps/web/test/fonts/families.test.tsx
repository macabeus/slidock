// The fonts panel's labels follow the worker's face reports (src/fonts/families.ts). The cases are shaped like what
// the worker reports for a deck that embeds Barlow Condensed, Oswald and Roboto faces, some of them mislabelled
// (Google Slides embeds a regular file for "Roboto Medium"), with Google Fonts on, off, and a dropped file.
import type { Deck, DeckFont, FaceRequest, FontNeed, FontReport, ServedFace } from '@slidock/types';
import { cleanup, render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { type FamilyRow, familyRows, missingFamilies, personalFamilies } from '../../src/fonts/families';
import { sampleDeck } from '../../src/testing/sample-deck';
import { FontsPanel } from '../../src/views/FontsPanel';

type Style = FaceRequest['style'];

const n = (weight: number): FaceRequest => ({ weight, style: 'normal' });
const i = (weight: number): FaceRequest => ({ weight, style: 'italic' });

function deckOf(fonts: [string, FaceRequest, DeckFont['source']][]): Deck {
  return {
    ...sampleDeck().deck,
    fonts: fonts.map(([family, face, source]) => ({ family, ...face, source, licence: null })),
  };
}

function served(family: string, ...faces: FaceRequest[]): ServedFace[] {
  return faces.map((face) => ({ family, ...face }));
}

function report(
  family: string,
  postscriptName: string,
  face: FaceRequest,
  source: FontReport['source'],
  serves: ServedFace[],
  extra: Partial<FontReport> = {},
): FontReport {
  return {
    family,
    postscriptName,
    weight: face.weight,
    style: face.style,
    source,
    shipped: true,
    bytes: 1000,
    subset: true,
    demoted: false,
    restricted: false,
    unstated: false,
    repaired: null,
    refused: null,
    notice: source === 'local' || source === 'user',
    serves,
    licence: 'OFL-1.1',
    ...extra,
  };
}

function need(family: string, faces: FaceRequest[], substitute?: string): FontNeed {
  return {
    family,
    key: family.toLowerCase().replace(/[\s-]/g, ''),
    faces,
    postscriptNames: [],
    embedded: substitute === undefined,
    ...(substitute === undefined ? {} : { substitute }),
  };
}

/** Each row's faces as the panel's pills read. */
function labels(rows: readonly FamilyRow[]): Record<string, string[]> {
  const label = (face: { weight: number; style: Style }) => `${face.weight}${face.style === 'italic' ? 'i' : ''}`;
  return Object.fromEntries(
    rows.map((row) => [
      row.family,
      [
        ...row.faces.map((face) => {
          let text = `${label(face)} ${face.synthesized ? 'synthesized' : face.source}`;
          if (face.from) {
            text += ` from ${label(face.from)}`;
          }
          if (face.demoted) {
            text += ' mislabeled';
          }
          if (face.restricted) {
            text += ' restricted';
          }
          return text;
        }),
        ...row.missing.map((face) => `${label(face)} missing`),
      ],
    ]),
  );
}

/** The deck's `fonts`: one source per family, the requested faces at their drawn weights. */
function deck(sources: Record<string, DeckFont['source']>): Deck {
  const four = [n(400), i(400), n(700), i(700)];
  const medium = [n(500), i(500), n(700), i(700)];
  return deckOf([
    ['Arial', n(400), 'bundled'],
    ['Arial', n(700), 'bundled'],
    ...four.map((face): [string, FaceRequest, DeckFont['source']] => ['Barlow Condensed', face, 'embedded']),
    ['Oswald', n(400), 'embedded'],
    ['Oswald', n(700), 'embedded'],
    ...four.map((face): [string, FaceRequest, DeckFont['source']] => ['Roboto', face, 'embedded']),
    ...medium.map((face): [string, FaceRequest, DeckFont['source']] => [
      'Roboto Medium',
      face,
      sources['Roboto Medium'] ?? 'embedded',
    ]),
  ]);
}

const LIBERATION = [
  report('Liberation Sans', 'LiberationSans', n(400), 'bundled', served('Arial', n(400))),
  report('Liberation Sans', 'LiberationSans-Bold', n(700), 'bundled', served('Arial', n(700))),
];
const ARIAL_NEED = need('Arial', [n(400), n(700)], 'Liberation Sans');

/** Google Fonts on: Google's files draw the faces the embedded ones lack. */
const GOOGLE_ON = {
  deck: deck({ 'Roboto Medium': 'google' }),
  reports: [
    report('Barlow Condensed', 'BarlowCondensed-Regular', n(400), 'embedded', served('Barlow Condensed', n(400))),
    report('Barlow Condensed', 'BarlowCondensed-Bold', n(700), 'google', served('Barlow Condensed', n(700))),
    ...LIBERATION,
    report('Oswald', 'Oswald-Regular', n(400), 'embedded', served('Oswald', n(400))),
    report('Oswald', 'Oswald-Bold', n(700), 'embedded', served('Oswald', n(700))),
    report('Roboto', 'Roboto-Regular', n(400), 'embedded', served('Roboto', n(400))),
    report('Roboto', 'Roboto-Medium', n(500), 'google', served('Roboto Medium', n(500))),
    report('Roboto', 'Roboto-Bold', n(700), 'embedded', served('Roboto', n(700))),
  ],
  needs: [ARIAL_NEED],
};

/** The regular file embedded for "Roboto Medium" fails its identity check, and draws it only because nothing else does. */
const DEMOTED_ROBOTO = report(
  'Roboto',
  'Roboto-Regular',
  n(400),
  'embedded',
  [...served('Roboto', n(400), n(500)), ...served('Roboto Medium', n(400), n(500))],
  { demoted: true },
);

/** Google Fonts off: the embedded regular files draw the bold and medium faces. */
const GOOGLE_OFF = {
  deck: deck({}),
  reports: [
    report(
      'Barlow Condensed',
      'BarlowCondensed-Regular',
      n(400),
      'embedded',
      served('Barlow Condensed', n(400), n(700)),
    ),
    ...LIBERATION,
    report(
      'Oswald',
      'Oswald-Regular',
      n(400),
      'embedded',
      [...served('Oswald', n(400), n(500)), ...served('Oswald Medium', n(400), n(500))],
      { demoted: true },
    ),
    report('Oswald', 'Oswald-Bold', n(700), 'embedded', served('Oswald', n(700))),
    DEMOTED_ROBOTO,
    report('Roboto', 'Roboto-Bold', n(700), 'embedded', served('Roboto', n(700))),
  ],
  needs: [
    ARIAL_NEED,
    need('Barlow Condensed', [n(700), i(700)]),
    need('Oswald Medium', [n(400)]),
    need('Roboto Medium', [n(400), i(400)]),
  ],
};

/** Google Fonts off and a static Roboto Medium (500, upright) dropped. */
const DROPPED = {
  deck: deck({ 'Roboto Medium': 'user' }),
  reports: [
    ...GOOGLE_OFF.reports.filter((face) => face !== DEMOTED_ROBOTO),
    report('Roboto', 'Roboto-Regular', n(400), 'embedded', served('Roboto', n(400))),
    report('Roboto', 'Roboto-Medium', n(500), 'user', served('Roboto Medium', n(500))),
  ],
  needs: [ARIAL_NEED, need('Barlow Condensed', [n(700), i(700)]), need('Roboto Medium', [i(400)])],
};

const rowsOf = (state: { deck: Deck; reports: FontReport[]; needs: FontNeed[] }, leaveOut = new Set<string>()) =>
  familyRows(state.deck, state.reports, state.needs, leaveOut);

describe('fonts panel labels', () => {
  it('labels a face by the report that draws it, not by the source the deck names for its family', () => {
    // The deck names the embedded file for every Barlow Condensed face; Google's bold file draws the 700.
    expect(labels(rowsOf(GOOGLE_ON))['Barlow Condensed']).toEqual(['400 embedded', '700 google']);
    expect(labels(rowsOf(GOOGLE_ON))['Arial']).toEqual(['400 bundled', '700 bundled']);
  });

  it('lists a face no file draws nowhere, and never under the source the deck names for its family', () => {
    // No report serves Roboto Medium at 700: the text never draws it.
    expect(labels(rowsOf(GOOGLE_ON))['Roboto Medium']).toEqual(['500 google']);
  });

  it('a face only a file of the other slant could draw is listed only where the needs ask for it', () => {
    // The deck names an italic face for each family, but the needs ask for none: the text draws no italic, so the
    // upright files' reports (which name their own slant) say nothing about one. The CLI's deck names such a face as
    // `system`, the app's under the family's source; neither is listed.
    const system = deckOf([
      ['Roboto', n(400), 'embedded'],
      ['Roboto', i(400), 'system'],
      ['Roboto', n(700), 'embedded'],
      ['Roboto', i(700), 'system'],
    ]);
    const upright = [
      report('Roboto', 'Roboto-Regular', n(400), 'embedded', served('Roboto', n(400))),
      report('Roboto', 'Roboto-Bold', n(700), 'embedded', served('Roboto', n(700))),
    ];
    expect(labels(familyRows(system, upright, [], new Set()))['Roboto']).toEqual(['400 embedded', '700 embedded']);
    // Asked for (no source has it), the upright file draws it slanted; a need at another weight of that slant draws
    // as the deck's italic face.
    expect(labels(familyRows(system, upright, [need('Roboto', [i(400)])], new Set()))['Roboto']).toEqual([
      '400 embedded',
      '400i synthesized from 400',
      '700 embedded',
    ]);
    expect(labels(familyRows(system, upright, [need('Roboto', [i(300)])], new Set()))['Roboto']).toEqual([
      '400 embedded',
      '400i synthesized from 400',
      '700 embedded',
    ]);
  });

  it('a dropped file is listed only on the faces it has; a face it is slanted for is synthesized', () => {
    // The needs still name 400 italic (no source has it): it is requested at 400 and drawn at 500, the 500 italic.
    expect(labels(rowsOf(DROPPED))['Roboto Medium']).toEqual(['500 user', '500i synthesized from 500']);
  });

  it('a need at a requested weight is the face the deck lists in its slant, and a substitute drawing it is missing', () => {
    const narrow = deckOf([['Helvetica Neue Medium', n(500), 'bundled']]);
    const rows = familyRows(
      narrow,
      LIBERATION,
      [need('Helvetica Neue Medium', [n(400)], 'Liberation Sans')],
      new Set(),
    );
    expect(labels(rows)['Helvetica Neue Medium']).toEqual(['500 bundled']);
    expect(missingFamilies(rows)).toEqual(['Helvetica Neue Medium']);
  });

  it('a face the needs list that a demoted embedded file draws is that file, mislabeled, not missing', () => {
    const rows = labels(rowsOf(GOOGLE_OFF));
    expect(rows['Roboto Medium']).toEqual([
      '400 embedded mislabeled',
      '400i embedded from 400 mislabeled',
      '500 embedded from 400 mislabeled',
    ]);
    expect(rows['Oswald Medium']).toEqual(['400 embedded mislabeled']);
    // The demoted file draws its own face as itself.
    expect(rows['Roboto']).toEqual(['400 embedded', '700 embedded']);
    expect(rows['Oswald']).toEqual(['400 embedded', '700 embedded']);
  });

  it('a face no source has that a file of the family draws is synthesized from that file', () => {
    expect(labels(rowsOf(GOOGLE_OFF))['Barlow Condensed']).toEqual([
      '400 embedded',
      '700 synthesized from 400',
      '700i synthesized from 400',
    ]);
  });

  it('the banner lists families with a missing face or a face a substitute draws, not those their files draw', () => {
    expect(missingFamilies(rowsOf(GOOGLE_ON))).toEqual(['Arial']);
    // Barlow Condensed, Oswald Medium and Roboto Medium have open faces, but files of the deck draw them all.
    expect(missingFamilies(rowsOf(GOOGLE_OFF))).toEqual(['Arial']);
    expect(missingFamilies(rowsOf(DROPPED))).toEqual(['Arial']);
    // A substitute's own family drawn by its bundled file is not missing.
    const liberation = deckOf([['Liberation Sans', n(400), 'bundled']]);
    expect(missingFamilies(familyRows(liberation, [], [], new Set()))).toEqual([]);
  });

  it('a family only the needs name is listed with its missing faces', () => {
    const rows = familyRows(
      GOOGLE_ON.deck,
      GOOGLE_ON.reports,
      [need('Gotham', [n(400)], 'Liberation Sans')],
      new Set(),
    );
    expect(labels(rows)['Gotham']).toEqual(['400 missing']);
    expect(missingFamilies(rows)).toEqual(['Gotham']);
  });

  it('without reports (a deck opened without fonts) the deck’s sources are the labels', () => {
    const rows = familyRows(GOOGLE_OFF.deck, [], [], new Set());
    expect(labels(rows)['Roboto Medium']).toEqual(['500 embedded', '500i embedded', '700 embedded', '700i embedded']);
    const user = familyRows(DROPPED.deck, [], [], new Set());
    expect(personalFamilies(user)).toEqual(['Roboto Medium']);
  });

  it('the download notice covers the families a file from this computer or the user draws', () => {
    expect(personalFamilies(rowsOf(DROPPED))).toEqual(['Roboto Medium']);
    expect(personalFamilies(rowsOf(DROPPED, new Set(['Roboto Medium'])))).toEqual([]);
    expect(personalFamilies(rowsOf(GOOGLE_ON))).toEqual([]);
  });

  it('a face whose licence restricts embedding draws, and stays out of the download until included', () => {
    const deck = deckOf([['8BITWONDERNominal', n(400), 'local']]);
    const reports = [
      report('8BIT WONDER', '8BITWONDERNominal', n(400), 'local', served('8BITWONDERNominal', n(400)), {
        restricted: true,
      }),
    ];
    const rows = familyRows(deck, reports, [], new Set());
    expect(labels(rows)['8BITWONDERNominal']).toEqual(['400 local restricted']);
    expect(rows[0]).toMatchObject({ personal: true, restricted: true, leftOut: true });
    expect(missingFamilies(rows)).toEqual([]);
    expect(personalFamilies(rows)).toEqual([]);
    const included = familyRows(deck, reports, [], new Set(), new Set(['8BITWONDERNominal']));
    expect(included[0]).toMatchObject({ restricted: true, leftOut: false });
    expect(personalFamilies(included)).toEqual(['8BITWONDERNominal']);
    // Leaving out wins, as in the bundle.
    const both = familyRows(deck, reports, [], new Set(['8BITWONDERNominal']), new Set(['8BITWONDERNominal']));
    expect(both[0]?.leftOut).toBe(true);
  });

  it('a substitute that draws a face whose held file never binds for its licence says so, by PostScript name too', () => {
    const refused = report('8BIT WONDER', '8BITWONDERNominal', n(400), 'local', [], {
      shipped: false,
      restricted: true,
      refused: 'its fsType allows bitmap embedding only',
      notice: false,
    });
    const rows = familyRows(deckOf([['8BITWONDERNominal', n(400), 'bundled']]), [refused], [], new Set());
    expect(labels(rows)['8BITWONDERNominal']).toEqual(['400 bundled restricted']);
    expect(rows[0]?.restricted).toBe(false);
  });

  it('a substitute that draws a face whose held file its licence keeps out says so', () => {
    const rows = familyRows(
      deckOf([['Optima', n(400), 'bundled']]),
      [
        report('Optima', 'Optima-Regular', n(400), 'local', [], {
          shipped: false,
          restricted: true,
          refused: 'its fsType allows restricted-licence embedding only',
          notice: false,
        }),
      ],
      [],
      new Set(),
    );
    expect(labels(rows)['Optima']).toEqual(['400 bundled restricted']);
    expect(missingFamilies(rows)).toEqual([]);
  });
});

describe('fonts panel', () => {
  it('shows the labels, the banner, and each family name in the interface font', () => {
    render(
      <FontsPanel
        open
        onClose={() => undefined}
        deck={GOOGLE_OFF.deck}
        faces={GOOGLE_OFF.reports}
        needs={GOOGLE_OFF.needs}
        fonts={null}
      />,
    );
    const drawer = screen.getByRole('dialog', { name: 'Fonts' });
    expect(within(drawer).getByText("1 font isn't in the file: Arial")).toBeTruthy();
    const pills = (family: string) =>
      within(within(drawer).getByRole('list', { name: `Faces of ${family}` }))
        .getAllByRole('listitem')
        .map((item) => (item.textContent ?? '').replace(/\s*·\s*/g, ' · '));
    expect(pills('Oswald Medium')).toEqual(['400 · Embedded in the file · mislabeled']);
    expect(pills('Barlow Condensed')).toEqual([
      '400 · Embedded in the file',
      '700 · Synthesized from 400',
      '700 italic · Synthesized from 400',
    ]);
    for (const family of ['Barlow Condensed', 'Roboto Medium']) {
      const name = within(within(drawer).getByRole('listitem', { name: family })).getByText(family);
      expect(name.style.fontFamily, `${family} is not drawn with the deck's face`).toBe('');
    }
  });
});

describe('fonts panel: found but not drawn, repaired, emoji', () => {
  const panel = (deck: Deck, reports: FontReport[], needs: FontNeed[] = []) => {
    render(<FontsPanel open onClose={() => undefined} deck={deck} faces={reports} needs={needs} fonts={null} />);
    return screen.getByRole('dialog', { name: 'Fonts' });
  };
  const row = (drawer: HTMLElement, family: string) => within(drawer).getByRole('listitem', { name: family });

  it('a face a substitute draws because browsers would refuse the file found for it says so', () => {
    const refused = report('Courier', 'Courier', n(400), 'user', [], {
      shipped: false,
      refused: 'browsers would refuse its file (The face has no post table)',
      notice: false,
    });
    const drawer = panel(
      deckOf([['Courier', n(400), 'bundled']]),
      [refused],
      [need('Courier', [n(400)], 'Liberation Mono')],
    );
    expect(row(drawer, 'Courier').textContent).toContain(
      'Found in your font file, but browsers would refuse its file (The face has no post table): a substitute draws it.',
    );
    expect(within(row(drawer, 'Courier')).getByText(/file refused/)).toBeTruthy();
    // The banner still counts it: a substitute draws it.
    expect(within(drawer).getByText("1 font isn't in the file: Courier")).toBeTruthy();
  });

  it('negative control: without a refused file, a substitute face says nothing about a file', () => {
    const drawer = panel(deckOf([['Courier', n(400), 'bundled']]), [], [need('Courier', [n(400)], 'Liberation Mono')]);
    expect(row(drawer, 'Courier').textContent).not.toMatch(/Found|refused/);
  });

  it('a repaired face that states no licence draws here, and says what was written and why it is restricted', () => {
    const courier = report('Courier', 'Courier', n(400), 'local', served('Courier', n(400)), {
      restricted: true,
      unstated: true,
      repaired: 'OS/2',
    });
    const drawer = panel(deckOf([['Courier', n(400), 'local']]), [courier]);
    const text = row(drawer, 'Courier').textContent ?? '';
    expect(text).toContain(
      "Its file has no OS/2 table, which browsers require: slidock wrote one from the file's other tables, so browsers load it.",
    );
    expect(text).toContain(
      'Found on this computer, but it states no embedding permissions (its file has no OS/2 table), so it is treated as restricted: the slides here draw with it, and the download leaves it out',
    );
    // Negative control: a face whose fsType restricts it says its licence does.
    cleanup();
    const stated = panel(deckOf([['Courier', n(400), 'local']]), [{ ...courier, unstated: false, repaired: null }]);
    const plain = row(stated, 'Courier').textContent ?? '';
    expect(plain).toContain('Found on this computer, but its licence restricts embedding it');
    expect(plain).not.toMatch(/OS\/2/);
  });

  it('an emoji family is drawn by the viewer’s emoji font and is not missing', () => {
    const emojiDeck = deckOf([['Apple Color Emoji', n(400), 'emoji']]);
    const rows = familyRows(
      emojiDeck,
      [report('Helvetica', 'Helvetica', n(400), 'local', served('Helvetica', n(400)))],
      [],
      new Set(),
    );
    expect(labels(rows)['Apple Color Emoji']).toEqual(['400 emoji']);
    expect(missingFamilies(rows)).toEqual([]);
    const drawer = panel(emojiDeck, []);
    expect(within(row(drawer, 'Apple Color Emoji')).getByText(/The viewer's emoji font/)).toBeTruthy();
    expect(row(drawer, 'Apple Color Emoji').textContent).toContain('An emoji font: no file ships');
    // Negative control: the same family with no file and an open need is missing.
    const missing = familyRows(
      deckOf([['Apple Color Emoji', n(400), 'system']]),
      [],
      [need('Apple Color Emoji', [n(400)], 'Liberation Sans')],
      new Set(),
    );
    expect(missingFamilies(missing)).toEqual(['Apple Color Emoji']);
  });
});
