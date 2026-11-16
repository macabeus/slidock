//! Font requests and bindings: what the source asked for, and what renders it, with metrics read from the real font
//! file.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::Sha256;
use crate::validate::{Validate, Validator};

/// The key of a family in `presentation.fonts` and in font catalogs: ASCII letters lowercased
/// (locale-independent), JS whitespace and hyphens removed.
pub fn font_key(family: &str) -> String {
    family
        .chars()
        .filter(|c| *c != '-' && !slidock_canon::is_js_whitespace(*c))
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub enum FaceStyle {
    Normal,
    Italic,
}

fn validate_weight(weight: u32, v: &mut Validator) {
    v.at("weight", |v| v.int(i64::from(weight), 100, 900));
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct FaceRequest {
    pub weight: u32,
    pub style: FaceStyle,
}

impl Validate for FaceRequest {
    fn validate(&self, v: &mut Validator) {
        validate_weight(self.weight, v);
    }
}

/// What the source asked for. `embedded`: the source embeds the font; the compiler only records it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FontRequest {
    pub family: String,
    pub faces: Vec<FaceRequest>,
    pub postscript_names: Vec<String>,
    pub embedded: bool,
}

impl Validate for FontRequest {
    fn validate(&self, v: &mut Validator) {
        v.at("family", |v| v.string(&self.family, 1, 256));
        v.at("faces", |v| {
            v.array(self.faces.len(), 0, 64);
            self.faces.validate(v);
        });
        v.at("postscriptNames", |v| {
            v.array(self.postscript_names.len(), 0, 64);
            for (i, name) in self.postscript_names.iter().enumerate() {
                v.at(i, |v| v.string(name, 1, 256));
            }
        });
    }
}

/// Font units: -32768 to 65535.
pub type FontUnits = i32;

fn units(value: FontUnits, key: &'static str, v: &mut Validator) {
    v.at(key, |v| v.int(i64::from(value), -32_768, 65_535));
}

/// One vertical metrics table (hhea or OS/2 typo).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct VMetrics {
    pub ascender: FontUnits,
    pub descender: FontUnits,
    pub line_gap: FontUnits,
}

impl Validate for VMetrics {
    fn validate(&self, v: &mut Validator) {
        units(self.ascender, "ascender", v);
        units(self.descender, "descender", v);
        units(self.line_gap, "lineGap", v);
    }
}

/// OS/2 win metrics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WinMetrics {
    pub ascent: FontUnits,
    pub descent: FontUnits,
}

impl Validate for WinMetrics {
    fn validate(&self, v: &mut Validator) {
        units(self.ascent, "ascent", v);
        units(self.descent, "descent", v);
    }
}

/// The vertical extent of every glyph (`head`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GlyphBounds {
    pub y_min: FontUnits,
    pub y_max: FontUnits,
}

impl Validate for GlyphBounds {
    fn validate(&self, v: &mut Validator) {
        units(self.y_min, "yMin", v);
        units(self.y_max, "yMax", v);
    }
}

/// A font's vertical metrics in font units, all three tables, so the line basis picks one. `bounds` with
/// `graveTop` (the top of À's glyph box in a TrueType font) decide CoreText's synthesized leading.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FontMetrics {
    pub units_per_em: u32,
    pub hhea: VMetrics,
    pub typo: VMetrics,
    pub win: WinMetrics,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub bounds: Option<GlyphBounds>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub grave_top: Option<FontUnits>,
}

impl Validate for FontMetrics {
    fn validate(&self, v: &mut Validator) {
        v.at("unitsPerEm", |v| {
            v.int(i64::from(self.units_per_em), 16, 16_384)
        });
        v.field("hhea", &self.hhea);
        v.field("typo", &self.typo);
        v.field("win", &self.win);
        v.field("bounds", &self.bounds);
        if let Some(grave_top) = self.grave_top {
            units(grave_top, "graveTop", v);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BindingKind {
    Installed,
    Substitute,
    Generic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GenericFamily {
    SansSerif,
    Serif,
    Monospace,
    Cursive,
    Fantasy,
    /// CSS Fonts 4 `emoji`: the viewer's emoji font draws the text, and no file ships.
    Emoji,
}

/// The platforms' colour emoji families, in the order a stack for the `emoji` generic names them before it.
pub const PLATFORM_EMOJI_FAMILIES: [&str; 4] = [
    "Apple Color Emoji",
    "Segoe UI Emoji",
    "Segoe UI Symbol",
    "Noto Color Emoji",
];

impl GenericFamily {
    /// The name the IR's JSON spells.
    pub const fn as_str(self) -> &'static str {
        match self {
            GenericFamily::SansSerif => "sans-serif",
            GenericFamily::Serif => "serif",
            GenericFamily::Monospace => "monospace",
            GenericFamily::Cursive => "cursive",
            GenericFamily::Fantasy => "fantasy",
            GenericFamily::Emoji => "emoji",
        }
    }

    /// The families a font stack names before the generic itself: the platforms' emoji fonts for `emoji` (not every
    /// browser maps the generic to its platform's font), none for the others.
    pub const fn platform_families(self) -> &'static [&'static str] {
        match self {
            GenericFamily::Emoji => &PLATFORM_EMOJI_FAMILIES,
            _ => &[],
        }
    }
}

/// A face whose file measures otherwise than the binding's `metrics`, by the weight drawn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundFace {
    pub weight: u32,
    pub style: FaceStyle,
    pub metrics: FontMetrics,
}

impl Validate for BoundFace {
    fn validate(&self, v: &mut Validator) {
        validate_weight(self.weight, v);
        v.field("metrics", &self.metrics);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FontFile {
    pub sha256: Sha256,
    pub postscript_name: String,
}

/// What renders a request. `weight` is the weight a family name carried (`Roboto Medium` → 500); `metrics` are
/// those of the upright face nearest the binding's weight.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontBinding {
    pub kind: BindingKind,
    pub family: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub weight: Option<u32>,
    pub generic: GenericFamily,
    pub metrics: FontMetrics,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub faces: Option<Vec<BoundFace>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub file: Option<FontFile>,
}

impl Validate for FontBinding {
    fn validate(&self, v: &mut Validator) {
        v.at("family", |v| v.string(&self.family, 1, 256));
        if let Some(weight) = self.weight {
            validate_weight(weight, v);
        }
        v.field("metrics", &self.metrics);
        if let Some(faces) = &self.faces {
            v.at("faces", |v| {
                v.array(faces.len(), 0, 64);
                faces.validate(v);
            });
        }
        if let Some(file) = &self.file {
            v.at("file", |v| {
                v.at("postscriptName", |v| {
                    v.string(&file.postscript_name, 1, 256)
                })
            });
        }
    }
}

impl FontBinding {
    /// The weight a run draws with: a weight its family name carried wins, and a bold run raises it to 700.
    pub fn drawn_weight(&self, weight: u32) -> u32 {
        match self.weight {
            None => weight,
            Some(named) if weight >= 700 => named.max(700),
            Some(named) => named,
        }
    }

    /// The metrics of the face a run draws with.
    pub fn face_metrics(&self, weight: u32, italic: bool) -> &FontMetrics {
        let drawn = self.drawn_weight(weight);
        let style = if italic {
            FaceStyle::Italic
        } else {
            FaceStyle::Normal
        };
        self.faces
            .iter()
            .flatten()
            .find(|face| face.weight == drawn && face.style == style)
            .map_or(&self.metrics, |face| &face.metrics)
    }
}

/// A font the presentation uses: the request, and its binding once bind-fonts ran.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontEntry {
    pub request: FontRequest,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub binding: Option<FontBinding>,
}

/// A bound font.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedFontEntry {
    pub request: FontRequest,
    pub binding: FontBinding,
}

impl Validate for FontEntry {
    fn validate(&self, v: &mut Validator) {
        v.field("request", &self.request);
        v.field("binding", &self.binding);
    }
}

impl Validate for ResolvedFontEntry {
    fn validate(&self, v: &mut Validator) {
        v.field("request", &self.request);
        v.field("binding", &self.binding);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_keys_lowercase_ascii_only_and_drop_spaces_and_hyphens() {
        assert_eq!(font_key("Liberation Sans"), "liberationsans");
        assert_eq!(font_key("Helvetica-Neue Bold"), "helveticaneuebold");
        assert_eq!(font_key("Ärial ÉTÉ"), "ÄrialÉtÉ");
        assert_eq!(font_key("A\u{feff}B\u{a0}c"), "abc");
    }

    fn metrics(ascender: FontUnits) -> FontMetrics {
        let table = VMetrics {
            ascender,
            descender: -200,
            line_gap: 0,
        };
        FontMetrics {
            units_per_em: 1000,
            hhea: table.clone(),
            typo: table,
            win: WinMetrics {
                ascent: ascender,
                descent: 200,
            },
            bounds: None,
            grave_top: None,
        }
    }

    #[test]
    fn draws_the_named_weight_and_bold() {
        let binding = FontBinding {
            kind: BindingKind::Installed,
            family: "Roboto".into(),
            weight: Some(500),
            generic: GenericFamily::SansSerif,
            metrics: metrics(900),
            faces: Some(vec![BoundFace {
                weight: 700,
                style: FaceStyle::Normal,
                metrics: metrics(950),
            }]),
            file: None,
        };
        assert_eq!(binding.drawn_weight(400), 500);
        assert_eq!(binding.drawn_weight(700), 700);
        assert_eq!(binding.face_metrics(700, false).hhea.ascender, 950);
        assert_eq!(binding.face_metrics(700, true).hhea.ascender, 900);
        assert_eq!(binding.face_metrics(400, false).hhea.ascender, 900);
        let unnamed = FontBinding {
            weight: None,
            ..binding
        };
        assert_eq!(unnamed.drawn_weight(300), 300);
    }
}
