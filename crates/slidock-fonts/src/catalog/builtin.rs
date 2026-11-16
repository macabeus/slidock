//! The fonts every compile can bind to without font bytes: metrics read from pinned release files (names and
//! numbers only, `data/builtin-metrics.json`, compiled in by `build.rs`). The table covers every generic family
//! and every substitution target but the floor's.
//!
//! The floor table (`data/floor-metrics.json`, read from the bundled WOFF2 files the hosts ship) holds the bundled
//! families only a deck that ships its fonts binds to (Caladea), so the parity path binds as the built-in table does.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use slidock_ir::fonts::{FaceStyle, FontMetrics, GlyphBounds, VMetrics, WinMetrics};

use super::{CatalogFace, FontCatalog};
use crate::bind::NORMAL_WIDTH;

struct BuiltinFamily {
    family: &'static str,
    faces: &'static [BuiltinFace],
}

struct BuiltinFace {
    weight: u32,
    width: u16,
    style: FaceStyle,
    postscript_name: &'static str,
    variable: bool,
    metrics: FontMetrics,
}

static FAMILIES: &[BuiltinFamily] = include!(concat!(env!("OUT_DIR"), "/builtin_metrics.rs"));
static FLOOR_FAMILIES: &[BuiltinFamily] = include!(concat!(env!("OUT_DIR"), "/floor_metrics.rs"));

fn catalog_of(families: &'static [BuiltinFamily]) -> FontCatalog {
    FontCatalog::build(families.iter().flat_map(|family| {
        family.faces.iter().map(|face| {
            (
                family.family,
                CatalogFace {
                    weight: face.weight,
                    width: face.width,
                    style: face.style,
                    postscript_name: face.postscript_name.to_owned(),
                    metrics: face.metrics.clone(),
                    variable: face.variable,
                    monospace: false,
                    sha256: None,
                    variation: None,
                    file: None,
                },
            )
        })
    }))
}

/// The built-in catalog.
pub fn builtin_catalog() -> &'static FontCatalog {
    static CATALOG: OnceLock<FontCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| catalog_of(FAMILIES))
}

/// The bundled families beyond the built-in catalog that a deck shipping its fonts binds to until their files
/// arrive.
pub fn floor_catalog() -> &'static FontCatalog {
    static CATALOG: OnceLock<FontCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| catalog_of(FLOOR_FAMILIES))
}

/// The shape of `builtin-metrics.json`: families in key order, each face with its style, names and metrics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsTable {
    pub families: Vec<MetricsFamily>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsFamily {
    pub family: String,
    pub faces: Vec<MetricsFace>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MetricsFace {
    pub weight: u32,
    /// `OS/2.usWidthClass`, left out when it is normal.
    #[serde(default = "normal_width", skip_serializing_if = "is_normal_width")]
    pub width: u16,
    pub style: FaceStyle,
    pub postscript_name: String,
    pub variable: bool,
    pub metrics: FontMetrics,
}

fn normal_width() -> u16 {
    NORMAL_WIDTH
}

fn is_normal_width(width: &u16) -> bool {
    *width == NORMAL_WIDTH
}

/// The metrics table that describes a catalog (file hashes are left out).
pub fn metrics_table(catalog: &FontCatalog) -> MetricsTable {
    MetricsTable {
        families: catalog
            .families()
            .map(|(_, family)| MetricsFamily {
                family: family.family.clone(),
                faces: family
                    .faces
                    .iter()
                    .map(|face| MetricsFace {
                        weight: face.weight,
                        width: face.width,
                        style: face.style,
                        postscript_name: face.postscript_name.clone(),
                        variable: face.variable,
                        metrics: face.metrics.clone(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use slidock_canon::{Layout, sha256_hex, to_canonical_json};
    use slidock_ir::fonts::GenericFamily;

    use super::super::{FamilyKey, style_name};
    use super::*;

    const SEED_FAMILIES: &[&str] = &[
        "Carlito",
        "DejaVu Sans",
        "Liberation Mono",
        "Liberation Sans",
        "Liberation Serif",
    ];
    /// The release fonts of the bench's font set beyond the seed (its Apple and user fonts stay out).
    const BENCH_FAMILIES: &[&str] = &[
        "Barlow Condensed",
        "Noto Sans Symbols 2",
        "Oswald",
        "Roboto",
        "Roboto Mono",
    ];
    /// The seed rows as generated from the seed files: the bench set's regeneration keeps them byte for byte.
    const SEED_ROWS_SHA256: &str =
        "865451c6d3b1a2e28b911ee2ec3156e03006390e4467b9c33ba90e1e7ac1c56a";

    const TABLE: &str = include_str!("../../data/builtin-metrics.json");

    fn table() -> MetricsTable {
        serde_json::from_str(TABLE).unwrap()
    }

    #[test]
    fn holds_the_seed_families_with_four_faces_each_and_the_bench_release_families() {
        let catalog = builtin_catalog();
        let mut families: Vec<&str> = catalog.families().map(|(_, f)| f.family.as_str()).collect();
        families.sort_unstable();
        let mut expected: Vec<&str> = SEED_FAMILIES
            .iter()
            .chain(BENCH_FAMILIES)
            .copied()
            .collect();
        expected.sort_unstable();
        assert_eq!(families, expected);
        for (_, family) in catalog
            .families()
            .filter(|(_, f)| SEED_FAMILIES.contains(&f.family.as_str()))
        {
            let slots: Vec<String> = family
                .faces
                .iter()
                .map(|face| format!("{}/{}", face.weight, style_name(face.style)))
                .collect();
            assert_eq!(
                slots,
                ["400/italic", "400/normal", "700/italic", "700/normal"],
                "{}",
                family.family
            );
        }
    }

    #[test]
    fn reads_the_generic_families_from_the_names() {
        let generic = |family: &str| {
            builtin_catalog()
                .find_family(family)
                .unwrap()
                .family
                .generic
        };
        assert_eq!(
            [
                "Liberation Sans",
                "Liberation Serif",
                "Liberation Mono",
                "DejaVu Sans",
                "Carlito"
            ]
            .map(generic),
            [
                GenericFamily::SansSerif,
                GenericFamily::Serif,
                GenericFamily::Monospace,
                GenericFamily::SansSerif,
                GenericFamily::SansSerif
            ]
        );
    }

    #[test]
    fn records_the_metrics_read_from_the_pinned_files() {
        let sans = builtin_catalog()
            .get(&FamilyKey::of("Liberation Sans"))
            .unwrap();
        let regular = sans
            .faces
            .iter()
            .find(|face| face.postscript_name == "LiberationSans")
            .unwrap();
        assert_eq!(
            regular.metrics,
            FontMetrics {
                units_per_em: 2048,
                hhea: VMetrics {
                    ascender: 1854,
                    descender: -434,
                    line_gap: 67
                },
                typo: VMetrics {
                    ascender: 1491,
                    descender: -431,
                    line_gap: 307
                },
                win: WinMetrics {
                    ascent: 1854,
                    descent: 434
                },
                bounds: Some(GlyphBounds {
                    y_min: -621,
                    y_max: 2007
                }),
                grave_top: Some(1776),
            }
        );
    }

    #[test]
    fn keeps_the_seed_rows_unchanged() {
        let seed: Vec<MetricsFamily> = table()
            .families
            .into_iter()
            .filter(|family| SEED_FAMILIES.contains(&family.family.as_str()))
            .collect();
        assert_eq!(
            sha256_hex(to_canonical_json(&seed, Layout::Compact).unwrap()),
            SEED_ROWS_SHA256
        );
    }

    #[test]
    fn is_the_table_of_the_catalog_it_describes() {
        let printed =
            to_canonical_json(&metrics_table(builtin_catalog()), Layout::Indented).unwrap();
        assert_eq!(printed, TABLE);
    }

    const FLOOR_TABLE: &str = include_str!("../../data/floor-metrics.json");
    /// The bundled Caladea files the hosts ship, from `slidock-substitutes`.
    fn caladea() -> Vec<&'static [u8]> {
        let files: Vec<&[u8]> = slidock_substitutes::FACES
            .iter()
            .filter(|face| face.family == "Caladea")
            .map(|face| face.bytes)
            .collect();
        assert_eq!(files.len(), 4);
        files
    }

    #[test]
    fn the_floor_table_holds_the_metrics_of_the_bundled_caladea_files() {
        let faces: Vec<(String, CatalogFace)> = caladea()
            .iter()
            .flat_map(|woff2| {
                let sfnt = crate::container::unwrap(woff2, &crate::Hint::default())
                    .unwrap()
                    .sfnt;
                crate::read_sfnt(&sfnt).unwrap().into_iter().map(|face| {
                    let catalog_face = CatalogFace::from_sfnt(&face, None);
                    (face.family, catalog_face)
                })
            })
            .collect();
        let catalog = FontCatalog::build(faces.iter().map(|(f, face)| (f.as_str(), face.clone())));
        let printed = to_canonical_json(&metrics_table(&catalog), Layout::Indented).unwrap();
        assert_eq!(printed, FLOOR_TABLE);
        let floor = to_canonical_json(&metrics_table(floor_catalog()), Layout::Indented).unwrap();
        assert_eq!(floor, FLOOR_TABLE);
        let caladea = floor_catalog().get(&FamilyKey::of("Caladea")).unwrap();
        assert_eq!(caladea.faces.len(), 4);
    }

    #[test]
    fn the_floor_families_are_not_built_in() {
        for (key, _) in floor_catalog().families() {
            assert!(builtin_catalog().get(key).is_none(), "{key:?}");
        }
    }
}
