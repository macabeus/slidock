// What the fonts panel lists: each family the deck uses, each face it draws and where that face's file comes from,
// the faces nothing provides yet, and what the licence policy says about the
// download.
//
// The worker's face reports say which file draws which face: a shipped face lists the deck faces it draws (`serves`,
// by drawn weight, with the file's own slant). The deck's `fonts` name one source per family, so a face is labelled
// from the report that draws it, and from the deck's `fonts` only where no report does (a substitute, no font at all,
// a deck opened without fonts).
import type { Deck, DeckFont, FaceRequest, FontNeed, FontReport, FontSource } from '@slidock/types';

export interface FamilyFace {
  weight: number;
  style: FaceRequest['style'];
  /** The source of the file that draws it (for a synthesized face, of the file the browser emboldens or slants). */
  source: FontSource;
  /** An embedded file that names another face, used because no source had this one. */
  demoted: boolean;
  /**
   * Its file's licence restricts embedding it: a substitute draws it (its source is a stand-in), or the slides here
   * draw with it and the download leaves it out unless the user includes its family.
   */
  restricted: boolean;
  /**
   * A substitute draws it although a source has a file for it: why that file never binds (browsers would refuse it,
   * or its licence keeps it out), and where the file came from. `null` when no held file was refused.
   */
  refused: { reason: string; source: FontSource } | null;
  /** No file has this face: the browser emboldens or slants another face of a file (`from`). */
  synthesized: boolean;
  /** The face of the file that draws it, when that is another weight or slant (a synthesized or demoted face). */
  from: FaceRequest | null;
  /** No source has this face (the worker's needs list it). */
  open: boolean;
}

export interface FamilyRow {
  family: string;
  faces: FamilyFace[];
  /** Faces the text asks for that no source has and nothing in the deck draws. */
  missing: FaceRequest[];
  /** It ships a face read from this computer or added by the user: the download notice and toggle apply. */
  personal: boolean;
  /**
   * It draws with a file whose licence restricts embedding it: the download leaves it out unless the user includes it
   * (the toggle of a personal family), and draws it with the viewer's installed font or a substitute.
   */
  restricted: boolean;
  /** It stays out of the download: the user left it out, or it is restricted and the user did not include it. */
  leftOut: boolean;
  /** Its restricted file states no embedding permissions (it has no `OS/2` table), so it is treated as restricted. */
  unstated: boolean;
  /** The table a file it draws with lacked that browsers require (`OS/2`), which the compiler wrote so they load it. */
  repaired: string | null;
}

/** Sources whose files the user may not be allowed to share: they ship with a notice and a per-family toggle. */
export const PERSONAL_SOURCES: ReadonlySet<FontSource> = new Set(['local', 'user']);

/** What draws a face that no font of the deck's family provides. */
export const STAND_INS: ReadonlySet<FontSource> = new Set(['bundled', 'system']);

/** A face of an emoji family: no file ships, and the viewer's emoji font draws it. */
export const EMOJI_SOURCE: FontSource = 'emoji';

/** A family's match key, as `FontNeed.key` spells it: ASCII lowercased, whitespace and hyphens removed. */
function familyKey(family: string): string {
  return family.replace(/[A-Z]/g, (letter) => letter.toLowerCase()).replace(/[\s-]/g, '');
}

function sameFace(a: FaceRequest, b: FaceRequest): boolean {
  return a.weight === b.weight && a.style === b.style;
}

interface Drawer {
  report: FontReport;
  /** It serves the face in the slant asked for; otherwise the browser slants (or unslants) it. */
  exact: boolean;
}

/**
 * The shipped report that draws `family`'s `face`: one that serves that face, else one that serves the other slant at
 * that weight (a report names the file's own slant). `null` when no shipped file draws that weight of the family.
 */
function drawer(reports: readonly FontReport[], family: string, face: FaceRequest): Drawer | null {
  const key = familyKey(family);
  const serving = (style: FaceRequest['style']) =>
    reports.find(
      (report) =>
        report.shipped &&
        report.serves.some(
          (served) => served.weight === face.weight && served.style === style && familyKey(served.family) === key,
        ),
    );
  const exact = serving(face.style);
  if (exact) {
    return { report: exact, exact: true };
  }
  const slanted = serving(face.style === 'italic' ? 'normal' : 'italic');
  return slanted ? { report: slanted, exact: false } : null;
}

/**
 * The report of the file held for the face that never binds, when a substitute draws the face for that reason
 * (browsers would refuse the file, or its licence keeps it out). The file is named by its own family or its
 * PostScript name, as the binder looks it up (a Keynote deck names its fonts by PostScript name).
 */
function refusedReport(family: string, face: FaceRequest, reports: readonly FontReport[]): FontReport | undefined {
  const key = familyKey(family);
  return reports.find(
    (report) =>
      report.refused !== null &&
      !report.shipped &&
      (familyKey(report.family) === key || familyKey(report.postscriptName) === key) &&
      sameFace(report, face),
  );
}

/**
 * `family`'s `face` as `found` draws it. `open` says no source has that face (the worker's needs list it), so a file
 * that draws it is another face of the family, emboldened or slanted; a demoted file says so itself.
 */
function drawnFace(family: string, face: FaceRequest, found: Drawer, open: boolean): FamilyFace {
  const { report, exact } = found;
  // A demoted file is mislabeled where it stands in for another family or weight; slanted for its own face, it is not.
  const own = familyKey(report.family) === familyKey(family) && report.weight === face.weight;
  const demoted = report.demoted && !own;
  const synthesized = !demoted && report.source !== 'bundled' && (!exact || open);
  return {
    weight: face.weight,
    style: face.style,
    source: report.source,
    demoted,
    restricted: report.restricted,
    refused: null,
    synthesized,
    from: (demoted || synthesized) && !sameFace(report, face) ? { weight: report.weight, style: report.style } : null,
    open,
  };
}

/**
 * `font`, a face of the deck's `fonts`, as the panel shows it; `null` for a face no file draws. For a face only a file
 * of the other slant could draw that the needs do not list, the `Drawer` of that file: a report names the file's own
 * slant, so it does not say whether the text draws the face slanted, and the face is listed only where a need draws as
 * it (`familyRows`).
 */
function deckFace(font: DeckFont, reports: readonly FontReport[], open: boolean): FamilyFace | Drawer | null {
  const found = drawer(reports, font.family, font);
  if (found) {
    return found.exact || open ? drawnFace(font.family, font, found, open) : found;
  }
  // The reports list every weight a shipped file draws, so a face the deck gives a file for that none of them draws
  // is one the text never draws. Without reports (a deck opened without fonts) the deck's `fonts` are all there is.
  // An emoji face has no file: the viewer's emoji font draws it.
  if (!STAND_INS.has(font.source) && font.source !== EMOJI_SOURCE && reports.some((report) => report.shipped)) {
    return null;
  }
  const refused = STAND_INS.has(font.source) ? refusedReport(font.family, font, reports) : undefined;
  return {
    weight: font.weight,
    style: font.style,
    source: font.source,
    demoted: false,
    restricted: refused?.restricted ?? false,
    refused:
      refused === undefined || refused.refused === null ? null : { reason: refused.refused, source: refused.source },
    synthesized: false,
    from: null,
    open,
  };
}

/**
 * Every family the deck draws or still needs, in the deck's order, then the families only the needs name. `reports`
 * are the deck message's `faces`. A face the needs list that a shipped file draws (a demoted file, a face emboldened
 * or slanted) is listed with that file; a face nothing in the deck draws is missing. `leaveOut` and `include` are the
 * user's download toggles.
 */
export function familyRows(
  deck: Deck,
  reports: readonly FontReport[],
  needs: readonly FontNeed[],
  leaveOut: ReadonlySet<string>,
  include: ReadonlySet<string> = new Set(),
): FamilyRow[] {
  const open = new Map<string, FaceRequest[]>();
  for (const need of needs) {
    const key = familyKey(need.family);
    open.set(key, [...(open.get(key) ?? []), ...need.faces]);
  }
  const isOpen = (family: string, face: FaceRequest) =>
    (open.get(familyKey(family)) ?? []).some((needed) => sameFace(needed, face));
  const shipping = reports.some((report) => report.shipped);

  const rows = new Map<string, FamilyRow>();
  const row = (family: string): FamilyRow => {
    let entry = rows.get(family);
    if (!entry) {
      entry = {
        family,
        faces: [],
        missing: [],
        personal: false,
        restricted: false,
        leftOut: false,
        unstated: false,
        repaired: null,
      };
      rows.set(family, entry);
    }
    return entry;
  };
  const listed = (entry: FamilyRow, face: FaceRequest) => entry.faces.some((drawn) => sameFace(drawn, face));

  /** Per family, the deck's faces only a file of the other slant could draw, that no need asks for yet. */
  const slanted = new Map<string, { font: DeckFont; found: Drawer }[]>();
  for (const font of deck.fonts) {
    const entry = row(font.family);
    const face = deckFace(font, reports, isOpen(font.family, font));
    if (face && 'report' in face) {
      slanted.set(font.family, [...(slanted.get(font.family) ?? []), { font, found: face }]);
    } else if (face && !listed(entry, face)) {
      entry.faces.push(face);
    }
  }
  for (const need of needs) {
    if (need.faces.length === 0) {
      continue;
    }
    const entry = row(need.family);
    for (const face of need.faces) {
      if (listed(entry, face)) {
        continue;
      }
      const found = drawer(reports, need.family, face);
      // Needs name requested weights and the deck's `fonts` drawn ones ("Roboto Medium" 400 draws at 500): a face the
      // deck lists in the same slant is the one this request draws as.
      const drawnAs = entry.faces.find((drawn) => drawn.style === face.style);
      const slantedAs = (slanted.get(need.family) ?? []).find(({ font }) => font.style === face.style);
      if (found) {
        entry.faces.push(drawnFace(need.family, face, found, true));
      } else if (drawnAs) {
        drawnAs.open = true;
      } else if (slantedAs && !listed(entry, slantedAs.font)) {
        entry.faces.push(drawnFace(need.family, slantedAs.font, slantedAs.found, true));
      } else {
        entry.missing.push(face);
      }
    }
  }
  const order = (a: FaceRequest, b: FaceRequest) =>
    a.weight - b.weight || Number(a.style === 'italic') - Number(b.style === 'italic');
  for (const entry of rows.values()) {
    entry.faces.sort(order);
    const key = familyKey(entry.family);
    const draws = (report: FontReport) =>
      report.shipped && report.serves.some((served) => familyKey(served.family) === key);
    entry.personal = shipping
      ? reports.some((report) => draws(report) && report.notice)
      : entry.faces.some((face) => PERSONAL_SOURCES.has(face.source));
    entry.restricted = reports.some((report) => draws(report) && report.restricted);
    entry.unstated = reports.some((report) => draws(report) && report.restricted && report.unstated);
    entry.repaired = reports.find((report) => draws(report) && report.repaired !== null)?.repaired ?? null;
    entry.leftOut = leaveOut.has(entry.family) || (entry.restricted && !include.has(entry.family));
  }
  return [...rows.values()].filter((entry) => entry.faces.length > 0 || entry.missing.length > 0);
}

/** The families whose shipped faces come from this computer or the user's files, and that still ship. */
export function personalFamilies(rows: readonly FamilyRow[]): string[] {
  return rows.filter((row) => row.personal && !row.leftOut).map((row) => row.family);
}

/** The families with a face no source has that no file of the deck draws: it is missing, or a substitute draws it. */
export function missingFamilies(rows: readonly FamilyRow[]): string[] {
  return rows
    .filter((row) => row.missing.length > 0 || row.faces.some((face) => face.open && STAND_INS.has(face.source)))
    .map((row) => row.family);
}
