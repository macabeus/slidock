//! Text: fonts, spacing, bullets, links, run and paragraph properties, level styles, inlines, paragraphs, autofit, body
//! properties and text bodies, in the source form (declared values, any subset) and the resolved form (cascaded,
//! theme-resolved, measured).

use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::color::{Color, ResolvedColor, ThemeTag};
use crate::de::{
    Nullable, from_value, literal_tag, self_tagged_union, split_tag, tagged_union, unknown_tag,
};
use crate::ids::NodeId;
use crate::shadow::{ResolvedShadow, SourceShadow};
use crate::units::{Pt, Ratio, Size, UnitRatio};
use crate::validate::{Validate, Validator};

/// Paragraph levels are 0-based everywhere: `Paragraph.level`, `LevelStyles` keys and `defaults.text`.
pub const LEVELS: usize = 9;
pub const MAX_LEVEL: u32 = 8;

/// The longest string one run, field or name may hold, in UTF-16 code units.
pub const MAX_STRING: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemeFontSlot {
    Major,
    Minor,
}

impl ThemeFontSlot {
    pub const fn as_str(self) -> &'static str {
        match self {
            ThemeFontSlot::Major => "major",
            ThemeFontSlot::Minor => "minor",
        }
    }
}

literal_tag!(FamilyTag = "family");

/// A named family; `theme` records the slot it came from once resolve-theme replaced a theme reference.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyFont {
    pub kind: FamilyTag,
    pub family: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub theme: Option<ThemeFontSlot>,
}

impl FamilyFont {
    pub fn new(family: impl Into<String>) -> Self {
        FamilyFont {
            kind: FamilyTag,
            family: family.into(),
            theme: None,
        }
    }
}

impl Validate for FamilyFont {
    fn validate(&self, v: &mut Validator) {
        v.at("family", |v| v.string(&self.family, 1, 256));
    }
}

/// The theme's heading or body font; source phase only.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeFont {
    pub kind: ThemeTag,
    pub slot: ThemeFontSlot,
}

self_tagged_union! {
    /// A font as declared: a family, or a theme slot.
    pub enum FontRef ("kind") {
        Family(FamilyFont) = "family",
        Theme(ThemeFont) = "theme",
    }
}

impl Validate for FontRef {
    fn validate(&self, v: &mut Validator) {
        if let FontRef::Family(family) = self {
            family.validate(v);
        }
    }
}

tagged_union! {
    /// Points, or a fraction of the paragraph's line (see text-metrics).
    pub enum Spacing ("kind") {
        Pt = "pt" { value: Pt },
        Pct = "pct" { value: Ratio },
    }
}

impl Validate for Spacing {
    fn validate(&self, v: &mut Validator) {
        match self {
            Spacing::Pt { value } => v.field("value", value),
            Spacing::Pct { value } => v.at("value", |v| {
                value.validate(v);
                v.min(value.to_f64(), 0.0, false);
            }),
        }
    }
}

/// What "single" line spacing means, which apps disagree on: 1.2 × the size (PowerPoint), the height a font metric
/// table gives (hhea, OS/2 typo, OS/2 win), or CoreText's line (Keynote). Each frontend writes its app's basis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineBasis {
    #[serde(rename = "size-1.2")]
    Size1_2,
    Hhea,
    Typo,
    Win,
    Coretext,
}

impl LineBasis {
    /// The name the IR's JSON spells.
    pub const fn as_str(self) -> &'static str {
        match self {
            LineBasis::Size1_2 => "size-1.2",
            LineBasis::Hhea => "hhea",
            LineBasis::Typo => "typo",
            LineBasis::Win => "win",
            LineBasis::Coretext => "coretext",
        }
    }
}

tagged_union! {
    pub enum LineSpacing ("kind") {
        Multiple = "multiple" { value: Ratio, basis: LineBasis },
        Exact = "exact" { value: Size },
        AtLeast = "atLeast" { value: Size },
        AtMost = "atMost" { value: Size },
        Between = "between" { min: Size, max: Size },
    }
}

impl Validate for LineSpacing {
    fn validate(&self, v: &mut Validator) {
        match self {
            LineSpacing::Multiple { value, .. } => v.at("value", |v| {
                value.validate(v);
                v.min(value.to_f64(), 0.0, false);
            }),
            LineSpacing::Exact { value }
            | LineSpacing::AtLeast { value }
            | LineSpacing::AtMost { value } => {
                v.field("value", value);
            }
            LineSpacing::Between { min, max } => {
                v.field("min", min);
                v.field("max", max);
            }
        }
    }
}

tagged_union! {
    /// The line height a backend prints, computed by text-metrics: a unitless ratio of the size, or points.
    pub enum LineHeight ("kind") {
        Ratio = "ratio" { value: Ratio },
        Pt = "pt" { value: Size },
    }
}

impl Validate for LineHeight {
    fn validate(&self, v: &mut Validator) {
        match self {
            LineHeight::Ratio { value } => v.at("value", |v| {
                value.validate(v);
                v.min(value.to_f64(), 0.0, false);
            }),
            LineHeight::Pt { value } => v.field("value", value),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AutoNumScheme {
    ArabicPeriod,
    ArabicParenR,
    ArabicParenBoth,
    ArabicPlain,
    AlphaLcPeriod,
    AlphaUcPeriod,
    AlphaLcParenR,
    AlphaUcParenR,
    AlphaLcParenBoth,
    AlphaUcParenBoth,
    RomanLcPeriod,
    RomanUcPeriod,
    RomanLcParenR,
    RomanUcParenR,
    RomanLcParenBoth,
    RomanUcParenBoth,
}

/// The highest auto-number start.
pub const MAX_START_AT: u32 = 32_767;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AutoNumbering {
    pub scheme: AutoNumScheme,
    pub start_at: u32,
}

impl Validate for AutoNumbering {
    fn validate(&self, v: &mut Validator) {
        v.at("startAt", |v| {
            v.int(i64::from(self.start_at), 0, i64::from(MAX_START_AT))
        });
    }
}

tagged_union! {
    /// A bullet's size: a fraction of the text size, points, or the text size.
    pub enum BulletSize ("kind") {
        Pct = "pct" { value: Ratio },
        Pt = "pt" { value: Size },
        Text = "text" {},
    }
}

impl Validate for BulletSize {
    fn validate(&self, v: &mut Validator) {
        match self {
            BulletSize::Pct { value } => v.at("value", |v| {
                value.validate(v);
                v.min(value.to_f64(), 0.0, false);
            }),
            BulletSize::Pt { value } => v.field("value", value),
            BulletSize::Text {} => {}
        }
    }
}

/// A bullet property that follows the text (`"text"`) or states its own value.
#[derive(Clone, Debug, PartialEq)]
pub enum OrText<T> {
    Text,
    Value(T),
}

impl<T: Serialize> Serialize for OrText<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            OrText::Text => serializer.serialize_str("text"),
            OrText::Value(value) => value.serialize(serializer),
        }
    }
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for OrText<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::String(text) if text == "text" => Ok(OrText::Text),
            value => from_value(value).map(OrText::Value),
        }
    }
}

impl<T: Validate> Validate for OrText<T> {
    fn validate(&self, v: &mut Validator) {
        if let OrText::Value(value) = self {
            value.validate(v);
        }
    }
}

tagged_union! {
    /// Every bullet a presentation between passes may hold: as written (a character or an auto-number, each
    /// property optionally following the text), or labelled by number-lists.
    pub enum Bullet ("kind") {
        None = "none" {},
        Char = "char" {
            char: String,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            font: Option<OrText<FontRef>>,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            color: Option<OrText<Color>>,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            size: Option<BulletSize>,
        },
        Auto = "auto" {
            scheme: AutoNumScheme,
            start_at: u32,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            font: Option<OrText<FontRef>>,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            color: Option<OrText<Color>>,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            size: Option<BulletSize>,
        },
        Label = "label" {
            text: String,
            font: FamilyFont,
            size: Size,
            color: Color,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            auto: Option<AutoNumbering>,
        },
    }
}

tagged_union! {
    /// A bullet number-lists computed: the label text with its own font, size and colour.
    pub enum ResolvedBullet ("kind") {
        None = "none" {},
        Label = "label" {
            text: String,
            font: FamilyFont,
            size: Size,
            color: ResolvedColor,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            auto: Option<AutoNumbering>,
        },
    }
}

impl Bullet {
    pub fn kind(&self) -> &'static str {
        match self {
            Bullet::None {} => "none",
            Bullet::Char { .. } => "char",
            Bullet::Auto { .. } => "auto",
            Bullet::Label { .. } => "label",
        }
    }
}

fn validate_label<C: Validate>(
    text: &str,
    font: &FamilyFont,
    size: &Size,
    color: &C,
    auto: &Option<AutoNumbering>,
    v: &mut Validator,
) {
    v.at("text", |v| v.string(text, 0, 64));
    v.field("font", font);
    v.field("size", size);
    v.field("color", color);
    v.field("auto", auto);
}

impl Validate for Bullet {
    fn validate(&self, v: &mut Validator) {
        match self {
            Bullet::None {} => {}
            Bullet::Char {
                char,
                font,
                color,
                size,
            } => {
                v.at("char", |v| v.string(char, 1, 8));
                v.field("font", font);
                v.field("color", color);
                v.field("size", size);
            }
            Bullet::Auto {
                start_at,
                font,
                color,
                size,
                ..
            } => {
                v.at("startAt", |v| {
                    v.int(i64::from(*start_at), 0, i64::from(MAX_START_AT))
                });
                v.field("font", font);
                v.field("color", color);
                v.field("size", size);
            }
            Bullet::Label {
                text,
                font,
                size,
                color,
                auto,
            } => validate_label(text, font, size, color, auto, v),
        }
    }
}

impl Validate for ResolvedBullet {
    fn validate(&self, v: &mut Validator) {
        if let ResolvedBullet::Label {
            text,
            font,
            size,
            color,
            auto,
        } = self
        {
            validate_label(text, font, size, color, auto, v);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkAction {
    Next,
    Prev,
    First,
    Last,
}

impl LinkAction {
    /// The name the IR's JSON spells.
    pub const fn as_str(self) -> &'static str {
        match self {
            LinkAction::Next => "next",
            LinkAction::Prev => "prev",
            LinkAction::First => "first",
            LinkAction::Last => "last",
        }
    }
}

tagged_union! {
    pub enum Link ("kind") {
        Url = "url" { url: String },
        Slide = "slide" { slide: NodeId },
        Action = "action" { action: LinkAction },
    }
}

impl Validate for Link {
    fn validate(&self, v: &mut Validator) {
        if let Link::Url { url } = self {
            v.at("url", |v| v.string(url, 1, 2048));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Decoration {
    None,
    Single,
    Double,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Caps {
    None,
    All,
    Small,
}

/// The keys a cascaded run holds (the resolved run's keys), in declaration order.
pub const RUN_PROP_KEYS: [&str; 13] = [
    "font",
    "size",
    "weight",
    "italic",
    "underline",
    "strike",
    "color",
    "highlight",
    "baseline",
    "caps",
    "letterSpacing",
    "link",
    "synthesizeBold",
];

fn validate_size(size: &Size, v: &mut Validator) {
    v.at("size", |v| {
        size.validate(v);
        v.max(size.to_f64(), 4000.0);
    });
}

fn validate_weight(weight: u32, v: &mut Validator) {
    v.at("weight", |v| v.int(i64::from(weight), 100, 900));
}

fn validate_baseline(baseline: &Ratio, v: &mut Validator) {
    v.at("baseline", |v| {
        baseline.validate(v);
        v.min(baseline.to_f64(), -1.0, false);
        v.max(baseline.to_f64(), 1.0);
    });
}

fn validate_script_scale(scale: &UnitRatio, v: &mut Validator) {
    v.at("scriptScale", |v| {
        scale.validate(v);
        v.min(scale.to_f64(), 0.0, true);
    });
}

/// Complete run properties as a source declares them (`defaults.text`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RunProps {
    pub font: FontRef,
    pub size: Size,
    pub weight: u32,
    pub italic: bool,
    pub underline: Decoration,
    pub strike: Decoration,
    pub color: Color,
    #[serde(with = "crate::de::nullable")]
    pub highlight: Option<Color>,
    /// A fraction of the size: positive raises (superscript), negative lowers.
    pub baseline: Ratio,
    pub caps: Caps,
    pub letter_spacing: Pt,
    #[serde(with = "crate::de::nullable")]
    pub link: Option<Link>,
    /// Whether a bold run in a family without a bold face draws the regular face emboldened (PowerPoint, Google
    /// Slides, browsers); Keynote draws the regular face as it is.
    pub synthesize_bold: bool,
    /// The fraction of its size a raised or lowered run draws at, which source apps apply themselves;
    /// cascade-text folds it into `size`.
    pub script_scale: UnitRatio,
    /// The glyphs' shadow; absent is none.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub shadow: Option<SourceShadow>,
}

impl Validate for RunProps {
    fn validate(&self, v: &mut Validator) {
        v.field("font", &self.font);
        validate_size(&self.size, v);
        validate_weight(self.weight, v);
        v.field("color", &self.color);
        v.field("highlight", &self.highlight);
        validate_baseline(&self.baseline, v);
        v.field("letterSpacing", &self.letter_spacing);
        v.field("link", &self.link);
        validate_script_scale(&self.script_scale, v);
        v.field("shadow", &self.shadow);
    }
}

/// Run properties as declared: any subset. `highlight`, `link` and `shadow` may be absent, `null` or a value.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PartialRunProps {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub font: Option<FontRef>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub size: Option<Size>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub weight: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub italic: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub underline: Option<Decoration>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub strike: Option<Decoration>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub color: Option<Color>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::double_option"
    )]
    pub highlight: Option<Option<Color>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub baseline: Option<Ratio>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub caps: Option<Caps>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub letter_spacing: Option<Pt>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::double_option"
    )]
    pub link: Option<Option<Link>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub synthesize_bold: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub script_scale: Option<UnitRatio>,
    /// The glyphs' shadow. Not one of [`RUN_PROP_KEYS`]: once cascaded, an absent shadow is none.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::double_option"
    )]
    pub shadow: Nullable<SourceShadow>,
}

impl PartialRunProps {
    /// The keys of [`RUN_PROP_KEYS`] this subset lacks (absent, not `null`).
    pub fn missing_keys(&self) -> Vec<&'static str> {
        let present = [
            self.font.is_some(),
            self.size.is_some(),
            self.weight.is_some(),
            self.italic.is_some(),
            self.underline.is_some(),
            self.strike.is_some(),
            self.color.is_some(),
            self.highlight.is_some(),
            self.baseline.is_some(),
            self.caps.is_some(),
            self.letter_spacing.is_some(),
            self.link.is_some(),
            self.synthesize_bold.is_some(),
        ];
        RUN_PROP_KEYS
            .iter()
            .zip(present)
            .filter(|(_, present)| !present)
            .map(|(key, _)| *key)
            .collect()
    }
}

impl Validate for PartialRunProps {
    fn validate(&self, v: &mut Validator) {
        v.field("font", &self.font);
        if let Some(size) = &self.size {
            validate_size(size, v);
        }
        if let Some(weight) = self.weight {
            validate_weight(weight, v);
        }
        v.field("color", &self.color);
        v.field(
            "highlight",
            &self.highlight.as_ref().and_then(Option::as_ref),
        );
        if let Some(baseline) = &self.baseline {
            validate_baseline(baseline, v);
        }
        v.field("letterSpacing", &self.letter_spacing);
        v.field("link", &self.link.as_ref().and_then(Option::as_ref));
        if let Some(scale) = &self.script_scale {
            validate_script_scale(scale, v);
        }
        v.field("shadow", &self.shadow.as_ref().and_then(Option::as_ref));
    }
}

/// Cascaded, theme-resolved run properties, `size` being the size drawn. Sizes stay unscaled by a shrink: the
/// printer applies a stored `fontScale`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ResolvedRunProps {
    pub font: FamilyFont,
    pub size: Size,
    pub weight: u32,
    pub italic: bool,
    pub underline: Decoration,
    pub strike: Decoration,
    pub color: ResolvedColor,
    #[serde(with = "crate::de::nullable")]
    pub highlight: Option<ResolvedColor>,
    pub baseline: Ratio,
    pub caps: Caps,
    pub letter_spacing: Pt,
    #[serde(with = "crate::de::nullable")]
    pub link: Option<Link>,
    pub synthesize_bold: bool,
    /// The glyphs' shadow; absent is none, and a cascaded `null` reads as absent.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::nullish"
    )]
    pub shadow: Option<ResolvedShadow>,
}

impl Validate for ResolvedRunProps {
    fn validate(&self, v: &mut Validator) {
        v.field("font", &self.font);
        validate_size(&self.size, v);
        validate_weight(self.weight, v);
        v.field("color", &self.color);
        v.field("highlight", &self.highlight);
        validate_baseline(&self.baseline, v);
        v.field("letterSpacing", &self.letter_spacing);
        v.field("link", &self.link);
        v.field("shadow", &self.shadow);
    }
}

/// The size of a run's properties, if they state one.
pub trait RunSize {
    fn run_size(&self) -> Option<Size>;
}

impl RunSize for PartialRunProps {
    fn run_size(&self) -> Option<Size> {
        self.size
    }
}

impl RunSize for ResolvedRunProps {
    fn run_size(&self) -> Option<Size> {
        Some(self.size)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

/// The keys a cascaded paragraph holds before text-metrics.
pub const CASCADED_PARA_KEYS: [&str; 10] = [
    "align",
    "marginLeft",
    "marginRight",
    "indent",
    "spaceBefore",
    "spaceAfter",
    "lineSpacing",
    "tabSize",
    "bullet",
    "rtl",
];

/// The paragraph values text-metrics computes.
pub const LINE_METRIC_KEYS: [&str; 3] = ["lineHeight", "spaceBeforePt", "spaceAfterPt"];

/// Complete paragraph properties as a source declares them (`defaults.text`); the line metrics come later.
/// `leadingSkew`: where a line's leading goes when its basis does not centre the text, as a ratio of the size;
/// `lineTailPt`: how far the last line box reaches below where the text ends. Both are absent when zero.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ParaProps {
    pub align: Align,
    pub margin_left: Pt,
    pub margin_right: Pt,
    pub indent: Pt,
    pub space_before: Spacing,
    pub space_after: Spacing,
    pub line_spacing: LineSpacing,
    pub tab_size: Size,
    pub bullet: Bullet,
    pub rtl: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub line_height: Option<LineHeight>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub space_before_pt: Option<Pt>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub space_after_pt: Option<Pt>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub leading_skew: Option<Ratio>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub line_tail_pt: Option<Pt>,
}

impl Validate for ParaProps {
    fn validate(&self, v: &mut Validator) {
        v.field("marginLeft", &self.margin_left);
        v.field("marginRight", &self.margin_right);
        v.field("indent", &self.indent);
        v.field("spaceBefore", &self.space_before);
        v.field("spaceAfter", &self.space_after);
        v.field("lineSpacing", &self.line_spacing);
        v.field("tabSize", &self.tab_size);
        v.field("bullet", &self.bullet);
        v.field("lineHeight", &self.line_height);
        v.field("spaceBeforePt", &self.space_before_pt);
        v.field("spaceAfterPt", &self.space_after_pt);
        v.field("leadingSkew", &self.leading_skew);
        v.field("lineTailPt", &self.line_tail_pt);
    }
}

/// Paragraph properties as declared: any subset.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PartialParaProps {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub align: Option<Align>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub margin_left: Option<Pt>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub margin_right: Option<Pt>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub indent: Option<Pt>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub space_before: Option<Spacing>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub space_after: Option<Spacing>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub line_spacing: Option<LineSpacing>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub tab_size: Option<Size>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub bullet: Option<Bullet>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub rtl: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub line_height: Option<LineHeight>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub space_before_pt: Option<Pt>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub space_after_pt: Option<Pt>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub leading_skew: Option<Ratio>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub line_tail_pt: Option<Pt>,
}

impl PartialParaProps {
    /// The keys of [`CASCADED_PARA_KEYS`] this subset lacks.
    pub fn missing_cascaded_keys(&self) -> Vec<&'static str> {
        let present = [
            self.align.is_some(),
            self.margin_left.is_some(),
            self.margin_right.is_some(),
            self.indent.is_some(),
            self.space_before.is_some(),
            self.space_after.is_some(),
            self.line_spacing.is_some(),
            self.tab_size.is_some(),
            self.bullet.is_some(),
            self.rtl.is_some(),
        ];
        missing(&CASCADED_PARA_KEYS, &present)
    }

    /// The keys of [`LINE_METRIC_KEYS`] this subset lacks.
    pub fn missing_line_metric_keys(&self) -> Vec<&'static str> {
        let present = [
            self.line_height.is_some(),
            self.space_before_pt.is_some(),
            self.space_after_pt.is_some(),
        ];
        missing(&LINE_METRIC_KEYS, &present)
    }
}

fn missing(keys: &[&'static str], present: &[bool]) -> Vec<&'static str> {
    keys.iter()
        .zip(present)
        .filter(|(_, present)| !**present)
        .map(|(key, _)| *key)
        .collect()
}

impl Validate for PartialParaProps {
    fn validate(&self, v: &mut Validator) {
        v.field("marginLeft", &self.margin_left);
        v.field("marginRight", &self.margin_right);
        v.field("indent", &self.indent);
        v.field("spaceBefore", &self.space_before);
        v.field("spaceAfter", &self.space_after);
        v.field("lineSpacing", &self.line_spacing);
        v.field("tabSize", &self.tab_size);
        v.field("bullet", &self.bullet);
        v.field("lineHeight", &self.line_height);
        v.field("spaceBeforePt", &self.space_before_pt);
        v.field("spaceAfterPt", &self.space_after_pt);
        v.field("leadingSkew", &self.leading_skew);
        v.field("lineTailPt", &self.line_tail_pt);
    }
}

/// Cascaded paragraph properties with a labelled bullet and the line metrics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ResolvedParaProps {
    pub align: Align,
    pub margin_left: Pt,
    pub margin_right: Pt,
    pub indent: Pt,
    pub space_before: Spacing,
    pub space_after: Spacing,
    pub line_spacing: LineSpacing,
    pub tab_size: Size,
    pub bullet: ResolvedBullet,
    pub rtl: bool,
    pub line_height: LineHeight,
    pub space_before_pt: Pt,
    pub space_after_pt: Pt,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub leading_skew: Option<Ratio>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub line_tail_pt: Option<Pt>,
}

impl Validate for ResolvedParaProps {
    fn validate(&self, v: &mut Validator) {
        v.field("marginLeft", &self.margin_left);
        v.field("marginRight", &self.margin_right);
        v.field("indent", &self.indent);
        v.field("spaceBefore", &self.space_before);
        v.field("spaceAfter", &self.space_after);
        v.field("lineSpacing", &self.line_spacing);
        v.field("tabSize", &self.tab_size);
        v.field("bullet", &self.bullet);
        v.field("lineHeight", &self.line_height);
        v.field("spaceBeforePt", &self.space_before_pt);
        v.field("spaceAfterPt", &self.space_after_pt);
        v.field("leadingSkew", &self.leading_skew);
        v.field("lineTailPt", &self.line_tail_pt);
    }
}

/// A level's declared styles.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LevelStyle {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub para: Option<PartialParaProps>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub run: Option<PartialRunProps>,
}

impl Validate for LevelStyle {
    fn validate(&self, v: &mut Validator) {
        v.field("para", &self.para);
        v.field("run", &self.run);
    }
}

/// The level keys `"0"` to `"8"`.
pub const LEVEL_KEYS: [&str; LEVELS] = ["0", "1", "2", "3", "4", "5", "6", "7", "8"];

crate::de::partial_record! {
    keys: LEVEL_KEYS;
    /// Styles per paragraph level (a shape's `lstStyle`, a master's text styles): declared values only, any levels.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct LevelStyles {
        #[serde(
            rename = "0",
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional"
        )]
        pub l0: Option<Box<LevelStyle>>,
        #[serde(
            rename = "1",
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional"
        )]
        pub l1: Option<Box<LevelStyle>>,
        #[serde(
            rename = "2",
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional"
        )]
        pub l2: Option<Box<LevelStyle>>,
        #[serde(
            rename = "3",
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional"
        )]
        pub l3: Option<Box<LevelStyle>>,
        #[serde(
            rename = "4",
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional"
        )]
        pub l4: Option<Box<LevelStyle>>,
        #[serde(
            rename = "5",
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional"
        )]
        pub l5: Option<Box<LevelStyle>>,
        #[serde(
            rename = "6",
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional"
        )]
        pub l6: Option<Box<LevelStyle>>,
        #[serde(
            rename = "7",
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional"
        )]
        pub l7: Option<Box<LevelStyle>>,
        #[serde(
            rename = "8",
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional"
        )]
        pub l8: Option<Box<LevelStyle>>,
    }
}

impl LevelStyles {
    /// The levels in order, with their keys.
    pub fn levels(&self) -> [(&'static str, Option<&LevelStyle>); LEVELS] {
        [
            ("0", self.l0.as_deref()),
            ("1", self.l1.as_deref()),
            ("2", self.l2.as_deref()),
            ("3", self.l3.as_deref()),
            ("4", self.l4.as_deref()),
            ("5", self.l5.as_deref()),
            ("6", self.l6.as_deref()),
            ("7", self.l7.as_deref()),
            ("8", self.l8.as_deref()),
        ]
    }

    pub fn get(&self, level: usize) -> Option<&LevelStyle> {
        self.levels().get(level).and_then(|(_, style)| *style)
    }

    /// The nine level slots, in level order.
    pub fn slots_mut(&mut self) -> [&mut Option<Box<LevelStyle>>; LEVELS] {
        [
            &mut self.l0,
            &mut self.l1,
            &mut self.l2,
            &mut self.l3,
            &mut self.l4,
            &mut self.l5,
            &mut self.l6,
            &mut self.l7,
            &mut self.l8,
        ]
    }

    /// The slot of level `level` (0–8).
    pub fn slot_mut(&mut self, level: usize) -> Option<&mut Option<Box<LevelStyle>>> {
        self.slots_mut().into_iter().nth(level)
    }
}

impl Validate for LevelStyles {
    fn validate(&self, v: &mut Validator) {
        for (key, style) in self.levels() {
            if let Some(style) = style {
                v.at(key, |v| style.validate(v));
            }
        }
    }
}

/// A level's complete defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LevelDefaults {
    pub para: ParaProps,
    pub run: RunProps,
}

impl Validate for LevelDefaults {
    fn validate(&self, v: &mut Validator) {
        v.field("para", &self.para);
        v.field("run", &self.run);
    }
}

/// The format's defaults: complete properties for all nine levels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefaultTextStyles {
    #[serde(rename = "0")]
    pub l0: LevelDefaults,
    #[serde(rename = "1")]
    pub l1: LevelDefaults,
    #[serde(rename = "2")]
    pub l2: LevelDefaults,
    #[serde(rename = "3")]
    pub l3: LevelDefaults,
    #[serde(rename = "4")]
    pub l4: LevelDefaults,
    #[serde(rename = "5")]
    pub l5: LevelDefaults,
    #[serde(rename = "6")]
    pub l6: LevelDefaults,
    #[serde(rename = "7")]
    pub l7: LevelDefaults,
    #[serde(rename = "8")]
    pub l8: LevelDefaults,
}

impl DefaultTextStyles {
    pub fn levels(&self) -> [(&'static str, &LevelDefaults); LEVELS] {
        [
            ("0", &self.l0),
            ("1", &self.l1),
            ("2", &self.l2),
            ("3", &self.l3),
            ("4", &self.l4),
            ("5", &self.l5),
            ("6", &self.l6),
            ("7", &self.l7),
            ("8", &self.l8),
        ]
    }

    /// The defaults of `level`, clamped to the last level.
    pub fn get(&self, level: usize) -> &LevelDefaults {
        match level {
            0 => &self.l0,
            1 => &self.l1,
            2 => &self.l2,
            3 => &self.l3,
            4 => &self.l4,
            5 => &self.l5,
            6 => &self.l6,
            7 => &self.l7,
            _ => &self.l8,
        }
    }
}

impl Validate for DefaultTextStyles {
    fn validate(&self, v: &mut Validator) {
        for (key, level) in self.levels() {
            v.field(key, level);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldKind {
    SlideNumber,
    SlideCount,
    Date,
    Time,
}

impl FieldKind {
    /// The name the IR's JSON spells.
    pub const fn as_str(self) -> &'static str {
        match self {
            FieldKind::SlideNumber => "slideNumber",
            FieldKind::SlideCount => "slideCount",
            FieldKind::Date => "date",
            FieldKind::Time => "time",
        }
    }
}

/// An inline: a run of text, a line break, a tab, or a field (which keeps its value as written: the backend
/// prints `cached`, the runtime updates slide numbers). `R` is the run properties: [`PartialRunProps`] in the
/// source phase, [`ResolvedRunProps`] once cascaded. A run's or field's own `lineHeight` is computed by
/// text-metrics when it differs from its paragraph's.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Inline<R> {
    #[serde(rename_all = "camelCase")]
    Run {
        text: String,
        props: R,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional"
        )]
        line_height: Option<LineHeight>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional"
        )]
        leading_skew: Option<Ratio>,
    },
    Break {
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional"
        )]
        props: Option<R>,
    },
    Tab {},
    #[serde(rename_all = "camelCase")]
    Field {
        field: FieldKind,
        cached: String,
        props: R,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional"
        )]
        line_height: Option<LineHeight>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional"
        )]
        leading_skew: Option<Ratio>,
    },
}

impl<'de, R: DeserializeOwned> Deserialize<'de> for Inline<R> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, rename_all = "camelCase")]
        struct RunWire<R> {
            text: String,
            props: R,
            #[serde(default, with = "crate::de::optional")]
            line_height: Option<LineHeight>,
            #[serde(default, with = "crate::de::optional")]
            leading_skew: Option<Ratio>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, bound(deserialize = "R: DeserializeOwned"))]
        struct BreakWire<R> {
            #[serde(default = "none", with = "crate::de::optional")]
            props: Option<R>,
        }
        fn none<R>() -> Option<R> {
            None
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct TabWire {}
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, rename_all = "camelCase")]
        struct FieldWire<R> {
            field: FieldKind,
            cached: String,
            props: R,
            #[serde(default, with = "crate::de::optional")]
            line_height: Option<LineHeight>,
            #[serde(default, with = "crate::de::optional")]
            leading_skew: Option<Ratio>,
        }
        let (wire, payload) = split_tag(deserializer, "kind")?;
        match wire.as_str() {
            "run" => {
                let RunWire {
                    text,
                    props,
                    line_height,
                    leading_skew,
                } = from_value(payload)?;
                Ok(Inline::Run {
                    text,
                    props,
                    line_height,
                    leading_skew,
                })
            }
            "break" => {
                from_value::<BreakWire<R>, _>(payload).map(|w| Inline::Break { props: w.props })
            }
            "tab" => from_value::<TabWire, _>(payload).map(|_| Inline::Tab {}),
            "field" => {
                let FieldWire {
                    field,
                    cached,
                    props,
                    line_height,
                    leading_skew,
                } = from_value(payload)?;
                Ok(Inline::Field {
                    field,
                    cached,
                    props,
                    line_height,
                    leading_skew,
                })
            }
            other => Err(unknown_tag(
                other,
                "kind",
                &["run", "break", "tab", "field"],
            )),
        }
    }
}

impl<R> Inline<R> {
    pub fn kind(&self) -> &'static str {
        match self {
            Inline::Run { .. } => "run",
            Inline::Break { .. } => "break",
            Inline::Tab {} => "tab",
            Inline::Field { .. } => "field",
        }
    }

    /// The inline's run properties: a run's or field's, a break's when it has them.
    pub fn props(&self) -> Option<&R> {
        match self {
            Inline::Run { props, .. } | Inline::Field { props, .. } => Some(props),
            Inline::Break { props } => props.as_ref(),
            Inline::Tab {} => None,
        }
    }
}

impl<R: Validate> Validate for Inline<R> {
    fn validate(&self, v: &mut Validator) {
        match self {
            Inline::Run {
                text,
                props,
                line_height,
                leading_skew,
            } => {
                v.at("text", |v| v.string(text, 0, MAX_STRING));
                v.field("props", props);
                v.field("lineHeight", line_height);
                v.field("leadingSkew", leading_skew);
            }
            Inline::Break { props } => v.field("props", props),
            Inline::Tab {} => {}
            Inline::Field {
                cached,
                props,
                line_height,
                leading_skew,
                ..
            } => {
                v.at("cached", |v| v.string(cached, 0, 256));
                v.field("props", props);
                v.field("lineHeight", line_height);
                v.field("leadingSkew", leading_skew);
            }
        }
    }
}

pub type SourceInline = Inline<PartialRunProps>;
pub type ResolvedInline = Inline<ResolvedRunProps>;

/// Which inline sets a paragraph's line box: the run or field with the largest size, the first of equals, or
/// `None` when it has neither and the paragraph mark (`endProps`) does. An empty run draws nothing, so it is
/// skipped. text-metrics measures it; printers size the paragraph's line box with it.
pub fn strut_index<R: RunSize>(inlines: &[Inline<R>]) -> Option<usize> {
    let mut strut: Option<(usize, Option<Size>)> = None;
    for (index, inline) in inlines.iter().enumerate() {
        let props = match inline {
            Inline::Field { props, .. } => props,
            Inline::Run { text, props, .. } if !text.is_empty() => props,
            _ => continue,
        };
        let size = props.run_size();
        // An absent size is -Infinity: never larger, but a first candidate.
        let larger = match (&strut, size) {
            (None, _) => true,
            (Some((_, Some(largest))), Some(size)) => size > *largest,
            (Some((_, None)), Some(_)) => true,
            (Some(_), None) => false,
        };
        if larger {
            strut = Some((index, size));
        }
    }
    strut.map(|(index, _)| index)
}

/// A paragraph level, 0 to 8.
pub type Level = u32;

/// A paragraph: its level, properties, inlines and the paragraph mark's properties, which size an empty
/// paragraph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    deny_unknown_fields,
    rename_all = "camelCase",
    bound(
        serialize = "P: Serialize, R: Serialize",
        deserialize = "P: Deserialize<'de>, R: DeserializeOwned"
    )
)]
pub struct Paragraph<P, R> {
    pub level: Level,
    pub props: P,
    pub inlines: Vec<Inline<R>>,
    #[serde(
        default = "Option::default",
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub end_props: Option<R>,
}

pub type SourceParagraph = Paragraph<PartialParaProps, PartialRunProps>;
pub type ResolvedParagraph = Paragraph<ResolvedParaProps, ResolvedRunProps>;

impl<P, R> Paragraph<P, R> {
    /// Whether the paragraph's inlines, if any, are all empty runs. Presentation apps draw it without a bullet,
    /// and its list numbering skips it.
    pub fn shows_no_text(&self) -> bool {
        self.inlines
            .iter()
            .all(|inline| matches!(inline, Inline::Run { text, .. } if text.is_empty()))
    }
}

impl<P: Validate, R: Validate> Validate for Paragraph<P, R> {
    fn validate(&self, v: &mut Validator) {
        v.at("level", |v| {
            v.int(i64::from(self.level), 0, i64::from(MAX_LEVEL))
        });
        v.field("props", &self.props);
        v.field("inlines", &self.inlines);
        v.field("endProps", &self.end_props);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnchorX {
    Left,
    Center,
    Right,
}

/// How the source app searches for a shrink's scale when it computes one: the scales it tries, largest first, and
/// whether it takes line spacing off with them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShrinkSteps {
    /// PowerPoint's steps, which LibreOffice takes (`editeng` `constScaleLevels`): line spacing down 10 %, then the
    /// font down 7.5 % at a time to 25 % with line spacing down 10 % or 20 %.
    Office,
    /// Keynote's whole percents of the font size, 100 % down to 1 %, with no line spacing taken off.
    Percent,
}

tagged_union! {
    /// Autofit as declared: a stored shrink keeps the source app's scale; an unstored one has none until text-metrics
    /// fits it by `steps` (`percent` when absent), which a scale makes moot (cascade-text and text-metrics drop them).
    /// `fontScale` multiplies run, label and strut sizes; `lineSpacingReduction` is subtracted from line spacing
    /// multiples; `fitted` is true when the scale is the source app's: stored, or fitted as the app fits it.
    pub enum Autofit ("kind") {
        None = "none" {},
        Shrink = "shrink" {
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            font_scale: Option<UnitRatio>,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            line_spacing_reduction: Option<UnitRatio>,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            fitted: Option<bool>,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            steps: Option<ShrinkSteps>,
        },
        Resize = "resize" { width: bool, height: bool, anchor_x: AnchorX },
    }
}

tagged_union! {
    /// Autofit once measured: a shrink has its scale, reduction and whether it was fitted.
    pub enum ResolvedAutofit ("kind") {
        None = "none" {},
        Shrink = "shrink" { font_scale: UnitRatio, line_spacing_reduction: UnitRatio, fitted: bool },
        Resize = "resize" { width: bool, height: bool, anchor_x: AnchorX },
    }
}

fn validate_font_scale(scale: &UnitRatio, v: &mut Validator) {
    v.at("fontScale", |v| {
        scale.validate(v);
        v.min(scale.to_f64(), 0.0, true);
    });
}

impl Autofit {
    pub fn kind(&self) -> &'static str {
        match self {
            Autofit::None {} => "none",
            Autofit::Shrink { .. } => "shrink",
            Autofit::Resize { .. } => "resize",
        }
    }
}

impl Validate for Autofit {
    fn validate(&self, v: &mut Validator) {
        if let Autofit::Shrink {
            font_scale,
            line_spacing_reduction,
            ..
        } = self
        {
            if let Some(scale) = font_scale {
                validate_font_scale(scale, v);
            }
            v.field("lineSpacingReduction", line_spacing_reduction);
        }
    }
}

impl Validate for ResolvedAutofit {
    fn validate(&self, v: &mut Validator) {
        if let ResolvedAutofit::Shrink {
            font_scale,
            line_spacing_reduction,
            ..
        } = self
        {
            validate_font_scale(font_scale, v);
            v.field("lineSpacingReduction", line_spacing_reduction);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Insets {
    pub l: Pt,
    pub t: Pt,
    pub r: Pt,
    pub b: Pt,
}

impl Validate for Insets {
    fn validate(&self, v: &mut Validator) {
        v.field("l", &self.l);
        v.field("t", &self.t);
        v.field("r", &self.r);
        v.field("b", &self.b);
    }
}

/// Any sides of the insets, each inheriting on its own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialInsets {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub l: Option<Pt>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub t: Option<Pt>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub r: Option<Pt>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub b: Option<Pt>,
}

impl Validate for PartialInsets {
    fn validate(&self, v: &mut Validator) {
        v.field("l", &self.l);
        v.field("t", &self.t);
        v.field("r", &self.r);
        v.field("b", &self.b);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Anchor {
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Wrap {
    Square,
    None,
}

/// Where text taller than a middle- or bottom-anchored box goes: `spread` keeps it on its anchor, past the box's
/// top too (PowerPoint, Google Slides); `below` starts it at the top inset and runs it below the box (Keynote).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Overflow {
    Spread,
    Below,
}

/// Complete text-box properties. `maxWidth` is the grow limit of a box that resizes its width.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BodyProps<A> {
    pub insets: Insets,
    pub anchor: Anchor,
    pub wrap: Wrap,
    pub autofit: A,
    pub overflow: Overflow,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub max_width: Option<Size>,
}

pub type ResolvedBodyProps = BodyProps<ResolvedAutofit>;

impl<A: Validate> Validate for BodyProps<A> {
    fn validate(&self, v: &mut Validator) {
        v.field("insets", &self.insets);
        v.field("autofit", &self.autofit);
        v.field("maxWidth", &self.max_width);
    }
}

/// Body properties as declared: any subset.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PartialBodyProps {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub insets: Option<PartialInsets>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub anchor: Option<Anchor>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub wrap: Option<Wrap>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub autofit: Option<Autofit>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub overflow: Option<Overflow>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub max_width: Option<Size>,
}

impl PartialBodyProps {
    /// The keys a body lacks to be complete (`maxWidth` excluded), an inset side as `insets.<side>`.
    pub fn missing_keys(&self) -> Vec<String> {
        let mut out: Vec<String> = [
            ("insets", self.insets.is_some()),
            ("anchor", self.anchor.is_some()),
            ("wrap", self.wrap.is_some()),
            ("autofit", self.autofit.is_some()),
            ("overflow", self.overflow.is_some()),
        ]
        .into_iter()
        .filter(|(_, present)| !present)
        .map(|(key, _)| key.to_owned())
        .collect();
        if let Some(insets) = &self.insets {
            for (side, present) in [
                ("l", insets.l.is_some()),
                ("t", insets.t.is_some()),
                ("r", insets.r.is_some()),
                ("b", insets.b.is_some()),
            ] {
                if !present {
                    out.push(format!("insets.{side}"));
                }
            }
        }
        out
    }
}

impl Validate for PartialBodyProps {
    fn validate(&self, v: &mut Validator) {
        v.field("insets", &self.insets);
        v.field("autofit", &self.autofit);
        v.field("maxWidth", &self.max_width);
    }
}

/// The half turn lower-geometry adds to a text body: 180 when the frame flips vertically and the text does not flip
/// with it (DrawingML keeps such text readable, so it turns rather than mirrors), else 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextRotation {
    #[default]
    Zero,
    Half,
}

impl TextRotation {
    pub const fn degrees(self) -> u32 {
        match self {
            TextRotation::Zero => 0,
            TextRotation::Half => 180,
        }
    }
}

impl Serialize for TextRotation {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(self.degrees())
    }
}

impl<'de> Deserialize<'de> for TextRotation {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let degrees = crate::units::deserialize_number(deserializer)?;
        if degrees == 0.0 {
            Ok(TextRotation::Zero)
        } else if degrees == 180.0 {
            Ok(TextRotation::Half)
        } else {
            Err(serde::de::Error::custom(format_args!(
                "Invalid input: expected 0 or 180, not {}",
                slidock_canon::to_js_string(degrees)
            )))
        }
    }
}

impl fmt::Display for TextRotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.degrees())
    }
}

/// A text body as declared. `rotation` is written by lower-geometry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TextBody {
    pub body: PartialBodyProps,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub list_style: Option<LevelStyles>,
    pub paragraphs: Vec<SourceParagraph>,
    /// The text is mirrored with its frame, drawn by the frame's flips as the rest of the element is (Keynote). Unset,
    /// the text stays readable in a flipped frame (DrawingML): unmirrored, turned 180 under a vertical flip. Set by
    /// the frontend.
    #[serde(default, skip_serializing_if = "crate::elements::is_false")]
    pub flips_with_frame: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub rotation: Option<TextRotation>,
}

impl Validate for TextBody {
    fn validate(&self, v: &mut Validator) {
        v.field("body", &self.body);
        v.field("listStyle", &self.list_style);
        v.field("paragraphs", &self.paragraphs);
    }
}

/// Resolved text. `listStyle` stays as provenance; every value a backend prints is on the paragraphs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ResolvedTextBody {
    pub body: ResolvedBodyProps,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub list_style: Option<LevelStyles>,
    pub paragraphs: Vec<ResolvedParagraph>,
    /// As [`TextBody::flips_with_frame`].
    #[serde(default, skip_serializing_if = "crate::elements::is_false")]
    pub flips_with_frame: bool,
    pub rotation: TextRotation,
}

impl Validate for ResolvedTextBody {
    fn validate(&self, v: &mut Validator) {
        v.field("body", &self.body);
        v.field("listStyle", &self.list_style);
        v.field("paragraphs", &self.paragraphs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn black() -> Value {
        json!({ "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 })
    }

    fn tx1() -> Value {
        json!({ "kind": "theme", "slot": "tx1", "mods": [] })
    }

    fn run_props() -> Value {
        json!({
            "font": { "kind": "family", "family": "Liberation Sans" }, "size": 18, "weight": 400, "italic": false,
            "underline": "none", "strike": "none", "color": black(), "highlight": null, "baseline": 0,
            "caps": "none", "letterSpacing": 0, "link": null, "synthesizeBold": true
        })
    }

    fn para_props() -> Value {
        json!({
            "align": "left", "marginLeft": 0, "marginRight": 0, "indent": 0,
            "spaceBefore": { "kind": "pt", "value": 0 }, "spaceAfter": { "kind": "pct", "value": 0.2 },
            "lineSpacing": { "kind": "multiple", "value": 1, "basis": "size-1.2" }, "tabSize": 72,
            "bullet": { "kind": "none" }, "rtl": false
        })
    }

    fn with_metrics(mut props: Value) -> Value {
        props["lineHeight"] = json!({ "kind": "ratio", "value": 1.2 });
        props["spaceBeforePt"] = json!(0);
        props["spaceAfterPt"] = json!(4.32);
        props
    }

    fn ok<T: DeserializeOwned>(value: Value) -> bool {
        serde_json::from_value::<T>(value).is_ok()
    }

    fn canon<T: Serialize>(value: &T) -> String {
        crate::canonical::to_json(value).unwrap()
    }

    #[test]
    fn lets_a_source_state_any_sides_of_its_insets() {
        let body = json!({ "anchor": "top", "wrap": "square", "autofit": { "kind": "none" }, "insets": { "l": 0 } });
        let text: TextBody =
            serde_json::from_value(json!({ "body": body, "paragraphs": [] })).unwrap();
        assert!(!ok::<ResolvedTextBody>(
            json!({ "body": body, "paragraphs": [], "rotation": 0 })
        ));
        let missing = text.body.missing_keys();
        assert_eq!(missing, ["overflow", "insets.t", "insets.r", "insets.b"]);
    }

    #[test]
    fn keeps_declared_properties_sparse_in_the_source_phase() {
        let paragraph = json!({
            "level": 1,
            "props": { "align": "center", "bullet": { "kind": "auto", "scheme": "arabicPeriod", "startAt": 1 } },
            "inlines": [
                { "kind": "run", "text": "Hello ", "props": { "font": { "kind": "theme", "slot": "minor" }, "color": tx1() } },
                { "kind": "field", "field": "slideNumber", "cached": "3", "props": {} },
                { "kind": "break" },
                { "kind": "tab" }
            ],
            "endProps": { "size": 12 }
        });
        let parsed: SourceParagraph = serde_json::from_value(paragraph.clone()).unwrap();
        assert_eq!(canon(&parsed), canon(&paragraph));
        assert!(crate::validate::validate(&parsed).is_empty());
        let mut deep = paragraph;
        deep["level"] = json!(9);
        let parsed: SourceParagraph = serde_json::from_value(deep).unwrap();
        assert_eq!(crate::validate::validate(&parsed).len(), 1);
    }

    #[test]
    fn requires_complete_family_bound_theme_resolved_properties_when_resolved() {
        let paragraph = json!({ "level": 0, "props": with_metrics(para_props()), "inlines": [{ "kind": "run", "text": "Hi", "props": run_props() }] });
        let parsed: ResolvedParagraph = serde_json::from_value(paragraph.clone()).unwrap();
        assert_eq!(canon(&parsed), canon(&paragraph));

        let mut no_line_height = paragraph.clone();
        no_line_height["props"]
            .as_object_mut()
            .unwrap()
            .remove("lineHeight");
        assert!(!ok::<ResolvedParagraph>(no_line_height));
        for (key, value) in [
            ("font", json!({ "kind": "theme", "slot": "minor" })),
            ("color", tx1()),
        ] {
            let mut bad = paragraph.clone();
            bad["inlines"][0]["props"][key] = value;
            assert!(!ok::<ResolvedParagraph>(bad), "{key}");
        }
        let mut undecided = paragraph.clone();
        undecided["inlines"][0]["props"]
            .as_object_mut()
            .unwrap()
            .remove("synthesizeBold");
        assert!(!ok::<ResolvedParagraph>(undecided));
        let mut no_highlight = paragraph;
        no_highlight["inlines"][0]["props"]
            .as_object_mut()
            .unwrap()
            .remove("highlight");
        assert!(!ok::<ResolvedParagraph>(no_highlight));
    }

    #[test]
    fn labels_bullets_only_in_the_resolved_phase() {
        let label = json!({ "kind": "label", "text": "1.", "font": { "kind": "family", "family": "Liberation Sans" },
            "size": 18, "color": black(), "auto": { "scheme": "arabicPeriod", "startAt": 1 } });
        let mut props = with_metrics(para_props());
        props["bullet"] = label;
        let paragraph = json!({ "level": 0, "props": props, "inlines": [] });
        assert!(ok::<ResolvedParagraph>(paragraph.clone()));
        let mut char = paragraph;
        char["props"]["bullet"] = json!({ "kind": "char", "char": "•" });
        assert!(!ok::<ResolvedParagraph>(char.clone()));
        assert!(ok::<SourceParagraph>(char));
    }

    #[test]
    fn declares_the_line_spacing_basis_on_every_multiple() {
        let spacing = |line_spacing: Value| {
            ok::<SourceParagraph>(
                json!({ "level": 0, "props": { "lineSpacing": line_spacing }, "inlines": [] }),
            )
        };
        assert!(spacing(
            json!({ "kind": "multiple", "value": 1.15, "basis": "hhea" })
        ));
        assert!(!spacing(json!({ "kind": "multiple", "value": 1.15 })));
        assert!(spacing(json!({ "kind": "between", "min": 20, "max": 30 })));
    }

    #[test]
    fn completes_autofit_in_the_resolved_phase() {
        assert!(ok::<Autofit>(json!({ "kind": "shrink" })));
        assert!(ok::<Autofit>(json!({ "kind": "shrink", "fontScale": 0.9 })));
        assert!(!ok::<ResolvedAutofit>(
            json!({ "kind": "shrink", "fontScale": 0.9 })
        ));
        assert!(ok::<ResolvedAutofit>(
            json!({ "kind": "shrink", "fontScale": 1, "lineSpacingReduction": 0, "fitted": false })
        ));
        let zero: Autofit =
            serde_json::from_value(json!({ "kind": "shrink", "fontScale": 0 })).unwrap();
        assert_eq!(crate::validate::validate(&zero).len(), 1);
    }

    #[test]
    fn a_source_shrink_names_the_steps_its_app_fits_by_and_a_resolved_one_none() {
        let office: Autofit =
            serde_json::from_value(json!({ "kind": "shrink", "steps": "office" })).unwrap();
        assert_eq!(
            office,
            Autofit::Shrink {
                font_scale: None,
                line_spacing_reduction: None,
                fitted: None,
                steps: Some(ShrinkSteps::Office),
            }
        );
        assert_eq!(
            serde_json::to_value(&office).unwrap(),
            json!({ "kind": "shrink", "steps": "office" })
        );
        assert!(ok::<Autofit>(
            json!({ "kind": "shrink", "steps": "percent" })
        ));
        assert!(!ok::<Autofit>(
            json!({ "kind": "shrink", "steps": "points" })
        ));
        assert!(!ok::<ResolvedAutofit>(
            json!({ "kind": "shrink", "fontScale": 1, "lineSpacingReduction": 0, "fitted": true, "steps": "office" })
        ));
    }

    #[test]
    fn keys_level_styles_0_to_8_and_requires_all_nine_in_the_defaults() {
        assert!(ok::<LevelStyles>(
            json!({ "0": { "run": { "size": 44 } }, "8": {} })
        ));
        assert!(!ok::<LevelStyles>(json!({ "9": {} })));
        let mut run = run_props();
        run["scriptScale"] = json!(0.58);
        let level = json!({ "para": para_props(), "run": run });
        let all: serde_json::Map<String, Value> = LEVEL_KEYS
            .iter()
            .map(|k| ((*k).to_owned(), level.clone()))
            .collect();
        assert!(ok::<DefaultTextStyles>(Value::Object(all.clone())));
        let mut unscaled = all.clone();
        unscaled.insert(
            "0".into(),
            json!({ "para": para_props(), "run": run_props() }),
        );
        assert!(!ok::<DefaultTextStyles>(Value::Object(unscaled)));
        assert!(!ok::<ResolvedParagraph>(
            json!({ "level": 0, "props": with_metrics(para_props()), "inlines": [{ "kind": "run", "text": "x", "props": run }] })
        ));
        let mut eight = all;
        eight.remove("8");
        assert!(!ok::<DefaultTextStyles>(Value::Object(eight)));
    }

    #[test]
    fn requires_a_complete_body_and_the_rotation_when_resolved() {
        let source = json!({ "body": { "anchor": "middle" }, "paragraphs": [] });
        assert!(ok::<TextBody>(source.clone()));
        assert!(!ok::<ResolvedTextBody>(source));
        let body = json!({ "insets": { "l": 7.2, "t": 3.6, "r": 7.2, "b": 3.6 }, "anchor": "top", "wrap": "square",
            "autofit": { "kind": "resize", "width": false, "height": true, "anchorX": "left" }, "overflow": "spread" });
        assert!(ok::<ResolvedTextBody>(
            json!({ "body": body, "paragraphs": [], "rotation": 180 })
        ));
        let mut no_overflow = body.clone();
        no_overflow.as_object_mut().unwrap().remove("overflow");
        assert!(!ok::<ResolvedTextBody>(
            json!({ "body": no_overflow, "paragraphs": [], "rotation": 0 })
        ));
        assert!(!ok::<ResolvedTextBody>(
            json!({ "body": body, "paragraphs": [], "rotation": 90 })
        ));
    }

    #[test]
    fn writes_flips_with_frame_only_when_the_text_flips_with_its_frame() {
        let plain: TextBody =
            serde_json::from_value(json!({ "body": {}, "paragraphs": [] })).unwrap();
        assert!(!plain.flips_with_frame);
        assert_eq!(
            serde_json::to_value(&plain).unwrap(),
            json!({ "body": {}, "paragraphs": [] })
        );
        let mirrored: TextBody =
            serde_json::from_value(json!({ "body": {}, "paragraphs": [], "flipsWithFrame": true }))
                .unwrap();
        assert!(mirrored.flips_with_frame);
        assert_eq!(
            serde_json::to_value(&mirrored).unwrap()["flipsWithFrame"],
            json!(true)
        );
        assert!(!ok::<TextBody>(
            json!({ "body": {}, "paragraphs": [], "flipsWithFrame": "yes" })
        ));
    }

    fn inline(value: Value) -> SourceInline {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn picks_the_largest_run_or_field_as_the_strut() {
        let run = |size: Option<u32>| match size {
            Some(size) => inline(json!({ "kind": "run", "text": "x", "props": { "size": size } })),
            None => inline(json!({ "kind": "run", "text": "x", "props": {} })),
        };
        let field = |size: u32| {
            inline(
                json!({ "kind": "field", "field": "slideNumber", "cached": "1", "props": { "size": size } }),
            )
        };
        assert_eq!(
            strut_index(&[run(Some(12)), run(Some(18)), field(18)]),
            Some(1)
        );
        assert_eq!(
            strut_index(&[inline(json!({ "kind": "tab" })), field(9)]),
            Some(1)
        );
        assert_eq!(strut_index::<PartialRunProps>(&[]), None);
        assert_eq!(
            strut_index(&[
                inline(json!({ "kind": "break" })),
                inline(json!({ "kind": "tab" }))
            ]),
            None
        );
        let empty = inline(json!({ "kind": "run", "text": "", "props": { "size": 40 } }));
        assert_eq!(strut_index(std::slice::from_ref(&empty)), None);
        assert_eq!(strut_index(&[run(Some(12)), empty]), Some(0));
        assert_eq!(strut_index(&[run(None), run(Some(1))]), Some(1));
        assert_eq!(strut_index(&[run(None), run(None)]), Some(0));
    }

    #[test]
    fn tri_state_run_fields_round_trip() {
        for props in [
            json!({}),
            json!({ "highlight": null }),
            json!({ "link": { "kind": "action", "action": "next" } }),
        ] {
            let parsed: PartialRunProps = serde_json::from_value(props.clone()).unwrap();
            assert_eq!(canon(&parsed), canon(&props));
        }
        let null: PartialRunProps = serde_json::from_value(json!({ "highlight": null })).unwrap();
        assert_eq!(null.highlight, Some(None));
        assert!(!null.missing_keys().contains(&"highlight"));
        assert!(
            PartialRunProps::default()
                .missing_keys()
                .contains(&"highlight")
        );
    }
}
