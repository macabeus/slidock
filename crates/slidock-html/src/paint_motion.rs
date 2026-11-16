//! The deck path's colour and font hooks: a step that animates an element's text, fill or line
//! colour, or turns its text bold, italic or underlined, plays through a registered custom property the element
//! declares with its static value (`--sd-fill:#4472c4`), which its paint then reads
//! (`fill:var(--sd-fill,#4472c4)`). The player animates the property on the element, and its descendants follow.
//!
//! Only elements a step animates that way declare a hook, and only on the deck path, so the parity documents are
//! unchanged. An element declares a hook only where one value stands for all of that paint, so the declaration
//! changes nothing it draws: a solid opaque sRGB fill or line, text of one colour, one weight (normal or bold), one
//! style, and one underline (none or single, with no strike). A paint the element does not draw is one the source
//! app turns on or leaves off: every fill colour animation of LibreOffice's `effects.xml` sets the fill solid with it
//! (PowerPoint's Change Fill Color sets `fill.on`), and Change Line Color sets the line on, so a colour step of a fill
//! the element lacks, or a colour change of its missing line, shows a paint the deck cannot draw; an HSL shift of a
//! missing line leaves it off, and missing text shows nothing either way, so a track of those is left out and a
//! text colour change plays as it is. A step whose target lacks a paint it turns on, or draws the paint with no
//! single value, plays its fallback, noted with HTML6011 (and HTML6010, as every printed fallback); so does a step
//! left with no track.
//!
//! The deck builder computes the colour tracks' and switches' shifts and mixes from the same static colour, so a deck's tracks
//! hold only `#rrggbb`. A switch (`set`) needs the hook of the property it switches, as a track of it does.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use slidock_ir::color::{ColorSpace, ResolvedColor};
use slidock_ir::effects::{
    BuildEffect, ColorProperty, ColorValue, HexColor, Keyframes, Paint as TrackPaint, SetTrack,
    SetValue, Track,
};
use slidock_ir::fonts::font_key;
use slidock_ir::paint::{ResolvedFill, StrokePaint};
use slidock_ir::resolved::{Element, Presentation, Shape, Slide};
use slidock_ir::text::{Decoration, Inline, ResolvedRunProps};
use slidock_ir::timeline::TimelineStep;

use crate::color::css_color;
use crate::error::Result;
use crate::paint::strokes;

/// A paint a step animates through a custom property.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Hook {
    Fill,
    Line,
    Text,
    Bold,
    Italic,
    Underline,
}

impl Hook {
    /// The custom property the element declares.
    pub const fn property(self) -> &'static str {
        match self {
            Hook::Fill => "--sd-fill",
            Hook::Line => "--sd-line",
            Hook::Text => "--sd-color",
            Hook::Bold => "--sd-bold",
            Hook::Italic => "--sd-italic",
            Hook::Underline => "--sd-underline",
        }
    }

    /// What HTML6011 names.
    const fn name(self) -> &'static str {
        match self {
            Hook::Fill => "fill",
            Hook::Line => "line",
            Hook::Text => "text colour",
            Hook::Bold => "font weight",
            Hook::Italic => "font style",
            Hook::Underline => "underline",
        }
    }

    /// `var(--sd-fill,<value>)`: the paint as a hooked element prints it.
    pub fn var(self, value: &str) -> String {
        format!("var({},{value})", self.property())
    }
}

/// The static values of an element's hooks, by hook.
pub type Hooks = BTreeMap<Hook, String>;

/// What a hook's static value is: what the element declares, and its colour for a colour hook.
struct Value {
    css: String,
    color: Option<HexColor>,
}

/// The hooks an effect needs.
fn needs(effect: &BuildEffect) -> BTreeSet<Hook> {
    let mut out = BTreeSet::new();
    match effect {
        BuildEffect::ColorChange(change) => {
            out.insert(match change.property {
                ColorProperty::Fill => Hook::Fill,
                ColorProperty::Line => Hook::Line,
                ColorProperty::Text => Hook::Text,
            });
        }
        BuildEffect::Behaviors(tracks) => out.extend(tracks.iter().filter_map(track_hook)),
        _ => {}
    }
    out
}

/// An opaque sRGB colour's `#rrggbb`.
fn hex(color: &ResolvedColor) -> Option<HexColor> {
    let rgb = color.rgb();
    if rgb.space != ColorSpace::Srgb || rgb.a.to_f64() < 1.0 {
        return None;
    }
    let byte =
        |c: slidock_ir::units::UnitRatio| (c.to_f64() * 255.0).round().clamp(0.0, 255.0) as u8;
    Some(HexColor::rgb(byte(rgb.r), byte(rgb.g), byte(rgb.b)))
}

/// What a shape has of a paint a hook exposes.
enum Paint {
    /// One value stands for all of it.
    Hooked(Value),
    /// The shape does not draw it (no fill, no line, no text): the effect turns it on ([`turns_on`]) or leaves it
    /// unseen.
    Absent(&'static str),
    /// No one value stands for it (a gradient, text of several colours): the step plays its fallback.
    Mixed(&'static str),
}

fn color_paint(color: &ResolvedColor) -> Paint {
    let (Some(hex), Ok(css)) = (hex(color), css_color(color)) else {
        return Paint::Mixed("its colour is translucent or outside sRGB");
    };
    Paint::Hooked(Value {
        css,
        color: Some(hex),
    })
}

/// The runs and fields of a shape's text, and its paragraph marks when `marks`: the props a hook reads.
fn text_props(shape: &Shape, marks: bool) -> Vec<&ResolvedRunProps> {
    let Some(text) = &shape.text else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for paragraph in &text.paragraphs {
        for inline in &paragraph.inlines {
            if let Inline::Run { props, .. } | Inline::Field { props, .. } = inline {
                out.push(props);
            }
        }
        if marks {
            out.extend(paragraph.end_props.as_ref());
        }
    }
    out
}

/// Whether a shape's text draws a glyph.
fn draws_text(shape: &Shape) -> bool {
    shape.text.as_ref().is_some_and(|text| {
        text.paragraphs.iter().any(|p| {
            p.inlines.iter().any(|inline| match inline {
                Inline::Run { text, .. } => !text.is_empty(),
                Inline::Field { .. } => true,
                _ => false,
            })
        })
    })
}

/// The one value `read` gives every props of the text, if there is one.
fn uniform<T: PartialEq>(
    props: &[&ResolvedRunProps],
    read: impl Fn(&ResolvedRunProps) -> T,
) -> Option<T> {
    let mut values = props.iter().map(|props| read(props));
    let first = values.next()?;
    values.all(|value| value == first).then_some(first)
}

/// The weight a family prints at: its binding's drawn weight.
pub(crate) fn drawn_weight(presentation: &Presentation, family: &str, weight: u32) -> u32 {
    match presentation.fonts.get(&font_key(family)) {
        Some(entry) => entry.binding.drawn_weight(weight),
        None => weight,
    }
}

/// What a shape has of a hook's paint. The font hooks read the paragraph marks too, which set an empty
/// paragraph's line.
fn shape_paint(shape: &Shape, hook: Hook, presentation: &Presentation) -> Paint {
    let keyword = |css: &str| {
        Paint::Hooked(Value {
            css: css.to_owned(),
            color: None,
        })
    };
    if matches!(
        hook,
        Hook::Text | Hook::Bold | Hook::Italic | Hook::Underline
    ) && !draws_text(shape)
    {
        return Paint::Absent("it has no text");
    }
    match hook {
        Hook::Fill => match &shape.fill {
            Some(ResolvedFill::Solid { color }) => color_paint(color),
            None | Some(ResolvedFill::None) => Paint::Absent("it has no fill"),
            Some(_) => Paint::Mixed("its fill is not one solid colour"),
        },
        Hook::Line => match strokes(shape.stroke.as_ref().and_then(Option::as_ref)) {
            Some(stroke) => match &stroke.fill {
                StrokePaint::Solid { color } => color_paint(color),
                StrokePaint::None => Paint::Absent("it has no line"),
            },
            None => Paint::Absent("it has no line"),
        },
        Hook::Text => {
            let props = text_props(shape, false);
            let Ok(printed) = props
                .iter()
                .map(|props| css_color(&props.color))
                .collect::<Result<Vec<_>>>()
            else {
                return Paint::Mixed("its colour does not print");
            };
            match (props.first(), printed.split_first()) {
                (Some(first), Some((css, rest))) if rest.iter().all(|other| other == css) => {
                    color_paint(&first.color)
                }
                _ => Paint::Mixed("its text has more than one colour"),
            }
        }
        Hook::Bold => {
            let props = text_props(shape, true);
            match uniform(&props, |p| {
                drawn_weight(presentation, &p.font.family, p.weight)
            }) {
                Some(400) => keyword("normal"),
                Some(700) => keyword("bold"),
                Some(_) => Paint::Mixed("its weight is neither normal nor bold"),
                None => Paint::Mixed("its text has more than one weight"),
            }
        }
        Hook::Italic => match uniform(&text_props(shape, true), |p| p.italic) {
            Some(italic) => keyword(if italic { "italic" } else { "normal" }),
            None => Paint::Mixed("its text is partly italic"),
        },
        Hook::Underline => {
            let props = text_props(shape, false);
            if props.iter().any(|p| p.strike != Decoration::None) {
                return Paint::Mixed("its text is struck through");
            }
            match uniform(&props, |p| p.underline) {
                Some(Decoration::None) => keyword("none"),
                Some(Decoration::Single) => keyword("underline"),
                Some(Decoration::Double) => Paint::Mixed("its underline is double"),
                None => Paint::Mixed("its text is partly underlined"),
            }
        }
    }
}

/// Why a step plays its fallback when its target does not draw a paint the source app turns on as `effect` colours
/// it: a fill always, a line only for a colour change (Change Line Color). `None` for a paint left off: the HSL
/// shifts leave a missing line off, and text has nothing to turn on.
fn turns_on(hook: Hook, effect: &BuildEffect) -> Option<&'static str> {
    match hook {
        Hook::Fill => Some("it has no fill, which the effect turns on"),
        Hook::Line if matches!(effect, BuildEffect::ColorChange(_)) => {
            Some("it has no line, which the effect turns on")
        }
        Hook::Line | Hook::Text | Hook::Bold | Hook::Italic | Hook::Underline => None,
    }
}

/// The element of a slide with `id`, group children and foreign fallbacks included.
fn find<'s>(slide: &'s Slide, id: &str) -> Option<&'s Element> {
    let mut stack: Vec<&Element> = slide.body.elements().iter().collect();
    while let Some(element) = stack.pop() {
        if element.id().as_str() == id {
            return Some(element);
        }
        match element {
            Element::Group(group) => stack.extend(group.children.iter()),
            Element::Foreign(foreign) => {
                if let Some(elements) = foreign
                    .fallback
                    .as_ref()
                    .and_then(|fallback| fallback.elements.as_ref())
                {
                    stack.extend(elements.iter());
                }
            }
            _ => {}
        }
    }
    None
}

/// A colour track with every shift and mix computed from `own`.
fn resolve(track: &Track<ColorValue>, own: HexColor) -> Track<ColorValue> {
    let mut track = track.clone();
    for frame in &mut track.frames {
        frame.value = ColorValue::Hex(frame.value.on(own));
    }
    track
}

/// A step as the deck plays it.
pub struct Played<'s> {
    pub step: Cow<'s, TimelineStep>,
    /// The hook its target lacks, and why: the step plays its fallback.
    pub unhooked: Option<(Hook, &'static str)>,
    /// The hooks its target declares, with their static values.
    pub hooks: Hooks,
}

/// The hook a track or a switch animates through, if any.
fn track_hook(track: &Keyframes) -> Option<Hook> {
    Some(match track.paint()? {
        TrackPaint::Text => Hook::Text,
        TrackPaint::Fill => Hook::Fill,
        TrackPaint::Line => Hook::Line,
        TrackPaint::Bold => Hook::Bold,
        TrackPaint::Italic => Hook::Italic,
        TrackPaint::Underline => Hook::Underline,
    })
}

/// A switch's colour with a shift computed from `own`.
fn resolve_set(set: &SetTrack, own: HexColor) -> SetTrack {
    let resolved = |value: ColorValue| ColorValue::Hex(value.on(own));
    let to = match set.to {
        SetValue::Text(value) => SetValue::Text(resolved(value)),
        SetValue::Fill(value) => SetValue::Fill(resolved(value)),
        SetValue::Line(value) => SetValue::Line(resolved(value)),
        other => other,
    };
    SetTrack { to, ..set.clone() }
}

/// `step` as the deck plays it on `slide`: its colour shifts computed from its target's paint and its tracks of a
/// paint the target does not draw (and the effect leaves off) left out, or its fallback when the target lacks a
/// paint the effect turns on, has no single value for a paint it animates, or no track is left.
pub fn played<'s>(
    step: &'s TimelineStep,
    slide: &Slide,
    presentation: &Presentation,
) -> Played<'s> {
    let unchanged = |step: &'s TimelineStep| Played {
        step: Cow::Borrowed(step),
        unhooked: None,
        hooks: Hooks::new(),
    };
    let demoted = |hook: Hook, reason: &'static str| {
        let mut demoted = step.clone();
        demoted.effect.native = None;
        Played {
            step: Cow::Owned(demoted),
            unhooked: Some((hook, reason)),
            hooks: Hooks::new(),
        }
    };
    let Some(native) = &step.effect.native else {
        return unchanged(step);
    };
    let wanted = needs(&native.effect);
    if wanted.is_empty() {
        return unchanged(step);
    }
    let target = find(slide, step.target.element.as_str());
    let mut values = BTreeMap::new();
    let mut absent = BTreeMap::new();
    for hook in wanted {
        let paint = match target {
            Some(Element::Shape(shape)) => shape_paint(shape, hook, presentation),
            Some(_) => Paint::Mixed("it is not a shape"),
            None => Paint::Mixed("it is not on the slide"),
        };
        match paint {
            Paint::Hooked(value) => {
                values.insert(hook, value);
            }
            Paint::Absent(reason) => match turns_on(hook, &native.effect) {
                Some(turned_on) => return demoted(hook, turned_on),
                None => {
                    absent.insert(hook, reason);
                }
            },
            Paint::Mixed(reason) => return demoted(hook, reason),
        }
    }
    let own = |hook: Hook| values.get(&hook).and_then(|value| value.color);
    let mut step_out = Cow::Borrowed(step);
    if let BuildEffect::Behaviors(tracks) = &native.effect {
        let resolved: Vec<Keyframes> = tracks
            .iter()
            .filter(|track| track_hook(track).is_none_or(|hook| !absent.contains_key(&hook)))
            .map(|track| match (track, track_hook(track).and_then(own)) {
                (Keyframes::Text(t), Some(c)) => Keyframes::Text(resolve(t, c)),
                (Keyframes::Fill(t), Some(c)) => Keyframes::Fill(resolve(t, c)),
                (Keyframes::Line(t), Some(c)) => Keyframes::Line(resolve(t, c)),
                (Keyframes::Set(set), Some(c)) => Keyframes::Set(resolve_set(set, c)),
                (other, _) => other.clone(),
            })
            .collect();
        if resolved.is_empty()
            && let Some((hook, reason)) = absent.into_iter().next()
        {
            return demoted(hook, reason);
        }
        if resolved != *tracks {
            let mut owned = step.clone();
            if let Some(native) = owned.effect.native.as_mut() {
                native.effect = BuildEffect::Behaviors(resolved);
            }
            step_out = Cow::Owned(owned);
        }
    }
    Played {
        step: step_out,
        unhooked: None,
        hooks: values
            .into_iter()
            .map(|(hook, value)| (hook, value.css))
            .collect(),
    }
}

/// The steps of a slide as the deck plays them.
pub fn played_steps<'s>(slide: &'s Slide, presentation: &Presentation) -> Vec<Played<'s>> {
    slide
        .timeline
        .iter()
        .map(|step| played(step, slide, presentation))
        .collect()
}

/// The hooks each element of a slide declares: the union of what the steps that play on it need.
pub fn slide_hooks(slide: &Slide, presentation: &Presentation) -> BTreeMap<String, Hooks> {
    let mut out: BTreeMap<String, Hooks> = BTreeMap::new();
    for played in played_steps(slide, presentation) {
        if played.hooks.is_empty() {
            continue;
        }
        out.entry(played.step.target.element.to_string())
            .or_default()
            .extend(played.hooks);
    }
    out
}

/// What HTML6011 says of an unhooked step: the paint and the reason.
pub fn unhooked_data(hook: Hook, reason: &'static str) -> (&'static str, &'static str) {
    (hook.name(), reason)
}
