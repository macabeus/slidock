//! Features and census: the closed vocabulary of source features, the severity floor each loss must reach, and the
//! census of what an IR holds.

use std::collections::BTreeSet;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::color::Color;
use crate::diagnostics::Severity;
use crate::elements::{ForeignType, MediaKind};
use crate::geometry::Geometry;
use crate::paint::{CellFill, Dash, Fill, OdfStyle, PathShape, StrokePaint};
use crate::source::{Container, Element, Presentation};
use crate::table::table_cells;
use crate::text::{
    Autofit, Bullet, Caps, Decoration, Inline, LineSpacing, PartialRunProps, TextBody,
};

macro_rules! features {
    ($($variant:ident = $wire:literal,)*) => {
        /// The closed vocabulary of source features. Frontends report what they detect in these terms, censuses count
        /// what an IR holds, and backends declare what they print. Ordered by name in code-unit order, as
        /// `sortFeatures` sorts.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum FeatureId {
            $(#[serde(rename = $wire)] $variant,)*
        }

        impl FeatureId {
            /// Every feature, in vocabulary order.
            pub const ALL: &'static [FeatureId] = &[$(FeatureId::$variant,)*];

            /// The dotted name (`"text.bullet.auto"`).
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(FeatureId::$variant => $wire,)*
                }
            }
        }
    };
}

features! {
    PresentationHiddenSlide = "presentation.hidden-slide",
    SlideBackground = "slide.background",
    SlideNotes = "slide.notes",
    SlideTransition = "slide.transition",
    SlideTimeline = "slide.timeline",
    LayoutElements = "layout.elements",
    Placeholder = "placeholder",
    ElementShape = "element.shape",
    ElementPicture = "element.picture",
    ElementVideo = "element.video",
    ElementAudio = "element.audio",
    ElementGroup = "element.group",
    ElementTable = "element.table",
    ElementChart = "element.chart",
    ElementSmartart = "element.smartart",
    ElementOle = "element.ole",
    ElementMath = "element.math",
    ElementOther = "element.other",
    ElementHidden = "element.hidden",
    ElementLink = "element.link",
    ElementAlt = "element.alt",
    ElementOpacity = "element.opacity",
    GeometryPreset = "geometry.preset",
    GeometryCustom = "geometry.custom",
    GeometryConnector = "geometry.connector",
    FillSolid = "fill.solid",
    FillLinear = "fill.linear",
    FillRadial = "fill.radial",
    FillPath = "fill.path",
    FillImage = "fill.image",
    FillPattern = "fill.pattern",
    FillTile = "fill.tile",
    Stroke = "stroke",
    StrokeDash = "stroke.dash",
    StrokeLineEnd = "stroke.line-end",
    StrokeGradient = "stroke.gradient",
    EffectShadow = "effect.shadow",
    EffectGlow = "effect.glow",
    EffectReflection = "effect.reflection",
    EffectSoftEdge = "effect.soft-edge",
    Effect3d = "effect.3d",
    ImageCrop = "image.crop",
    ImageClip = "image.clip",
    ImageMask = "image.mask",
    ImageRecolor = "image.recolor",
    ImageAnimated = "image.animated",
    ImageVector = "image.vector",
    ImageOpacity = "image.opacity",
    FrameRotation = "frame.rotation",
    FrameFlip = "frame.flip",
    FrameShear = "frame.shear",
    TextRun = "text.run",
    TextWeight = "text.weight",
    TextItalic = "text.italic",
    TextUnderline = "text.underline",
    TextStrike = "text.strike",
    TextHighlight = "text.highlight",
    TextShadow = "text.shadow",
    TextBaseline = "text.baseline",
    TextCaps = "text.caps",
    TextLetterSpacing = "text.letter-spacing",
    TextLink = "text.link",
    TextField = "text.field",
    TextTab = "text.tab",
    TextBreak = "text.break",
    TextBulletChar = "text.bullet.char",
    TextBulletAuto = "text.bullet.auto",
    TextBulletImage = "text.bullet.image",
    TextAutofitShrink = "text.autofit.shrink",
    TextAutofitResize = "text.autofit.resize",
    TextColumns = "text.columns",
    TextVertical = "text.vertical",
    TextRtl = "text.rtl",
    TextLineSpacing = "text.line-spacing",
    FontEmbedded = "font.embedded",
    Animation = "animation",
    Transition = "transition",
}

impl PartialOrd for FeatureId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Features order by name in code-unit order (ASCII), as `sortFeatures` sorts them.
impl Ord for FeatureId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

/// Whole objects whose absence a viewer sees, so the slide goes unscored.
const DECLINE_FLOOR: [FeatureId; 13] = [
    FeatureId::LayoutElements,
    FeatureId::ElementShape,
    FeatureId::ElementPicture,
    FeatureId::ElementVideo,
    FeatureId::ElementGroup,
    FeatureId::ElementTable,
    FeatureId::ElementChart,
    FeatureId::ElementSmartart,
    FeatureId::ElementOle,
    FeatureId::ElementMath,
    FeatureId::ElementOther,
    FeatureId::GeometryCustom,
    FeatureId::ImageVector,
];

impl FeatureId {
    /// The lowest severity a diagnostic naming the feature must have to account for its loss: `decline` where the
    /// miss is visible and unscoreable, `info` for facts with no visible effect (`font.embedded`), `lossy` for
    /// approximations and losses invisible in the static render.
    pub fn floor(self) -> Severity {
        if DECLINE_FLOOR.contains(&self) {
            Severity::Decline
        } else if self == FeatureId::FontEmbedded {
            Severity::Info
        } else {
            Severity::Lossy
        }
    }
}

impl ForeignType {
    /// The feature a Foreign node of this type carries.
    pub fn feature(self) -> FeatureId {
        match self {
            ForeignType::Table => FeatureId::ElementTable,
            ForeignType::Chart => FeatureId::ElementChart,
            ForeignType::Smartart => FeatureId::ElementSmartart,
            ForeignType::Ole => FeatureId::ElementOle,
            ForeignType::Math => FeatureId::ElementMath,
            ForeignType::Ink | ForeignType::ThreeD | ForeignType::Zoom | ForeignType::Unknown => {
                FeatureId::ElementOther
            }
        }
    }
}

/// Media types drawn as vector images.
pub const VECTOR_MEDIA: [&str; 6] = [
    "image/svg+xml",
    "image/emf",
    "image/wmf",
    "image/x-emf",
    "image/x-wmf",
    "application/pdf",
];

fn geometry_feature(geometry: &Geometry) -> FeatureId {
    let preset = match geometry {
        Geometry::Preset(preset) => Some(preset.name),
        Geometry::Paths(paths) => paths.preset,
        Geometry::Custom(_) => None,
    };
    match preset {
        None => FeatureId::GeometryCustom,
        Some(name) if name.is_connector() => FeatureId::GeometryConnector,
        Some(_) => FeatureId::GeometryPreset,
    }
}

/// The feature a fill draws: `fill.path` for contours that follow a rectangle or an ellipse family (lowered `rect`
/// and `ellipse`, and the source forms that lower to them).
pub fn fill_feature(fill: &Fill) -> Option<FeatureId> {
    Some(match fill {
        Fill::None => return None,
        Fill::Solid { .. } => FeatureId::FillSolid,
        Fill::Image { .. } => FeatureId::FillImage,
        Fill::LinearAngle(_) | Fill::Linear(_) => FeatureId::FillLinear,
        Fill::RadialSource(_) | Fill::Radial(_) => FeatureId::FillRadial,
        Fill::Rect(_) | Fill::Ellipse(_) => FeatureId::FillPath,
        Fill::Path(path) => match path.path {
            PathShape::Circle => FeatureId::FillRadial,
            PathShape::Rect => FeatureId::FillPath,
        },
        Fill::Odf(odf) => match odf.style {
            OdfStyle::Linear | OdfStyle::Axial => FeatureId::FillLinear,
            OdfStyle::Radial => FeatureId::FillRadial,
            OdfStyle::Ellipsoid | OdfStyle::Square | OdfStyle::Rectangular => FeatureId::FillPath,
        },
    })
}

fn add_fill(out: &mut BTreeSet<FeatureId>, fill: &Fill) {
    out.extend(fill_feature(fill));
}

fn add_cell_fill(out: &mut BTreeSet<FeatureId>, fill: &CellFill) {
    match fill {
        CellFill::Fill(fill) => add_fill(out, fill),
        CellFill::Angled(_) => {
            out.insert(FeatureId::FillLinear);
        }
    }
}

fn add_stroke(out: &mut BTreeSet<FeatureId>, paint: &StrokePaint<Color>, dash: &Dash, ends: bool) {
    if matches!(paint, StrokePaint::None) {
        return;
    }
    out.insert(FeatureId::Stroke);
    if !dash.is_solid() {
        out.insert(FeatureId::StrokeDash);
    }
    if ends {
        out.insert(FeatureId::StrokeLineEnd);
    }
}

fn add_run_props(out: &mut BTreeSet<FeatureId>, props: Option<&PartialRunProps>) {
    let Some(props) = props else { return };
    if props.weight.is_some_and(|weight| weight != 400) {
        out.insert(FeatureId::TextWeight);
    }
    if props.italic == Some(true) {
        out.insert(FeatureId::TextItalic);
    }
    if props.underline.is_some_and(|d| d != Decoration::None) {
        out.insert(FeatureId::TextUnderline);
    }
    if props.strike.is_some_and(|d| d != Decoration::None) {
        out.insert(FeatureId::TextStrike);
    }
    if matches!(props.highlight, Some(Some(_))) {
        out.insert(FeatureId::TextHighlight);
    }
    if props.baseline.is_some_and(|b| b.steps() != 0) {
        out.insert(FeatureId::TextBaseline);
    }
    if props.caps.is_some_and(|caps| caps != Caps::None) {
        out.insert(FeatureId::TextCaps);
    }
    if props.letter_spacing.is_some_and(|s| s.steps() != 0) {
        out.insert(FeatureId::TextLetterSpacing);
    }
    if matches!(props.link, Some(Some(_))) {
        out.insert(FeatureId::TextLink);
    }
    if matches!(props.shadow, Some(Some(_))) {
        out.insert(FeatureId::TextShadow);
    }
}

fn add_text(out: &mut BTreeSet<FeatureId>, text: &TextBody) {
    match &text.body.autofit {
        Some(Autofit::Shrink { .. }) => {
            out.insert(FeatureId::TextAutofitShrink);
        }
        Some(Autofit::Resize { .. }) => {
            out.insert(FeatureId::TextAutofitResize);
        }
        _ => {}
    }
    for paragraph in &text.paragraphs {
        let props = &paragraph.props;
        match &props.bullet {
            Some(Bullet::Char { .. } | Bullet::Label { auto: None, .. }) => {
                out.insert(FeatureId::TextBulletChar);
            }
            Some(Bullet::Auto { .. } | Bullet::Label { auto: Some(_), .. }) => {
                out.insert(FeatureId::TextBulletAuto);
            }
            _ => {}
        }
        if props.rtl == Some(true) {
            out.insert(FeatureId::TextRtl);
        }
        match &props.line_spacing {
            Some(LineSpacing::Multiple { value, .. }) if value.to_f64() == 1.0 => {}
            Some(_) => {
                out.insert(FeatureId::TextLineSpacing);
            }
            None => {}
        }
        for inline in &paragraph.inlines {
            match inline {
                Inline::Run { text, .. } if !text.is_empty() => {
                    out.insert(FeatureId::TextRun);
                }
                Inline::Field { .. } => {
                    out.insert(FeatureId::TextField);
                }
                Inline::Tab {} => {
                    out.insert(FeatureId::TextTab);
                }
                Inline::Break { .. } => {
                    out.insert(FeatureId::TextBreak);
                }
                Inline::Run { .. } => {}
            }
            add_run_props(out, inline.props());
        }
    }
}

fn add_elements(out: &mut BTreeSet<FeatureId>, elements: &[Element], presentation: &Presentation) {
    for element in elements {
        if element.hidden() == Some(true) {
            out.insert(FeatureId::ElementHidden);
        }
        if element.link().is_some() {
            out.insert(FeatureId::ElementLink);
        }
        if element.alt().is_some_and(|alt| !alt.is_empty()) {
            out.insert(FeatureId::ElementAlt);
        }
        if element.placeholder().is_some() {
            out.insert(FeatureId::Placeholder);
        }
        if element
            .opacity()
            .is_some_and(|opacity| opacity.to_f64() < 1.0)
        {
            out.insert(if matches!(element, Element::Picture(_)) {
                FeatureId::ImageOpacity
            } else {
                FeatureId::ElementOpacity
            });
        }
        if matches!(element.shadow(), Some(Some(_))) {
            out.insert(FeatureId::EffectShadow);
        }
        if let Some(frame) = element.frame() {
            if frame.rotation.steps() != 0 {
                out.insert(FeatureId::FrameRotation);
            }
            if frame.flip_h || frame.flip_v {
                out.insert(FeatureId::FrameFlip);
            }
        }
        match element {
            Element::Shape(shape) => {
                out.insert(FeatureId::ElementShape);
                if let Some(geometry) = &shape.geometry {
                    out.insert(geometry_feature(geometry));
                }
                if let Some(fill) = &shape.fill {
                    add_fill(out, fill);
                }
                if let Some(Some(stroke)) = &shape.stroke {
                    add_stroke(
                        out,
                        &stroke.fill,
                        &stroke.dash,
                        stroke.head.is_some() || stroke.tail.is_some(),
                    );
                }
                if let Some(text) = &shape.text {
                    add_text(out, text);
                }
            }
            Element::Picture(picture) => {
                out.insert(FeatureId::ElementPicture);
                if picture
                    .crop
                    .as_ref()
                    .is_some_and(crate::elements::Crop::is_set)
                {
                    out.insert(FeatureId::ImageCrop);
                }
                if picture.clip.is_some() {
                    out.insert(FeatureId::ImageClip);
                }
                if picture.mask.as_ref().is_some_and(|mask| !mask.is_empty()) {
                    out.insert(FeatureId::ImageMask);
                }
                if let Some(Some(stroke)) = &picture.stroke {
                    add_stroke(
                        out,
                        &stroke.fill,
                        &stroke.dash,
                        stroke.head.is_some() || stroke.tail.is_some(),
                    );
                }
                if let Some(media) = presentation.media.get(&picture.media) {
                    if media.frames.is_some_and(|frames| frames > 1) {
                        out.insert(FeatureId::ImageAnimated);
                    }
                    if VECTOR_MEDIA.contains(&media.mime.as_str()) {
                        out.insert(FeatureId::ImageVector);
                    }
                }
            }
            Element::Media(media) => {
                out.insert(match media.media_kind {
                    MediaKind::Video => FeatureId::ElementVideo,
                    MediaKind::Audio => FeatureId::ElementAudio,
                });
            }
            Element::Table(table) => {
                out.insert(FeatureId::ElementTable);
                for (cell, _, _) in table_cells(&table.rows) {
                    if let Some(fill) = &cell.fill {
                        add_cell_fill(out, fill);
                    }
                    if let Some(borders) = &cell.borders {
                        for (_, border) in borders.edges() {
                            if let Some(border) = border {
                                add_stroke(out, &border.fill, &border.dash, false);
                            }
                        }
                    }
                    add_text(out, &cell.text);
                }
            }
            Element::Group(group) => {
                out.insert(FeatureId::ElementGroup);
                add_elements(out, &group.children, presentation);
            }
            Element::Foreign(foreign) => {
                out.insert(foreign.foreign_type.feature());
                if let Some(children) = foreign.fallback.as_ref().and_then(|f| f.elements.as_ref())
                {
                    add_elements(out, children, presentation);
                }
            }
        }
    }
}

/// The features present in one container's IR, sorted by name. Pure; reads only what the container renders.
pub fn census_container(container: Container<'_>, presentation: &Presentation) -> Vec<FeatureId> {
    let mut out = BTreeSet::new();
    if container
        .background()
        .is_some_and(|fill| !matches!(fill, crate::paint::Fill::None))
    {
        out.insert(FeatureId::SlideBackground);
    }
    match container {
        Container::Slide(slide) => {
            if slide.hidden {
                out.insert(FeatureId::PresentationHiddenSlide);
            }
            if slide
                .notes
                .as_ref()
                .is_some_and(|notes| !notes.paragraphs.is_empty())
            {
                out.insert(FeatureId::SlideNotes);
            }
            if slide.transition.is_some() {
                out.insert(FeatureId::SlideTransition);
            }
            if !slide.timeline.is_empty() {
                out.insert(FeatureId::SlideTimeline);
            }
        }
        Container::Master(_) | Container::Layout(_) => {
            if !container.elements().is_empty() {
                out.insert(FeatureId::LayoutElements);
            }
        }
    }
    add_elements(&mut out, container.elements(), presentation);
    out.into_iter().collect()
}

/// Features per container id, after the presentation-level ones under `"presentation"`.
pub fn census_presentation(presentation: &Presentation) -> IndexMap<String, Vec<FeatureId>> {
    let mut out = IndexMap::new();
    let mut level = Vec::new();
    if presentation
        .fonts
        .values()
        .any(|entry| entry.request.embedded)
    {
        level.push(FeatureId::FontEmbedded);
    }
    out.insert("presentation".to_owned(), level);
    for container in presentation.containers() {
        out.insert(
            container.id().to_string(),
            census_container(container, presentation),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_feature_has_a_floor() {
        assert_eq!(FeatureId::ALL.len(), 77);
        assert_eq!(FeatureId::ElementTable.floor(), Severity::Decline);
        assert_eq!(FeatureId::FillPath.floor(), Severity::Lossy);
        assert_eq!(FeatureId::EffectShadow.floor(), Severity::Lossy);
        assert_eq!(FeatureId::TextShadow.floor(), Severity::Lossy);
        assert_eq!(FeatureId::FontEmbedded.floor(), Severity::Info);
        let declines = FeatureId::ALL
            .iter()
            .filter(|f| f.floor() == Severity::Decline)
            .count();
        assert_eq!(declines, DECLINE_FLOOR.len());
    }

    #[test]
    fn counts_rect_and_ellipse_contours_and_the_sources_that_lower_to_them_as_path_fills() {
        let fill = |value: serde_json::Value| -> Fill { serde_json::from_value(value).unwrap() };
        let stops = serde_json::json!([{ "pos": 0, "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 0,
                                                               "b": 0, "a": 1 } }]);
        let p = serde_json::json!({ "x": 0.5, "y": 0.5 });
        let zero = serde_json::json!({ "l": 0, "t": 0, "r": 0, "b": 0 });
        let cases = [
            (
                serde_json::json!({ "kind": "rect", "origin": p, "axisX": { "x": 0.5, "y": 0 },
                                 "axisY": { "x": 0, "y": 0.5 }, "inner": zero, "stops": stops }),
                FeatureId::FillPath,
            ),
            (
                serde_json::json!({ "kind": "ellipse", "origin": p, "axisX": { "x": 0.5, "y": 0 },
                                 "axisY": { "x": 0, "y": 0.5 }, "inner": { "rx": 0, "ry": 0 }, "steps": 4,
                                 "stops": stops }),
                FeatureId::FillPath,
            ),
            (
                serde_json::json!({ "kind": "path", "path": "rect", "focus": zero, "tile": zero, "stops": stops }),
                FeatureId::FillPath,
            ),
            (
                serde_json::json!({ "kind": "path", "path": "circle", "focus": zero, "tile": zero, "stops": stops }),
                FeatureId::FillRadial,
            ),
            (
                serde_json::json!({ "kind": "odf", "style": "square", "angle": 0, "border": 0, "center": p,
                                 "stops": stops }),
                FeatureId::FillPath,
            ),
            (
                serde_json::json!({ "kind": "odf", "style": "axial", "angle": 0, "border": 0, "center": p,
                                 "stops": stops }),
                FeatureId::FillLinear,
            ),
            (
                serde_json::json!({ "kind": "radial", "center": p, "radius": 0.5, "radiusY": 1, "stops": stops }),
                FeatureId::FillRadial,
            ),
        ];
        for (value, feature) in cases {
            assert_eq!(fill_feature(&fill(value.clone())), Some(feature), "{value}");
        }
    }

    #[test]
    fn maps_every_foreign_type_to_an_element_feature() {
        for foreign in [
            ForeignType::Table,
            ForeignType::Chart,
            ForeignType::Smartart,
            ForeignType::Ole,
            ForeignType::Math,
            ForeignType::Ink,
            ForeignType::ThreeD,
            ForeignType::Zoom,
            ForeignType::Unknown,
        ] {
            assert!(
                foreign.feature().as_str().starts_with("element."),
                "{foreign:?}"
            );
        }
    }

    #[test]
    fn orders_by_name_and_serialises_dotted() {
        assert!(FeatureId::TextBaseline < FeatureId::TextBreak);
        assert!(FeatureId::Animation < FeatureId::ElementAlt);
        assert_eq!(
            serde_json::to_string(&FeatureId::TextBulletAuto).unwrap(),
            "\"text.bullet.auto\""
        );
        let names: Vec<&str> = FeatureId::ALL.iter().map(|f| f.as_str()).collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len());
    }
}
