//! Colour: literal sRGB or Display P3 components, and theme references that keep DrawingML's modifiers symbolically.

use serde::{Deserialize, Serialize};

use crate::de::{literal_tag, self_tagged_union};
use crate::units::{Ratio, UnitRatio};
use crate::validate::{Validate, Validator};

/// The twelve colours of a theme (DrawingML's `clrScheme`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SchemeSlot {
    Dk1,
    Lt1,
    Dk2,
    Lt2,
    Accent1,
    Accent2,
    Accent3,
    Accent4,
    Accent5,
    Accent6,
    Hlink,
    FolHlink,
}

/// What a colour reference may name: a scheme slot, or a background/text role the colour map assigns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MappedSlot {
    Dk1,
    Lt1,
    Dk2,
    Lt2,
    Accent1,
    Accent2,
    Accent3,
    Accent4,
    Accent5,
    Accent6,
    Hlink,
    FolHlink,
    Bg1,
    Tx1,
    Bg2,
    Tx2,
}

impl MappedSlot {
    /// The name the IR's JSON spells.
    pub const fn as_str(self) -> &'static str {
        match self {
            MappedSlot::Dk1 => "dk1",
            MappedSlot::Lt1 => "lt1",
            MappedSlot::Dk2 => "dk2",
            MappedSlot::Lt2 => "lt2",
            MappedSlot::Accent1 => "accent1",
            MappedSlot::Accent2 => "accent2",
            MappedSlot::Accent3 => "accent3",
            MappedSlot::Accent4 => "accent4",
            MappedSlot::Accent5 => "accent5",
            MappedSlot::Accent6 => "accent6",
            MappedSlot::Hlink => "hlink",
            MappedSlot::FolHlink => "folHlink",
            MappedSlot::Bg1 => "bg1",
            MappedSlot::Tx1 => "tx1",
            MappedSlot::Bg2 => "bg2",
            MappedSlot::Tx2 => "tx2",
        }
    }

    /// The scheme slot this names directly, if it is one.
    pub fn scheme(self) -> Option<SchemeSlot> {
        Some(match self {
            MappedSlot::Dk1 => SchemeSlot::Dk1,
            MappedSlot::Lt1 => SchemeSlot::Lt1,
            MappedSlot::Dk2 => SchemeSlot::Dk2,
            MappedSlot::Lt2 => SchemeSlot::Lt2,
            MappedSlot::Accent1 => SchemeSlot::Accent1,
            MappedSlot::Accent2 => SchemeSlot::Accent2,
            MappedSlot::Accent3 => SchemeSlot::Accent3,
            MappedSlot::Accent4 => SchemeSlot::Accent4,
            MappedSlot::Accent5 => SchemeSlot::Accent5,
            MappedSlot::Accent6 => SchemeSlot::Accent6,
            MappedSlot::Hlink => SchemeSlot::Hlink,
            MappedSlot::FolHlink => SchemeSlot::FolHlink,
            MappedSlot::Bg1 | MappedSlot::Tx1 | MappedSlot::Bg2 | MappedSlot::Tx2 => return None,
        })
    }
}

/// DrawingML's colour modifiers, kept symbolically on theme references only; literal colours arrive applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorModOp {
    LumMod,
    LumOff,
    Tint,
    Shade,
    Alpha,
    AlphaMod,
    AlphaOff,
    SatMod,
    SatOff,
    HueMod,
    HueOff,
    Inv,
    Gray,
    Comp,
}

/// One modifier. `value` is a fraction (1 = 100%) for every operator except `hueOff`, which is in degrees; `inv`,
/// `gray` and `comp` take no value and are written with 0.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorMod {
    pub op: ColorModOp,
    pub value: Ratio,
}

impl Validate for ColorMod {
    fn validate(&self, v: &mut Validator) {
        v.field("value", &self.value);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorSpace {
    Srgb,
    P3,
}

literal_tag!(
    /// The `kind` of an [`RgbColor`].
    RgbTag = "rgb"
);
literal_tag!(
    /// The `kind` of a theme reference, and of a theme font.
    ThemeTag = "theme"
);

/// Components in [0, 1], as written: sRGB or Display P3. CMYK and system colours arrive converted to sRGB.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RgbColor {
    pub kind: RgbTag,
    pub space: ColorSpace,
    pub r: UnitRatio,
    pub g: UnitRatio,
    pub b: UnitRatio,
    pub a: UnitRatio,
}

impl RgbColor {
    pub fn new(space: ColorSpace, r: UnitRatio, g: UnitRatio, b: UnitRatio, a: UnitRatio) -> Self {
        RgbColor {
            kind: RgbTag,
            space,
            r,
            g,
            b,
            a,
        }
    }
}

impl Validate for RgbColor {
    fn validate(&self, v: &mut Validator) {
        v.field("r", &self.r);
        v.field("g", &self.g);
        v.field("b", &self.b);
        v.field("a", &self.a);
    }
}

/// A theme reference. `resolved` is computed by resolve-theme on every run, so re-theming keeps working.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeColor {
    pub kind: ThemeTag,
    pub slot: MappedSlot,
    pub mods: Vec<ColorMod>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub resolved: Option<RgbColor>,
}

/// A theme reference in the resolved phase: it carries its value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedThemeColor {
    pub kind: ThemeTag,
    pub slot: MappedSlot,
    pub mods: Vec<ColorMod>,
    pub resolved: RgbColor,
}

impl Validate for ThemeColor {
    fn validate(&self, v: &mut Validator) {
        v.at("mods", |v| {
            v.array(self.mods.len(), 0, 16);
            self.mods.validate(v);
        });
        v.field("resolved", &self.resolved);
    }
}

impl Validate for ResolvedThemeColor {
    fn validate(&self, v: &mut Validator) {
        v.at("mods", |v| {
            v.array(self.mods.len(), 0, 16);
            self.mods.validate(v);
        });
        v.field("resolved", &self.resolved);
    }
}

self_tagged_union! {
    /// A colour in the source phase.
    pub enum Color ("kind") {
        Rgb(RgbColor) = "rgb",
        Theme(ThemeColor) = "theme",
    }
}

self_tagged_union! {
    /// A colour in the resolved phase: every theme reference carries its value.
    pub enum ResolvedColor ("kind") {
        Rgb(RgbColor) = "rgb",
        Theme(ResolvedThemeColor) = "theme",
    }
}

impl Color {
    /// The literal value: the colour itself, or a theme reference's resolved value.
    pub fn rgb(&self) -> Option<&RgbColor> {
        match self {
            Color::Rgb(rgb) => Some(rgb),
            Color::Theme(theme) => theme.resolved.as_ref(),
        }
    }
}

impl ResolvedColor {
    pub fn rgb(&self) -> &RgbColor {
        match self {
            ResolvedColor::Rgb(rgb) => rgb,
            ResolvedColor::Theme(theme) => &theme.resolved,
        }
    }
}

impl From<ResolvedColor> for Color {
    fn from(color: ResolvedColor) -> Self {
        match color {
            ResolvedColor::Rgb(rgb) => Color::Rgb(rgb),
            ResolvedColor::Theme(theme) => Color::Theme(ThemeColor {
                kind: ThemeTag,
                slot: theme.slot,
                mods: theme.mods,
                resolved: Some(theme.resolved),
            }),
        }
    }
}

impl Validate for Color {
    fn validate(&self, v: &mut Validator) {
        match self {
            Color::Rgb(rgb) => rgb.validate(v),
            Color::Theme(theme) => theme.validate(v),
        }
    }
}

impl Validate for ResolvedColor {
    fn validate(&self, v: &mut Validator) {
        match self {
            ResolvedColor::Rgb(rgb) => rgb.validate(v),
            ResolvedColor::Theme(theme) => theme.validate(v),
        }
    }
}

/// The colours a theme declares, one per scheme slot (an exhaustive record).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SchemeColors {
    pub dk1: RgbColor,
    pub lt1: RgbColor,
    pub dk2: RgbColor,
    pub lt2: RgbColor,
    pub accent1: RgbColor,
    pub accent2: RgbColor,
    pub accent3: RgbColor,
    pub accent4: RgbColor,
    pub accent5: RgbColor,
    pub accent6: RgbColor,
    pub hlink: RgbColor,
    pub fol_hlink: RgbColor,
}

impl SchemeColors {
    pub fn get(&self, slot: SchemeSlot) -> &RgbColor {
        match slot {
            SchemeSlot::Dk1 => &self.dk1,
            SchemeSlot::Lt1 => &self.lt1,
            SchemeSlot::Dk2 => &self.dk2,
            SchemeSlot::Lt2 => &self.lt2,
            SchemeSlot::Accent1 => &self.accent1,
            SchemeSlot::Accent2 => &self.accent2,
            SchemeSlot::Accent3 => &self.accent3,
            SchemeSlot::Accent4 => &self.accent4,
            SchemeSlot::Accent5 => &self.accent5,
            SchemeSlot::Accent6 => &self.accent6,
            SchemeSlot::Hlink => &self.hlink,
            SchemeSlot::FolHlink => &self.fol_hlink,
        }
    }

    /// The slots with their colours, in slot order.
    pub fn entries(&self) -> [(&'static str, &RgbColor); 12] {
        [
            ("dk1", &self.dk1),
            ("lt1", &self.lt1),
            ("dk2", &self.dk2),
            ("lt2", &self.lt2),
            ("accent1", &self.accent1),
            ("accent2", &self.accent2),
            ("accent3", &self.accent3),
            ("accent4", &self.accent4),
            ("accent5", &self.accent5),
            ("accent6", &self.accent6),
            ("hlink", &self.hlink),
            ("folHlink", &self.fol_hlink),
        ]
    }
}

impl Validate for SchemeColors {
    fn validate(&self, v: &mut Validator) {
        for (key, color) in self.entries() {
            v.field(key, color);
        }
    }
}

/// A colour map: which scheme slot each role and slot shows (OOXML `clrMap`, an exhaustive record).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ColorMap {
    pub dk1: SchemeSlot,
    pub lt1: SchemeSlot,
    pub dk2: SchemeSlot,
    pub lt2: SchemeSlot,
    pub accent1: SchemeSlot,
    pub accent2: SchemeSlot,
    pub accent3: SchemeSlot,
    pub accent4: SchemeSlot,
    pub accent5: SchemeSlot,
    pub accent6: SchemeSlot,
    pub hlink: SchemeSlot,
    pub fol_hlink: SchemeSlot,
    pub bg1: SchemeSlot,
    pub tx1: SchemeSlot,
    pub bg2: SchemeSlot,
    pub tx2: SchemeSlot,
}

impl ColorMap {
    pub fn get(&self, slot: MappedSlot) -> SchemeSlot {
        match slot {
            MappedSlot::Dk1 => self.dk1,
            MappedSlot::Lt1 => self.lt1,
            MappedSlot::Dk2 => self.dk2,
            MappedSlot::Lt2 => self.lt2,
            MappedSlot::Accent1 => self.accent1,
            MappedSlot::Accent2 => self.accent2,
            MappedSlot::Accent3 => self.accent3,
            MappedSlot::Accent4 => self.accent4,
            MappedSlot::Accent5 => self.accent5,
            MappedSlot::Accent6 => self.accent6,
            MappedSlot::Hlink => self.hlink,
            MappedSlot::FolHlink => self.fol_hlink,
            MappedSlot::Bg1 => self.bg1,
            MappedSlot::Tx1 => self.tx1,
            MappedSlot::Bg2 => self.bg2,
            MappedSlot::Tx2 => self.tx2,
        }
    }
}

impl Validate for ColorMap {
    fn validate(&self, _: &mut Validator) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn accent() -> serde_json::Value {
        json!({ "kind": "theme", "slot": "accent1", "mods": [{ "op": "lumMod", "value": 0.75 }] })
    }

    fn red() -> serde_json::Value {
        json!({ "kind": "rgb", "space": "srgb", "r": 1, "g": 0, "b": 0, "a": 1 })
    }

    #[test]
    fn keeps_theme_references_symbolic_in_the_source_phase() {
        assert!(serde_json::from_value::<Color>(accent()).is_ok());
        let mut resolved = accent();
        resolved["resolved"] = red();
        assert!(serde_json::from_value::<Color>(resolved.clone()).is_ok());
        assert!(serde_json::from_value::<ResolvedColor>(accent()).is_err());
        assert!(serde_json::from_value::<ResolvedColor>(resolved).is_ok());
        assert!(serde_json::from_value::<ResolvedColor>(red()).is_ok());
    }

    #[test]
    fn refuses_unknown_slots_modifiers_and_keys() {
        let mut slot = accent();
        slot["slot"] = json!("accent7");
        assert!(serde_json::from_value::<Color>(slot).is_err());
        let mut op = accent();
        op["mods"] = json!([{ "op": "blur", "value": 1 }]);
        assert!(serde_json::from_value::<Color>(op).is_err());
        let mut css = red();
        css["css"] = json!("#f00");
        assert!(serde_json::from_value::<Color>(css).is_err());
        let mut over = red();
        over["r"] = json!(1.2);
        let color: Color = serde_json::from_value(over).unwrap();
        assert_eq!(crate::validate::validate(&color).len(), 1);
    }

    #[test]
    fn round_trips() {
        let color: Color = serde_json::from_value(accent()).unwrap();
        assert_eq!(serde_json::to_value(&color).unwrap(), accent());
    }
}
