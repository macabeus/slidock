//! The resolved phase: what backends accept, every value computed, references kept as provenance. Frames everywhere,
//! lowered geometry and paint, cascaded text. A container is built (resolved form) or failed (source form, no
//! elements): [`Body`].

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::color::{ColorMap, ResolvedColor};
use crate::de::{Nullable, from_value};
use crate::elements::{
    Crop, ForeignFallback, ForeignType, MediaKind, Trim, element_enum, element_struct,
};
use crate::fonts::ResolvedFontEntry;
use crate::formats::FileFormat;
use crate::frame::Frame;
use crate::geometry::{Path, PathsGeometry};
use crate::ids::{ElementId, LayoutId, MasterId, NodeId, Sha256, SlideId, ThemeId};
use crate::json_value::{Ext, JsonValue};
use crate::media::ResolvedMediaEntry;
use crate::origin::Origin;
use crate::paint::{Fill, ResolvedCellFill, ResolvedFill, ResolvedStroke};
use crate::presentation::{
    Defaults, Failed, ResolvedPhase, SchemaUrl, SlideSize, TextStyles, Theme,
};
use crate::shadow::ResolvedShadow;
use crate::table::{TableCell, TableColumn, TableRow};
use crate::text::{ResolvedTextBody, TextBody};
use crate::timeline::{TimelineStep, Transition};

pub type Cell = TableCell<ResolvedCellFill, ResolvedColor, ResolvedTextBody>;

element_struct! {
    pub struct Shape {
        frame: [] Frame;
        pub geometry: PathsGeometry,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub fill: Option<ResolvedFill>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::double_option")]
        pub stroke: Nullable<ResolvedStroke>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub text: Option<ResolvedTextBody>,
        /// The element's shadow; a source `null` (none) reads as absent.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::nullish")]
        pub shadow: Option<ResolvedShadow>,
    }
}

element_struct! {
    pub struct Picture {
        frame: [] Frame;
        pub media: Sha256,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub crop: Option<Crop>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub clip: Option<PathsGeometry>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub mask: Option<Vec<Path>>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::double_option")]
        pub stroke: Nullable<ResolvedStroke>,
        /// The element's shadow; a source `null` (none) reads as absent.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::nullish")]
        pub shadow: Option<ResolvedShadow>,
    }
}

element_struct! {
    pub struct MediaElement {
        frame: [] Frame;
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
        /// The element's shadow; a source `null` (none) reads as absent.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::nullish")]
        pub shadow: Option<ResolvedShadow>,
    }
}

element_struct! {
    pub struct Table {
        frame: [] Frame;
        pub columns: Vec<TableColumn>,
        pub rows: Vec<TableRow<Cell>>,
    }
}

element_struct! {
    /// A group; its child space was folded into its children's frames by normalize-frames.
    pub struct Group {
        frame: [] Frame;
        pub children: Vec<Element>,
        /// The element's shadow; a source `null` (none) reads as absent.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::nullish")]
        pub shadow: Option<ResolvedShadow>,
    }
}

element_struct! {
    pub struct Foreign {
        frame: [] Frame;
        pub dialect: FileFormat,
        #[serde(rename = "type")]
        pub foreign_type: ForeignType,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub payload: Option<JsonValue>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
        pub fallback: Option<ForeignFallback<Element>>,
    }
}

element_enum! {
    /// A resolved element.
    pub enum Element, frame: Frame {
        Shape(Shape) = "shape",
        Picture(Picture) = "picture",
        Media(MediaElement) = "media",
        Table(Table) = "table",
        Group(Group) = "group",
        Foreign(Foreign) = "foreign",
    }
}

use crate::validate::{Validate, Validator};

impl Element {
    /// The element's shadow, for the kinds that carry one.
    pub fn shadow(&self) -> Option<&ResolvedShadow> {
        match self {
            Element::Shape(e) => e.shadow.as_ref(),
            Element::Picture(e) => e.shadow.as_ref(),
            Element::Media(e) => e.shadow.as_ref(),
            Element::Group(e) => e.shadow.as_ref(),
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
        v.field("shadow", &self.shadow);
    }
}

impl Validate for MediaElement {
    fn validate(&self, v: &mut Validator) {
        self.validate_common(v);
        v.field("trim", &self.trim);
        v.field("shadow", &self.shadow);
    }
}

impl Validate for Group {
    fn validate(&self, v: &mut Validator) {
        self.validate_common(v);
        v.field("children", &self.children);
        v.field("shadow", &self.shadow);
    }
}

impl Validate for Foreign {
    fn validate(&self, v: &mut Validator) {
        self.validate_common(v);
        v.field("payload", &self.payload);
        v.field("fallback", &self.fallback);
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
        v.field("shadow", &self.shadow);
    }
}

impl Validate for Table {
    fn validate(&self, v: &mut Validator) {
        self.validate_common(v);
        crate::table::validate_tracks(&self.columns, &self.rows, v);
    }
}

/// A container's content: built in the resolved form, or failed, keeping its source-form background and no
/// elements.
#[derive(Clone, Debug, PartialEq)]
pub enum Body<B> {
    Built(B),
    Failed {
        failed: Failed,
        background: Option<Fill>,
    },
}

/// A built master's or layout's content.
#[derive(Clone, Debug, PartialEq)]
pub struct Built {
    pub background: Option<ResolvedFill>,
    pub elements: Vec<Element>,
}

/// A built slide's content, with what a11y wrote.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltSlide {
    pub background: Option<ResolvedFill>,
    pub elements: Vec<Element>,
    /// The slide's title for navigation; `""` when it has none.
    pub title: String,
    /// Element ids in reading order.
    pub reading_order: Vec<ElementId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Master {
    pub id: MasterId,
    pub origin: Origin,
    pub name: Option<String>,
    pub theme: Option<ThemeId>,
    pub color_map: Option<ColorMap>,
    pub text_styles: Option<TextStyles>,
    pub ext: Option<Ext>,
    pub body: Body<Built>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub id: LayoutId,
    pub origin: Origin,
    pub name: Option<String>,
    pub layout_type: Option<String>,
    pub master: Option<MasterId>,
    pub show_master: bool,
    pub color_map_override: Option<ColorMap>,
    pub ext: Option<Ext>,
    pub body: Body<Built>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Slide {
    pub id: SlideId,
    pub origin: Origin,
    pub layout: Option<LayoutId>,
    pub hidden: bool,
    pub show_layout: bool,
    pub color_map_override: Option<ColorMap>,
    pub notes: Option<TextBody>,
    pub transition: Option<Transition>,
    pub timeline: Vec<TimelineStep>,
    pub ext: Option<Ext>,
    /// A failed slide keeps the title and reading order it had, if any.
    pub failed_a11y: Option<FailedA11y>,
    pub body: Body<BuiltSlide>,
}

/// What a failed slide kept of a11y's output.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FailedA11y {
    pub title: Option<String>,
    pub reading_order: Option<Vec<ElementId>>,
}

impl<B> Body<B> {
    pub fn built(&self) -> Option<&B> {
        match self {
            Body::Built(built) => Some(built),
            Body::Failed { .. } => None,
        }
    }

    pub fn failed(&self) -> Option<&Failed> {
        match self {
            Body::Built(_) => None,
            Body::Failed { failed, .. } => Some(failed),
        }
    }
}

impl Body<Built> {
    pub fn elements(&self) -> &[Element] {
        self.built().map_or(&[], |built| &built.elements)
    }
}

impl Body<BuiltSlide> {
    pub fn elements(&self) -> &[Element] {
        self.built().map_or(&[], |built| &built.elements)
    }
}

/// A container's background in either form, for serialisation.
#[derive(Serialize)]
#[serde(untagged)]
enum BackgroundRef<'a> {
    Resolved(&'a ResolvedFill),
    Source(&'a Fill),
}

fn background_ref<B>(
    body: &Body<B>,
    built: impl Fn(&B) -> Option<&ResolvedFill>,
) -> Option<BackgroundRef<'_>> {
    match body {
        Body::Built(b) => built(b).map(BackgroundRef::Resolved),
        Body::Failed { background, .. } => background.as_ref().map(BackgroundRef::Source),
    }
}

/// A failed container's elements: always empty.
fn no_elements<'de, D: Deserializer<'de>>(deserializer: D) -> Result<(), D::Error> {
    let elements: Vec<Value> = Vec::deserialize(deserializer)?;
    if elements.is_empty() {
        Ok(())
    } else {
        Err(serde::de::Error::custom(
            "Invalid input: a failed container holds no elements",
        ))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MasterOut<'a> {
    id: &'a MasterId,
    origin: &'a Origin,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    theme: Option<&'a ThemeId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color_map: Option<&'a ColorMap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_styles: Option<&'a TextStyles>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ext: Option<&'a Ext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    background: Option<BackgroundRef<'a>>,
    elements: &'a [Element],
    #[serde(skip_serializing_if = "Option::is_none")]
    failed: Option<&'a Failed>,
}

impl Serialize for Master {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        MasterOut {
            id: &self.id,
            origin: &self.origin,
            name: self.name.as_ref(),
            theme: self.theme.as_ref(),
            color_map: self.color_map.as_ref(),
            text_styles: self.text_styles.as_ref(),
            ext: self.ext.as_ref(),
            background: background_ref(&self.body, |b| b.background.as_ref()),
            elements: self.body.elements(),
            failed: self.body.failed(),
        }
        .serialize(serializer)
    }
}

macro_rules! container_wire {
    ($name:ident { $($(#[$m:meta])* $field:ident : $ty:ty,)* } background: $bg:ty, $($tail:tt)*) => {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, rename_all = "camelCase")]
        struct $name {
            $($(#[$m])* $field: $ty,)*
            #[serde(default, with = "crate::de::optional")]
            background: Option<$bg>,
            $($tail)*
        }
    };
}

container_wire!(MasterBuiltIn {
    id: MasterId,
    origin: Origin,
    #[serde(default, with = "crate::de::optional")] name: Option<String>,
    #[serde(default, with = "crate::de::optional")] theme: Option<ThemeId>,
    #[serde(default, with = "crate::de::optional")] color_map: Option<ColorMap>,
    #[serde(default, with = "crate::de::optional")] text_styles: Option<TextStyles>,
    #[serde(default, with = "crate::de::optional")] ext: Option<Ext>,
} background: ResolvedFill, elements: Vec<Element>,);

container_wire!(MasterFailedIn {
    id: MasterId,
    origin: Origin,
    #[serde(default, with = "crate::de::optional")] name: Option<String>,
    #[serde(default, with = "crate::de::optional")] theme: Option<ThemeId>,
    #[serde(default, with = "crate::de::optional")] color_map: Option<ColorMap>,
    #[serde(default, with = "crate::de::optional")] text_styles: Option<TextStyles>,
    #[serde(default, with = "crate::de::optional")] ext: Option<Ext>,
} background: Fill, #[serde(rename = "elements", deserialize_with = "no_elements")] _elements: (), failed: Failed,);

fn has_failed(value: &Value) -> bool {
    value.get("failed").is_some()
}

impl<'de> Deserialize<'de> for Master {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        if has_failed(&value) {
            let w: MasterFailedIn = from_value(value)?;
            Ok(Master {
                id: w.id,
                origin: w.origin,
                name: w.name,
                theme: w.theme,
                color_map: w.color_map,
                text_styles: w.text_styles,
                ext: w.ext,
                body: Body::Failed {
                    failed: w.failed,
                    background: w.background,
                },
            })
        } else {
            let w: MasterBuiltIn = from_value(value)?;
            Ok(Master {
                id: w.id,
                origin: w.origin,
                name: w.name,
                theme: w.theme,
                color_map: w.color_map,
                text_styles: w.text_styles,
                ext: w.ext,
                body: Body::Built(Built {
                    background: w.background,
                    elements: w.elements,
                }),
            })
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LayoutOut<'a> {
    id: &'a LayoutId,
    origin: &'a Origin,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    layout_type: Option<&'a String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    master: Option<&'a MasterId>,
    show_master: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    color_map_override: Option<&'a ColorMap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ext: Option<&'a Ext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    background: Option<BackgroundRef<'a>>,
    elements: &'a [Element],
    #[serde(skip_serializing_if = "Option::is_none")]
    failed: Option<&'a Failed>,
}

impl Serialize for Layout {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        LayoutOut {
            id: &self.id,
            origin: &self.origin,
            name: self.name.as_ref(),
            layout_type: self.layout_type.as_ref(),
            master: self.master.as_ref(),
            show_master: self.show_master,
            color_map_override: self.color_map_override.as_ref(),
            ext: self.ext.as_ref(),
            background: background_ref(&self.body, |b| b.background.as_ref()),
            elements: self.body.elements(),
            failed: self.body.failed(),
        }
        .serialize(serializer)
    }
}

container_wire!(LayoutBuiltIn {
    id: LayoutId,
    origin: Origin,
    #[serde(default, with = "crate::de::optional")] name: Option<String>,
    #[serde(rename = "type", default, with = "crate::de::optional")] layout_type: Option<String>,
    #[serde(default, with = "crate::de::optional")] master: Option<MasterId>,
    show_master: bool,
    #[serde(default, with = "crate::de::optional")] color_map_override: Option<ColorMap>,
    #[serde(default, with = "crate::de::optional")] ext: Option<Ext>,
} background: ResolvedFill, elements: Vec<Element>,);

container_wire!(LayoutFailedIn {
    id: LayoutId,
    origin: Origin,
    #[serde(default, with = "crate::de::optional")] name: Option<String>,
    #[serde(rename = "type", default, with = "crate::de::optional")] layout_type: Option<String>,
    #[serde(default, with = "crate::de::optional")] master: Option<MasterId>,
    show_master: bool,
    #[serde(default, with = "crate::de::optional")] color_map_override: Option<ColorMap>,
    #[serde(default, with = "crate::de::optional")] ext: Option<Ext>,
} background: Fill, #[serde(rename = "elements", deserialize_with = "no_elements")] _elements: (), failed: Failed,);

impl<'de> Deserialize<'de> for Layout {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        if has_failed(&value) {
            let w: LayoutFailedIn = from_value(value)?;
            Ok(Layout {
                id: w.id,
                origin: w.origin,
                name: w.name,
                layout_type: w.layout_type,
                master: w.master,
                show_master: w.show_master,
                color_map_override: w.color_map_override,
                ext: w.ext,
                body: Body::Failed {
                    failed: w.failed,
                    background: w.background,
                },
            })
        } else {
            let w: LayoutBuiltIn = from_value(value)?;
            Ok(Layout {
                id: w.id,
                origin: w.origin,
                name: w.name,
                layout_type: w.layout_type,
                master: w.master,
                show_master: w.show_master,
                color_map_override: w.color_map_override,
                ext: w.ext,
                body: Body::Built(Built {
                    background: w.background,
                    elements: w.elements,
                }),
            })
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SlideOut<'a> {
    id: &'a SlideId,
    origin: &'a Origin,
    #[serde(skip_serializing_if = "Option::is_none")]
    layout: Option<&'a LayoutId>,
    hidden: bool,
    show_layout: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    color_map_override: Option<&'a ColorMap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<&'a TextBody>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transition: Option<&'a Transition>,
    timeline: &'a [TimelineStep],
    #[serde(skip_serializing_if = "Option::is_none")]
    ext: Option<&'a Ext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    background: Option<BackgroundRef<'a>>,
    elements: &'a [Element],
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reading_order: Option<&'a [ElementId]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failed: Option<&'a Failed>,
}

impl Serialize for Slide {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (title, reading_order) = match (&self.body, &self.failed_a11y) {
            (Body::Built(b), _) => (Some(&b.title), Some(b.reading_order.as_slice())),
            (Body::Failed { .. }, Some(kept)) => {
                (kept.title.as_ref(), kept.reading_order.as_deref())
            }
            (Body::Failed { .. }, None) => (None, None),
        };
        SlideOut {
            id: &self.id,
            origin: &self.origin,
            layout: self.layout.as_ref(),
            hidden: self.hidden,
            show_layout: self.show_layout,
            color_map_override: self.color_map_override.as_ref(),
            notes: self.notes.as_ref(),
            transition: self.transition.as_ref(),
            timeline: &self.timeline,
            ext: self.ext.as_ref(),
            background: background_ref(&self.body, |b| b.background.as_ref()),
            elements: self.body.elements(),
            title,
            reading_order,
            failed: self.body.failed(),
        }
        .serialize(serializer)
    }
}

container_wire!(SlideBuiltIn {
    id: SlideId,
    origin: Origin,
    #[serde(default, with = "crate::de::optional")] layout: Option<LayoutId>,
    hidden: bool,
    show_layout: bool,
    #[serde(default, with = "crate::de::optional")] color_map_override: Option<ColorMap>,
    #[serde(default, with = "crate::de::optional")] notes: Option<TextBody>,
    #[serde(default, with = "crate::de::optional")] transition: Option<Transition>,
    timeline: Vec<TimelineStep>,
    #[serde(default, with = "crate::de::optional")] ext: Option<Ext>,
} background: ResolvedFill, elements: Vec<Element>, title: String, reading_order: Vec<ElementId>,);

container_wire!(SlideFailedIn {
    id: SlideId,
    origin: Origin,
    #[serde(default, with = "crate::de::optional")] layout: Option<LayoutId>,
    hidden: bool,
    show_layout: bool,
    #[serde(default, with = "crate::de::optional")] color_map_override: Option<ColorMap>,
    #[serde(default, with = "crate::de::optional")] notes: Option<TextBody>,
    #[serde(default, with = "crate::de::optional")] transition: Option<Transition>,
    timeline: Vec<TimelineStep>,
    #[serde(default, with = "crate::de::optional")] ext: Option<Ext>,
} background: Fill, #[serde(rename = "elements", deserialize_with = "no_elements")] _elements: (), #[serde(default, with = "crate::de::optional")] title: Option<String>,
  #[serde(default, with = "crate::de::optional")] reading_order: Option<Vec<ElementId>>, failed: Failed,);

impl<'de> Deserialize<'de> for Slide {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        if has_failed(&value) {
            let w: SlideFailedIn = from_value(value)?;
            let kept = (w.title.is_some() || w.reading_order.is_some()).then_some(FailedA11y {
                title: w.title,
                reading_order: w.reading_order,
            });
            Ok(Slide {
                id: w.id,
                origin: w.origin,
                layout: w.layout,
                hidden: w.hidden,
                show_layout: w.show_layout,
                color_map_override: w.color_map_override,
                notes: w.notes,
                transition: w.transition,
                timeline: w.timeline,
                ext: w.ext,
                failed_a11y: kept,
                body: Body::Failed {
                    failed: w.failed,
                    background: w.background,
                },
            })
        } else {
            let w: SlideBuiltIn = from_value(value)?;
            Ok(Slide {
                id: w.id,
                origin: w.origin,
                layout: w.layout,
                hidden: w.hidden,
                show_layout: w.show_layout,
                color_map_override: w.color_map_override,
                notes: w.notes,
                transition: w.transition,
                timeline: w.timeline,
                ext: w.ext,
                failed_a11y: None,
                body: Body::Built(BuiltSlide {
                    background: w.background,
                    elements: w.elements,
                    title: w.title,
                    reading_order: w.reading_order,
                }),
            })
        }
    }
}

/// The presentation backends accept.
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
    pub phase: ResolvedPhase,
    pub masters: IndexMap<MasterId, Master>,
    pub layouts: IndexMap<LayoutId, Layout>,
    pub slides: Vec<Slide>,
    #[serde(deserialize_with = "crate::de::js_record")]
    pub fonts: IndexMap<String, ResolvedFontEntry>,
    pub media: IndexMap<Sha256, ResolvedMediaEntry>,
}

impl Presentation {
    /// The ids of every container, masters then layouts then slides.
    pub fn container_ids(&self) -> impl Iterator<Item = &NodeId> {
        self.masters
            .keys()
            .chain(self.layouts.keys())
            .chain(self.slides.iter().map(|slide| &slide.id))
    }
}
