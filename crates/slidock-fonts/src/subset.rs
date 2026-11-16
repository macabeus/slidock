//! Subsetting one face to the code points a deck draws with it, with `skera` (the fontations port of
//! `hb-subset`): the glyphs those code points map to plus their layout closure (ligatures, alternates, marks and
//! composite parts), the layout features browsers apply by default (and small caps when the deck draws them),
//! hinting, variations, and the family and style names.
//!
//! skera 0.9 subsets TrueType outlines only (it leaves `CFF `/`CFF2` out of its output), so a CFF face is refused
//! and ships whole. It cannot subset the tables of Apple's layout either: a face the browser's shaper (HarfBuzz)
//! would shape with `morx`, `mort` or `kerx` is refused and ships whole, and a face it would kern with the legacy
//! `kern` table keeps that table as it is, its glyph ids unchanged. Every output is checked to hold the tables a
//! font needs, and that a browser's sanitizer requires, before it is used.
//!
//! skera copies `hhea`, `OS/2`'s line metrics and the `head` glyph box from the full face (it does not recompute
//! the box), and À is always kept, so the shipped file measures as the face the compiler bound.

use std::panic::{AssertUnwindSafe, catch_unwind};

use read_fonts::collections::IntSet;
use read_fonts::types::{GlyphId, NameId, Tag};
use read_fonts::{FontRef, TableProvider};
use skera::{DEFAULT_DROP_TABLES, DEFAULT_LAYOUT_FEATURES, Plan, SubsetFlags, subset_font};
use skrifa::MetadataProvider;

use crate::glyphs::{ALWAYS_DRAWN, CodepointSet};
use crate::limits::FontLimits;
use crate::sfnt::{read_sfnt, sanitizer_refusal};

/// The name ids a subset keeps: copyright, family, subfamily, unique id, full name, version, PostScript name, the
/// licence description and URL, and the typographic family and subfamily.
const NAME_IDS: [u16; 11] = [0, 1, 2, 3, 4, 5, 6, 13, 14, 16, 17];

/// The name languages a subset keeps: Windows English (US), and language 0 (Mac English, and the Unicode
/// platform's only language).
const NAME_LANGUAGES: [u16; 2] = [0, 0x409];

/// Where the code points lie that skera's bidi closure adds to a subset (the mirrored pairs of Unicode 17's
/// `BidiMirroring.txt`), as runs: every code point here is checked along with the drawn ones.
const MIRRORED_RUNS: [(u32, u32); 13] = [
    (0x28, 0xbb),
    (0xf3a, 0xf3d),
    (0x169b, 0x169c),
    (0x2039, 0x208e),
    (0x2208, 0x232a),
    (0x2768, 0x2775),
    (0x27c3, 0x27ef),
    (0x2983, 0x2afa),
    (0x2bfe, 0x2bfe),
    (0x2e02, 0x2e5c),
    (0x3008, 0x301b),
    (0xfe59, 0xfe65),
    (0xff08, 0xff63),
];

/// The most `cmap` pairs a subset iterates; a subtable mapping more is not subset.
const MAX_CMAP_PAIRS: usize = 1 << 20;

/// The small-caps features, kept when the deck draws small caps.
const SMALL_CAPS: [&[u8; 4]; 2] = [b"smcp", b"c2sc"];

/// The tables skera copies unchanged by default (its plan's own list, which setting the list replaces).
const PASSED_THROUGH: [&[u8; 4]; 5] = [b"gasp", b"fpgm", b"prep", b"VDMX", b"DSIG"];

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SubsetError {
    #[error("The font file is {0} bytes, over the limit")]
    TooLarge(usize),
    #[error("The face draws {0} code points, over the limit")]
    TooManyCodepoints(u64),
    #[error("The font cannot be read: {0}")]
    Unreadable(String),
    #[error("The subsetter does not subset CFF outlines")]
    CffOutlines,
    #[error("The face shapes with its {0} table, which the subsetter cannot subset")]
    AppleLayout(String),
    #[error("The subsetter failed: {0}")]
    Failed(String),
    #[error("The subsetter stopped on a malformed font")]
    Panicked,
    #[error("The subset is not a usable font: {0}")]
    BadOutput(String),
}

/// What a subset keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubsetOptions {
    /// The deck draws small caps with the face: keep `smcp` and `c2sc`.
    pub small_caps: bool,
    pub limits: FontLimits,
}

/// Face `index` of `sfnt`, subset to `codepoints` (plus [`ALWAYS_DRAWN`] and `.notdef`), as a TrueType or
/// OpenType file.
pub fn subset_face(
    sfnt: &[u8],
    index: u32,
    codepoints: &CodepointSet,
    options: &SubsetOptions,
) -> Result<Vec<u8>, SubsetError> {
    if sfnt.len() > options.limits.font_bytes {
        return Err(SubsetError::TooLarge(sfnt.len()));
    }
    if codepoints.len() > options.limits.codepoints {
        return Err(SubsetError::TooManyCodepoints(codepoints.len()));
    }
    let font =
        FontRef::from_index(sfnt, index).map_err(|e| SubsetError::Unreadable(e.to_string()))?;
    check_tables(&font)?;
    check_cmap(&font, codepoints)?;
    let keep_kern = shaper_tables(&font)?;

    let mut unicodes = IntSet::<u32>::empty();
    for code_point in codepoints.iter().chain(ALWAYS_DRAWN) {
        unicodes.insert(code_point);
    }
    let mut glyphs = IntSet::<GlyphId>::empty();
    glyphs.insert(GlyphId::NOTDEF);
    let kern = Tag::new(b"kern");
    let drop_tables: IntSet<Tag> = DEFAULT_DROP_TABLES
        .iter()
        .copied()
        .filter(|&tag| !(keep_kern && tag == kern))
        .collect();
    let mut passed_through: IntSet<Tag> = PASSED_THROUGH.iter().map(|tag| Tag::new(tag)).collect();
    // Legacy names (Mac platform ones) are kept: some faces name themselves only there.
    let mut flags = SubsetFlags::SUBSET_FLAGS_NAME_LEGACY;
    if keep_kern {
        // `kern` indexes glyphs; it stays valid copied as it is only when the glyph ids do not change.
        passed_through.insert(kern);
        flags |= SubsetFlags::SUBSET_FLAGS_RETAIN_GIDS;
    }
    let mut layout_scripts = IntSet::<Tag>::empty();
    layout_scripts.invert();
    let mut layout_features: IntSet<Tag> = DEFAULT_LAYOUT_FEATURES.iter().copied().collect();
    if options.small_caps {
        layout_features.extend(SMALL_CAPS.iter().map(|tag| Tag::new(tag)));
    }
    let name_ids: IntSet<NameId> = NAME_IDS.iter().map(|&id| NameId::new(id)).collect();
    let name_languages: IntSet<u16> = NAME_LANGUAGES.iter().copied().collect();

    // skera indexes its own tables and may panic on fonts it cannot follow. A native build recovers here; a wasm
    // build aborts, so the checks above refuse what is known to reach those paths.
    let subset = catch_unwind(AssertUnwindSafe(|| {
        let mut plan = Plan::new(
            &glyphs,
            &unicodes,
            &font,
            flags,
            &drop_tables,
            &layout_scripts,
            &layout_features,
            &name_ids,
            &name_languages,
        );
        plan.set_no_subset_tables(&passed_through);
        subset_font(&font, &plan)
    }))
    .map_err(|_| SubsetError::Panicked)?
    .map_err(|e| SubsetError::Failed(e.to_string()))?;

    check_output(&font, &subset)?;
    read_sfnt(&subset).map_err(|e| SubsetError::BadOutput(e.to_string()))?;
    if let Some(refusal) = sanitizer_refusal(&subset, 0) {
        return Err(SubsetError::BadOutput(refusal.to_string()));
    }
    Ok(subset)
}

/// Whether HarfBuzz shapes the face with tables skera drops. `morx` or `mort` in a face without `GSUB`, and `kerx`
/// in a face without both `GSUB` and a `GPOS` `kern` feature, refuse the subset. The legacy `kern` table, which
/// HarfBuzz applies when `GPOS` has no `kern` feature, is kept (true).
fn shaper_tables(font: &FontRef<'_>) -> Result<bool, SubsetError> {
    let has = |tag: &[u8; 4]| {
        font.table_data(Tag::new(tag))
            .is_some_and(|data| !data.as_bytes().is_empty())
    };
    let gsub = font.gsub().is_ok();
    let gpos_kerns = font
        .gpos()
        .and_then(|gpos| gpos.feature_list())
        .is_ok_and(|features| {
            features
                .feature_records()
                .iter()
                .any(|record| record.feature_tag() == Tag::new(b"kern"))
        });
    for tag in [b"morx", b"mort"] {
        if has(tag) && !gsub {
            return Err(SubsetError::AppleLayout(
                String::from_utf8_lossy(tag).into_owned(),
            ));
        }
    }
    if has(b"kerx") && !(gsub && gpos_kerns) {
        return Err(SubsetError::AppleLayout("kerx".to_owned()));
    }
    Ok(has(b"kern") && !gpos_kerns)
}

/// Fails unless the tables every subset needs parse: `head`, `maxp`, `hhea`, `hmtx`, `cmap`, `glyf` and `loca`.
fn check_tables(font: &FontRef<'_>) -> Result<(), SubsetError> {
    let unreadable = |what: &str, error: read_fonts::ReadError| {
        SubsetError::Unreadable(format!("{what}: {error}"))
    };
    font.head().map_err(|e| unreadable("head", e))?;
    let maxp = font.maxp().map_err(|e| unreadable("maxp", e))?;
    font.hhea().map_err(|e| unreadable("hhea", e))?;
    font.hmtx().map_err(|e| unreadable("hmtx", e))?;
    font.cmap().map_err(|e| unreadable("cmap", e))?;
    if maxp.num_glyphs() == 0 {
        return Err(SubsetError::Unreadable("the font has no glyphs".to_owned()));
    }
    if font.table_data(Tag::new(b"CFF ")).is_some() || font.table_data(Tag::new(b"CFF2")).is_some()
    {
        return Err(SubsetError::CffOutlines);
    }
    font.glyf().map_err(|e| unreadable("glyf", e))?;
    font.loca(None).map_err(|e| unreadable("loca", e))?;
    Ok(())
}

/// Fails when a delta-set index map of `HVAR` or `VVAR` sends a glyph to variation data the item variation store
/// lacks, or cannot be read: skera follows those indexes unchecked.
fn check_variation_maps(font: &FontRef<'_>, glyphs: u32) -> Result<(), SubsetError> {
    let unreadable = |what: &str| SubsetError::Unreadable(format!("{what} cannot be followed"));
    let mut tables: Vec<(&str, Result<_, _>, Vec<Option<Result<_, _>>>)> = Vec::new();
    if let Ok(hvar) = font.hvar() {
        tables.push((
            "HVAR",
            hvar.item_variation_store(),
            vec![
                hvar.advance_width_mapping(),
                hvar.lsb_mapping(),
                hvar.rsb_mapping(),
            ],
        ));
    }
    if let Ok(vvar) = font.vvar() {
        tables.push((
            "VVAR",
            vvar.item_variation_store(),
            vec![
                vvar.advance_height_mapping(),
                vvar.tsb_mapping(),
                vvar.bsb_mapping(),
                vvar.v_org_mapping(),
            ],
        ));
    }
    for (name, store, maps) in tables {
        let store = store.map_err(|_| unreadable(name))?;
        let mut items: Vec<u32> = Vec::new();
        for data in store.item_variation_data().iter() {
            items.push(match data {
                Some(Ok(data)) => u32::from(data.item_count()),
                Some(Err(_)) => return Err(unreadable(name)),
                None => 0,
            });
        }
        let in_store = |outer: u16, inner: u16| {
            items
                .get(usize::from(outer))
                .is_some_and(|&count| u32::from(inner) < count)
        };
        let implicit = maps.first().is_some_and(Option::is_none);
        for map in maps.into_iter().flatten() {
            let map = map.map_err(|_| unreadable(name))?;
            for glyph in 0..glyphs {
                let index = map.get(glyph).map_err(|_| unreadable(name))?;
                if !in_store(index.outer, index.inner) {
                    return Err(unreadable(name));
                }
            }
        }
        // Without an advance map, glyph ids index the first variation data directly.
        let first = items.first().copied().unwrap_or(0);
        if implicit && glyphs > first {
            return Err(unreadable(name));
        }
    }
    Ok(())
}

/// Fails when the `cmap` subtable skera reads (skrifa's charmap) maps a code point the subset may look up (drawn,
/// always kept, or a bidi mirror) to a glyph past `maxp.numGlyphs`, by lookup or by iteration (skera does either,
/// and a malformed format 4 subtable can answer them differently), or holds more than [`MAX_CMAP_PAIRS`] pairs:
/// skera expects every glyph it maps in its glyph map, and iterates the whole subtable.
fn check_cmap(font: &FontRef<'_>, codepoints: &CodepointSet) -> Result<(), SubsetError> {
    let glyphs = font
        .maxp()
        .map(|maxp| u32::from(maxp.num_glyphs()))
        .map_err(|e| SubsetError::Unreadable(e.to_string()))?;
    check_variation_maps(font, glyphs)?;
    let mirrored = MIRRORED_RUNS.iter().flat_map(|&(first, last)| first..=last);
    let looked_up: CodepointSet = codepoints
        .iter()
        .chain(ALWAYS_DRAWN)
        .chain(mirrored)
        .collect();
    let past = |code_point: u32, glyph: GlyphId| {
        SubsetError::Unreadable(format!(
            "the cmap maps U+{code_point:04X} to glyph {} of a font of {glyphs} glyphs",
            glyph.to_u32()
        ))
    };
    let charmap = font.charmap();
    for code_point in looked_up.iter() {
        if let Some(glyph) = charmap.map(code_point)
            && glyph.to_u32() >= glyphs
        {
            return Err(past(code_point, glyph));
        }
    }
    for (count, (code_point, glyph)) in charmap.mappings().enumerate() {
        if count >= MAX_CMAP_PAIRS {
            return Err(SubsetError::Unreadable(format!(
                "the cmap holds more than {MAX_CMAP_PAIRS} mappings"
            )));
        }
        if glyph.to_u32() >= glyphs && looked_up.contains(code_point) {
            return Err(past(code_point, glyph));
        }
    }
    Ok(())
}

/// The tables a subset must keep: the ones every font needs, and `name`, `OS/2` and `post` when the full face
/// has them.
fn check_output(full: &FontRef<'_>, subset: &[u8]) -> Result<(), SubsetError> {
    let out = FontRef::new(subset).map_err(|e| SubsetError::BadOutput(e.to_string()))?;
    let required = [
        b"cmap", b"glyf", b"head", b"hhea", b"hmtx", b"loca", b"maxp",
    ];
    let kept_if_present = [b"name", b"OS/2", b"post"];
    let lost = required
        .iter()
        .chain(
            kept_if_present
                .iter()
                .filter(|tag| full.table_data(Tag::new(tag)).is_some()),
        )
        .find(|tag| out.table_data(Tag::new(tag)).is_none());
    match lost {
        Some(tag) => Err(SubsetError::BadOutput(format!(
            "the {} table is missing",
            String::from_utf8_lossy(&tag[..])
        ))),
        None => Ok(()),
    }
}
