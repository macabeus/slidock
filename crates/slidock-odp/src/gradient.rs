//! ODF gradients as LibreOffice 25.2 reads and draws them, written as the IR's `odf` source gradient, which
//! lower-geometry places over the page-aligned bounds of the drawn outline.
//! - Defaults (GradientStyle.cxx): linear, black to black, intensities 100 %, centre 0 % 0 %, angle 0, border 0.
//!   Percentages are whole and angles whole tenths of a degree, as LibreOffice keeps them.
//! - A unitless `draw:angle` is in tenths of a degree in ODF before 1.2, and in 1.2 when OpenOffice.org, Apache
//!   OpenOffice or a LibreOffice before 7 wrote it (tdf#89475); in degrees otherwise.
//! - `loext:gradient-stop`s replace the start and end colours. Stops are sorted and cut to [0, 1]
//!   (`BColorStops::sortAndCorrect`), then blended toward black by the intensities, each by its offset.
//! - `draw:gradient-step-count` draws each pair of stops in at most as many bands as LibreOffice's automatic count
//!   for its colours (`calculateNumberOfSteps`); the largest over the pairs is kept.
//! - A transparency gradient (`draw:opacity-name`: `draw:opacity`, `loext:opacity-stop`) is ignored under a uniform
//!   `draw:opacity` other than 100 %. Otherwise it folds into the stop alphas: over a solid colour as a gradient of its
//!   own, over a colour gradient when both draw the same contours. LibreOffice's PDF draws a radial transparency's
//!   border twice (on its contours and its stops) and a rectangular one's on its stops only. What does not fold is
//!   reported (OD9103) and drawn without the transparency gradient.

use slidock_ir::color::{Color, RgbColor};
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::features::{FeatureId, fill_feature};
use slidock_ir::paint::{Fill, GradientStop, MAX_STOPS, OdfGradient, OdfStyle, Point};
use slidock_ir::units::Angle;
use slidock_math::{Grid, quantise};

use crate::graphic::Paint;
use crate::names::qn;
use crate::odf::{Element, Ns};
use crate::units::{angle_deg, hex_color, percent, plain_number, ratio_of, rgb};

type Rgb = [f64; 3];

/// A stop: its offset and its colour (an opacity gradient's grey, its transparence).
type Stop = (f64, Rgb);

/// A gradient definition (`draw:gradient`, `draw:opacity`) as LibreOffice imports it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Definition {
    pub style: OdfStyle,
    /// Tenths of a degree, 0 to 3599.
    pub tenths: i64,
    pub border: f64,
    pub center: (f64, f64),
    pub stops: Vec<Stop>,
}

/// The largest border LibreOffice keeps a ramp under: the last whole percent below 100.
const MAX_BORDER: f64 = 0.99;

fn whole_percent(value: Option<&str>) -> Option<f64> {
    percent(value).map(|p| (p * 100.0).round())
}

fn style_of(value: Option<&str>) -> Result<OdfStyle, &str> {
    Ok(match value {
        None | Some("linear") => OdfStyle::Linear,
        Some("axial") => OdfStyle::Axial,
        Some("radial") => OdfStyle::Radial,
        Some("ellipsoid") => OdfStyle::Ellipsoid,
        Some("square") => OdfStyle::Square,
        Some("rectangular") => OdfStyle::Rectangular,
        Some(other) => return Err(other),
    })
}

/// Whether a unitless `draw:angle` is in tenths of a degree (`XMLGradientStyleImport`, tdf#89475): ODF before 1.2
/// (`office:version` compared as text, absent before every version), or 1.2 written by OpenOffice.org, Apache
/// OpenOffice or a LibreOffice before 7 (`meta:generator`).
pub(crate) fn unitless_angle_in_tenths(version: Option<&str>, generator: Option<&str>) -> bool {
    match version.unwrap_or("").cmp("1.2") {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => generator.is_some_and(older_than_libreoffice_7),
    }
}

/// `SvXMLMetaDocumentContext::setBuildId` and `getGeneratorVersion`: a LibreOffice names its version after the first
/// slash when `LibreOffice_project/` follows; otherwise an OpenOffice.org build id (`…/320m12$Build-9483`) or an old
/// product name makes it OpenOffice.org, which is older; anything else is unknown, which is not.
fn older_than_libreoffice_7(generator: &str) -> bool {
    if generator.contains("LibreOffice_project/") {
        let version: String = generator
            .split_once('/')
            .map(|(_, rest)| {
                rest.chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.')
                    .collect()
            })
            .unwrap_or_default();
        if let Some(major) = version.chars().next() {
            return ('3'..='6').contains(&major);
        }
    }
    let build = generator.split_once(' ').and_then(|(_, rest)| {
        let (_, after) = rest.split_once('/')?;
        let (upd, tail) = after.split_once('m')?;
        (!upd.is_empty() && tail.contains("$Build-")).then_some(())
    });
    build.is_some()
        || [
            "StarOffice 7",
            "StarSuite 7",
            "StarOffice 6",
            "StarSuite 6",
            "OpenOffice.org 1",
            "NeoOffice/2",
        ]
        .iter()
        .any(|name| generator.starts_with(name))
}

/// `draw:angle` in whole tenths of a degree within a turn; 0 when it does not parse.
fn tenths(value: Option<&str>, unitless_tenths: bool) -> i64 {
    let Some(degrees) = angle_deg(value, if unitless_tenths { 0.1 } else { 1.0 }) else {
        return 0;
    };
    let tenths = (degrees * 10.0).round();
    if !tenths.is_finite() || tenths.abs() > 1e12 {
        return 0;
    }
    (tenths as i64).rem_euclid(3600)
}

fn channels(color: &RgbColor) -> Rgb {
    [color.r.to_f64(), color.g.to_f64(), color.b.to_f64()]
}

fn lerp(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let ([a0, a1, a2], [b0, b1, b2]) = (a, b);
    [a0 + (b0 - a0) * t, a1 + (b1 - a1) * t, a2 + (b2 - a2) * t]
}

/// `BColorStops::sortAndCorrect`: sorted by offset (equal offsets keep their order); a stop before 0 or after 1 is
/// dropped, or moved onto 0 or 1 at the colour of the ramp there when its neighbour lies inside; repeated stops kept
/// once.
fn sort_and_correct(mut stops: Vec<Stop>) -> Vec<Stop> {
    if stops.len() <= 1 {
        return stops
            .into_iter()
            .map(|(offset, color)| (offset.clamp(0.0, 1.0), color))
            .collect();
    }
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<Stop> = Vec::with_capacity(stops.len());
    let mut i = 0;
    while i < stops.len() {
        let Some(&(mut offset, mut color)) = stops.get(i) else {
            break;
        };
        let next = stops.get(i + 1).copied();
        if offset < 0.0
            && let Some((next_offset, next_color)) = next
            && next_offset > 0.0
        {
            color = lerp(color, next_color, -offset / (next_offset - offset));
            offset = 0.0;
        }
        if offset < 0.0 {
            i += 1;
            continue;
        }
        if offset < 1.0
            && let Some((next_offset, next_color)) = next
            && next_offset > 1.0
            && let Some(slot) = stops.get_mut(i + 1)
        {
            *slot = (
                1.0,
                lerp(color, next_color, (1.0 - offset) / (next_offset - offset)),
            );
        }
        if offset > 1.0 {
            break;
        }
        if out.last() != Some(&(offset, color)) {
            out.push((offset, color));
        }
        i += 1;
    }
    out
}

/// `BColorStops::blendToIntensity` toward black: each colour scaled by the intensity at its offset.
fn blend_to_intensity(stops: &mut [Stop], start: f64, end: f64) {
    let (start, end) = (start.clamp(0.0, 1.0), end.clamp(0.0, 1.0));
    if start == 1.0 && end == 1.0 {
        return;
    }
    for (offset, color) in stops.iter_mut() {
        let intensity = start * (1.0 - *offset) + end * *offset;
        *color = color.map(|c| c * intensity);
    }
}

/// An opacity percentage as LibreOffice's grey transparence: ⌊(100 − opacity)·255/100⌋/255.
fn transparence_grey(opacity: f64) -> Rgb {
    let level = ((100.0 - opacity.clamp(0.0, 100.0)) * 255.0 / 100.0).floor() / 255.0;
    [level; 3]
}

/// A colour (`draw:gradient`) or opacity (`draw:opacity`) definition, and the `draw:style` it did not know.
pub(crate) fn read_definition(
    element: &Element,
    unitless_tenths: bool,
    opacity: bool,
) -> (Definition, Option<String>) {
    let (style, unknown) = match style_of(element.attr(qn!("draw:style"))) {
        Ok(style) => (style, None),
        Err(other) => (OdfStyle::Linear, Some(other.to_owned())),
    };
    let percent_or = |name, fallback: f64| whole_percent(element.attr(name)).unwrap_or(fallback);
    let (start, end, listed) = if opacity {
        let grey = |name| transparence_grey(percent_or(name, 100.0));
        let stops: Vec<Stop> = element
            .children_of(Ns::Loext, "opacity-stop")
            .filter_map(|stop| {
                let offset = match stop.attr(qn!("svg:offset")) {
                    Some(value) => plain_number(Some(value))?,
                    None => -1.0,
                };
                let transparence = match stop.attr(qn!("svg:stop-opacity")) {
                    Some(value) => (1.0 - plain_number(Some(value))?).clamp(0.0, 1.0),
                    None => 0.0,
                };
                Some((offset, [transparence; 3]))
            })
            .collect();
        (grey(qn!("draw:start")), grey(qn!("draw:end")), stops)
    } else {
        let color = |name| {
            hex_color(element.attr(name))
                .map(|c| channels(&c))
                .unwrap_or([0.0; 3])
        };
        let stops: Vec<Stop> = element
            .children_of(Ns::Loext, "gradient-stop")
            .filter_map(|stop| {
                let offset = match stop.attr(qn!("svg:offset")) {
                    Some(value) => plain_number(Some(value))?,
                    None => -1.0,
                };
                if stop.attr(qn!("loext:color-type")) != Some("rgb") {
                    return None;
                }
                let color = hex_color(stop.attr(qn!("loext:color-value")))?;
                Some((offset, channels(&color)))
            })
            .collect();
        (
            color(qn!("draw:start-color")),
            color(qn!("draw:end-color")),
            stops,
        )
    };
    let mut stops = sort_and_correct(listed);
    if stops.is_empty() {
        stops = vec![(0.0, start), (1.0, end)];
    }
    blend_to_intensity(
        &mut stops,
        percent_or(qn!("draw:start-intensity"), 100.0) / 100.0,
        percent_or(qn!("draw:end-intensity"), 100.0) / 100.0,
    );
    let definition = Definition {
        style,
        tenths: tenths(element.attr(qn!("draw:angle")), unitless_tenths),
        border: (percent_or(qn!("draw:border"), 0.0) / 100.0).clamp(0.0, MAX_BORDER),
        center: (
            percent_or(qn!("draw:cx"), 0.0) / 100.0,
            percent_or(qn!("draw:cy"), 0.0) / 100.0,
        ),
        stops,
    };
    (definition, unknown)
}

/// The bands per pair of stops for `draw:gradient-step-count`: none when smooth, else at most LibreOffice's automatic
/// count, ⌊maxChannelΔ·127.5 + 0.5⌋, over the pairs that span a range.
pub(crate) fn band_count(requested: u32, stops: &[Stop]) -> Option<u16> {
    if requested == 0 {
        return None;
    }
    let most = stops
        .windows(2)
        .filter_map(|pair| match pair {
            [a, b] if a.0 != b.0 => Some(
                a.1.iter()
                    .zip(b.1)
                    .fold(0.0f64, |m, (x, y)| m.max((x - y).abs())),
            ),
            _ => None,
        })
        .fold(0.0f64, f64::max);
    let automatic = (most * 127.5 + 0.5).floor().max(0.0) as u32;
    u16::try_from(requested.min(automatic).max(1)).ok()
}

/// A definition's contours and its stops on them, for comparing two definitions: the border folded into the stops
/// where the contours scale with it (every style but ellipsoid and rectangular), and what a style ignores cleared
/// (the centre of linear and axial ones, the angle of a radial one).
fn ramp(definition: &Definition) -> Definition {
    let b = definition.border;
    let fold = |pos: &dyn Fn(f64) -> f64| -> Vec<Stop> {
        definition
            .stops
            .iter()
            .map(|(offset, color)| (pos(*offset), *color))
            .collect()
    };
    let mut out = definition.clone();
    match definition.style {
        OdfStyle::Linear | OdfStyle::Radial | OdfStyle::Square => {
            out.stops = fold(&|o| b + o * (1.0 - b));
            out.border = 0.0;
        }
        OdfStyle::Axial => {
            out.stops = fold(&|o| o * (1.0 - b));
            out.border = 0.0;
        }
        OdfStyle::Ellipsoid | OdfStyle::Rectangular => {}
    }
    if matches!(definition.style, OdfStyle::Linear | OdfStyle::Axial) {
        out.center = (0.0, 0.0);
    }
    if definition.style == OdfStyle::Radial {
        out.tenths = 0;
    }
    out
}

/// A transparency gradient as LibreOffice's PDF draws it: a radial one's border also on its stops, a rectangular
/// one's on its stops instead of its contours. `Err` with the reason for an ellipsoid or square one with a border,
/// which is unmeasured.
fn as_drawn(transparency: &Definition) -> Result<Definition, String> {
    let b = transparency.border;
    let mut out = transparency.clone();
    if b > 0.0 {
        let moved = |pos: &dyn Fn(f64) -> f64| -> Vec<Stop> {
            transparency
                .stops
                .iter()
                .map(|(offset, color)| (pos(*offset), *color))
                .collect()
        };
        match transparency.style {
            OdfStyle::Radial => out.stops = moved(&|o| o * (1.0 - b)),
            OdfStyle::Rectangular => {
                out.stops = moved(&|o| b + o * (1.0 - b));
                out.border = 0.0;
            }
            OdfStyle::Ellipsoid => {
                return Err("an ellipsoid transparency gradient with a border".to_owned());
            }
            OdfStyle::Square => {
                return Err("a square transparency gradient with a border".to_owned());
            }
            OdfStyle::Linear | OdfStyle::Axial => {}
        }
    }
    Ok(out)
}

/// The value of a stop list at `at`, approached from below or from above (`after`); padded past both ends.
fn value_at(stops: &[Stop], at: f64, after: bool) -> Rgb {
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
        return [0.0; 3];
    };
    if (!after && at <= first.0) || (after && at < first.0) {
        return first.1;
    }
    if (after && at >= last.0) || (!after && at > last.0) {
        return last.1;
    }
    let upper = if after {
        stops.iter().position(|s| s.0 > at)
    } else {
        stops.iter().position(|s| s.0 >= at)
    };
    match upper.and_then(|j| Some((stops.get(j.checked_sub(1)?)?, stops.get(j)?))) {
        Some((a, b)) if b.0 > a.0 => lerp(a.1, b.1, (at - a.0) / (b.0 - a.0)),
        Some((_, b)) => b.1,
        None => last.1,
    }
}

/// Two stop lists over one ramp as one, colours from the first and transparences from the second, under the
/// uniform opacity `alpha`: a stop at every offset of either, twice where either changes there.
fn merge(colors: &[Stop], greys: &[Stop], alpha: f64) -> Vec<(f64, Rgb, f64)> {
    let mut offsets: Vec<f64> = colors.iter().chain(greys).map(|s| s.0).collect();
    offsets.sort_by(f64::total_cmp);
    offsets.dedup();
    let mut out = Vec::new();
    for at in offsets {
        let before = (value_at(colors, at, false), value_at(greys, at, false));
        let after = (value_at(colors, at, true), value_at(greys, at, true));
        out.push((at, before.0, alpha * (1.0 - before.1[0])));
        if after != before {
            out.push((at, after.0, alpha * (1.0 - after.1[0])));
        }
    }
    out
}

/// What reading a gradient fill reports.
pub(crate) struct Note {
    pub feature: FeatureId,
    pub reason: String,
}

fn feature_of(style: OdfStyle) -> FeatureId {
    match style {
        OdfStyle::Linear | OdfStyle::Axial => FeatureId::FillLinear,
        OdfStyle::Radial => FeatureId::FillRadial,
        OdfStyle::Ellipsoid | OdfStyle::Square | OdfStyle::Rectangular => FeatureId::FillPath,
    }
}

/// At most [`MAX_STOPS`] stops in the IR, evenly picked from more, which is reported.
fn ir_stops(
    stops: &[(f64, Rgb, f64)],
    feature: FeatureId,
    notes: &mut Vec<Note>,
) -> Vec<GradientStop<Color>> {
    let kept: Vec<&(f64, Rgb, f64)> = if stops.len() > MAX_STOPS {
        notes.push(Note {
            feature,
            reason: format!("its {} stops are drawn as {MAX_STOPS}", stops.len()),
        });
        (0..MAX_STOPS)
            .filter_map(|i| stops.get(i * (stops.len() - 1) / (MAX_STOPS - 1)))
            .collect()
    } else {
        stops.iter().collect()
    };
    kept.into_iter()
        .map(|(offset, color, alpha)| GradientStop {
            pos: crate::units::unit_ratio(crate::units::ratio(*offset)),
            color: Color::Rgb(rgb(color[0], color[1], color[2], *alpha)),
        })
        .collect()
}

/// An ellipsoid's bands when a transparency gradient folded into its stops: LibreOffice draws the transparency with
/// its own automatic count, from its greys, so the alpha counts as a channel (`calculateNumberOfSteps`). Other styles
/// stay smooth.
fn folded_bands(style: OdfStyle, stops: &[(f64, Rgb, f64)]) -> Option<u16> {
    if style != OdfStyle::Ellipsoid {
        return None;
    }
    let most = stops
        .windows(2)
        .filter_map(|pair| match pair {
            [a, b] if a.0 != b.0 => Some(
                a.1.iter()
                    .zip(b.1)
                    .map(|(x, y)| (x - y).abs())
                    .fold((a.2 - b.2).abs(), f64::max),
            ),
            _ => None,
        })
        .fold(0.0f64, f64::max);
    // On the ratio grid first, so that a difference of two alphas floors as LibreOffice's grey difference does.
    let count = quantise(most * 127.5 + 0.5, Grid::HundredThousandths).floor();
    u16::try_from((count as u32).max(1)).ok()
}

fn odf(definition: &Definition, stops: Vec<GradientStop<Color>>, steps: Option<u16>) -> Fill {
    let degrees = quantise(definition.tenths as f64 / 10.0, Grid::Thousandths);
    Fill::Odf(OdfGradient {
        style: definition.style,
        angle: Angle::quantise(degrees).unwrap_or(Angle::ZERO),
        border: crate::units::unit_ratio(crate::units::ratio(definition.border)),
        center: Point {
            x: ratio_of(quantise(definition.center.0, Grid::HundredThousandths)),
            y: ratio_of(quantise(definition.center.1, Grid::HundredThousandths)),
        },
        stops,
        steps,
    })
}

/// A fill read with a transparency gradient: what is drawn, the features the source holds, and what is reported.
pub(crate) struct Folded {
    pub fill: Fill,
    pub features: Vec<FeatureId>,
    pub notes: Vec<Note>,
}

impl Folded {
    fn of(fill: Fill, notes: Vec<Note>) -> Folded {
        let features = fill_feature(&fill).into_iter().collect();
        Folded {
            fill,
            features,
            notes,
        }
    }

    /// `fill` drawn without the transparency gradient, which is reported.
    fn without(fill: Fill, transparency: &Definition, reason: String, notes: Vec<Note>) -> Folded {
        let mut folded = Folded::of(fill, notes);
        let feature = feature_of(transparency.style);
        if !folded.features.contains(&feature) {
            folded.features.push(feature);
        }
        folded.notes.push(Note { feature, reason });
        folded
    }
}

/// The unknown style of a definition, reported; it is drawn linear, LibreOffice's import default.
fn unknown_style(unknown: Option<String>, notes: &mut Vec<Note>) {
    if let Some(style) = unknown {
        notes.push(Note {
            feature: FeatureId::FillLinear,
            reason: format!("unknown style {style}"),
        });
    }
}

/// A colour gradient with the uniform opacity `alpha`, and the transparency gradient that applies to it.
pub(crate) fn gradient_fill(
    colors: (Definition, Option<String>),
    steps: u32,
    alpha: f64,
    transparency: Option<(Definition, Option<String>)>,
) -> Folded {
    let (colors, unknown) = colors;
    let mut notes = Vec::new();
    unknown_style(unknown, &mut notes);
    let steps = band_count(steps, &colors.stops);
    let feature = feature_of(colors.style);
    let plain = |notes: &mut Vec<Note>| {
        let stops: Vec<(f64, Rgb, f64)> = colors
            .stops
            .iter()
            .map(|(offset, color)| (*offset, *color, alpha))
            .collect();
        odf(&colors, ir_stops(&stops, feature, notes), steps)
    };
    let Some((transparency, unknown)) = transparency else {
        let fill = plain(&mut notes);
        return Folded::of(fill, notes);
    };
    unknown_style(unknown, &mut notes);
    let drawn = match as_drawn(&transparency) {
        Ok(drawn) => drawn,
        Err(reason) => {
            let fill = plain(&mut notes);
            return Folded::without(fill, &transparency, reason, notes);
        }
    };
    if steps.is_some() {
        let fill = plain(&mut notes);
        let reason = "a transparency gradient over a gradient with steps".to_owned();
        return Folded::without(fill, &transparency, reason, notes);
    }
    let (colors, greys) = (ramp(&colors), ramp(&drawn));
    let same = |a: &Definition, b: &Definition| {
        (a.style, a.tenths, a.border, a.center) == (b.style, b.tenths, b.border, b.center)
    };
    if !same(&colors, &greys) {
        let fill = plain(&mut notes);
        let reason = "a transparency gradient with its own geometry".to_owned();
        return Folded::without(fill, &transparency, reason, notes);
    }
    let stops = merge(&colors.stops, &greys.stops, alpha);
    let bands = folded_bands(colors.style, &stops);
    let fill = odf(&colors, ir_stops(&stops, feature, &mut notes), bands);
    Folded::of(fill, notes)
}

/// A solid colour (its alpha the uniform opacity) under a transparency gradient: a gradient of the transparency's
/// contours in that colour.
pub(crate) fn solid_fill(color: &RgbColor, transparency: (Definition, Option<String>)) -> Folded {
    let (transparency, unknown) = transparency;
    let mut notes = Vec::new();
    unknown_style(unknown, &mut notes);
    let solid = Fill::Solid {
        color: Color::Rgb(color.clone()),
    };
    let drawn = match as_drawn(&transparency) {
        Ok(drawn) => drawn,
        Err(reason) => return Folded::without(solid, &transparency, reason, notes),
    };
    let (rgb, alpha) = (channels(color), color.a.to_f64());
    let stops: Vec<(f64, Rgb, f64)> = drawn
        .stops
        .iter()
        .map(|(offset, grey)| (*offset, rgb, alpha * (1.0 - grey[0])))
        .collect();
    let bands = folded_bands(drawn.style, &stops);
    let fill = odf(
        &drawn,
        ir_stops(&stops, feature_of(drawn.style), &mut notes),
        bands,
    );
    Folded::of(fill, notes)
}

/// A fill LibreOffice cannot apply a transparency gradient to here (a picture), drawn without it.
pub(crate) fn unfolded(fill: Fill, transparency: &Definition, reason: &str) -> Folded {
    Folded::without(fill, transparency, reason.to_owned(), Vec::new())
}

/// Reports what reading a fill noted, each as OD9103 with its feature.
pub(crate) fn report(notes: Vec<Note>, paint: &Paint<'_, '_>) {
    for note in notes {
        paint.report(
            codes::OD9103,
            DiagnosticSite::new()
                .feature(note.feature)
                .data("reason", note.reason),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_unitless_angles_as_libreoffice_25_2_does() {
        let lo6 = "LibreOfficeDev/6.0.5.2$Linux_X86_64 LibreOffice_project/";
        let lo7 = "LibreOffice/7.6.4.1$Linux_X86_64 LibreOffice_project/e19e193f88cd6c0525a17fb7a176ed8e6a3e2aa1";
        let lo25 = "LibreOffice/25.2.3.2$MacOSX_AARCH64 LibreOffice_project/bbb074479178df812d175f709636b368952c2ce3";
        let ooo = "OpenOffice.org/3.2$Win32 OpenOffice.org_project/320m12$Build-9483";
        let aoo = "Apache_OpenOffice/4.1.15$Win32 OpenOffice.org_project/4115m1$Build-9813";
        assert!(!unitless_angle_in_tenths(Some("1.3"), Some(lo6)));
        assert!(unitless_angle_in_tenths(Some("1.2"), Some(lo6)));
        assert!(!unitless_angle_in_tenths(Some("1.2"), Some(lo7)));
        assert!(!unitless_angle_in_tenths(Some("1.2"), Some(lo25)));
        assert!(unitless_angle_in_tenths(Some("1.2"), Some(ooo)));
        assert!(unitless_angle_in_tenths(Some("1.2"), Some(aoo)));
        assert!(!unitless_angle_in_tenths(Some("1.2"), Some("Google")));
        assert!(!unitless_angle_in_tenths(Some("1.2"), None));
        assert!(unitless_angle_in_tenths(Some("1.1"), Some(lo25)));
        assert!(unitless_angle_in_tenths(None, None));
        assert_eq!(tenths(Some("300"), true), 300);
        assert_eq!(tenths(Some("300"), false), 3000);
        assert_eq!(tenths(Some("-30deg"), false), 3300);
        assert_eq!(tenths(Some("bad"), false), 0);
    }

    #[test]
    fn sorts_and_cuts_stops_as_libreoffice_does() {
        let (r, g, b) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]);
        assert_eq!(
            sort_and_correct(vec![(1.0, b), (0.3, g), (0.0, r)]),
            [(0.0, r), (0.3, g), (1.0, b)]
        );
        // Outside [0, 1]: moved onto the end at the ramp's colour there, or dropped.
        assert_eq!(
            sort_and_correct(vec![(-1.0, r), (1.0, b), (3.0, g)]),
            [(0.0, [0.5, 0.0, 0.5]), (1.0, b)]
        );
        assert_eq!(
            sort_and_correct(vec![(-2.0, r), (-1.0, g), (1.0, b), (1.0, b)]),
            [(0.0, [0.0, 0.5, 0.5]), (1.0, b)]
        );
    }

    #[test]
    fn counts_bands_up_to_libreoffices_automatic_count() {
        let (black, white) = ([0.0; 3], [1.0; 3]);
        assert_eq!(band_count(0, &[(0.0, black), (1.0, white)]), None);
        assert_eq!(band_count(4, &[(0.0, black), (1.0, white)]), Some(4));
        assert_eq!(band_count(200, &[(0.0, black), (1.0, white)]), Some(128));
        assert_eq!(band_count(4, &[(0.0, black), (1.0, [0.01; 3])]), Some(1));
        // A hard stop spans nothing.
        assert_eq!(
            band_count(9, &[(0.0, black), (0.5, black), (0.5, white), (1.0, white)]),
            Some(1)
        );
    }

    fn definition(style: OdfStyle, border: f64, stops: Vec<Stop>) -> Definition {
        Definition {
            style,
            tenths: 0,
            border,
            center: (0.5, 0.5),
            stops,
        }
    }

    #[test]
    fn draws_a_transparency_border_as_libreoffices_pdf_does() {
        let stops = vec![(0.0, [1.0; 3]), (1.0, [0.0; 3])];
        let radial = as_drawn(&definition(OdfStyle::Radial, 0.2, stops.clone())).unwrap();
        assert_eq!(radial.border, 0.2);
        assert_eq!(radial.stops, [(0.0, [1.0; 3]), (0.8, [0.0; 3])]);
        let rectangular = as_drawn(&definition(OdfStyle::Rectangular, 0.2, stops.clone())).unwrap();
        assert_eq!(rectangular.border, 0.0);
        assert_eq!(rectangular.stops, [(0.2, [1.0; 3]), (1.0, [0.0; 3])]);
        let linear = definition(OdfStyle::Linear, 0.25, stops.clone());
        assert_eq!(as_drawn(&linear).unwrap(), linear);
        assert_eq!(
            as_drawn(&definition(OdfStyle::Square, 0.2, stops)).unwrap_err(),
            "a square transparency gradient with a border"
        );
    }

    #[test]
    fn merges_two_stop_lists_over_one_ramp() {
        let colors = vec![
            (0.0, [0.0; 3]),
            (0.5, [0.0; 3]),
            (0.5, [1.0; 3]),
            (1.0, [1.0; 3]),
        ];
        let greys = vec![(0.0, [0.0; 3]), (1.0, [0.8; 3])];
        let merged = merge(&colors, &greys, 1.0);
        let alphas: Vec<(f64, f64, f64)> = merged.iter().map(|s| (s.0, s.1[0], s.2)).collect();
        let near = |a: f64, b: f64| (a - b).abs() < 1e-12;
        let expected = [
            (0.0, 0.0, 1.0),
            (0.5, 0.0, 0.6),
            (0.5, 1.0, 0.6),
            (1.0, 1.0, 0.2),
        ];
        assert_eq!(alphas.len(), expected.len());
        for (got, want) in alphas.iter().zip(expected) {
            assert!(
                near(got.0, want.0) && near(got.1, want.1) && near(got.2, want.2),
                "{alphas:?}"
            );
        }
    }

    #[test]
    fn compares_contours_without_what_a_style_ignores() {
        let stops = vec![(0.0, [0.0; 3]), (1.0, [1.0; 3])];
        let mut a = definition(OdfStyle::Linear, 0.25, stops.clone());
        a.center = (0.1, 0.9);
        let folded = ramp(&a);
        assert_eq!((folded.border, folded.center), (0.0, (0.0, 0.0)));
        assert_eq!(folded.stops, [(0.25, [0.0; 3]), (1.0, [1.0; 3])]);
        let axial = ramp(&definition(OdfStyle::Axial, 0.2, stops.clone()));
        assert_eq!(axial.stops, [(0.0, [0.0; 3]), (0.8, [1.0; 3])]);
        let mut radial = definition(OdfStyle::Radial, 0.0, stops.clone());
        radial.tenths = 300;
        assert_eq!(ramp(&radial).tenths, 0);
        let ellipsoid = definition(OdfStyle::Ellipsoid, 0.2, stops);
        assert_eq!(ramp(&ellipsoid), ellipsoid);
    }
}
