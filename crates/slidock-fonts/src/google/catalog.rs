//! The offline Google Fonts catalog: the open-licence families of google/fonts at a pinned commit, generated into
//! `data/google-fonts.txt` by `cargo xtask google-catalog`. Google's `css2` endpoint also serves commercially licensed
//! families it does not list (Helvetica Neue, Calibri), so a family outside this catalog is never requested.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use slidock_ir::fonts::{FaceStyle, GenericFamily};

use crate::bind::{FaceDescriptor, WeightRange, css_match};
use crate::catalog::FamilyKey;
use crate::names::split_weight_name;

const DATA: &str = include_str!("../../data/google-fonts.txt");

/// The open licence a Google family ships under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Licence {
    Ofl,
    Apache,
    Ufl,
}

impl Licence {
    /// The SPDX identifier.
    pub const fn spdx(self) -> &'static str {
        match self {
            Licence::Ofl => "OFL-1.1",
            Licence::Apache => "Apache-2.0",
            Licence::Ufl => "UFL-1.0",
        }
    }

    fn parse(value: &str) -> Option<Licence> {
        match value {
            "OFL-1.1" => Some(Licence::Ofl),
            "Apache-2.0" => Some(Licence::Apache),
            "UFL-1.0" => Some(Licence::Ufl),
            _ => None,
        }
    }
}

/// The weights of one style of a family: static weights, or the range of a `wght` axis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Weights {
    Statics(Vec<u32>),
    Range(WeightRange),
}

impl Weights {
    pub fn contains(&self, weight: u32) -> bool {
        match self {
            Weights::Statics(weights) => weights.contains(&weight),
            Weights::Range(range) => (range.min..=range.max).contains(&weight),
        }
    }

    fn descriptors(&self, style: FaceStyle) -> Vec<FaceDescriptor> {
        match self {
            Weights::Statics(weights) => weights
                .iter()
                .map(|&weight| FaceDescriptor {
                    weight: WeightRange::point(weight),
                    style,
                })
                .collect(),
            Weights::Range(range) => vec![FaceDescriptor {
                weight: *range,
                style,
            }],
        }
    }
}

/// A design axis: `wght`, `wdth`, `opsz`, …
#[derive(Clone, Debug, PartialEq)]
pub struct Axis {
    pub tag: String,
    pub min: f64,
    pub max: f64,
}

/// One family of the catalog.
#[derive(Clone, Debug, PartialEq)]
pub struct GoogleFamily {
    /// As `css2` spells it.
    pub family: String,
    /// Google's category as a generic hint.
    pub generic: GenericFamily,
    pub licence: Licence,
    pub upright: Option<Weights>,
    pub italic: Option<Weights>,
    pub axes: Vec<Axis>,
    pub subsets: Vec<&'static str>,
}

impl GoogleFamily {
    pub fn weights(&self, style: FaceStyle) -> Option<&Weights> {
        match style {
            FaceStyle::Normal => self.upright.as_ref(),
            FaceStyle::Italic => self.italic.as_ref(),
        }
    }

    /// Whether the family has this face exactly (a static weight, or a weight inside the `wght` range).
    pub fn has_face(&self, weight: u32, style: FaceStyle) -> bool {
        self.weights(style)
            .is_some_and(|weights| weights.contains(weight))
    }

    /// The face of the family a browser would draw for this weight and style (CSS Fonts 4 matching), as the weight
    /// and style to request: a weight inside a `wght` range is requested as it is.
    pub fn nearest_face(&self, weight: u32, style: FaceStyle) -> Option<(u32, FaceStyle)> {
        let descriptors: Vec<FaceDescriptor> = [FaceStyle::Normal, FaceStyle::Italic]
            .into_iter()
            .filter_map(|style| self.weights(style).map(|w| w.descriptors(style)))
            .flatten()
            .collect();
        let index = css_match(descriptors.iter().copied(), weight, style)?;
        let face = descriptors.get(index)?;
        Some((weight.clamp(face.weight.min, face.weight.max), face.style))
    }
}

/// A catalog family a requested name names, and the weight a trailing weight name asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GoogleMatch<'a> {
    pub family: &'a GoogleFamily,
    pub weight: Option<u32>,
}

/// The catalog, by family key.
#[derive(Debug, Default)]
pub struct GoogleCatalog {
    commit: String,
    families: Vec<GoogleFamily>,
    keys: BTreeMap<FamilyKey, usize>,
}

impl GoogleCatalog {
    /// The google/fonts commit the catalog was generated from.
    pub fn commit(&self) -> &str {
        &self.commit
    }

    /// Families in name order.
    pub fn families(&self) -> &[GoogleFamily] {
        &self.families
    }

    pub fn get(&self, key: &FamilyKey) -> Option<&GoogleFamily> {
        self.keys
            .get(key)
            .and_then(|&index| self.families.get(index))
    }

    /// The family a requested name names, directly or through a trailing weight name (`Roboto Medium` → Roboto 500).
    pub fn find(&self, requested: &str) -> Option<GoogleMatch<'_>> {
        if let Some(family) = self.get(&FamilyKey::of(requested)) {
            return Some(GoogleMatch {
                family,
                weight: None,
            });
        }
        let split = split_weight_name(requested)?;
        self.get(&FamilyKey::of(split.family))
            .map(|family| GoogleMatch {
                family,
                weight: Some(split.weight),
            })
    }

    /// Reads the generated data; a line that does not parse is left out (the crate's tests read every line).
    pub fn parse(data: &'static str) -> GoogleCatalog {
        let mut catalog = GoogleCatalog::default();
        let mut subsets: Vec<&'static str> = Vec::new();
        for line in data.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if let Some(commit) = line.strip_prefix("@commit ") {
                catalog.commit = commit.to_owned();
                continue;
            }
            if let Some(names) = line.strip_prefix("@subsets ") {
                subsets = names.split(' ').collect();
                continue;
            }
            let Some(family) = parse_family(line, &subsets) else {
                continue;
            };
            let key = FamilyKey::of(&family.family);
            if catalog.keys.contains_key(&key) {
                continue;
            }
            catalog.keys.insert(key, catalog.families.len());
            catalog.families.push(family);
        }
        catalog
    }

    /// The number of data lines that name a family, parsed or not.
    pub fn data_lines(data: &str) -> usize {
        data.lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with('@'))
            .count()
    }
}

fn parse_generic(value: &str) -> Option<GenericFamily> {
    Some(match value {
        "sans-serif" => GenericFamily::SansSerif,
        "serif" => GenericFamily::Serif,
        "monospace" => GenericFamily::Monospace,
        "cursive" => GenericFamily::Cursive,
        "fantasy" => GenericFamily::Fantasy,
        _ => return None,
    })
}

fn parse_weight(value: &str) -> Option<u32> {
    value
        .parse::<u32>()
        .ok()
        .filter(|weight| (1..=1000).contains(weight))
}

/// `400,700` or `100-900`.
fn parse_weights(value: &str) -> Option<Weights> {
    if let Some((min, max)) = value.split_once('-') {
        let (min, max) = (parse_weight(min)?, parse_weight(max)?);
        return (min <= max).then_some(Weights::Range(WeightRange { min, max }));
    }
    let weights: Option<Vec<u32>> = value.split(',').map(parse_weight).collect();
    weights
        .filter(|weights| !weights.is_empty())
        .map(Weights::Statics)
}

fn parse_family(line: &str, subsets: &[&'static str]) -> Option<GoogleFamily> {
    let mut fields = line.split('\t');
    let family = fields.next().filter(|name| !name.is_empty())?.to_owned();
    let generic = parse_generic(fields.next()?)?;
    let licence = Licence::parse(fields.next()?)?;
    let (mut upright, mut italic) = (None, None);
    for face in fields.next()?.split(' ').filter(|f| !f.is_empty()) {
        let (style, weights) = face.split_at_checked(1)?;
        let weights = parse_weights(weights)?;
        match style {
            "n" => upright = Some(weights),
            "i" => italic = Some(weights),
            _ => return None,
        }
    }
    if upright.is_none() && italic.is_none() {
        return None;
    }
    let mut axes = Vec::new();
    for axis in fields.next()?.split(' ').filter(|a| !a.is_empty()) {
        let mut parts = axis.split(':');
        let tag = parts.next()?.to_owned();
        let min = parts.next()?.parse::<f64>().ok()?;
        let max = parts.next()?.parse::<f64>().ok()?;
        axes.push(Axis { tag, min, max });
    }
    let subsets = fields
        .next()?
        .split(' ')
        .filter(|s| !s.is_empty())
        .map(|index| {
            index
                .parse::<usize>()
                .ok()
                .and_then(|i| subsets.get(i).copied())
        })
        .collect::<Option<Vec<_>>>()?;
    if fields.next().is_some() {
        return None;
    }
    Some(GoogleFamily {
        family,
        generic,
        licence,
        upright,
        italic,
        axes,
        subsets,
    })
}

/// The catalog generated into this build.
pub fn google_catalog() -> &'static GoogleCatalog {
    static CATALOG: OnceLock<GoogleCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| GoogleCatalog::parse(DATA))
}

/// The raw generated data, for tests that check every line parses.
pub fn google_catalog_data() -> &'static str {
    DATA
}
