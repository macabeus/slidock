//! Effects (ECMA-376 Part 1 §20.1.8) and the shadow among them. An element's effects come from
//! one place, by PowerPoint's precedence: its own `a:effectLst` (an empty one means none), else its own `a:effectDag`,
//! else the theme effect style its `p:style/a:effectRef` names (`phClr` in the reference's colour; index 0 means none).
//! With none of these it states nothing and takes its placeholder's. 3D properties (`a:scene3d`, `a:sp3d`) are separate
//! from both, so the element's own and its style's are both read.
//!
//! An outer shadow (`a:outerShdw`, §20.1.8.49) at 100 % scale without skew is drawn: an offset of `dist` along `dir`
//! (clockwise from +x, y down), σ = 0.43 · `blurRad` (LibreOffice's blur fits a Gaussian of 0.42–0.43 times its blur
//! length; PowerPoint's kernel is unmeasured), its colour with `alpha` folded in, and `rotWithShape` (default true).
//! `algn` only places a scaled or skewed shadow, so it is not read. Inner, preset, scaled and skewed shadows, shadows
//! in a DAG, and those of tables are reported (PX2111); glow, reflection, soft edges and 3D stay PX2101's.

use std::rc::Rc;

use slidock_ir::color::Color;
use slidock_ir::diagnostics::{DiagnosticCode, DiagnosticSite, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::ids::NodeId;
use slidock_ir::shadow::{MAX_SIGMA, SourceShadow};
use slidock_ir::units::Size;

use crate::color::{color_element, fill_placeholder, read_color};
use crate::context::Lowering;
use crate::report::Report;
use crate::style_refs::effect_features;
use crate::units::{EMU_PER_PT, pt};
use crate::xml::{El, Element, Ns};

/// The Gaussian σ per unit of `blurRad`.
pub(crate) const SIGMA_PER_BLUR: f64 = 0.43;

const SHADOWS: [&str; 3] = ["outerShdw", "innerShdw", "prstShdw"];
const UNSCALED: i64 = 100_000;
/// The IR's bound on a length in points; a farther offset (`dist` may reach 2.1 · 10⁹ pt) is drawn at it.
const MAX_OFFSET: f64 = 100_000.0;

/// An `a:effectRef`: the theme's effect style it names (none for index 0 or an index past the list) and its colour,
/// which stands for `phClr`.
#[derive(Clone, Debug)]
pub(crate) struct EffectRef {
    pub idx: i64,
    pub entry: Option<Rc<Element>>,
    pub color: Option<Color>,
}

/// What an effect list says about shadows: the shadow it draws, and the shadows it holds that are not drawn.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Stated {
    pub shadow: Option<SourceShadow>,
    pub not_drawn: Vec<String>,
}

impl Stated {
    /// Whether the list holds a shadow that is drawn or reported (a shadow that paints nothing is neither).
    pub fn has_shadow(&self) -> bool {
        self.shadow.is_some() || !self.not_drawn.is_empty()
    }
}

/// `(cos θ, sin θ)` of a clockwise angle in degrees, exact on quarter turns.
fn cos_sin(degrees: f64) -> (f64, f64) {
    let quarter = degrees / 90.0;
    if quarter == quarter.trunc() {
        return match (quarter as i64).rem_euclid(4) {
            0 => (1.0, 0.0),
            1 => (0.0, 1.0),
            2 => (-1.0, 0.0),
            _ => (0.0, -1.0),
        };
    }
    let radians = degrees * std::f64::consts::PI / 180.0;
    (slidock_math::cos(radians), slidock_math::sin(radians))
}

/// A non-negative EMU length attribute in points (ST_PositiveCoordinate; a negative value reads as 0).
fn length(element: &Element, name: &str) -> f64 {
    element.int_attr(name).unwrap_or(0).max(0) as f64 / EMU_PER_PT
}

/// An `a:outerShdw` as an IR shadow; `Ok(None)` for one that paints nothing; `Err` with the reason it is not drawn.
pub(crate) fn outer_shadow(
    element: &Element,
    ph_color: Option<&Color>,
    report: &mut dyn Report,
) -> Result<Option<SourceShadow>, String> {
    let scaled = |name: &str| element.int_attr(name).unwrap_or(UNSCALED) != UNSCALED;
    let skewed = |name: &str| element.int_attr(name).unwrap_or(0) != 0;
    if scaled("sx") || scaled("sy") || skewed("kx") || skewed("ky") {
        return Err("scaled or skewed shadow".to_owned());
    }
    let Some(color) = color_element(element) else {
        return Err("shadow without a colour".to_owned());
    };
    let color = fill_placeholder(read_color(color, report), ph_color);
    if let Color::Rgb(rgb) = &color
        && rgb.a.to_f64() == 0.0
    {
        return Ok(None);
    }
    let sigma = SIGMA_PER_BLUR * length(element, "blurRad");
    if sigma > MAX_SIGMA {
        return Err("blur wider than the IR draws".to_owned());
    }
    let dist = length(element, "dist");
    let dir = crate::units::angle(element.int_attr("dir").unwrap_or(0)).to_f64();
    let (cos, sin) = cos_sin(dir);
    Ok(Some(SourceShadow {
        dx: pt((dist * cos).clamp(-MAX_OFFSET, MAX_OFFSET)),
        dy: pt((dist * sin).clamp(-MAX_OFFSET, MAX_OFFSET)),
        sigma: Size::from_steps(pt(sigma).steps()),
        color,
        rotate_with_shape: element.bool_attr("rotWithShape").unwrap_or(true),
        overflow_casts: false,
    }))
}

/// Whether an element or one of its descendants is a shadow.
fn holds_shadow(element: &Element) -> bool {
    element
        .kids()
        .any(|kid| (kid.ns == Ns::A && SHADOWS.contains(&&*kid.local)) || holds_shadow(kid))
}

/// The shadows an `a:effectLst` or `a:effectDag` states.
pub(crate) fn stated(
    effects: &Element,
    ph_color: Option<&Color>,
    report: &mut dyn Report,
) -> Stated {
    let mut out = Stated::default();
    if &*effects.local == "effectDag" {
        if holds_shadow(effects) {
            out.not_drawn.push("effect DAG".to_owned());
        }
        return out;
    }
    for kid in effects.kids().filter(|kid| kid.ns == Ns::A) {
        match &*kid.local {
            "outerShdw" if out.shadow.is_none() => match outer_shadow(kid, ph_color, report) {
                Ok(shadow) => out.shadow = shadow,
                Err(reason) => out.not_drawn.push(reason),
            },
            "innerShdw" => out.not_drawn.push("inner shadow".to_owned()),
            "prstShdw" => out.not_drawn.push(format!(
                "preset shadow {}",
                kid.attr("prst").unwrap_or("(none)")
            )),
            _ => {}
        }
    }
    out
}

/// The effect list or DAG among an element's children (`a:spPr`, `a:rPr`, an effect style); a list first.
pub(crate) fn effect_container<'a>(parent: impl El<'a>) -> Option<&'a Element> {
    parent
        .child(Ns::A, "effectLst")
        .or_else(|| parent.child(Ns::A, "effectDag"))
}

/// What a run's properties (`a:rPr`, `a:defRPr`, `a:endParaRPr`) say about its shadow: absent without an effect list,
/// `null` for a list that draws none.
pub(crate) fn run_shadow(r_pr: &Element, report: &mut dyn Report) -> Option<Option<SourceShadow>> {
    let effects = effect_container(r_pr)?;
    let stated = stated(effects, None, report);
    for reason in stated.not_drawn {
        report.report(
            codes::PX2111,
            DiagnosticSite::new()
                .feature(FeatureId::TextShadow)
                .data("reason", reason),
        );
    }
    Some(stated.shadow)
}

/// A report that keeps nothing, for reading a colour a second time.
struct Quiet;

impl Report for Quiet {
    fn report(&mut self, _code: DiagnosticCode, _site: DiagnosticSite) {}

    fn dropped(&mut self, _modifier: &str) {}
}

/// Whether a run's own properties hold a shadow that is drawn or reported.
pub(crate) fn run_has_shadow(r_pr: &Element) -> bool {
    effect_container(r_pr).is_some_and(|effects| stated(effects, None, &mut Quiet).has_shadow())
}

/// The 3D properties among an element's children.
fn three_d(parent: Option<&Element>) -> impl Iterator<Item = &Element> {
    parent
        .kids()
        .into_iter()
        .filter(|kid| kid.ns == Ns::A && matches!(&*kid.local, "scene3d" | "sp3d"))
}

/// Reports each effect other than a shadow with PX2101, once per kind.
fn drop_others<'s>(
    effects: impl IntoIterator<Item = &'s Element>,
    node: &NodeId,
    ctx: &mut Lowering<'_>,
) {
    for feature in effect_features(effects) {
        ctx.seen(feature);
        let effect = feature
            .as_str()
            .strip_prefix("effect.")
            .unwrap_or(feature.as_str());
        ctx.report_on(
            node,
            codes::PX2101,
            DiagnosticSite::new()
                .feature(feature)
                .data("effect", effect),
        );
    }
}

/// The shadow an element (`p:sp`, `p:cxnSp`, `p:pic`, `p:grpSp`) states, by the precedence of the module doc: absent
/// when it states no effects, `null` when its effects draw no shadow. Shadows not drawn are reported (PX2111), and the
/// other effects are dropped (PX2101).
pub(crate) fn element_shadow(
    properties: Option<&Element>,
    style: Option<&EffectRef>,
    node: &NodeId,
    ctx: &mut Lowering<'_>,
) -> Option<Option<SourceShadow>> {
    let entry = style.and_then(|style| style.entry.as_deref());
    let (effects, ph_color) = match effect_container(properties) {
        Some(own) => (Some(own), None),
        None => (
            effect_container(entry),
            style.and_then(|style| style.color.as_ref()),
        ),
    };
    drop_others(
        effects
            .into_iter()
            .chain(three_d(properties))
            .chain(three_d(entry)),
        node,
        ctx,
    );
    let Some(effects) = effects else {
        // A reference to no effect style states that there is none; without a reference nothing is stated.
        return style.filter(|style| style.idx <= 0).map(|_| None);
    };
    let stated = stated(effects, ph_color, ctx);
    if stated.has_shadow() {
        ctx.seen(FeatureId::EffectShadow);
    }
    for reason in stated.not_drawn {
        ctx.report_on(
            node,
            codes::PX2111,
            DiagnosticSite::new().data("reason", reason),
        );
    }
    Some(stated.shadow)
}

/// A table's effects (`a:tblPr`'s own list, else those of its style's `a:tblBg`: its `a:effect` or the effect style
/// its `a:effectRef` names): a shadow is reported, as tables carry none in the IR (PowerPoint shadows the cell fills
/// and borders, which needs a background layer), and the other effects are dropped.
pub(crate) fn table_effects(
    tbl_pr: Option<&Element>,
    styled: Option<&Element>,
    node: &NodeId,
    ctx: &mut Lowering<'_>,
) {
    let effects = effect_container(tbl_pr).or_else(|| effect_container(styled));
    drop_others(effects.into_iter().chain(three_d(styled)), node, ctx);
    if effects.is_some_and(holds_shadow) {
        ctx.seen(FeatureId::EffectShadow);
        ctx.report_on(
            node,
            codes::PX2111,
            DiagnosticSite::new().data("reason", "table shadow"),
        );
    }
}

/// A background's effects (`p:bgPr`): a shadow is reported, as backgrounds carry none in the IR.
pub(crate) fn background_effects(bg_pr: Option<&Element>, ctx: &mut Lowering<'_>) {
    if effect_container(bg_pr).is_some_and(holds_shadow) {
        ctx.seen(FeatureId::EffectShadow);
        ctx.report(
            codes::PX2111,
            DiagnosticSite::new().data("reason", "background shadow"),
        );
    }
}
