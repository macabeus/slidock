//! The faces a finished deck ships: every face the deck draws with that comes from a
//! font the session holds, subset to the code points drawn with it, encoded as WOFF2 and described by `@font-face`
//! rules that state the file's own weight and style. A face of the base catalog (built-in metrics, no file) ships
//! nothing: the viewer's fonts draw it.
//!
//! A shipped face's rules define the family it is bound as (the catalog's spelling), and the stack the backend prints
//! for each family the slides name starts with that family ([`font_stacks`]): a deck asking for Arial
//! prints `"Liberation Sans",sans-serif` over the rules of its Liberation Sans subset, so no font installed on the
//! viewer's computer can draw the text, and each subset ships once however the slides spell the families it draws.
//! The code points a shipped face lacks are drawn by a bundled last-resort face ([`LAST_RESORT`]), subset to them
//! alone and shipped once in its own family, which the stacks of the families it fills list next; those it lacks too
//! are coverage gaps (MID5415) the viewer's fonts draw (colour emoji). A symbol font
//! ([`SYMBOL_FAMILIES`](slidock_fonts::catalog::substitutes::SYMBOL_FAMILIES)) drawn with
//! another family's face keeps the backend's stack, its own name first: no other face draws its pictures.
//!
//! The code points come from the passes' collect-glyphs, which the host injects ([`CollectGlyphs`]): this crate knows
//! no pass. Rules claim weights so no weight falls in two rules of one family and style
//! ([`slidock_fonts::face_css::claimed_ranges`]).

use std::collections::{BTreeMap, BTreeSet};

use slidock_fonts::catalog::substitutes::{LAST_RESORT, is_symbol_family};
use slidock_fonts::face_css::{EmittedFace, claimed_ranges};
use slidock_fonts::glyphs::draws_no_glyph;
use slidock_fonts::google::{GoogleCatalog, UnicodeRange};
use slidock_fonts::policy::Exclusion;
use slidock_fonts::resolve::SourceOrder;
use slidock_fonts::{
    CodepointSet, Decision, FaceDescriptor, FaceRepair, FamilyKey, FontCatalog, FontFaceRule,
    FontNotice, LicenceEntry, ShipInput, ShipOptions, ShippedFont, Toggle, ship_face,
};
use slidock_ir::fonts::{FaceStyle, font_key};
use slidock_ir::ids::Sha256;
use slidock_ir::resolved;

use crate::session::{AddedFont, FontIssue, FontSource};

/// What a deck draws with one face of the catalog it was bound against.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlyphUse {
    /// The family the binding ships, as the catalog spells it.
    pub family: String,
    pub postscript_name: String,
    /// The face's own weight and slant, as the catalog holds them.
    pub weight: u32,
    pub italic: bool,
    /// The file it comes from; `None` for a face with no file.
    pub sha256: Option<Sha256>,
    pub codepoints: CodepointSet,
    /// The drawn weights it renders (a variable face renders several).
    pub weights: BTreeSet<u32>,
    pub small_caps: bool,
    /// The families the slides name that draw with it, as the requests spell them.
    pub requested: BTreeSet<String>,
    /// The code points of `codepoints` the face keeps whatever the text draws, and the text does not draw: a face
    /// lacking one leaves no gap.
    pub forced: CodepointSet,
}

/// Collects what the deck draws with each face: `slidock-passes`' collect-glyphs, which the host injects.
pub trait CollectGlyphs {
    /// `catalog` is the catalog the presentation's fonts were bound against.
    fn collect(
        &self,
        presentation: &resolved::Presentation,
        catalog: &FontCatalog,
    ) -> Result<Vec<GlyphUse>, String>;
}

/// A face of the deck a shipped file draws: the family as the slides name it, the drawn weight and the style, as the
/// deck JSON's `fonts` list it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServedFace {
    pub family: String,
    pub weight: u32,
    pub style: FaceStyle,
}

/// A face the deck ships.
#[derive(Clone, Debug, PartialEq)]
pub struct ShippedFace {
    /// Its `@font-face` rules, of the family it is bound as: one per weight range it claims (several for a variable
    /// face cut around static ones) and file.
    pub rules: Vec<FontFaceRule>,
    /// Its `LICENSES.txt` entry, which names the file's own family.
    pub licence: LicenceEntry,
    pub source: FontSource,
    /// It came from this computer or the user: the download ships it with a notice, and may leave its family out.
    pub notice: bool,
    /// The deck faces it draws; none for the last-resort face.
    pub serves: Vec<ServedFace>,
    /// The face's own weight and slant, as the catalog holds them.
    pub weight: u32,
    pub style: FaceStyle,
    pub postscript_name: String,
    /// Of the font file it was cut from.
    pub sha256: Sha256,
    /// Subset to the drawn code points (else whole).
    pub subset: bool,
    /// It draws a face no source has, as the embedded file declared for that face that failed its identity check.
    pub demoted: bool,
    /// What was written into its file so that browsers load it.
    pub repair: Option<FaceRepair>,
    /// The shipped files' size: one file, or one per `unicode-range` chunk of the face (and per rule for the
    /// last-resort face).
    pub bytes: u64,
    /// For the last-resort face, the families of the shipped faces whose missing code points it draws, as their rules
    /// spell them; empty for every other face.
    pub fills: Vec<String>,
}

/// The slots resolution left to an embedded file that failed its identity check: the requested family's key, the
/// drawn weight, italic, and the PostScript name of the file's face.
pub type DemotedSlots = BTreeSet<(FamilyKey, u32, bool, String)>;

/// The families the backend prints before the generic for each family the slides name, by font key: the family its
/// shipped faces are bound as, then the last-resort family when that fills code points they lack.
pub type FontStacks = BTreeMap<String, Vec<String>>;

/// What [`ship_faces`] shipped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shipping {
    /// The shipped faces, in the order of the glyph uses, then the last-resort face.
    pub faces: Vec<ShippedFace>,
    /// The problems met.
    pub issues: Vec<FontIssue>,
    /// A shipped face lacks code points the deck draws with it, and the session holds no last-resort file.
    pub wants_last_resort: bool,
}

/// A font's place among the sources that are on; the bundled floor comes after every source and is always on.
fn rank(font: &AddedFont, order: &SourceOrder) -> Option<usize> {
    match font.source.kind() {
        Some(kind) => order.rank(kind),
        None => Some(usize::MAX),
    }
}

/// The font a glyph use's file is, of the sources that are on: the first in the order, of one source's the first added.
fn font_of<'f>(
    fonts: &[&'f AddedFont],
    sha256: &Sha256,
    order: &SourceOrder,
) -> Option<&'f AddedFont> {
    fonts
        .iter()
        .filter_map(|font| Some((*font, rank(font, order)?)))
        .filter(|(font, _)| font.sha256 == *sha256)
        .min_by_key(|(_, rank)| *rank)
        .map(|(font, _)| font)
}

/// The bundled last-resort file the session holds, with the face's index: the first added.
fn last_resort<'f>(fonts: &[&'f AddedFont]) -> Option<(&'f AddedFont, u32)> {
    let (family, postscript_name) = LAST_RESORT;
    fonts
        .iter()
        .filter(|font| font.source == FontSource::Bundled)
        .find_map(|font| {
            let index = font.faces.iter().position(|face| {
                face.postscript_name == postscript_name && face.family == family
            })?;
            let refused = font.refused.get(index).is_some_and(Option::is_some);
            (!refused).then(|| (*font, u32::try_from(index).unwrap_or(u32::MAX)))
        })
}

/// The files a glyph use ships with the code points each holds: its font's, or for a `unicode-range` chunk every
/// chunk of the face from the same source, in range order, each with the drawn code points of its range. A chunk
/// that holds none of them ships nothing.
fn chunks<'f>(
    fonts: &[&'f AddedFont],
    font: &'f AddedFont,
    glyph: &GlyphUse,
) -> Vec<(&'f AddedFont, CodepointSet)> {
    if font.unicode_range.is_none() {
        return vec![(font, glyph.codepoints.clone())];
    }
    let mut chunks: Vec<(&AddedFont, &UnicodeRange)> = Vec::new();
    for &candidate in fonts {
        if let Some(range) = &candidate.unicode_range
            && candidate.source == font.source
            && candidate
                .faces
                .iter()
                .any(|face| face.postscript_name == glyph.postscript_name)
            && !chunks
                .iter()
                .any(|(chunk, _)| chunk.sha256 == candidate.sha256)
        {
            chunks.push((candidate, range));
        }
    }
    chunks.sort_by(|a, b| {
        a.1.ranges()
            .cmp(b.1.ranges())
            .then(a.0.sha256.cmp(&b.0.sha256))
    });
    let shipped: Vec<(&AddedFont, CodepointSet)> = chunks
        .into_iter()
        .filter_map(|(chunk, range)| {
            let points: CodepointSet = glyph
                .codepoints
                .iter()
                .filter(|&point| range.contains(point))
                .collect();
            (!points.is_empty()).then_some((chunk, points))
        })
        .collect();
    if shipped.is_empty() {
        vec![(font, glyph.codepoints.clone())]
    } else {
        shipped
    }
}

fn notice_reason(notice: &FontNotice) -> String {
    match notice {
        FontNotice::NoSubsetting => "its fsType forbids subsetting".to_owned(),
        FontNotice::SubsetFailed(error) => error.to_string(),
        FontNotice::EncodeFailed(error) => error.to_string(),
    }
}

/// The code points a glyph use draws that its shipped files lack: a coverage gap. Code points browsers show without
/// a glyph are none, and nor are those the face keeps that the text does not draw.
fn gap_of(glyph: &GlyphUse, files: &[ShippedFont]) -> CodepointSet {
    let mut covered = CodepointSet::new();
    for file in files {
        if let Some(range) = &file.rule.unicode_range {
            covered.extend(range);
        }
    }
    glyph
        .codepoints
        .difference(&covered)
        .iter()
        .filter(|&point| !draws_no_glyph(point) && !glyph.forced.contains(point))
        .collect()
}

/// A face on its way out: what it ships, its file's rules before claiming, and what it lacks.
struct Pending {
    face: ShippedFace,
    /// The rules of its files, one per file, with the file's own descriptor.
    files: Vec<FontFaceRule>,
    descriptor: FaceDescriptor,
    wins: BTreeSet<u32>,
    gap: CodepointSet,
}

/// Ships each face of `uses` that comes from one of `fonts`, in the order given, with the problems met. A shipped face
/// is demoted when it draws one of the `demoted` slots as the demoted file's face. The code points a face lacks ship
/// from the last-resort face when `fonts` holds its bundled file.
pub fn ship_faces(
    fonts: &[&AddedFont],
    uses: &[GlyphUse],
    demoted: &DemotedSlots,
    order: &SourceOrder,
    google: &GoogleCatalog,
) -> Shipping {
    let options = ShipOptions::default();
    let mut issues = Vec::new();
    let mut pending: Vec<Pending> = Vec::new();
    for glyph in uses {
        let Some(sha256) = glyph.sha256 else {
            continue;
        };
        let Some(font) = font_of(fonts, &sha256, order) else {
            continue;
        };
        let repair = font
            .faces
            .iter()
            .position(|face| face.postscript_name == glyph.postscript_name)
            .and_then(|index| font.repair(index));
        let source_licence = match font.source {
            FontSource::Google => google
                .find(&glyph.family)
                .map(|found| found.family.licence.spdx()),
            _ => None,
        };
        let mut files = Vec::new();
        for (chunk, codepoints) in chunks(fonts, font, glyph) {
            let index = chunk
                .faces
                .iter()
                .position(|face| face.postscript_name == glyph.postscript_name)
                .unwrap_or(0);
            let input = ShipInput {
                family: &glyph.family,
                bytes: &chunk.bytes,
                index: u32::try_from(index).unwrap_or(u32::MAX),
                origin: chunk.source.origin(),
                codepoints: &codepoints,
                small_caps: glyph.small_caps,
                source_licence,
            };
            match ship_face(&input, &options) {
                Ok(mut face) => {
                    // A chunk's rule claims only code points of its range, so the chunks' rules never overlap.
                    if let Some(range) = &chunk.unicode_range {
                        face.rule.unicode_range = face.rule.unicode_range.map(|covered| {
                            covered
                                .iter()
                                .filter(|&point| range.contains(point))
                                .collect()
                        });
                    }
                    files.push(face);
                }
                Err(error) => {
                    issues.push(FontIssue::NotShipped {
                        family: glyph.family.clone(),
                        postscript_name: glyph.postscript_name.clone(),
                        reason: error.to_string(),
                    });
                    files.clear();
                    break;
                }
            }
        }
        let Some(first) = files.first() else {
            continue;
        };
        let (descriptor, licence) = (first.rule.descriptor, first.licence.clone());
        let subset = files.iter().all(|file| file.subset);
        let mut notices: Vec<&FontNotice> = Vec::new();
        for notice in files.iter().flat_map(|file| &file.notices) {
            if !notices.contains(&notice) {
                notices.push(notice);
            }
        }
        for notice in notices {
            let (family, postscript_name, reason) = (
                glyph.family.clone(),
                glyph.postscript_name.clone(),
                notice_reason(notice),
            );
            issues.push(match notice {
                FontNotice::NoSubsetting | FontNotice::SubsetFailed(_) => FontIssue::ShippedWhole {
                    family,
                    postscript_name,
                    reason,
                },
                FontNotice::EncodeFailed(_) => FontIssue::ShippedRaw {
                    family,
                    postscript_name,
                    reason,
                },
            });
        }
        // A face the session's target binds that the download leaves out by default (a preview's restricted face).
        if let Decision::Exclude(exclusion @ (Exclusion::Restricted | Exclusion::BitmapOnly)) =
            licence.download(Toggle::Default)
        {
            issues.push(FontIssue::LicenceExcluded {
                family: glyph.family.clone(),
                postscript_name: glyph.postscript_name.clone(),
                source: font.source,
                bitmap_only: exclusion == Exclusion::BitmapOnly,
                unstated: repair == Some(FaceRepair::Os2),
                in_preview: true,
            });
        }
        let style = if glyph.italic {
            FaceStyle::Italic
        } else {
            FaceStyle::Normal
        };
        let serves: Vec<ServedFace> = glyph
            .requested
            .iter()
            .flat_map(|family| {
                glyph.weights.iter().map(move |&weight| ServedFace {
                    family: family.clone(),
                    weight,
                    style,
                })
            })
            .collect();
        let stands_in = serves.iter().any(|served| {
            demoted.contains(&(
                FamilyKey::of(&served.family),
                served.weight,
                glyph.italic,
                glyph.postscript_name.clone(),
            ))
        });
        let gap = gap_of(glyph, &files);
        let files: Vec<FontFaceRule> = files.into_iter().map(|file| file.rule).collect();
        pending.push(Pending {
            face: ShippedFace {
                bytes: files
                    .iter()
                    .map(|rule| u64::try_from(rule.bytes.len()).unwrap_or(u64::MAX))
                    .sum(),
                rules: Vec::new(),
                fills: Vec::new(),
                licence,
                source: font.source,
                notice: matches!(font.source, FontSource::Local | FontSource::User),
                serves,
                weight: glyph.weight,
                style,
                postscript_name: glyph.postscript_name.clone(),
                sha256,
                subset,
                demoted: stands_in,
                repair,
            },
            files,
            descriptor,
            wins: glyph.weights.clone(),
            gap,
        });
    }
    claim(&mut pending);
    let (last, gap_issues, wants_last_resort) = fill_gaps(fonts, &pending, &options);
    issues.extend(gap_issues);
    let mut faces: Vec<ShippedFace> = pending
        .into_iter()
        .map(|pending| pending.face)
        .filter(|face| !face.rules.is_empty())
        .collect();
    faces.extend(last);
    Shipping {
        faces,
        issues,
        wants_last_resort,
    }
}

/// The family a pending face is bound as, as its files' rules spell it.
fn family_of(face: &Pending) -> Option<&str> {
    face.files.first().map(|rule| rule.family.as_str())
}

/// Gives each pending face the rules of the weight ranges it claims among the faces of its family (by key): one per
/// range and file.
fn claim(pending: &mut [Pending]) {
    let mut families: BTreeMap<FamilyKey, Vec<usize>> = BTreeMap::new();
    for (index, face) in pending.iter().enumerate() {
        if let Some(family) = family_of(face) {
            families
                .entry(FamilyKey::of(family))
                .or_default()
                .push(index);
        }
    }
    for members in families.values() {
        let emitted: Vec<EmittedFace> = members
            .iter()
            .filter_map(|&index| pending.get(index))
            .map(|face| EmittedFace {
                descriptor: face.descriptor,
                wins: face.wins.clone(),
            })
            .collect();
        for (&index, ranges) in members.iter().zip(claimed_ranges(&emitted)) {
            let Some(face) = pending.get_mut(index) else {
                continue;
            };
            let style = face.descriptor.style;
            let rules: Vec<FontFaceRule> = ranges
                .into_iter()
                .flat_map(|weight| {
                    face.files.iter().map(move |rule| FontFaceRule {
                        descriptor: FaceDescriptor { weight, style },
                        ..rule.clone()
                    })
                })
                .collect();
            face.face.rules = rules;
        }
    }
}

/// The first code points of a coverage gap a diagnostic quotes.
const QUOTED_GAP: usize = 16;

fn coverage_gap(face: &ShippedFace, missing: &CodepointSet) -> FontIssue {
    FontIssue::CoverageGap {
        family: face.licence.family.clone(),
        postscript_name: face.postscript_name.clone(),
        missing: missing.len(),
        code_points: missing.iter().take(QUOTED_GAP).collect(),
    }
}

/// The last-resort face for the code points the pending faces lack: subset to them once, in the last-resort family
/// with that union as its `unicode-range`. Its rules take the descriptors of the family's shipped faces when the deck
/// draws with that family too (so the two compose, as CSS combines rules of one descriptor), else the face's own: one
/// rule, which CSS matches for every weight and style. Returns it (when it ships), the coverage gaps it leaves, and
/// whether a gap wanted a last-resort file the session does not hold.
fn fill_gaps(
    fonts: &[&AddedFont],
    pending: &[Pending],
    options: &ShipOptions,
) -> (Option<ShippedFace>, Vec<FontIssue>, bool) {
    let shipped_faces = || pending.iter().filter(|face| !face.face.rules.is_empty());
    let mut wanted = CodepointSet::new();
    for face in shipped_faces() {
        wanted.extend(&face.gap);
    }
    if wanted.is_empty() {
        return (None, Vec::new(), false);
    }
    let gaps = |covered: &CodepointSet| -> Vec<FontIssue> {
        shipped_faces()
            .filter_map(|face| {
                let missing = face.gap.difference(covered);
                (!missing.is_empty()).then(|| coverage_gap(&face.face, &missing))
            })
            .collect()
    };
    let Some((font, index)) = last_resort(fonts) else {
        return (None, gaps(&CodepointSet::new()), true);
    };
    let (family, postscript_name) = LAST_RESORT;
    let input = ShipInput {
        family,
        bytes: &font.bytes,
        index,
        origin: font.source.origin(),
        codepoints: &wanted,
        small_caps: false,
        source_licence: None,
    };
    let shipped = match ship_face(&input, options) {
        Ok(shipped) => shipped,
        Err(error) => {
            let mut issues = vec![FontIssue::NotShipped {
                family: family.to_owned(),
                postscript_name: postscript_name.to_owned(),
                reason: error.to_string(),
            }];
            issues.extend(gaps(&CodepointSet::new()));
            return (None, issues, false);
        }
    };
    let covered = shipped
        .rule
        .unicode_range
        .clone()
        .unwrap_or_default()
        .intersection(&wanted);
    let issues = gaps(&covered);
    if covered.is_empty() {
        return (None, issues, false);
    }
    let key = FamilyKey::of(family);
    // The faces it fills, by CSS family (ASCII case-insensitive, as CSS matches names); its own family needs no fill.
    let mut fills: BTreeMap<String, String> = BTreeMap::new();
    for face in shipped_faces().filter(|face| !face.gap.intersection(&covered).is_empty()) {
        if let Some(name) = family_of(face).filter(|name| FamilyKey::of(name) != key) {
            fills
                .entry(name.to_ascii_lowercase())
                .or_insert_with(|| name.to_owned());
        }
    }
    let mut descriptors: Vec<FaceDescriptor> = Vec::new();
    for rule in shipped_faces()
        .filter(|face| family_of(face).is_some_and(|name| FamilyKey::of(name) == key))
        .flat_map(|face| &face.face.rules)
    {
        if !descriptors.contains(&rule.descriptor) {
            descriptors.push(rule.descriptor);
        }
    }
    if descriptors.is_empty() {
        descriptors.push(shipped.rule.descriptor);
    }
    let rules: Vec<FontFaceRule> = descriptors
        .into_iter()
        .map(|descriptor| FontFaceRule {
            descriptor,
            unicode_range: Some(covered.clone()),
            ..shipped.rule.clone()
        })
        .collect();
    let style = shipped.rule.descriptor.style;
    let face = ShippedFace {
        bytes: rules
            .iter()
            .map(|rule| u64::try_from(rule.bytes.len()).unwrap_or(u64::MAX))
            .sum(),
        rules,
        fills: fills.into_values().collect(),
        licence: shipped.licence,
        source: font.source,
        notice: false,
        serves: Vec::new(),
        weight: shipped.face.weight,
        style,
        postscript_name: postscript_name.to_owned(),
        sha256: font.sha256,
        subset: shipped.subset,
        demoted: false,
        repair: None,
    };
    (Some(face), issues, false)
}

/// The stacks of the families the slides name that shipped faces draw ([`FontStacks`]). A symbol font drawn with a
/// face of another family has none: no other face draws its pictures, so the backend's own stack names it first.
pub fn font_stacks(faces: &[ShippedFace]) -> FontStacks {
    let mut stacks = FontStacks::new();
    for face in faces {
        let Some(bound) = face.rules.first().map(|rule| rule.family.as_str()) else {
            continue;
        };
        for served in &face.serves {
            let requested = FamilyKey::of(&served.family);
            if is_symbol_family(&requested) && requested != FamilyKey::of(bound) {
                continue;
            }
            stacks
                .entry(font_key(&served.family))
                .or_insert_with(|| vec![bound.to_owned()]);
        }
    }
    for face in faces.iter().filter(|face| !face.fills.is_empty()) {
        let Some(last) = face.rules.first().map(|rule| rule.family.as_str()) else {
            continue;
        };
        for stack in stacks.values_mut() {
            let fills = stack.first().is_some_and(|bound| {
                face.fills
                    .iter()
                    .any(|filled| filled.eq_ignore_ascii_case(bound))
            });
            if fills && !stack.iter().any(|family| family == last) {
                stack.push(last.to_owned());
            }
        }
    }
    stacks
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

    use slidock_fonts::WeightRange;
    use slidock_fonts::resolve::face_traits;
    use slidock_fonts::subset::{SubsetOptions, subset_face};
    use slidock_fonts::{FontLimits, read_sfnt};

    use super::*;

    /// Barlow Regular (OFL-1.1), from slidock-fonts' fixtures.
    const BARLOW: &[u8] = include_bytes!("../../slidock-fonts/tests/data/container/barlow.ttf");
    /// The bundled DejaVu Sans the hosts ship (the last-resort face), from `slidock-substitutes`.
    fn dejavu() -> &'static [u8] {
        slidock_substitutes::FACES
            .iter()
            .find(|face| face.file == "DejaVuSans.woff2")
            .unwrap()
            .bytes
    }

    fn font(bytes: Vec<u8>, unicode_range: Option<UnicodeRange>) -> AddedFont {
        let faces = read_sfnt(&bytes).unwrap();
        AddedFont {
            source: FontSource::Google,
            sha256: Sha256::of(&bytes),
            traits: vec![face_traits(&bytes, 0)],
            refused: vec![None],
            repairs: Vec::new(),
            faces,
            declared: Vec::new(),
            subset: false,
            missing: Vec::new(),
            unicode_range,
            bytes: bytes.into(),
        }
    }

    /// Barlow cut to the code points of `range`, as one of Google Fonts' per-script chunks.
    fn chunk(first: char, last: char) -> AddedFont {
        let points: CodepointSet = (u32::from(first)..=u32::from(last)).collect();
        let options = SubsetOptions {
            small_caps: false,
            limits: FontLimits::default(),
        };
        let bytes = subset_face(BARLOW, 0, &points, &options).unwrap();
        font(
            bytes,
            Some(UnicodeRange::from_ranges([(
                u32::from(first),
                u32::from(last),
            )])),
        )
    }

    fn uses(font: &AddedFont, text: &str) -> GlyphUse {
        let mut codepoints = CodepointSet::new();
        codepoints.insert_text(text);
        GlyphUse {
            family: "Barlow".to_owned(),
            postscript_name: font.faces[0].postscript_name.clone(),
            weight: 400,
            italic: false,
            sha256: Some(font.sha256),
            codepoints,
            weights: BTreeSet::from([400]),
            small_caps: false,
            requested: BTreeSet::from(["Barlow".to_owned()]),
            forced: CodepointSet::new(),
        }
    }

    fn ship(fonts: &[&AddedFont], glyph: &GlyphUse) -> Vec<ShippedFace> {
        let shipping = ship_faces(
            fonts,
            std::slice::from_ref(glyph),
            &DemotedSlots::new(),
            &SourceOrder::default(),
            &GoogleCatalog::default(),
        );
        let issues: Vec<&FontIssue> = shipping
            .issues
            .iter()
            .filter(|issue| !matches!(issue, FontIssue::CoverageGap { .. }))
            .collect();
        assert_eq!(issues, [] as [&FontIssue; 0]);
        shipping.faces
    }

    fn ranges(face: &ShippedFace) -> Vec<String> {
        face.rules
            .iter()
            .map(|rule| {
                rule.unicode_range
                    .as_ref()
                    .unwrap()
                    .unicode_range()
                    .unwrap()
            })
            .collect()
    }

    #[test]
    fn the_chunks_of_a_face_ship_together_each_with_the_drawn_points_of_its_range() {
        let (upper, lower, digits) = (chunk('A', 'Z'), chunk('a', 'z'), chunk('0', '9'));
        // Bound to the lower-case chunk, added last; the digits draw nothing.
        let glyph = uses(&lower, "Ab s");
        let faces = ship(&[&digits, &lower, &upper], &glyph);
        assert_eq!(faces.len(), 1);
        let face = &faces[0];
        assert_eq!(ranges(face), ["U+41", "U+62,U+73"]);
        assert_eq!(
            face.bytes,
            face.rules
                .iter()
                .map(|rule| rule.bytes.len() as u64)
                .sum::<u64>()
        );
        assert!(face.rules.iter().all(|rule| rule.family == "Barlow"));
        assert_ne!(face.rules[0].bytes, face.rules[1].bytes);
        // Added in another order, the same rules.
        let again = ship(&[&upper, &lower, &digits], &glyph);
        assert_eq!(again, faces);
    }

    #[test]
    fn a_whole_file_ships_one_rule_and_a_chunk_drawn_with_nothing_still_ships_its_own() {
        let whole = font(BARLOW.to_vec(), None);
        let faces = ship(&[&whole, &chunk('A', 'Z')], &uses(&whole, "Ab s"));
        assert_eq!(ranges(&faces[0]), ["U+20,U+41,U+62,U+73,U+C0"]);
        let digits = chunk('0', '9');
        let faces = ship(&[&digits], &uses(&digits, "Ab"));
        assert_eq!(faces[0].rules.len(), 1);
    }

    #[test]
    fn a_face_is_demoted_only_when_it_draws_a_demoted_slot() {
        let whole = font(BARLOW.to_vec(), None);
        let glyph = uses(&whole, "Ab");
        // Resolution left Barlow Medium 500 to this face's file, which failed its identity check as a Medium.
        let demoted = DemotedSlots::from([(
            FamilyKey::of("Barlow Medium"),
            500,
            false,
            glyph.postscript_name.clone(),
        )]);
        let ship = |glyph: &GlyphUse| {
            let shipping = ship_faces(
                &[&whole],
                std::slice::from_ref(glyph),
                &demoted,
                &SourceOrder::default(),
                &GoogleCatalog::default(),
            );
            shipping.faces[0].demoted
        };
        // The same file drawing only Barlow 400 stands in for nothing.
        assert!(!ship(&glyph));
        let standing_in = GlyphUse {
            requested: BTreeSet::from(["Barlow".to_owned(), "Barlow Medium".to_owned()]),
            weights: BTreeSet::from([400, 500]),
            ..glyph.clone()
        };
        assert!(ship(&standing_in));
        // Another face drawing the slot is not the demoted file.
        let other = GlyphUse {
            postscript_name: "Barlow-Black".to_owned(),
            ..standing_in
        };
        assert!(!ship(&other));
    }

    fn bundled(woff2: &[u8]) -> AddedFont {
        let sfnt = slidock_fonts::container::unwrap(woff2, &slidock_fonts::Hint::default())
            .unwrap()
            .sfnt;
        AddedFont {
            source: FontSource::Bundled,
            ..font(sfnt, None)
        }
    }

    fn ship_all(fonts: &[&AddedFont], uses: &[GlyphUse]) -> Shipping {
        ship_faces(
            fonts,
            uses,
            &DemotedSlots::new(),
            &SourceOrder::default(),
            &GoogleCatalog::default(),
        )
    }

    fn families(face: &ShippedFace) -> Vec<&str> {
        face.rules.iter().map(|rule| rule.family.as_str()).collect()
    }

    fn stack(stacks: &FontStacks, family: &str) -> Option<Vec<String>> {
        stacks.get(&font_key(family)).cloned()
    }

    #[test]
    fn rules_define_the_bound_family_once_and_every_requested_family_stacks_it_first() {
        let whole = font(BARLOW.to_vec(), None);
        // A thousand spellings of one family that fold to one font key, and another family drawn with the same face.
        let mut requested: BTreeSet<String> = (0..1000)
            .map(|n| format!("A{}rial", " ".repeat(n % 7)) + &"-".repeat(n / 7))
            .collect();
        requested.insert("Brand".to_owned());
        let glyph = GlyphUse {
            requested,
            ..uses(&whole, "Ab")
        };
        let shipping = ship_all(&[&whole], std::slice::from_ref(&glyph));
        assert_eq!(shipping.issues, []);
        let [face] = shipping.faces.as_slice() else {
            panic!("{:?}", shipping.faces);
        };
        // One rule, of the family the face is bound as, however many families draw with it.
        assert_eq!(families(face), ["Barlow"]);
        assert_eq!(face.bytes, face.rules[0].bytes.len() as u64);
        assert_eq!(face.licence.family, "Barlow");
        assert_eq!(face.serves.len(), 1001);
        let stacks = font_stacks(&shipping.faces);
        assert_eq!(stacks.len(), 2);
        assert_eq!(stack(&stacks, "Arial"), Some(vec!["Barlow".to_owned()]));
        assert_eq!(stack(&stacks, "A r-ial"), Some(vec!["Barlow".to_owned()]));
        assert_eq!(stack(&stacks, "Brand"), Some(vec!["Barlow".to_owned()]));
        // Negative control: a family nothing ships has no stack (the backend prints its own).
        assert_eq!(stack(&stacks, "Helvetica"), None);
    }

    #[test]
    fn a_symbol_font_drawn_with_another_family_s_face_keeps_its_own_stack() {
        let whole = font(BARLOW.to_vec(), None);
        let glyph = GlyphUse {
            requested: BTreeSet::from([
                "Arial".to_owned(),
                "Wingdings".to_owned(),
                "Symbol".to_owned(),
            ]),
            ..uses(&whole, "Ab\u{f06c}")
        };
        let shipping = ship_all(&[&whole], std::slice::from_ref(&glyph));
        let stacks = font_stacks(&shipping.faces);
        assert_eq!(stack(&stacks, "Wingdings"), None);
        assert_eq!(stack(&stacks, "Symbol"), None);
        assert_eq!(stack(&stacks, "Arial"), Some(vec!["Barlow".to_owned()]));
        // The PUA bullet is still a gap the viewer's fonts draw.
        assert!(matches!(
            &shipping.issues[..],
            [FontIssue::CoverageGap { code_points, .. }] if code_points == &[0xf06c]
        ));
        // Negative control: a symbol font drawn with a face of its own family stacks it.
        let own = GlyphUse {
            family: "Wingdings".to_owned(),
            requested: BTreeSet::from(["Wingdings".to_owned()]),
            ..uses(&whole, "Ab")
        };
        let shipping = ship_all(&[&whole], std::slice::from_ref(&own));
        let stacks = font_stacks(&shipping.faces);
        assert_eq!(
            stack(&stacks, "Wingdings"),
            Some(vec!["Wingdings".to_owned()])
        );
    }

    #[test]
    fn faces_of_one_family_claim_weights_among_themselves() {
        let whole = font(BARLOW.to_vec(), None);
        let glyph = GlyphUse {
            requested: BTreeSet::from(["Brand".to_owned(), "Brand Medium".to_owned()]),
            weights: BTreeSet::from([400, 500]),
            ..uses(&whole, "Ab")
        };
        let shipping = ship_all(&[&whole], std::slice::from_ref(&glyph));
        let face = &shipping.faces[0];
        assert_eq!(families(face), ["Barlow"]);
        assert_eq!(face.rules[0].descriptor.weight, WeightRange::point(400));
    }

    #[test]
    fn the_last_resort_ships_once_in_its_own_family_for_what_shipped_faces_lack() {
        let whole = font(BARLOW.to_vec(), None);
        let dejavu = bundled(dejavu());
        // ❖ and ☺ (Barlow lacks them, DejaVu has them), 中 (neither has it), a zero-width space (drawn without a
        // glyph), a forced U+2026 (kept whatever the text draws) and a real U+2010 hyphen the text draws.
        let mut glyph = GlyphUse {
            requested: BTreeSet::from(["Brand".to_owned()]),
            ..uses(&whole, "A\u{2756}\u{263a}\u{4e2d}\u{200b}\u{2010}")
        };
        glyph.codepoints.insert(0x2026);
        glyph.forced.insert(0x2026);
        let shipping = ship_all(&[&whole, &dejavu], std::slice::from_ref(&glyph));
        assert!(!shipping.wants_last_resort);
        let [face, last] = shipping.faces.as_slice() else {
            panic!("{:?}", shipping.faces);
        };
        assert_eq!(
            (last.postscript_name.as_str(), last.source, last.notice),
            ("DejaVuSans", FontSource::Bundled, false)
        );
        assert!(last.serves.is_empty());
        assert_eq!(last.licence.family, "DejaVu Sans");
        assert_eq!(families(last), ["DejaVu Sans"]);
        assert_eq!(last.fills, ["Barlow"]);
        let rule = &last.rules[0];
        assert_eq!(
            rule.descriptor,
            FaceDescriptor {
                weight: WeightRange::point(400),
                style: FaceStyle::Normal
            }
        );
        // Barlow lacks U+2010 (a gap the text draws) and the forced U+2026 (no gap).
        let range = rule.unicode_range.as_ref().unwrap().unicode_range();
        let own = face.rules[0].unicode_range.as_ref().unwrap();
        assert!(!own.contains(0x2010) && !own.contains(0x2026));
        assert_eq!(range.as_deref(), Some("U+2010,U+263A,U+2756"));
        // Negative control: drawn by the text, U+2026 is a gap the last resort fills.
        let drawn = GlyphUse {
            forced: CodepointSet::new(),
            ..glyph.clone()
        };
        let shipping_drawn = ship_all(&[&whole, &dejavu], std::slice::from_ref(&drawn));
        let filled = shipping_drawn.faces[1].rules[0]
            .unicode_range
            .as_ref()
            .unwrap();
        assert!(filled.contains(0x2026));
        assert_eq!(last.bytes, rule.bytes.len() as u64);
        assert_eq!(
            shipping.issues,
            [FontIssue::CoverageGap {
                family: "Barlow".to_owned(),
                postscript_name: face.postscript_name.clone(),
                missing: 1,
                code_points: vec![0x4e2d],
            }]
        );
        let stacks = font_stacks(&shipping.faces);
        assert_eq!(
            stack(&stacks, "Brand"),
            Some(vec!["Barlow".to_owned(), "DejaVu Sans".to_owned()])
        );

        // Without the last-resort file every gap is reported, and the session is asked for the file.
        let shipping = ship_all(&[&whole], std::slice::from_ref(&glyph));
        assert!(shipping.wants_last_resort);
        assert_eq!(shipping.faces.len(), 1);
        assert!(matches!(
            &shipping.issues[..],
            [FontIssue::CoverageGap { missing: 4, .. }]
        ));
        assert_eq!(
            stack(&font_stacks(&shipping.faces), "Brand"),
            Some(vec!["Barlow".to_owned()])
        );

        // A face lacking nothing the text draws ships no last resort.
        let plain = GlyphUse {
            requested: BTreeSet::from(["Brand".to_owned()]),
            ..uses(&whole, "Ab\u{200b}")
        };
        let shipping = ship_all(&[&whole, &dejavu], std::slice::from_ref(&plain));
        assert_eq!((shipping.faces.len(), shipping.issues.len()), (1, 0));
        assert!(!shipping.wants_last_resort);
    }

    #[test]
    fn the_last_resort_ships_one_subset_for_every_face_it_fills() {
        let whole = font(BARLOW.to_vec(), None);
        let dejavu = bundled(dejavu());
        let regular = GlyphUse {
            requested: BTreeSet::from(["Brand".to_owned()]),
            ..uses(&whole, "A\u{2756}")
        };
        let bold = GlyphUse {
            family: "Other".to_owned(),
            weight: 700,
            weights: BTreeSet::from([700]),
            requested: BTreeSet::from(["Other".to_owned()]),
            ..uses(&whole, "B\u{25cf}")
        };
        let shipping = ship_all(&[&whole, &dejavu], &[regular, bold]);
        let last = shipping.faces.last().unwrap();
        assert_eq!(last.rules.len(), 1);
        assert_eq!(
            last.rules[0]
                .unicode_range
                .as_ref()
                .unwrap()
                .unicode_range()
                .as_deref(),
            Some("U+25CF,U+2756")
        );
        assert_eq!(last.fills, ["Barlow", "Other"]);
        let stacks = font_stacks(&shipping.faces);
        assert_eq!(
            stack(&stacks, "Other"),
            Some(vec!["Other".to_owned(), "DejaVu Sans".to_owned()])
        );
    }

    #[test]
    fn the_last_resort_composes_with_the_shipped_faces_of_its_own_family() {
        let dejavu = bundled(dejavu());
        let whole = font(BARLOW.to_vec(), None);
        // OpenSymbol drawn with the bundled DejaVu Sans, and Barlow lacking ❖.
        let symbol = GlyphUse {
            family: "DejaVu Sans".to_owned(),
            postscript_name: "DejaVuSans".to_owned(),
            sha256: Some(dejavu.sha256),
            requested: BTreeSet::from(["OpenSymbol".to_owned()]),
            weights: BTreeSet::from([400, 700]),
            ..uses(&whole, "\u{25cf}")
        };
        let brand = GlyphUse {
            requested: BTreeSet::from(["Brand".to_owned()]),
            ..uses(&whole, "A\u{2756}")
        };
        let shipping = ship_all(&[&whole, &dejavu], &[symbol, brand]);
        let [own, _, last] = shipping.faces.as_slice() else {
            panic!("{:?}", shipping.faces);
        };
        assert_eq!(families(own), ["DejaVu Sans"]);
        assert_eq!(families(last), ["DejaVu Sans"]);
        // The same descriptors as the family's own rules, so CSS combines them.
        let descriptors = |face: &ShippedFace| -> Vec<FaceDescriptor> {
            face.rules.iter().map(|rule| rule.descriptor).collect()
        };
        assert_eq!(descriptors(last), descriptors(own));
        assert_eq!(last.fills, ["Barlow"]);
        let stacks = font_stacks(&shipping.faces);
        assert_eq!(
            stack(&stacks, "OpenSymbol"),
            Some(vec!["DejaVu Sans".to_owned()])
        );
    }
}
