//! The source phase: a presentation as a frontend writes it and as every pass hands it on, with inheritance open, theme
//! references unresolved and props partial. A superset of the resolved phase, so every intermediate presentation, and
//! every resolved one, reads as a source presentation.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::color::{Color, ColorMap};
use crate::de::Nullable;
use crate::elements::{
    Crop, ForeignFallback, ForeignType, MediaKind, Trim, element_enum, element_struct,
};
use crate::fonts::FontEntry;
use crate::formats::FileFormat;
use crate::frame::{ChildSpace, Frame};
use crate::geometry::{Geometry, Path};
use crate::ids::{ElementId, LayoutId, MasterId, NodeId, Sha256, SlideId, ThemeId};
use crate::json_value::{Ext, JsonValue};
use crate::media::MediaEntry;
use crate::origin::Origin;
use crate::paint::{CellFill, Fill, SourceStroke};
use crate::presentation::{Defaults, Failed, Phase, SchemaUrl, SlideSize, TextStyles, Theme};
use crate::shadow::SourceShadow;
use crate::table::{TableCell, TableColumn, TableRow, validate_tracks};
use crate::text::TextBody;
use crate::timeline::{TimelineStep, Transition};
use crate::validate::{Validate, Validator};

pub type Cell = TableCell<CellFill, Color, TextBody>;

element_struct! {
    /// A shape: geometry, fill, stroke (`null` for none) and text.
    pub struct Shape {
        frame: [#[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]] Option<Frame>;
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub geometry: Option<Geometry>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub fill: Option<Fill>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::double_option")]
        pub stroke: Nullable<SourceStroke>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub text: Option<TextBody>,
        /// An outer shadow: absent inherits, `null` is none.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::double_option")]
        pub shadow: Nullable<SourceShadow>,
    }
}

element_struct! {
    /// A picture. `clip` is a non-rectangular picture shape; `mask` frame-local mask paths (Keynote). `fallback` is a
    /// raster the source keeps beside an original only a host may decode (Keynote's JPEG preview of a HEIC photo):
    /// plan-media draws it in place of `media` when no host decoded that, and leaves no picture with one.
    pub struct Picture {
        frame: [#[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]] Option<Frame>;
        pub media: Sha256,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub fallback: Option<Sha256>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub crop: Option<Crop>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub clip: Option<Geometry>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub mask: Option<Vec<Path>>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::double_option")]
        pub stroke: Nullable<SourceStroke>,
        /// An outer shadow: absent inherits, `null` is none.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::double_option")]
        pub shadow: Nullable<SourceShadow>,
    }
}

element_struct! {
    /// A video or an audio clip.
    pub struct MediaElement {
        frame: [#[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]] Option<Frame>;
        pub media_kind: MediaKind,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub media: Option<Sha256>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub poster: Option<Sha256>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub trim: Option<Trim>,
        #[serde(rename = "loop")]
        pub looped: bool,
        pub muted: bool,
        pub autoplay: bool,
        /// Plays on across a move to the next slide when that slide shows the same clip, also marked: the clip goes on
        /// from where it is instead of starting again (Keynote's "play across slides").
        #[serde(default, skip_serializing_if = "crate::elements::is_false")]
        pub across_slides: bool,
        /// An outer shadow: absent inherits, `null` is none.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::double_option")]
        pub shadow: Nullable<SourceShadow>,
    }
}

element_struct! {
    /// A table: a grid of `columns` (widths) by `rows` (least heights), in points between grid lines. A placeholder
    /// table without a frame takes its position from the frame it inherits and its size from its grid. Cells have
    /// no ids: the table is the node diagnostics, links and builds name.
    pub struct Table {
        frame: [#[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]] Option<Frame>;
        pub columns: Vec<TableColumn>,
        pub rows: Vec<TableRow<Cell>>,
    }
}

element_struct! {
    /// A group. Children are bottom-to-top z-order, relative to the group's box.
    pub struct Group {
        frame: [#[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]] Option<Frame>;
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub child_space: Option<ChildSpace>,
        pub children: Vec<Element>,
        /// An outer shadow of the group's composite: absent inherits, `null` is none.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::double_option")]
        pub shadow: Nullable<SourceShadow>,
    }
}

element_struct! {
    /// An object outside the IR's model, kept with its frame and, when the source has one, a fallback to print.
    pub struct Foreign {
        frame: [#[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]] Option<Frame>;
        pub dialect: FileFormat,
        #[serde(rename = "type")]
        pub foreign_type: ForeignType,
        /// The source's own data for a later backend; bounded by the verifier.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub payload: Option<JsonValue>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub fallback: Option<ForeignFallback<Element>>,
    }
}

element_enum! {
    /// A node on a slide, layout or master. Connectors are shapes with a connector preset or path and line ends.
    pub enum Element, frame: Option<Frame> {
        Shape(Shape) = "shape",
        Picture(Picture) = "picture",
        Media(MediaElement) = "media",
        Table(Table) = "table",
        Group(Group) = "group",
        Foreign(Foreign) = "foreign",
    }
}

impl Element {
    /// The element's shadow (absent, `null` or a value), for the kinds that carry one.
    pub fn shadow(&self) -> Nullable<&SourceShadow> {
        self.shadow_field()
            .and_then(|shadow| shadow.as_ref().map(Option::as_ref))
    }

    fn shadow_field(&self) -> Option<&Nullable<SourceShadow>> {
        match self {
            Element::Shape(e) => Some(&e.shadow),
            Element::Picture(e) => Some(&e.shadow),
            Element::Media(e) => Some(&e.shadow),
            Element::Group(e) => Some(&e.shadow),
            Element::Table(_) | Element::Foreign(_) => None,
        }
    }

    /// The element's shadow field, for the kinds that carry one.
    pub fn shadow_mut(&mut self) -> Option<&mut Nullable<SourceShadow>> {
        match self {
            Element::Shape(e) => Some(&mut e.shadow),
            Element::Picture(e) => Some(&mut e.shadow),
            Element::Media(e) => Some(&mut e.shadow),
            Element::Group(e) => Some(&mut e.shadow),
            Element::Table(_) | Element::Foreign(_) => None,
        }
    }
}

impl Validate for Shape {
    fn validate(&self, v: &mut Validator) {
        self.validate_common(v);
        v.field("geometry", &self.geometry);
        v.field("fill", &self.fill);
        v.field("stroke", &self.stroke.as_ref().and_then(Option::as_ref));
        v.field("text", &self.text);
        v.field("shadow", &self.shadow.as_ref().and_then(Option::as_ref));
    }
}

impl Validate for Picture {
    fn validate(&self, v: &mut Validator) {
        self.validate_common(v);
        v.field("crop", &self.crop);
        v.field("clip", &self.clip);
        if let Some(mask) = &self.mask {
            v.at("mask", |v| {
                v.array(mask.len(), 0, 256);
                mask.validate(v);
            });
        }
        v.field("stroke", &self.stroke.as_ref().and_then(Option::as_ref));
        v.field("shadow", &self.shadow.as_ref().and_then(Option::as_ref));
    }
}

impl Validate for MediaElement {
    fn validate(&self, v: &mut Validator) {
        self.validate_common(v);
        v.field("trim", &self.trim);
        v.field("shadow", &self.shadow.as_ref().and_then(Option::as_ref));
    }
}

impl Validate for Table {
    fn validate(&self, v: &mut Validator) {
        self.validate_common(v);
        validate_tracks(&self.columns, &self.rows, v);
    }
}

impl Validate for Group {
    fn validate(&self, v: &mut Validator) {
        self.validate_common(v);
        v.field("childSpace", &self.child_space);
        v.field("children", &self.children);
        v.field("shadow", &self.shadow.as_ref().and_then(Option::as_ref));
    }
}

impl Validate for Foreign {
    fn validate(&self, v: &mut Validator) {
        self.validate_common(v);
        v.field("payload", &self.payload);
        v.field("fallback", &self.fallback);
    }
}

fn validate_name(name: &Option<String>, v: &mut Validator) {
    if let Some(name) = name {
        v.at("name", |v| v.string(name, 0, 256));
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Master {
    pub id: MasterId,
    pub origin: Origin,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub theme: Option<ThemeId>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub color_map: Option<ColorMap>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub text_styles: Option<TextStyles>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub ext: Option<Ext>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub background: Option<Fill>,
    pub elements: Vec<Element>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub failed: Option<Failed>,
}

impl Validate for Master {
    fn validate(&self, v: &mut Validator) {
        v.field("origin", &self.origin);
        validate_name(&self.name, v);
        v.field("textStyles", &self.text_styles);
        v.field("ext", &self.ext);
        v.field("background", &self.background);
        v.field("elements", &self.elements);
        v.field("failed", &self.failed);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Layout {
    pub id: LayoutId,
    pub origin: Origin,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub name: Option<String>,
    /// The source's layout type (`title`, `obj`, `TITLE_AND_BODY`), informational.
    #[serde(
        rename = "type",
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub layout_type: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub master: Option<MasterId>,
    /// Whether the master's non-placeholder elements show behind this layout.
    pub show_master: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub color_map_override: Option<ColorMap>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub ext: Option<Ext>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub background: Option<Fill>,
    pub elements: Vec<Element>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub failed: Option<Failed>,
}

impl Validate for Layout {
    fn validate(&self, v: &mut Validator) {
        v.field("origin", &self.origin);
        validate_name(&self.name, v);
        if let Some(layout_type) = &self.layout_type {
            v.at("type", |v| v.string(layout_type, 0, 64));
        }
        v.field("ext", &self.ext);
        v.field("background", &self.background);
        v.field("elements", &self.elements);
        v.field("failed", &self.failed);
    }
}

/// The longest slide title (a11y writes at most 200 characters; agents may write up to this).
pub const MAX_TITLE: usize = 256;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Slide {
    pub id: SlideId,
    pub origin: Origin,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub layout: Option<LayoutId>,
    pub hidden: bool,
    /// Whether the layout's (and through it the master's) elements show behind this slide.
    pub show_layout: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub color_map_override: Option<ColorMap>,
    /// Speaker notes: text content only, left uncascaded.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub notes: Option<TextBody>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub transition: Option<Transition>,
    pub timeline: Vec<TimelineStep>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub ext: Option<Ext>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub background: Option<Fill>,
    pub elements: Vec<Element>,
    /// The slide's title for navigation; `""` when it has none. Written by a11y.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub title: Option<String>,
    /// Element ids in reading order. Written by a11y.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub reading_order: Option<Vec<ElementId>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub failed: Option<Failed>,
}

impl Validate for Slide {
    fn validate(&self, v: &mut Validator) {
        v.field("origin", &self.origin);
        v.field("notes", &self.notes);
        v.field("transition", &self.transition);
        v.field("timeline", &self.timeline);
        v.field("ext", &self.ext);
        v.field("background", &self.background);
        v.field("elements", &self.elements);
        if let Some(title) = &self.title {
            v.at("title", |v| v.string(title, 0, MAX_TITLE));
        }
        v.field("failed", &self.failed);
    }
}

/// A container: a slide, a layout or a master.
#[derive(Clone, Copy, Debug)]
pub enum Container<'a> {
    Master(&'a Master),
    Layout(&'a Layout),
    Slide(&'a Slide),
}

impl<'a> Container<'a> {
    pub fn id(&self) -> &'a NodeId {
        match self {
            Container::Master(c) => &c.id,
            Container::Layout(c) => &c.id,
            Container::Slide(c) => &c.id,
        }
    }

    pub fn elements(&self) -> &'a [Element] {
        match self {
            Container::Master(c) => &c.elements,
            Container::Layout(c) => &c.elements,
            Container::Slide(c) => &c.elements,
        }
    }

    pub fn background(&self) -> Option<&'a Fill> {
        match self {
            Container::Master(c) => c.background.as_ref(),
            Container::Layout(c) => c.background.as_ref(),
            Container::Slide(c) => c.background.as_ref(),
        }
    }

    pub fn failed(&self) -> Option<&'a Failed> {
        match self {
            Container::Master(c) => c.failed.as_ref(),
            Container::Layout(c) => c.failed.as_ref(),
            Container::Slide(c) => c.failed.as_ref(),
        }
    }

    pub fn ext(&self) -> Option<&'a Ext> {
        match self {
            Container::Master(c) => c.ext.as_ref(),
            Container::Layout(c) => c.ext.as_ref(),
            Container::Slide(c) => c.ext.as_ref(),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Container::Master(_) => "master",
            Container::Layout(_) => "layout",
            Container::Slide(_) => "slide",
        }
    }
}

/// A presentation as a frontend writes it and as every pass hands it on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presentation {
    #[serde(
        rename = "$schema",
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub schema: Option<SchemaUrl>,
    pub size: SlideSize,
    pub defaults: Defaults,
    pub themes: IndexMap<ThemeId, Theme>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub ext: Option<Ext>,
    pub phase: Phase,
    pub masters: IndexMap<MasterId, Master>,
    pub layouts: IndexMap<LayoutId, Layout>,
    /// Show order, hidden slides included.
    pub slides: Vec<Slide>,
    /// Keyed by `font_key(family)`.
    #[serde(deserialize_with = "crate::de::js_record")]
    pub fonts: IndexMap<String, FontEntry>,
    pub media: IndexMap<Sha256, MediaEntry>,
}

impl Presentation {
    /// Masters, then layouts, then slides, each in presentation order: the order diagnostics follow.
    pub fn containers(&self) -> impl Iterator<Item = Container<'_>> {
        self.masters
            .values()
            .map(Container::Master)
            .chain(self.layouts.values().map(Container::Layout))
            .chain(self.slides.iter().map(Container::Slide))
    }
}

impl Validate for Presentation {
    fn validate(&self, v: &mut Validator) {
        v.field("size", &self.size);
        v.field("defaults", &self.defaults);
        v.field("themes", &self.themes);
        v.field("ext", &self.ext);
        v.field("masters", &self.masters);
        v.field("layouts", &self.layouts);
        v.field("slides", &self.slides);
        v.at("fonts", |v| {
            for (key, entry) in &self.fonts {
                v.at(key.clone(), |v| {
                    v.string(key, 1, 256);
                    entry.validate(v);
                });
            }
        });
        v.at("media", |v| {
            for (key, entry) in &self.media {
                v.at(key.to_hex(), |v| entry.validate(v));
            }
        });
    }
}
