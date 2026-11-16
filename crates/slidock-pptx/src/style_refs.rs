//! Shape style references (`p:style`, `p:bgRef`), resolved here so passes and backends see only the result. Each
//! reference indexes a list of the theme's format scheme and names a colour; the entry's `phClr` becomes that colour,
//! keeping its theme slot and composing the modifiers (the reference's, then the entry's), so re-theming still works.
//! The result is an ordinary fill, line and font that apply where the shape's own properties are silent.

use std::collections::BTreeSet;
use std::rc::Rc;

use indexmap::IndexMap;
use slidock_core_api::lowering::LowerError;
use slidock_ir::color::Color;
use slidock_ir::features::FeatureId;
use slidock_ir::json_value::JsonValue;
use slidock_ir::paint::Fill;
use slidock_ir::text::ThemeFontSlot;

use crate::color::{color_element, plain_color, read_color};
use crate::fill::{PaintCx, read_fill};
use crate::line::StyleLine;
use crate::report::Report;
use crate::shadow::EffectRef;
use crate::theme::FormatScheme;
use crate::xml::{El, Element, Ns};

/// A style's font reference: the theme font and the text colour.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FontStyle {
    pub slot: ThemeFontSlot,
    pub color: Option<Color>,
}

pub(crate) struct ShapeStyle {
    pub fill: Option<Fill>,
    pub line: Option<StyleLine>,
    /// The effect reference.
    pub effect: Option<EffectRef>,
    pub font: Option<FontStyle>,
    /// The references as written, kept in the element's `ext.pptx.styleRef`.
    pub raw: JsonValue,
}

pub(crate) fn ref_color(reference: Option<&Element>, report: &mut dyn Report) -> Option<Color> {
    color_element(reference).map(|element| plain_color(read_color(element, report)))
}

/// The fill style a fill reference names: 1–999 the fill styles, 1001 and up the background fill styles.
pub(crate) fn fill_style(idx: i64, formats: &FormatScheme) -> Option<Rc<Element>> {
    let at = |list: &[Rc<Element>], index: i64| {
        usize::try_from(index)
            .ok()
            .and_then(|index| list.get(index))
            .cloned()
    };
    if idx >= 1001 {
        at(&formats.backgrounds, idx - 1001)
    } else {
        at(&formats.fills, idx - 1)
    }
}

/// The line style `idx` (from 1) names.
pub(crate) fn line_style(idx: i64, formats: &FormatScheme) -> Option<Rc<Element>> {
    usize::try_from(idx - 1)
        .ok()
        .filter(|_| idx > 0)
        .and_then(|index| formats.lines.get(index))
        .cloned()
}

/// The effect style `idx` (from 1) names.
pub(crate) fn effect_style(idx: i64, formats: &FormatScheme) -> Option<Rc<Element>> {
    usize::try_from(idx - 1)
        .ok()
        .filter(|_| idx > 0)
        .and_then(|index| formats.effects.get(index))
        .cloned()
}

/// A fill reference: 0 is no fill, else its fill style in the reference's colour.
pub(crate) fn referenced_fill(
    idx: i64,
    color: Option<&Color>,
    paint: &mut PaintCx<'_, '_>,
) -> Result<Option<Fill>, LowerError> {
    if idx <= 0 {
        return Ok(Some(Fill::None));
    }
    match fill_style(idx, paint.ctx.formats) {
        Some(entry) => read_fill(&entry, paint, color),
        None => Ok(None),
    }
}

/// The effect reference of a shape style (`p:style/a:effectRef`).
pub(crate) fn read_effect_ref(
    style: Option<&Element>,
    formats: &FormatScheme,
    report: &mut dyn Report,
) -> Option<EffectRef> {
    let effect_ref = style.child(Ns::A, "effectRef")?;
    let idx = effect_ref.int_attr("idx").unwrap_or(0);
    Some(EffectRef {
        idx,
        entry: effect_style(idx, formats),
        color: ref_color(Some(effect_ref), report),
    })
}

/// The features of effects other than shadows, which [`crate::shadow`] reads.
fn effect_feature(local: &str) -> Option<FeatureId> {
    match local {
        "glow" => Some(FeatureId::EffectGlow),
        "reflection" => Some(FeatureId::EffectReflection),
        "softEdge" => Some(FeatureId::EffectSoftEdge),
        _ => None,
    }
}

/// Whether 3D properties change what is drawn: a bevel, an extrusion, a contour, or a camera off the front.
fn is_three_d(element: &Element) -> bool {
    if &*element.local == "sp3d" {
        let depth = |name: &str| element.int_attr(name).unwrap_or(0) > 0;
        return element.child(Ns::A, "bevelT").is_some()
            || element.child(Ns::A, "bevelB").is_some()
            || depth("extrusionH")
            || depth("contourW");
    }
    let camera = element.child(Ns::A, "camera");
    camera.attr("prst").unwrap_or("orthographicFront") != "orthographicFront"
        || camera.child(Ns::A, "rot").is_some()
}

/// The features of effect lists (`a:effectLst`, `a:effectDag`) and 3D properties, shadows aside, all of them
/// unmodelled, sorted by name.
pub(crate) fn effect_features<'a>(
    effects: impl IntoIterator<Item = &'a Element>,
) -> Vec<FeatureId> {
    let mut out = BTreeSet::new();
    for effect in effects {
        if matches!(&*effect.local, "scene3d" | "sp3d") {
            if is_three_d(effect) {
                out.insert(FeatureId::Effect3d);
            }
            continue;
        }
        out.extend(effect.kids().filter_map(|item| effect_feature(&item.local)));
    }
    out.into_iter().collect()
}

pub(crate) fn read_style(
    style: Option<&Element>,
    paint: &mut PaintCx<'_, '_>,
) -> Result<Option<ShapeStyle>, LowerError> {
    let Some(style) = style else { return Ok(None) };
    let fill_ref = style.child(Ns::A, "fillRef");
    let ln_ref = style.child(Ns::A, "lnRef");
    let effect_ref = style.child(Ns::A, "effectRef");
    let font_ref = style.child(Ns::A, "fontRef");
    let index = |reference: Option<&Rc<Element>>| reference.int_attr("idx").unwrap_or(0);

    let fill = match fill_ref {
        Some(fill_ref) => {
            let color = ref_color(Some(fill_ref), paint);
            referenced_fill(index(Some(fill_ref)), color.as_ref(), paint)?
        }
        None => None,
    };
    let line_idx = index(ln_ref);
    let line = match ln_ref {
        Some(ln_ref) => Some(StyleLine {
            element: line_style(line_idx, paint.ctx.formats),
            color: ref_color(Some(ln_ref), paint),
        }),
        None => None,
    };
    let effect = read_effect_ref(Some(style), paint.ctx.formats, paint);
    let font_slot = font_ref.attr("idx");
    let font_color = ref_color(font_ref.map(|rc| &**rc), paint);
    let font = match font_slot {
        Some("major") => Some(ThemeFontSlot::Major),
        Some("minor") => Some(ThemeFontSlot::Minor),
        _ => None,
    }
    .map(|slot| FontStyle {
        slot,
        color: font_color,
    });
    let mut raw = IndexMap::new();
    raw.insert(
        "fillRef".to_owned(),
        JsonValue::number(index(fill_ref) as f64),
    );
    raw.insert("lnRef".to_owned(), JsonValue::number(line_idx as f64));
    raw.insert(
        "effectRef".to_owned(),
        JsonValue::number(index(effect_ref) as f64),
    );
    raw.insert(
        "fontRef".to_owned(),
        JsonValue::string(font_slot.unwrap_or("none")),
    );
    Ok(Some(ShapeStyle {
        fill,
        line,
        effect,
        font,
        raw: JsonValue::Object(raw),
    }))
}

/// `p:bgRef`: a background from the theme's fill styles, coloured by the reference.
pub(crate) fn background_ref(
    bg_ref: &Element,
    paint: &mut PaintCx<'_, '_>,
) -> Result<Option<Fill>, LowerError> {
    let color = ref_color(Some(bg_ref), paint);
    referenced_fill(bg_ref.int_attr("idx").unwrap_or(0), color.as_ref(), paint)
}
