//! Builders for resolved presentations, and the fixture the printer tests change one rule at a time: two slides
//! that hold every element kind (a title and a list body, an arrow with a gradient and a line end, a cropped and
//! clipped photo, a nested group, a Foreign table with a fallback image, a video with a poster and a trim, a rounded
//! note with a slide link, a table with spans, borders and a gradient cell), a layout and its master with a logo.
//! It prints as the private full resolved fixture does (checked by the `golden` suite).
#![allow(dead_code, clippy::unwrap_used, reason = "test code")]

use indexmap::IndexMap;
use slidock_format::manifest::{Generator, ManifestSource};
use slidock_html::{EmitOptions, Emitted, emit_documents};
use slidock_ir::color::{
    ColorMap, ColorMod, ColorModOp, ColorSpace, MappedSlot, ResolvedColor, ResolvedThemeColor,
    RgbColor, SchemeColors, SchemeSlot, ThemeTag,
};
use slidock_ir::elements::{
    Crop, ForeignFallback, ForeignType, MediaKind, Placeholder, PlaceholderRole, Trim,
};
use slidock_ir::fonts::{
    BindingKind, FontBinding, FontMetrics, FontRequest, GenericFamily, ResolvedFontEntry, VMetrics,
    WinMetrics, font_key,
};
use slidock_ir::formats::{EffectId, FileFormat, SourceFormat};
use slidock_ir::frame::Frame;
use slidock_ir::geometry::{
    FillRule, Path, PathRole, PathsGeometry, PathsTag, PresetName, Seg, Segment,
};
use slidock_ir::ids::{NodeId, Sha256};
use slidock_ir::media::{MediaFile, MediaPlan, Mime, PlanAction, PlanOutput, ResolvedMediaEntry};
use slidock_ir::origin::Origin;
use slidock_ir::paint::{
    Cap, Dash, DashPreset, GradientStop, Join, LinearLowered, Point, RadialLowered,
    ResolvedCellFill, ResolvedFill, ResolvedStroke, Stroke, StrokePaint,
};
use slidock_ir::presentation::{Defaults, MajorMinor, ResolvedPhase, SlideSize, Theme, ThemeFonts};
use slidock_ir::resolved::{
    Body, Built, BuiltSlide, Cell, Element, Foreign, Group, Layout, Master, MediaElement, Picture,
    Presentation, Shape, Slide, Table,
};
use slidock_ir::table::{Border, CellBorders, TableColumn, TableRow};
use slidock_ir::text::{
    Align, Anchor, AutoNumScheme, AutoNumbering, Autofit, BodyProps, Bullet, Caps, Decoration,
    DefaultTextStyles, FamilyFont, FieldKind, FontRef, Inline, Insets, LevelDefaults, LineBasis,
    LineHeight, LineSpacing, Link, Overflow, ParaProps, Paragraph, PartialBodyProps,
    PartialParaProps, PartialRunProps, ResolvedAutofit, ResolvedBullet, ResolvedInline,
    ResolvedParaProps, ResolvedParagraph, ResolvedRunProps, ResolvedTextBody, RunProps, Spacing,
    TextBody, TextRotation, ThemeFontSlot, Wrap,
};
use slidock_ir::timeline::{
    Advance, Delivery, StepEffect, StepFallback, StepKind, StepTarget, TimelineStep, Transition,
    TransitionEffect, TransitionFallback, Trigger,
};
use slidock_ir::units::{Angle, Ms, Pt, Ratio, Size, UnitRatio};

// Units.

pub fn pt(x: f64) -> Pt {
    Pt::exact(x).unwrap()
}

pub fn size(x: f64) -> Size {
    Size::exact(x).unwrap()
}

pub fn ratio(x: f64) -> Ratio {
    Ratio::exact(x).unwrap()
}

pub fn unit(x: f64) -> UnitRatio {
    UnitRatio::exact(x).unwrap()
}

pub fn angle(x: f64) -> Angle {
    Angle::exact(x).unwrap()
}

pub fn id(text: &str) -> NodeId {
    NodeId::new(text).unwrap()
}

/// The SHA-256 of a fixture media item's bytes.
pub fn sha(name: &str) -> Sha256 {
    let hex = match name {
        "poster.png" => "10dd12d5330259fbf93005f63e29dfaae65885a80d0ea11cffe0cdd4c8f8f8ac",
        "logo.png" => "85c39d66deda90996b78a3a39c8f194089238ac7de266e97a20d26e7c7b86cbf",
        "table.png" => "8f7fa92dd6b7d0f869a860754ec4491aa5dc5a6c582b8bf2398c17a8623bb355",
        "clip.mp4" => "a926572f089a588fe12709a1aef00bd6b7131c7b1a5995d1914cd1917047c03d",
        "photo.jpg" => "f14fbb9805c892808f72571ae2701b88601c47aff9449a9b847439c24077a957",
        _ => return Sha256::of(name.as_bytes()),
    };
    Sha256::parse(hex).unwrap()
}

// Colours.

pub fn rgb(r: f64, g: f64, b: f64) -> RgbColor {
    rgba(r, g, b, 1.0)
}

pub fn rgba(r: f64, g: f64, b: f64, a: f64) -> RgbColor {
    RgbColor::new(ColorSpace::Srgb, unit(r), unit(g), unit(b), unit(a))
}

pub fn literal(color: RgbColor) -> ResolvedColor {
    ResolvedColor::Rgb(color)
}

pub fn theme_color(slot: MappedSlot, resolved: RgbColor) -> ResolvedColor {
    ResolvedColor::Theme(ResolvedThemeColor {
        kind: ThemeTag,
        slot,
        mods: Vec::new(),
        resolved,
    })
}

pub fn modified(slot: MappedSlot, op: ColorModOp, value: f64, resolved: RgbColor) -> ResolvedColor {
    ResolvedColor::Theme(ResolvedThemeColor {
        kind: ThemeTag,
        slot,
        mods: vec![ColorMod {
            op,
            value: ratio(value),
        }],
        resolved,
    })
}

pub fn black() -> RgbColor {
    rgb(0.0, 0.0, 0.0)
}

pub fn white() -> RgbColor {
    rgb(1.0, 1.0, 1.0)
}

/// `tx1`, which the fixture's master maps to black.
pub fn tx1() -> ResolvedColor {
    theme_color(MappedSlot::Tx1, black())
}

/// `tx2`, the theme's dark blue-grey.
pub fn tx2() -> ResolvedColor {
    theme_color(MappedSlot::Tx2, rgb(0.26667, 0.32941, 0.41569))
}

pub fn accent1() -> RgbColor {
    rgb(0.26667, 0.44706, 0.76863)
}

/// `accent1` at 75% luminance.
pub fn accent1_dark() -> ResolvedColor {
    modified(
        MappedSlot::Accent1,
        ColorModOp::LumMod,
        0.75,
        rgb(0.2, 0.33529, 0.57647),
    )
}

pub fn solid(color: ResolvedColor) -> ResolvedFill {
    ResolvedFill::Solid { color }
}

pub fn stop(pos: f64, color: ResolvedColor) -> GradientStop<ResolvedColor> {
    GradientStop {
        pos: unit(pos),
        color,
    }
}

/// A vertical linear gradient from `from` at the top to `to` at the bottom.
pub fn vertical(from: ResolvedColor, to: ResolvedColor) -> LinearLowered<ResolvedColor> {
    LinearLowered {
        from: Point {
            x: ratio(0.5),
            y: ratio(0.0),
        },
        to: Point {
            x: ratio(0.5),
            y: ratio(1.0),
        },
        stops: vec![stop(0.0, from), stop(1.0, to)],
        steps: None,
    }
}

pub fn radial(from: ResolvedColor, to: ResolvedColor) -> ResolvedFill {
    ResolvedFill::Radial(RadialLowered {
        center: Point {
            x: ratio(0.5),
            y: ratio(0.5),
        },
        radius: ratio(0.5),
        radius_y: None,
        focus: None,
        stops: vec![stop(0.0, from), stop(1.0, to)],
        steps: None,
    })
}

// Geometry.

pub fn frame(x: f64, y: f64, w: f64, h: f64) -> Frame {
    Frame {
        x: pt(x),
        y: pt(y),
        w: size(w),
        h: size(h),
        rotation: Angle::ZERO,
        flip_h: false,
        flip_v: false,
    }
}

pub fn m(x: f64, y: f64) -> Segment {
    Seg::M(pt(x), pt(y))
}

pub fn l(x: f64, y: f64) -> Segment {
    Seg::L(pt(x), pt(y))
}

pub fn c(x1: f64, y1: f64, x2: f64, y2: f64, x: f64, y: f64) -> Segment {
    Seg::C(pt(x1), pt(y1), pt(x2), pt(y2), pt(x), pt(y))
}

pub fn path(segments: Vec<Segment>) -> Path {
    Path {
        segments,
        fill: true,
        stroke: true,
        fill_rule: FillRule::Nonzero,
        role: None,
    }
}

pub fn paths(preset: Option<&str>, paths: Vec<Path>) -> PathsGeometry {
    PathsGeometry {
        kind: PathsTag,
        paths,
        preset: preset.map(|name| PresetName::parse(name).unwrap()),
    }
}

/// A `rect` as lower-geometry writes it: one path around the frame.
pub fn rect(w: f64, h: f64) -> PathsGeometry {
    paths(
        Some("rect"),
        vec![path(vec![
            m(0.0, 0.0),
            l(w, 0.0),
            l(w, h),
            l(0.0, h),
            Seg::Z,
        ])],
    )
}

pub fn stroke(width: f64, color: ResolvedColor, dash: Dash) -> ResolvedStroke {
    Stroke {
        width: size(width),
        fill: StrokePaint::Solid { color },
        dash,
        cap: Cap::Flat,
        join: Join::Miter,
        head: None,
        tail: None,
    }
}

pub fn border(width: f64, color: RgbColor, dash: DashPreset) -> Border<ResolvedColor> {
    Border {
        width: size(width),
        fill: StrokePaint::Solid {
            color: literal(color),
        },
        dash: Dash::Preset(dash),
        cap: Cap::Flat,
        join: Join::Round,
    }
}

// Text.

pub fn family(name: &str) -> FamilyFont {
    FamilyFont::new(name)
}

pub fn themed(name: &str, slot: ThemeFontSlot) -> FamilyFont {
    FamilyFont {
        theme: Some(slot),
        ..FamilyFont::new(name)
    }
}

/// Regular, undecorated run properties.
pub fn run_props(font: FamilyFont, size_pt: f64, color: ResolvedColor) -> ResolvedRunProps {
    ResolvedRunProps {
        font,
        size: size(size_pt),
        weight: 400,
        italic: false,
        underline: Decoration::None,
        strike: Decoration::None,
        color,
        highlight: None,
        baseline: Ratio::ZERO,
        caps: Caps::None,
        letter_spacing: Pt::ZERO,
        link: None,
        synthesize_bold: true,
        shadow: None,
    }
}

/// The theme's body face (Liberation Sans) in `tx1`.
pub fn sans(size_pt: f64) -> ResolvedRunProps {
    run_props(
        themed("Liberation Sans", ThemeFontSlot::Minor),
        size_pt,
        tx1(),
    )
}

pub fn run(text: &str, props: ResolvedRunProps) -> ResolvedInline {
    Inline::Run {
        text: text.to_owned(),
        props,
        line_height: None,
        leading_skew: None,
    }
}

/// Left-aligned single spacing, no bullet: `line-height` `value` of the size.
pub fn para_props(line_height: f64) -> ResolvedParaProps {
    ResolvedParaProps {
        align: Align::Left,
        margin_left: Pt::ZERO,
        margin_right: Pt::ZERO,
        indent: Pt::ZERO,
        space_before: Spacing::Pt { value: Pt::ZERO },
        space_after: Spacing::Pt { value: Pt::ZERO },
        line_spacing: LineSpacing::Multiple {
            value: ratio(1.0),
            basis: LineBasis::Size1_2,
        },
        tab_size: size(72.0),
        bullet: ResolvedBullet::None {},
        rtl: false,
        line_height: LineHeight::Ratio {
            value: ratio(line_height),
        },
        space_before_pt: Pt::ZERO,
        space_after_pt: Pt::ZERO,
        leading_skew: None,
        line_tail_pt: None,
    }
}

pub fn paragraph(props: ResolvedParaProps, inlines: Vec<ResolvedInline>) -> ResolvedParagraph {
    Paragraph {
        level: 0,
        props,
        inlines,
        end_props: None,
    }
}

pub fn insets() -> Insets {
    Insets {
        l: pt(7.2),
        t: pt(3.6),
        r: pt(7.2),
        b: pt(3.6),
    }
}

pub fn text_body(
    anchor: Anchor,
    autofit: ResolvedAutofit,
    paragraphs: Vec<ResolvedParagraph>,
) -> ResolvedTextBody {
    ResolvedTextBody {
        body: BodyProps {
            insets: insets(),
            anchor,
            wrap: Wrap::Square,
            autofit,
            overflow: Overflow::Spread,
            max_width: None,
        },
        list_style: None,
        paragraphs,
        flips_with_frame: false,
        rotation: TextRotation::Zero,
    }
}

pub fn shrink(font_scale: f64, line_spacing_reduction: f64, fitted: bool) -> ResolvedAutofit {
    ResolvedAutofit::Shrink {
        font_scale: unit(font_scale),
        line_spacing_reduction: unit(line_spacing_reduction),
        fitted,
    }
}

// Elements.

pub fn shape(element: &str, frame: Frame, geometry: PathsGeometry) -> Shape {
    Shape {
        id: id(element),
        origin: Origin::Agent {},
        name: None,
        placeholder: None,
        inherits: None,
        match_key: None,
        hidden: None,
        opacity: None,
        link: None,
        alt: None,
        ext: None,
        frame,
        geometry,
        fill: None,
        stroke: None,
        text: None,
        shadow: None,
    }
}

pub fn picture(element: &str, frame: Frame, media: Sha256) -> Picture {
    Picture {
        id: id(element),
        origin: Origin::Agent {},
        name: None,
        placeholder: None,
        inherits: None,
        match_key: None,
        hidden: None,
        opacity: None,
        link: None,
        alt: None,
        ext: None,
        frame,
        media,
        crop: None,
        clip: None,
        mask: None,
        stroke: None,
        shadow: None,
    }
}

pub fn group(element: &str, frame: Frame, children: Vec<Element>) -> Group {
    Group {
        id: id(element),
        origin: Origin::Agent {},
        name: None,
        placeholder: None,
        inherits: None,
        match_key: None,
        hidden: None,
        opacity: None,
        link: None,
        alt: None,
        ext: None,
        frame,
        children,
        shadow: None,
    }
}

fn placeholder(role: PlaceholderRole, index: Option<u64>) -> Option<Placeholder> {
    Some(Placeholder {
        role,
        index,
        target: None,
    })
}

/// A placeholder with no text, as layouts and masters hold them.
fn empty_placeholder(
    element: &str,
    role: PlaceholderRole,
    index: Option<u64>,
    frame: Frame,
    autofit: ResolvedAutofit,
    anchor: Anchor,
) -> Element {
    let (w, h) = (frame.w.to_f64(), frame.h.to_f64());
    Element::Shape(Box::new(Shape {
        placeholder: placeholder(role, index),
        text: Some(text_body(anchor, autofit, Vec::new())),
        ..shape(element, frame, rect(w, h))
    }))
}

// Media.

fn media_entry(name: &str, mime: &str, ext: &str, width: u64, height: u64) -> ResolvedMediaEntry {
    let sha256 = sha(name);
    ResolvedMediaEntry {
        sha256,
        mime: Mime::new(mime).unwrap(),
        bytes: 4096,
        width: Some(width),
        height: Some(height),
        frames: None,
        codec: None,
        duration_ms: None,
        origin: Origin::source(format!("media/{name}")),
        plan: MediaPlan {
            action: PlanAction::Copy,
            output: Some(PlanOutput {
                mime: Mime::new(mime).unwrap(),
                file: MediaFile::of(&sha256, ext).unwrap(),
            }),
            reason: None,
        },
    }
}

/// A plan that writes no file for the item.
pub fn omitted(reason: &str) -> MediaPlan {
    MediaPlan {
        action: PlanAction::Omit,
        output: None,
        reason: Some(reason.to_owned()),
    }
}

// Fonts.

fn metrics(units_per_em: u32, hhea: (i32, i32, i32), typo: (i32, i32, i32)) -> FontMetrics {
    FontMetrics {
        units_per_em,
        hhea: VMetrics {
            ascender: hhea.0,
            descender: hhea.1,
            line_gap: hhea.2,
        },
        typo: VMetrics {
            ascender: typo.0,
            descender: typo.1,
            line_gap: typo.2,
        },
        win: WinMetrics {
            ascent: hhea.0,
            descent: -hhea.1,
        },
        bounds: None,
        grave_top: None,
    }
}

fn installed(
    name: &str,
    generic: GenericFamily,
    metrics: FontMetrics,
) -> (String, ResolvedFontEntry) {
    (
        font_key(name),
        ResolvedFontEntry {
            request: FontRequest {
                family: name.to_owned(),
                faces: Vec::new(),
                postscript_names: Vec::new(),
                embedded: false,
            },
            binding: FontBinding {
                kind: BindingKind::Installed,
                family: name.to_owned(),
                weight: None,
                generic,
                metrics,
                faces: None,
                file: None,
            },
        },
    )
}

// The fixture.

fn theme() -> Theme {
    Theme {
        id: id("ir-theme"),
        origin: Origin::Agent {},
        name: Some("Fixture".to_owned()),
        colors: SchemeColors {
            dk1: black(),
            lt1: white(),
            dk2: rgb(0.26667, 0.32941, 0.41569),
            lt2: rgb(0.90588, 0.90196, 0.90196),
            accent1: accent1(),
            accent2: rgb(0.92941, 0.4902, 0.19216),
            accent3: rgb(0.64706, 0.64706, 0.64706),
            accent4: rgb(1.0, 0.75294, 0.0),
            accent5: rgb(0.35686, 0.60784, 0.83529),
            accent6: rgb(0.43922, 0.67843, 0.27843),
            hlink: rgb(0.01961, 0.38824, 0.75686),
            fol_hlink: rgb(0.58431, 0.3098, 0.44706),
        },
        fonts: MajorMinor {
            major: ThemeFonts {
                latin: "Liberation Serif".to_owned(),
                ea: None,
                cs: None,
            },
            minor: ThemeFonts {
                latin: "Liberation Sans".to_owned(),
                ea: None,
                cs: None,
            },
        },
        ext: None,
    }
}

fn defaults() -> Defaults {
    let level = || LevelDefaults {
        para: ParaProps {
            align: Align::Left,
            margin_left: Pt::ZERO,
            margin_right: Pt::ZERO,
            indent: Pt::ZERO,
            space_before: Spacing::Pt { value: Pt::ZERO },
            space_after: Spacing::Pt { value: Pt::ZERO },
            line_spacing: LineSpacing::Multiple {
                value: ratio(1.0),
                basis: LineBasis::Size1_2,
            },
            tab_size: size(72.0),
            bullet: Bullet::None {},
            rtl: false,
            line_height: None,
            space_before_pt: None,
            space_after_pt: None,
            leading_skew: None,
            line_tail_pt: None,
        },
        run: RunProps {
            font: FontRef::Family(family("Liberation Sans")),
            size: size(18.0),
            weight: 400,
            italic: false,
            underline: Decoration::None,
            strike: Decoration::None,
            color: slidock_ir::color::Color::Rgb(black()),
            highlight: None,
            baseline: Ratio::ZERO,
            caps: Caps::None,
            letter_spacing: Pt::ZERO,
            link: None,
            synthesize_bold: true,
            script_scale: unit(1.0),
            shadow: None,
        },
    };
    Defaults {
        text: Box::new(DefaultTextStyles {
            l0: level(),
            l1: level(),
            l2: level(),
            l3: level(),
            l4: level(),
            l5: level(),
            l6: level(),
            l7: level(),
            l8: level(),
        }),
        body: BodyProps {
            insets: insets(),
            anchor: Anchor::Top,
            wrap: Wrap::Square,
            autofit: Autofit::None {},
            overflow: Overflow::Spread,
            max_width: None,
        },
    }
}

fn color_map() -> ColorMap {
    ColorMap {
        dk1: SchemeSlot::Dk1,
        lt1: SchemeSlot::Lt1,
        dk2: SchemeSlot::Dk2,
        lt2: SchemeSlot::Lt2,
        accent1: SchemeSlot::Accent1,
        accent2: SchemeSlot::Accent2,
        accent3: SchemeSlot::Accent3,
        accent4: SchemeSlot::Accent4,
        accent5: SchemeSlot::Accent5,
        accent6: SchemeSlot::Accent6,
        hlink: SchemeSlot::Hlink,
        fol_hlink: SchemeSlot::FolHlink,
        bg1: SchemeSlot::Lt1,
        tx1: SchemeSlot::Dk1,
        bg2: SchemeSlot::Lt2,
        tx2: SchemeSlot::Dk2,
    }
}

fn master() -> Master {
    let logo = Picture {
        alt: Some("Fixture logo".to_owned()),
        ..picture(
            "ir-master.logo",
            frame(660.0, 375.0, 48.0, 24.0),
            sha("logo.png"),
        )
    };
    Master {
        id: id("ir-master"),
        origin: Origin::Agent {},
        name: Some("Fixture master".to_owned()),
        theme: Some(id("ir-theme")),
        color_map: Some(color_map()),
        text_styles: None,
        ext: None,
        body: Body::Built(Built {
            background: Some(solid(theme_color(MappedSlot::Bg1, white()))),
            elements: vec![
                empty_placeholder(
                    "ir-master.title",
                    PlaceholderRole::Title,
                    None,
                    frame(36.0, 20.0, 648.0, 60.0),
                    ResolvedAutofit::None {},
                    Anchor::Bottom,
                ),
                empty_placeholder(
                    "ir-master.body",
                    PlaceholderRole::Body,
                    Some(1),
                    frame(36.0, 96.0, 648.0, 270.0),
                    ResolvedAutofit::None {},
                    Anchor::Top,
                ),
                Element::Picture(Box::new(logo)),
            ],
        }),
    }
}

fn layout() -> Layout {
    Layout {
        id: id("ir-layout"),
        origin: Origin::Agent {},
        name: Some("Title and Content".to_owned()),
        layout_type: Some("obj".to_owned()),
        master: Some(id("ir-master")),
        show_master: true,
        color_map_override: None,
        ext: None,
        body: Body::Built(Built {
            background: None,
            elements: vec![
                empty_placeholder(
                    "ir-layout.title",
                    PlaceholderRole::Title,
                    None,
                    frame(36.0, 20.0, 648.0, 60.0),
                    ResolvedAutofit::None {},
                    Anchor::Bottom,
                ),
                empty_placeholder(
                    "ir-layout.body",
                    PlaceholderRole::Body,
                    Some(1),
                    frame(36.0, 96.0, 648.0, 270.0),
                    shrink(1.0, 0.0, false),
                    Anchor::Top,
                ),
            ],
        }),
    }
}

/// The slide title: "Presentation IR" in the theme's heading face, bottom-anchored.
fn title() -> Shape {
    let props = run_props(
        themed("Liberation Serif", ThemeFontSlot::Major),
        40.0,
        tx2(),
    );
    let body = text_body(
        Anchor::Bottom,
        ResolvedAutofit::None {},
        vec![paragraph(
            para_props(1.2),
            vec![run("Presentation IR", props)],
        )],
    );
    Shape {
        inherits: Some(id("ir-layout.title")),
        match_key: None,
        placeholder: placeholder(PlaceholderRole::Title, None),
        text: Some(body),
        ..shape(
            "ir-s1.title",
            frame(36.0, 20.0, 648.0, 60.0),
            rect(648.0, 60.0),
        )
    }
}

/// The slide body: a numbered item with a link, a tab and a field, then a dashed item in Liberation Mono split by a
/// break.
fn body() -> Shape {
    let dark = || {
        run_props(
            themed("Liberation Sans", ThemeFontSlot::Minor),
            18.0,
            accent1_dark(),
        )
    };
    let numbered = ResolvedParaProps {
        bullet: ResolvedBullet::Label {
            text: "1.".to_owned(),
            font: themed("Liberation Sans", ThemeFontSlot::Minor),
            size: size(18.0),
            color: accent1_dark(),
            auto: Some(AutoNumbering {
                scheme: AutoNumScheme::ArabicPeriod,
                start_at: 1,
            }),
        },
        indent: pt(-18.0),
        margin_left: pt(18.0),
        ..para_props(1.2)
    };
    let first = paragraph(
        numbered,
        vec![
            run("Read the ", dark()),
            run(
                "spec",
                ResolvedRunProps {
                    underline: Decoration::Single,
                    link: Some(Link::Url {
                        url: "https://example.com/spec".to_owned(),
                    }),
                    ..dark()
                },
            ),
            Inline::Tab {},
            Inline::Field {
                field: FieldKind::SlideNumber,
                cached: "1".to_owned(),
                props: ResolvedRunProps {
                    weight: 700,
                    ..dark()
                },
                line_height: None,
                leading_skew: None,
            },
        ],
    );
    let dashed = ResolvedParaProps {
        bullet: ResolvedBullet::Label {
            text: "\u{2013}".to_owned(),
            font: family("Liberation Mono"),
            size: size(16.0),
            color: tx1(),
            auto: None,
        },
        indent: pt(-18.0),
        margin_left: pt(36.0),
        line_height: LineHeight::Ratio {
            value: ratio(1.32258),
        },
        line_spacing: LineSpacing::Multiple {
            value: ratio(1.15),
            basis: LineBasis::Hhea,
        },
        ..para_props(1.2)
    };
    let second = Paragraph {
        level: 1,
        end_props: Some(sans(16.0)),
        ..paragraph(
            dashed,
            vec![
                run(
                    "Second line",
                    ResolvedRunProps {
                        italic: true,
                        ..run_props(family("Liberation Mono"), 16.0, tx1())
                    },
                ),
                Inline::Break {
                    props: Some(sans(16.0)),
                },
                run(
                    "after a break",
                    ResolvedRunProps {
                        baseline: ratio(0.3),
                        ..sans(12.0)
                    },
                ),
            ],
        )
    };
    Shape {
        inherits: Some(id("ir-layout.body")),
        match_key: None,
        placeholder: placeholder(PlaceholderRole::Body, Some(1)),
        text: Some(text_body(
            Anchor::Top,
            shrink(1.0, 0.0, false),
            vec![first, second],
        )),
        ..shape(
            "ir-s1.body",
            frame(36.0, 96.0, 648.0, 270.0),
            rect(648.0, 270.0),
        )
    }
}

/// A right arrow, turned and flipped, filled with a gradient, outlined dashed with a line end.
fn arrow() -> Shape {
    let outline = path(vec![
        m(0.0, 15.0),
        l(170.0, 15.0),
        l(170.0, 0.0),
        l(200.0, 30.0),
        l(170.0, 60.0),
        l(170.0, 45.0),
        l(0.0, 45.0),
        Seg::Z,
    ]);
    let end = Path {
        stroke: false,
        role: Some(PathRole::LineEnd),
        ..path(vec![
            m(196.25, 26.25),
            l(200.0, 30.0),
            l(196.25, 33.75),
            Seg::Z,
        ])
    };
    Shape {
        name: Some("Arrow 7".to_owned()),
        frame: Frame {
            rotation: angle(30.0),
            flip_h: true,
            ..frame(400.0, 300.0, 200.0, 60.0)
        },
        fill: Some(ResolvedFill::Linear(vertical(
            theme_color(MappedSlot::Accent1, accent1()),
            accent1_dark(),
        ))),
        stroke: Some(Some(stroke(1.5, tx1(), Dash::Preset(DashPreset::Dash)))),
        ..shape(
            "ir-s1.arrow",
            frame(400.0, 300.0, 200.0, 60.0),
            paths(Some("rightArrow"), vec![outline, end]),
        )
    }
}

/// A translucent photo, cropped and clipped to an ellipse.
fn photo() -> Picture {
    let ellipse = path(vec![
        m(0.0, 45.0),
        c(0.0, 20.147, 26.863, 0.0, 60.0, 0.0),
        c(93.137, 0.0, 120.0, 20.147, 120.0, 45.0),
        c(120.0, 69.853, 93.137, 90.0, 60.0, 90.0),
        c(26.863, 90.0, 0.0, 69.853, 0.0, 45.0),
        Seg::Z,
    ]);
    Picture {
        alt: Some("A photo".to_owned()),
        opacity: Some(unit(0.5)),
        crop: Some(Crop {
            l: ratio(0.1),
            t: Ratio::ZERO,
            r: ratio(0.1),
            b: ratio(-0.05),
        }),
        clip: Some(paths(Some("ellipse"), vec![ellipse])),
        ..picture(
            "ir-s1.photo",
            frame(36.0, 300.0, 120.0, 90.0),
            sha("photo.jpg"),
        )
    }
}

/// A group holding a radial-filled diamond and a group with a logo.
fn nested_group() -> Group {
    let diamond = Shape {
        fill: Some(radial(
            literal(white()),
            theme_color(MappedSlot::Accent2, rgb(0.92941, 0.4902, 0.19216)),
        )),
        stroke: Some(None),
        ..shape(
            "ir-s1.group.dot",
            frame(0.0, 0.0, 50.0, 50.0),
            paths(
                None,
                vec![Path {
                    stroke: false,
                    ..path(vec![
                        m(0.0, 25.0),
                        l(25.0, 0.0),
                        l(50.0, 25.0),
                        l(25.0, 50.0),
                        Seg::Z,
                    ])
                }],
            ),
        )
    };
    let inner = group(
        "ir-s1.group.inner",
        frame(50.0, 50.0, 50.0, 30.0),
        vec![Element::Picture(Box::new(picture(
            "ir-s1.group.inner.logo",
            frame(0.0, 0.0, 50.0, 30.0),
            sha("logo.png"),
        )))],
    );
    group(
        "ir-s1.group",
        frame(200.0, 300.0, 100.0, 80.0),
        vec![
            Element::Shape(Box::new(diamond)),
            Element::Group(Box::new(inner)),
        ],
    )
}

/// A Foreign table that prints its fallback image.
fn foreign_table() -> Foreign {
    Foreign {
        id: id("ir-s1.table"),
        origin: Origin::Agent {},
        name: None,
        placeholder: None,
        inherits: None,
        match_key: None,
        hidden: None,
        opacity: None,
        link: None,
        alt: None,
        ext: None,
        frame: frame(320.0, 100.0, 360.0, 120.0),
        dialect: FileFormat::Pptx,
        foreign_type: ForeignType::Table,
        payload: None,
        fallback: Some(ForeignFallback {
            media: Some(sha("table.png")),
            elements: None,
        }),
    }
}

/// A muted video with a poster, trimmed to 1–5 s.
fn video() -> MediaElement {
    MediaElement {
        id: id("ir-s1.video"),
        origin: Origin::Agent {},
        name: None,
        placeholder: None,
        inherits: None,
        match_key: None,
        hidden: None,
        opacity: None,
        link: None,
        alt: None,
        ext: None,
        frame: frame(500.0, 20.0, 160.0, 90.0),
        media_kind: MediaKind::Video,
        media: Some(sha("clip.mp4")),
        poster: Some(sha("poster.png")),
        trim: Some(Trim {
            start_ms: Ms(1000),
            end_ms: Some(Ms(5000)),
        }),
        looped: false,
        muted: true,
        autoplay: false,
        across_slides: false,
        shadow: None,
    }
}

fn slide_one() -> Slide {
    let elements = vec![
        Element::Shape(Box::new(title())),
        Element::Shape(Box::new(body())),
        Element::Shape(Box::new(arrow())),
        Element::Picture(Box::new(photo())),
        Element::Group(Box::new(nested_group())),
        Element::Foreign(Box::new(foreign_table())),
        Element::Media(Box::new(video())),
    ];
    let notes = TextBody {
        body: PartialBodyProps::default(),
        list_style: None,
        paragraphs: vec![Paragraph {
            level: 0,
            props: PartialParaProps::default(),
            inlines: vec![Inline::Run {
                text: "Speaker notes".to_owned(),
                props: PartialRunProps::default(),
                line_height: None,
                leading_skew: None,
            }],
            end_props: None,
        }],
        flips_with_frame: false,
        rotation: None,
    };
    Slide {
        id: id("ir-s1"),
        origin: Origin::Agent {},
        layout: Some(id("ir-layout")),
        hidden: false,
        show_layout: true,
        color_map_override: None,
        notes: Some(notes),
        transition: Some(Transition {
            effect: TransitionEffect {
                id: EffectId::new(SourceFormat::File(FileFormat::Pptx), "fade"),
                fallback: TransitionFallback::Dissolve,
                native: None,
            },
            duration_ms: Ms(500),
            advance: Some(Advance {
                on_click: true,
                after_ms: None,
            }),
            easing: None,
        }),
        timeline: vec![TimelineStep {
            target: StepTarget {
                element: id("ir-s1.body"),
                paragraphs: Some((0, 1)),
                background: false,
            },
            trigger: Trigger::Click,
            delay_ms: Ms(0),
            duration_ms: Ms(500),
            kind: StepKind::Entrance,
            effect: StepEffect {
                id: EffectId::new(SourceFormat::File(FileFormat::Keynote), "dissolve"),
                fallback: StepFallback::Dissolve,
                native: None,
            },
            delivery: Delivery::ByParagraph,
            timing: None,
        }],
        ext: None,
        failed_a11y: None,
        body: Body::Built(BuiltSlide {
            background: None,
            reading_order: elements
                .iter()
                .map(|element| element.id().clone())
                .collect(),
            elements,
            title: "Presentation IR".to_owned(),
        }),
    }
}

/// A one-paragraph cell in Liberation Sans 12.
pub fn cell(text: &str, anchor: Anchor, weight: u32) -> Cell {
    let props = ResolvedRunProps {
        weight,
        ..sans(12.0)
    };
    let paragraph = Paragraph {
        end_props: Some(sans(12.0)),
        ..paragraph(para_props(1.1), vec![run(text, props)])
    };
    Cell {
        col_span: None,
        row_span: None,
        fill: None,
        borders: None,
        text: text_body(anchor, ResolvedAutofit::None {}, vec![paragraph]),
    }
}

pub fn edges() -> CellBorders<ResolvedColor> {
    CellBorders {
        top: None,
        right: None,
        bottom: None,
        left: None,
        diagonal_down: None,
        diagonal_up: None,
    }
}

/// A 2 × 2 grid: a spanning header with a 2 pt bottom edge, a cell with a dashed right edge, a gradient cell.
fn grid() -> Table {
    let header = Cell {
        col_span: Some(2),
        fill: Some(ResolvedCellFill::Fill(solid(theme_color(
            MappedSlot::Accent1,
            accent1(),
        )))),
        borders: Some(CellBorders {
            bottom: Some(border(2.0, black(), DashPreset::Solid)),
            ..edges()
        }),
        ..cell("Header", Anchor::Middle, 700)
    };
    let left = Cell {
        borders: Some(CellBorders {
            right: Some(border(1.0, rgb(0.6, 0.6, 0.6), DashPreset::Dash)),
            ..edges()
        }),
        ..cell("A", Anchor::Top, 400)
    };
    let right = Cell {
        fill: Some(ResolvedCellFill::Fill(ResolvedFill::Linear(vertical(
            literal(white()),
            literal(rgb(0.8, 0.8, 0.8)),
        )))),
        ..cell("B", Anchor::Bottom, 400)
    };
    Table {
        id: id("ir-s2.grid"),
        origin: Origin::Agent {},
        name: None,
        placeholder: None,
        inherits: None,
        match_key: None,
        hidden: None,
        opacity: None,
        link: None,
        alt: None,
        ext: None,
        frame: frame(60.0, 360.0, 250.0, 70.0),
        columns: vec![
            TableColumn { width: size(100.0) },
            TableColumn { width: size(150.0) },
        ],
        rows: vec![
            TableRow {
                height: size(30.0),
                cells: vec![Some(header), None],
            },
            TableRow {
                height: size(40.0),
                cells: vec![Some(left), Some(right)],
            },
        ],
    }
}

/// A rounded rectangle linking back to the first slide, its text shrunk to 90%.
fn note() -> Shape {
    let rounded = path(vec![
        m(0.0, 46.668),
        c(0.0, 20.894, 20.894, 0.0, 46.668, 0.0),
        l(553.332, 0.0),
        c(579.106, 0.0, 600.0, 20.894, 600.0, 46.668),
        l(600.0, 233.332),
        c(600.0, 259.106, 579.106, 280.0, 553.332, 280.0),
        l(46.668, 280.0),
        c(20.894, 280.0, 0.0, 259.106, 0.0, 233.332),
        Seg::Z,
    ]);
    let text = text_body(
        Anchor::Middle,
        shrink(0.9, 0.1, true),
        vec![paragraph(
            ResolvedParaProps {
                align: Align::Center,
                ..para_props(1.1)
            },
            vec![run("Back to the start", sans(32.0))],
        )],
    );
    Shape {
        fill: Some(solid(literal(rgba(1.0, 1.0, 1.0, 0.8)))),
        link: Some(Link::Slide { slide: id("ir-s1") }),
        text: Some(text),
        ..shape(
            "ir-s2.note",
            frame(60.0, 60.0, 600.0, 280.0),
            paths(Some("roundRect"), vec![rounded]),
        )
    }
}

fn slide_two() -> Slide {
    let elements = vec![
        Element::Shape(Box::new(note())),
        Element::Table(Box::new(grid())),
    ];
    Slide {
        id: id("ir-s2"),
        origin: Origin::Agent {},
        layout: Some(id("ir-layout")),
        hidden: true,
        show_layout: false,
        color_map_override: None,
        notes: None,
        transition: None,
        timeline: Vec::new(),
        ext: None,
        failed_a11y: None,
        body: Body::Built(BuiltSlide {
            background: Some(radial(
                literal(white()),
                theme_color(MappedSlot::Bg2, rgb(0.90588, 0.90196, 0.90196)),
            )),
            reading_order: elements
                .iter()
                .map(|element| element.id().clone())
                .collect(),
            elements,
            title: "Back to the start".to_owned(),
        }),
    }
}

/// The fixture the printer tests start from.
pub fn full() -> Presentation {
    let media = [
        media_entry("poster.png", "image/png", "png", 1280, 720),
        media_entry("logo.png", "image/png", "png", 96, 48),
        media_entry("table.png", "image/png", "png", 720, 240),
        media_entry("clip.mp4", "video/mp4", "mp4", 1280, 720),
        media_entry("photo.jpg", "image/jpeg", "jpg", 800, 600),
    ];
    Presentation {
        schema: None,
        size: SlideSize {
            w: size(720.0),
            h: size(405.0),
        },
        defaults: defaults(),
        themes: IndexMap::from([(id("ir-theme"), theme())]),
        ext: None,
        phase: ResolvedPhase,
        masters: IndexMap::from([(id("ir-master"), master())]),
        layouts: IndexMap::from([(id("ir-layout"), layout())]),
        slides: vec![slide_one(), slide_two()],
        fonts: IndexMap::from([
            installed(
                "Liberation Mono",
                GenericFamily::Monospace,
                metrics(2048, (1705, -615, 0), (1255, -386, 409)),
            ),
            installed(
                "Liberation Sans",
                GenericFamily::SansSerif,
                metrics(2048, (1854, -434, 67), (1491, -431, 307)),
            ),
            installed(
                "Liberation Serif",
                GenericFamily::Serif,
                metrics(2048, (1825, -443, 87), (1420, -442, 307)),
            ),
        ]),
        media: media
            .into_iter()
            .map(|entry| (entry.sha256, entry))
            .collect(),
    }
}

/// Where the fixture came from.
pub fn provenance() -> ManifestSource {
    ManifestSource {
        format: "ir".to_owned(),
        name: "full.presentation-ir.json".to_owned(),
        sha256: "0".repeat(64),
        bytes: 0,
    }
}

pub fn emit_with(
    presentation: &Presentation,
    provenance: ManifestSource,
    generator: Generator,
) -> Emitted {
    emit_documents(
        presentation,
        &EmitOptions {
            provenance,
            generator,
        },
    )
    .unwrap()
}

pub fn emit(presentation: &Presentation) -> Emitted {
    emit_documents(presentation, &EmitOptions::new(provenance())).unwrap()
}

/// A document of the emitted set, by path.
pub fn document(presentation: &Presentation, path: &str) -> String {
    emit(presentation).files.text[path].clone()
}

pub fn slide_one_html(presentation: &Presentation) -> String {
    document(presentation, "slides/ir-s1.html")
}

pub fn slide_two_html(presentation: &Presentation) -> String {
    document(presentation, "slides/ir-s2.html")
}

/// A shape of a slide, by id, to change.
pub fn shape_mut<'a>(presentation: &'a mut Presentation, element: &str) -> &'a mut Shape {
    for slide in &mut presentation.slides {
        if let Body::Built(built) = &mut slide.body {
            for candidate in &mut built.elements {
                if let Element::Shape(shape) = candidate
                    && shape.id.as_str() == element
                {
                    return shape;
                }
            }
        }
    }
    panic!("no shape {element}")
}

/// The elements of a slide, to change.
pub fn elements_mut(presentation: &mut Presentation, slide: usize) -> &mut Vec<Element> {
    match &mut presentation.slides[slide].body {
        Body::Built(built) => &mut built.elements,
        Body::Failed { .. } => panic!("slide {slide} failed"),
    }
}

/// The built content of a slide, to change.
pub fn built_mut(presentation: &mut Presentation, slide: usize) -> &mut BuiltSlide {
    match &mut presentation.slides[slide].body {
        Body::Built(built) => built,
        Body::Failed { .. } => panic!("slide {slide} failed"),
    }
}

pub fn picture_mut<'a>(presentation: &'a mut Presentation, element: &str) -> &'a mut Picture {
    for candidate in elements_mut(presentation, 0) {
        if let Element::Picture(picture) = candidate
            && picture.id.as_str() == element
        {
            return picture;
        }
    }
    panic!("no picture {element}")
}

pub fn grid_mut(presentation: &mut Presentation) -> &mut Table {
    for candidate in elements_mut(presentation, 1) {
        if let Element::Table(table) = candidate {
            return table;
        }
    }
    panic!("no grid")
}

pub fn video_mut(presentation: &mut Presentation) -> &mut MediaElement {
    for candidate in elements_mut(presentation, 0) {
        if let Element::Media(media) = candidate {
            return media;
        }
    }
    panic!("no video")
}
