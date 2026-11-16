//! The fonts a compile can bind to, as plain data: the built-in metrics table plus the fonts a host added. Binding
//! only reads it, so the same catalog always yields the same bindings.

pub mod builtin;
pub mod emoji;
pub mod substitutes;

use std::cmp::Ordering;
use std::collections::BTreeMap;

use slidock_canon::utf16;
use slidock_ir::fonts::{FaceStyle, FontMetrics, GenericFamily, font_key};
use slidock_ir::ids::Sha256;

use crate::advance::FaceFile;
use crate::bind::{NORMAL_WIDTH, StretchRank, WeightRange, stretch_rank};
use crate::limits::FontLimits;
use crate::names::split_weight_name;
use crate::sfnt::{SfntFace, metrics_at, read_details};

/// The key of a family in catalogs and in `presentation.fonts` ([`font_key`]), ordered as JavaScript lists the keys
/// of an object whose keys were added in code-unit order: array indexes (`9`, `10`, `2001`) first, in numeric order,
/// then every other key by UTF-16 code unit.
///
/// Keys are plain data: `__proto__` and `constructor` name families like any other key, where a JavaScript object
/// would drop the first and find an inherited property for the second.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FamilyKey(String);

impl FamilyKey {
    pub fn of(family: &str) -> FamilyKey {
        FamilyKey(font_key(family))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The array index a key stands for: a canonical decimal (`0`, or no leading zero) below 2^32 - 1.
fn array_index(key: &str) -> Option<u32> {
    let canonical =
        key == "0" || (!key.starts_with('0') && key.bytes().all(|b| b.is_ascii_digit()));
    key.parse::<u32>()
        .ok()
        .filter(|&index| canonical && index != u32::MAX)
}

impl Ord for FamilyKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match (array_index(&self.0), array_index(&other.0)) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => utf16::cmp(&self.0, &other.0),
        }
    }
}

impl PartialOrd for FamilyKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// One face a family can render with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogFace {
    pub weight: u32,
    /// `OS/2.usWidthClass` ([`SfntFace::width`]); [`NORMAL_WIDTH`] for a face of normal width.
    pub width: u16,
    pub style: FaceStyle,
    pub postscript_name: String,
    pub metrics: FontMetrics,
    /// A variable font: measured at its default instance unless `variation` holds the weight drawn.
    pub variable: bool,
    /// The face's file advances by one width ([`SfntFace::fixed_pitch`]), which makes its family monospace.
    pub monospace: bool,
    /// The SHA-256 of the file the face came from, when it came from a file.
    pub sha256: Option<Sha256>,
    /// A variable face's `wght` axis and its metrics at the weights the presentation draws, when they were read
    /// from its file.
    pub variation: Option<Variation>,
    /// The file, when the host kept it: its glyph advances let text-metrics fit text that shrinks.
    pub file: Option<FaceFile>,
}

/// A variable face's weight axis, and its metrics at some weights of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variation {
    /// The `wght` axis range, which the face's `@font-face` rule states.
    pub range: WeightRange,
    /// The axis default, where the face's own metrics were read.
    pub default: u32,
    /// Metrics at other weights, by weight.
    pub instances: Vec<(u32, FontMetrics)>,
}

impl CatalogFace {
    /// The catalog face of an sfnt face, with the hash of its file.
    pub fn from_sfnt(sfnt: &SfntFace, sha256: Option<Sha256>) -> CatalogFace {
        CatalogFace {
            weight: sfnt.weight,
            width: sfnt.width,
            style: if sfnt.italic {
                FaceStyle::Italic
            } else {
                FaceStyle::Normal
            },
            postscript_name: sfnt.postscript_name.clone(),
            metrics: sfnt.metrics.clone(),
            variable: sfnt.variable,
            monospace: sfnt.fixed_pitch,
            sha256,
            variation: None,
            file: None,
        }
    }

    /// The face with its file kept.
    pub fn with_file(self, file: FaceFile) -> CatalogFace {
        CatalogFace {
            file: Some(file),
            ..self
        }
    }

    /// The catalog face of face `index` of a font file, with a variable face's axis and its metrics at each of
    /// `weights` (clamped to the axis; at most `limits.instances` of them). A file whose variations cannot be read
    /// gives the face of [`CatalogFace::from_sfnt`].
    pub fn from_file(
        sfnt: &SfntFace,
        bytes: &[u8],
        index: u32,
        weights: &[u32],
        limits: &FontLimits,
    ) -> CatalogFace {
        let mut face = CatalogFace::from_sfnt(sfnt, Some(Sha256::of(bytes)));
        if !sfnt.variable {
            return face;
        }
        let Some(axis) = read_details(bytes, index).ok().and_then(|d| d.weight_axis) else {
            return face;
        };
        let round = |value: f64| (value + 0.5).floor().clamp(1.0, 1000.0) as u32;
        let range = WeightRange::between(round(axis.min), round(axis.max));
        let default = round(axis.default);
        let mut wanted: Vec<u32> = weights
            .iter()
            .map(|&weight| weight.clamp(range.min, range.max))
            .filter(|&weight| weight != default)
            .collect();
        wanted.sort_unstable();
        wanted.dedup();
        wanted.truncate(limits.instances);
        let instances = wanted
            .into_iter()
            .filter_map(|weight| {
                metrics_at(bytes, index, weight, limits)
                    .ok()
                    .map(|metrics| (weight, metrics))
            })
            .collect();
        face.variation = Some(Variation {
            range,
            default,
            instances,
        });
        face
    }

    /// The metrics of the face drawn at `weight`: a static face's own, a variable face's at that weight (clamped
    /// to its axis), or `None` when those were not read.
    pub fn metrics_at(&self, weight: u32) -> Option<&FontMetrics> {
        if !self.variable {
            return Some(&self.metrics);
        }
        let variation = self.variation.as_ref()?;
        let weight = weight.clamp(variation.range.min, variation.range.max);
        if weight == variation.default {
            return Some(&self.metrics);
        }
        variation
            .instances
            .iter()
            .find(|(at, _)| *at == weight)
            .map(|(_, metrics)| metrics)
    }
}

pub(crate) fn style_name(style: FaceStyle) -> &'static str {
    match style {
        FaceStyle::Normal => "normal",
        FaceStyle::Italic => "italic",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogFamily {
    /// The family name of the first face added under this key.
    pub family: String,
    /// Monospace when one of its faces is ([`CatalogFace::monospace`]), else what the family name suggests
    /// ([`substitutes::family_generic`]).
    pub generic: GenericFamily,
    /// One per weight and style, by weight and then style name (`italic` before `normal`), all of the width
    /// `font-stretch: normal` matches.
    pub faces: Vec<CatalogFace>,
}

/// A catalog family, and the weight a trailing weight name in the request asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FamilyMatch<'a> {
    pub family: &'a CatalogFamily,
    pub weight: Option<u32>,
}

/// Families by [`FamilyKey`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontCatalog {
    families: BTreeMap<FamilyKey, CatalogFamily>,
    /// Why the faces of a font the host holds never bind, by the key of their family and of their PostScript names.
    refused: BTreeMap<FamilyKey, String>,
}

/// A family's faces by where their width ranks for `font-stretch: normal`, weight and style name, as they are added.
type FaceSlots = BTreeMap<(StretchRank, u32, &'static str), CatalogFace>;

/// The faces of a family a browser can draw when it asks for `font-stretch: normal`: those at the width ranked first
/// (CSS font matching narrows by stretch before style and weight), by weight and style name.
fn normal_stretch(faces: FaceSlots) -> Vec<CatalogFace> {
    let first = faces.keys().next().map(|&(rank, _, _)| rank);
    faces
        .into_iter()
        .filter(|&((rank, _, _), _)| Some(rank) == first)
        .map(|(_, face)| face)
        .collect()
}

impl FontCatalog {
    /// A catalog from faces named by family. A family keeps only its faces of the width `font-stretch: normal`
    /// matches ([`stretch_rank`]): of a collection holding Bold and Condensed Bold, Bold. A face with the same width,
    /// weight, style and family as an earlier one replaces it, so the later of two sources wins; families and faces
    /// come out in a fixed order whatever the input order. A family is monospace when one of the faces it keeps is.
    pub fn build<I, S>(entries: I) -> FontCatalog
    where
        I: IntoIterator<Item = (S, CatalogFace)>,
        S: AsRef<str>,
    {
        let mut families: BTreeMap<FamilyKey, (String, FaceSlots)> = BTreeMap::new();
        for (family, face) in entries {
            let family = family.as_ref();
            let (_, faces) = families
                .entry(FamilyKey::of(family))
                .or_insert_with(|| (family.to_owned(), BTreeMap::new()));
            let rank = stretch_rank(face.width, NORMAL_WIDTH);
            faces.insert((rank, face.weight, style_name(face.style)), face);
        }
        let families = families
            .into_iter()
            .map(|(key, (family, faces))| {
                let faces = normal_stretch(faces);
                let generic = if faces.iter().any(|face| face.monospace) {
                    GenericFamily::Monospace
                } else {
                    substitutes::family_generic(&family)
                };
                (
                    key,
                    CatalogFamily {
                        family,
                        generic,
                        faces,
                    },
                )
            })
            .collect();
        FontCatalog {
            families,
            refused: BTreeMap::new(),
        }
    }

    /// The catalogs together, built again as one ([`FontCatalog::build`]): a later catalog's face replaces an
    /// earlier one's with the same family, width, weight and style, and a family keeps only the faces of its sources
    /// at the width `font-stretch: normal` matches, so a later source's condensed face never replaces an earlier
    /// source's face of normal width.
    pub fn merge<'a>(catalogs: impl IntoIterator<Item = &'a FontCatalog>) -> FontCatalog {
        FontCatalog::build(catalogs.into_iter().flat_map(|catalog| {
            catalog.families.values().flat_map(|family| {
                family
                    .faces
                    .iter()
                    .map(|face| (family.family.as_str(), face.clone()))
            })
        }))
    }

    /// The catalog, knowing why the faces of fonts the host holds never bind: `(family or PostScript name, reason)`;
    /// the first reason given for a key stands.
    pub fn with_refused<'a>(
        mut self,
        refused: impl IntoIterator<Item = (&'a str, String)>,
    ) -> FontCatalog {
        for (name, reason) in refused {
            self.refused.entry(FamilyKey::of(name)).or_insert(reason);
        }
        self
    }

    /// Why a family the catalog lacks is held but never binds: a face the host holds of that family, or with one of
    /// these PostScript names, was refused (browsers would refuse the file, or its licence keeps it out).
    pub fn refusal<S: AsRef<str>>(&self, requested: &str, postscript_names: &[S]) -> Option<&str> {
        let split = split_weight_name(requested).map(|split| split.family);
        std::iter::once(requested)
            .chain(split)
            .chain(postscript_names.iter().map(AsRef::as_ref))
            .find_map(|name| self.refused.get(&FamilyKey::of(name)))
            .map(String::as_str)
    }

    /// Families in key order.
    pub fn families(&self) -> impl Iterator<Item = (&FamilyKey, &CatalogFamily)> {
        self.families.iter()
    }

    pub fn get(&self, key: &FamilyKey) -> Option<&CatalogFamily> {
        self.families.get(key)
    }

    pub fn is_empty(&self) -> bool {
        self.families.is_empty()
    }

    /// The family a requested name names, directly or through a trailing weight name (`Roboto Medium` → Roboto
    /// with weight 500).
    pub fn find_family(&self, requested: &str) -> Option<FamilyMatch<'_>> {
        if let Some(family) = self.get(&FamilyKey::of(requested)) {
            return Some(FamilyMatch {
                family,
                weight: None,
            });
        }
        let split = split_weight_name(requested)?;
        self.get(&FamilyKey::of(split.family))
            .map(|family| FamilyMatch {
                family,
                weight: Some(split.weight),
            })
    }

    /// The family holding a face with one of these PostScript names (the first name that has one). A PostScript
    /// name names one face exactly, so it binds a family whose name the request spells differently
    /// (`PartyLetPlain` is Party LET).
    pub fn find_postscript<S: AsRef<str>>(&self, names: &[S]) -> Option<&CatalogFamily> {
        names.iter().find_map(|name| {
            self.families.values().find(|family| {
                family
                    .faces
                    .iter()
                    .any(|face| face.postscript_name == name.as_ref())
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use slidock_ir::fonts::{VMetrics, WinMetrics};

    use super::*;

    fn metrics() -> FontMetrics {
        FontMetrics {
            units_per_em: 2048,
            hhea: VMetrics {
                ascender: 1854,
                descender: -434,
                line_gap: 67,
            },
            typo: VMetrics {
                ascender: 1491,
                descender: -431,
                line_gap: 307,
            },
            win: WinMetrics {
                ascent: 1854,
                descent: 434,
            },
            bounds: None,
            grave_top: None,
        }
    }

    fn face(weight: u32, style: FaceStyle, postscript_name: &str) -> CatalogFace {
        CatalogFace {
            weight,
            width: NORMAL_WIDTH,
            style,
            postscript_name: postscript_name.to_owned(),
            metrics: metrics(),
            variable: false,
            monospace: false,
            sha256: None,
            variation: None,
            file: None,
        }
    }

    fn upright(weight: u32) -> CatalogFace {
        face(weight, FaceStyle::Normal, &format!("F-{weight}"))
    }

    fn slots(family: &CatalogFamily) -> Vec<String> {
        family
            .faces
            .iter()
            .map(|f| format!("{}/{}", f.weight, style_name(f.style)))
            .collect()
    }

    fn keys(catalog: &FontCatalog) -> Vec<&str> {
        catalog.families().map(|(key, _)| key.as_str()).collect()
    }

    #[test]
    fn keys_families_by_font_key_and_orders_faces_whatever_the_input_order() {
        let a = FontCatalog::build([
            ("Liberation Sans", upright(700)),
            ("DejaVu Sans Mono", upright(400)),
            ("Liberation Sans", face(400, FaceStyle::Italic, "F-400")),
            ("Liberation Sans", upright(400)),
        ]);
        assert_eq!(keys(&a), ["dejavusansmono", "liberationsans"]);
        let sans = a.get(&FamilyKey::of("Liberation Sans")).unwrap();
        assert_eq!(slots(sans), ["400/italic", "400/normal", "700/normal"]);
        assert_eq!(
            a.get(&FamilyKey::of("DejaVu Sans Mono")).unwrap().generic,
            GenericFamily::Monospace
        );
        let reversed = FontCatalog::build([
            ("Liberation Sans", upright(400)),
            ("Liberation Sans", face(400, FaceStyle::Italic, "F-400")),
            ("DejaVu Sans Mono", upright(400)),
            ("Liberation Sans", upright(700)),
        ]);
        assert_eq!(reversed, a);
    }

    fn of_width(weight: u32, width: u16, postscript_name: &str) -> CatalogFace {
        CatalogFace {
            width,
            ..face(weight, FaceStyle::Normal, postscript_name)
        }
    }

    fn names(catalog: &FontCatalog, family: &str) -> Vec<String> {
        catalog
            .get(&FamilyKey::of(family))
            .unwrap()
            .faces
            .iter()
            .map(|face| face.postscript_name.clone())
            .collect()
    }

    #[test]
    fn a_family_keeps_its_faces_of_the_width_normal_stretch_matches() {
        // A collection's members in file order: Condensed Bold after Bold, Condensed Black the only 900.
        let members = [
            of_width(400, 5, "Neue"),
            of_width(700, 5, "Neue-Bold"),
            of_width(700, 3, "Neue-CondensedBold"),
            of_width(900, 3, "Neue-CondensedBlack"),
        ];
        let catalog = FontCatalog::build(members.iter().map(|face| ("Neue", face.clone())));
        assert_eq!(names(&catalog, "Neue"), ["Neue", "Neue-Bold"]);
        let reversed = FontCatalog::build(members.iter().rev().map(|face| ("Neue", face.clone())));
        assert_eq!(reversed, catalog);
        let family = catalog.get(&FamilyKey::of("Neue")).unwrap();
        assert_eq!(
            family
                .css_match(900, FaceStyle::Normal)
                .unwrap()
                .postscript_name,
            "Neue-Bold"
        );
        // A merge narrows again: a later catalog's condensed face does not replace a normal one.
        let condensed = FontCatalog::build([("Neue", of_width(400, 3, "Neue-Condensed"))]);
        assert_eq!(
            names(&FontCatalog::merge([&catalog, &condensed]), "Neue"),
            ["Neue", "Neue-Bold"]
        );
        // Negative control: of one width, the later face of a slot still wins.
        let same = FontCatalog::build([
            ("Neue", of_width(700, 5, "First")),
            ("Neue", of_width(700, 5, "Second")),
        ]);
        assert_eq!(names(&same, "Neue"), ["Second"]);
    }

    #[test]
    fn a_family_without_a_normal_face_keeps_the_width_ranked_next() {
        let condensed = FontCatalog::build([
            ("Narrow", of_width(400, 3, "Narrow-Regular")),
            ("Narrow", of_width(700, 3, "Narrow-Bold")),
            ("Narrow", of_width(400, 6, "Narrow-SemiExpanded")),
            ("Narrow", of_width(400, 2, "Narrow-ExtraCondensed")),
        ]);
        assert_eq!(
            names(&condensed, "Narrow"),
            ["Narrow-Regular", "Narrow-Bold"]
        );
        let wide = FontCatalog::build([
            ("Wide", of_width(400, 7, "Wide-Expanded")),
            ("Wide", of_width(400, 6, "Wide-SemiExpanded")),
        ]);
        assert_eq!(names(&wide, "Wide"), ["Wide-SemiExpanded"]);
    }

    #[test]
    fn an_unknown_width_is_kept_only_when_no_face_states_a_known_one() {
        let mixed = FontCatalog::build([
            ("Odd", of_width(400, 0, "Odd-Zero")),
            ("Odd", of_width(700, 3, "Odd-CondensedBold")),
            ("Odd", of_width(400, 12, "Odd-Twelve")),
        ]);
        assert_eq!(names(&mixed, "Odd"), ["Odd-CondensedBold"]);
        let unknown = FontCatalog::build([
            ("Odd", of_width(400, 0, "Odd-Zero")),
            ("Odd", of_width(700, 12, "Odd-Bold")),
        ]);
        assert_eq!(names(&unknown, "Odd"), ["Odd-Zero", "Odd-Bold"]);
    }

    #[test]
    fn a_family_with_a_face_of_fixed_pitch_is_monospace_whatever_its_name() {
        let fixed = |weight: u32| CatalogFace {
            monospace: true,
            ..upright(weight)
        };
        let generic = |catalog: &FontCatalog, family: &str| {
            catalog.get(&FamilyKey::of(family)).unwrap().generic
        };
        let files = FontCatalog::build([
            ("PragmataPro", fixed(400)),
            ("Letter Gothic Std", fixed(700)),
            ("Hasklig", upright(700)),
            ("Hasklig", fixed(400)),
        ]);
        for family in ["PragmataPro", "Letter Gothic Std", "Hasklig"] {
            assert_eq!(
                generic(&files, family),
                GenericFamily::Monospace,
                "{family}"
            );
        }
        // A face added from a file makes a family of name-only faces monospace too.
        let named = FontCatalog::build([("PragmataPro", upright(700))]);
        assert_eq!(generic(&named, "PragmataPro"), GenericFamily::SansSerif);
        assert_eq!(
            generic(&FontCatalog::merge([&named, &files]), "PragmataPro"),
            GenericFamily::Monospace
        );
        // Negative controls: proportional faces keep the generic the name suggests, and a fixed-pitch face of a
        // width the family does not keep does not count.
        let proportional =
            FontCatalog::build([("PragmataPro", upright(400)), ("Georgia", upright(400))]);
        assert_eq!(
            generic(&proportional, "PragmataPro"),
            GenericFamily::SansSerif
        );
        assert_eq!(generic(&proportional, "Georgia"), GenericFamily::Serif);
        let condensed = FontCatalog::build([
            ("Narrow", upright(400)),
            (
                "Narrow",
                CatalogFace {
                    width: 3,
                    ..fixed(400)
                },
            ),
        ]);
        assert_eq!(generic(&condensed, "Narrow"), GenericFamily::SansSerif);
    }

    #[test]
    fn keeps_the_first_spelling_of_a_family_and_the_last_face_of_a_slot() {
        let catalog = FontCatalog::build([
            ("Open Sans", face(400, FaceStyle::Normal, "First")),
            ("open-sans", face(400, FaceStyle::Normal, "Second")),
        ]);
        let family = catalog.get(&FamilyKey::of("OPEN SANS")).unwrap();
        assert_eq!(family.family, "Open Sans");
        assert_eq!(family.faces.len(), 1);
        assert_eq!(family.faces[0].postscript_name, "Second");
    }

    #[test]
    fn orders_keys_by_utf16_code_unit() {
        let catalog = FontCatalog::build([("\u{1f600}", upright(400)), ("\u{ff21}", upright(400))]);
        assert_eq!(keys(&catalog), ["\u{1f600}", "\u{ff21}"]);
    }

    #[test]
    fn orders_array_index_keys_first_and_numerically() {
        let families = [
            "Alpha",
            "10",
            "9",
            "2 0 0 1",
            "0",
            "01",
            "-1",
            "4294967294",
            "4294967295",
            "1e3",
        ];
        let catalog = FontCatalog::build(families.map(|family| (family, upright(400))));
        assert_eq!(
            keys(&catalog),
            [
                "0",
                "1",
                "9",
                "10",
                "2001",
                "4294967294",
                "01",
                "1e3",
                "4294967295",
                "alpha"
            ]
        );
    }

    #[test]
    fn finds_postscript_names_and_tables_families_in_key_order() {
        let shared = || face(400, FaceStyle::Normal, "Shared");
        let catalog = FontCatalog::build([("Alpha", shared()), ("10", shared()), ("9", shared())]);
        assert_eq!(catalog.find_postscript(&["Shared"]).unwrap().family, "9");
        let table = builtin::metrics_table(&catalog);
        let families: Vec<&str> = table.families.iter().map(|f| f.family.as_str()).collect();
        assert_eq!(families, ["9", "10", "Alpha"]);
    }

    #[test]
    fn object_prototype_names_are_ordinary_keys() {
        let catalog = FontCatalog::build([("__proto__", upright(400)), ("Roboto", upright(400))]);
        assert_eq!(keys(&catalog), ["__proto__", "roboto"]);
        assert!(catalog.find_family("constructor").is_none());
        assert_eq!(
            catalog.find_family("__PROTO__").unwrap().family.family,
            "__proto__"
        );
    }

    #[test]
    fn lets_a_later_catalog_replace_a_face() {
        let base = FontCatalog::build([("Carlito", face(400, FaceStyle::Normal, "Old"))]);
        let newer = FontCatalog::build([("Carlito", face(400, FaceStyle::Normal, "New"))]);
        let merged = FontCatalog::merge([&base, &newer]);
        let names: Vec<_> = merged
            .get(&FamilyKey::of("Carlito"))
            .unwrap()
            .faces
            .iter()
            .map(|f| &f.postscript_name)
            .collect();
        assert_eq!(names, ["New"]);
    }

    #[test]
    fn finds_a_family_directly_or_through_a_weight_name() {
        let catalog = FontCatalog::build([("Roboto", upright(400))]);
        assert_eq!(catalog.find_family("ROBOTO").unwrap().weight, None);
        let medium = catalog.find_family("Roboto Medium").unwrap();
        assert_eq!(
            (medium.family.family.as_str(), medium.weight),
            ("Roboto", Some(500))
        );
        assert!(catalog.find_family("Oswald Medium").is_none());
    }

    #[test]
    fn finds_a_family_by_the_first_postscript_name_it_holds() {
        let catalog = FontCatalog::build([
            ("Party LET", face(400, FaceStyle::Normal, "PartyLetPlain")),
            ("Roboto", face(400, FaceStyle::Normal, "Roboto-Regular")),
        ]);
        assert_eq!(
            catalog.find_postscript(&["PartyLetPlain"]).unwrap().family,
            "Party LET"
        );
        assert_eq!(
            catalog
                .find_postscript(&["Missing", "Roboto-Regular", "PartyLetPlain"])
                .unwrap()
                .family,
            "Roboto"
        );
        assert!(catalog.find_postscript::<&str>(&[]).is_none());
        assert!(catalog.find_postscript(&["partyletplain"]).is_none());
    }

    #[test]
    fn builds_faces_from_sfnt_faces_with_their_file_hash() {
        let sfnt = SfntFace {
            family: "A".into(),
            subfamily: "Light Italic".into(),
            full_name: None,
            postscript_name: "A-LightItalic".into(),
            weight: 300,
            width: 3,
            italic: true,
            italic_angle: -12.0,
            variable: true,
            use_typo_metrics: false,
            metrics: metrics(),
            fixed_pitch: true,
        };
        let hash = Sha256([0xab; 32]);
        let face = CatalogFace::from_sfnt(&sfnt, Some(hash));
        assert_eq!(
            face,
            CatalogFace {
                weight: 300,
                width: 3,
                style: FaceStyle::Italic,
                postscript_name: "A-LightItalic".into(),
                metrics: metrics(),
                variable: true,
                monospace: true,
                sha256: Some(hash),
                variation: None,
                file: None,
            }
        );
    }
}
