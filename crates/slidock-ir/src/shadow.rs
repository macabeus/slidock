//! Outer shadows: the drawn silhouette of their carrier (fill, stroke, line ends, text, a picture's alpha after crop,
//! clip and mask) in one colour, offset and blurred, painted under the carrier and over everything before it. One
//! app-neutral model: an offset in page axes, the standard deviation of the Gaussian that best fits the source app's
//! blur, and a colour with the shadow's opacity in its alpha. An element's opacity applies to it and its shadow as one
//! group; translucency of the content itself (a fill's alpha, an image's alpha) shows its shadow through it.
//!
//! [`Axes`] maps the offset into the axes a backend draws the shadow in, under the rotations and flips above it.

use serde::{Deserialize, Serialize};

use crate::color::{Color, ResolvedColor};
use crate::frame::Frame;
use crate::units::{Pt, Size};
use crate::validate::{Validate, Validator};

/// The largest standard deviation, in points: Keynote's largest radius (100) fits 51, PowerPoint's largest blur
/// (100 pt) about 43; the bound keeps filter regions sane.
pub const MAX_SIGMA: f64 = 500.0;

/// An outer shadow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    deny_unknown_fields,
    rename_all = "camelCase",
    bound(deserialize = "C: serde::de::DeserializeOwned")
)]
pub struct Shadow<C> {
    /// The offset in page axes (x right, y down), unless `rotate_with_shape`: then in the carrier's turned axes.
    pub dx: Pt,
    pub dy: Pt,
    /// The standard deviation of the Gaussian that best fits the source app's blur; 0 is a hard edge.
    pub sigma: Size,
    /// The colour, the shadow's opacity folded into its alpha.
    pub color: C,
    /// OOXML `rotWithShape`: the offset turns with the carrier's rotation and its groups'. Absent (false) elsewhere.
    #[serde(default, skip_serializing_if = "crate::elements::is_false")]
    pub rotate_with_shape: bool,
    /// Text overflowing a box that an opaque fill paints whole casts too (LibreOffice). Absent (false): such a box
    /// casts the box alone, its overflowing text nothing (Keynote, measured).
    #[serde(default, skip_serializing_if = "crate::elements::is_false")]
    pub overflow_casts: bool,
}

pub type SourceShadow = Shadow<Color>;
pub type ResolvedShadow = Shadow<ResolvedColor>;

/// A shadow colour's literal alpha, when it is a literal.
trait LiteralAlpha {
    fn literal_alpha(&self) -> Option<f64>;
}

impl LiteralAlpha for Color {
    fn literal_alpha(&self) -> Option<f64> {
        match self {
            Color::Rgb(rgb) => Some(rgb.a.to_f64()),
            Color::Theme(_) => None,
        }
    }
}

impl LiteralAlpha for ResolvedColor {
    fn literal_alpha(&self) -> Option<f64> {
        match self {
            ResolvedColor::Rgb(rgb) => Some(rgb.a.to_f64()),
            ResolvedColor::Theme(_) => None,
        }
    }
}

#[allow(private_bounds)]
impl<C: Validate + LiteralAlpha> Validate for Shadow<C> {
    fn validate(&self, v: &mut Validator) {
        v.field("dx", &self.dx);
        v.field("dy", &self.dy);
        v.at("sigma", |v| {
            self.sigma.validate(v);
            v.max(self.sigma.to_f64(), MAX_SIGMA);
        });
        v.at("color", |v| {
            self.color.validate(v);
            // A shadow that paints nothing is left out, not written.
            if let Some(alpha) = self.color.literal_alpha() {
                v.at("a", |v| v.min(alpha, 0.0, true));
            }
        });
    }
}

/// An orthogonal map from an element's local axes to the page's: the product, from the slide down, of each rotation
/// and flip a backend applies above the element (`R(ρ)·S(fx, fy)`, rotation after the flips). It also keeps the sum
/// of those rotations, which a shadow that turns with its shape turns by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axes {
    /// Row-major `[[a, b], [c, d]]`: a local vector `(x, y)` is `(a·x + b·y, c·x + d·y)` on the page.
    m: [f64; 4],
    /// The rotations summed, flips ignored, in degrees.
    turn: f64,
}

impl Default for Axes {
    fn default() -> Self {
        Axes::PAGE
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

impl Axes {
    /// The page's own axes.
    pub const PAGE: Axes = Axes {
        m: [1.0, 0.0, 0.0, 1.0],
        turn: 0.0,
    };

    /// These axes, then a rotation by `degrees` (clockwise, y down) and the flips (applied first).
    pub fn then(self, degrees: f64, flip_h: bool, flip_v: bool) -> Axes {
        let (c, s) = cos_sin(degrees);
        let (fx, fy) = (
            if flip_h { -1.0 } else { 1.0 },
            if flip_v { -1.0 } else { 1.0 },
        );
        // R(θ)·S(fx, fy) with y down: R = [[c, −s], [s, c]].
        let local = [c * fx, -s * fy, s * fx, c * fy];
        let [a, b, cc, d] = self.m;
        let [e, f, g, h] = local;
        Axes {
            m: [a * e + b * g, a * f + b * h, cc * e + d * g, cc * f + d * h],
            turn: self.turn + degrees,
        }
    }

    /// These axes, then a frame's rotation and flips.
    pub fn then_frame(self, frame: &Frame) -> Axes {
        self.then(frame.rotation.to_f64(), frame.flip_h, frame.flip_v)
    }

    /// The rotations summed, in degrees.
    pub fn turn(&self) -> f64 {
        self.turn
    }

    /// A page vector in these axes: `Mᵀ·v`, the inverse of an orthogonal map.
    pub fn from_page(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d] = self.m;
        (a * x + c * y, b * x + d * y)
    }
}

impl<C> Shadow<C> {
    /// The offset in the axes a carrier draws in, `axes` being its local axes: the page offset taken into them, or,
    /// for a shadow that turns with its shape, the offset turned by every rotation above it first.
    pub fn offset_in(&self, axes: &Axes) -> (f64, f64) {
        let (dx, dy) = (self.dx.to_f64(), self.dy.to_f64());
        let (x, y) = if self.rotate_with_shape {
            let (c, s) = cos_sin(axes.turn());
            (c * dx - s * dy, s * dx + c * dy)
        } else {
            (dx, dy)
        };
        axes.from_page(x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn black(a: f64) -> serde_json::Value {
        json!({ "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": a })
    }

    fn shadow(extra: serde_json::Value) -> serde_json::Value {
        let mut value = json!({ "dx": 0, "dy": 3.287, "sigma": 7.14, "color": black(0.5331) });
        if let (Some(map), Some(extra)) = (value.as_object_mut(), extra.as_object()) {
            map.extend(extra.clone());
        }
        value
    }

    fn issues(value: serde_json::Value) -> Vec<String> {
        let parsed: SourceShadow = serde_json::from_value(value).unwrap();
        crate::validate::validate(&parsed)
            .into_iter()
            .map(|issue| format!("{} {}", issue.code, issue.path))
            .collect()
    }

    #[test]
    fn round_trips_in_both_phases_and_writes_rotate_with_shape_only_when_set() {
        let value = shadow(json!({}));
        let source: SourceShadow = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(
            crate::canonical::to_json(&source).unwrap(),
            slidock_canon::value_to_canonical_json(&value, slidock_canon::Layout::Compact).unwrap()
        );
        let resolved: ResolvedShadow = serde_json::from_value(value).unwrap();
        assert_eq!(serde_json::to_value(&resolved).unwrap()["dy"], 3.287);
        let turned: SourceShadow =
            serde_json::from_value(shadow(json!({ "rotateWithShape": true }))).unwrap();
        assert!(turned.rotate_with_shape);
        assert_eq!(
            serde_json::to_value(&turned).unwrap()["rotateWithShape"],
            true
        );
        assert!(
            serde_json::to_value(&source)
                .unwrap()
                .get("overflowCasts")
                .is_none()
        );
        let overflowing: SourceShadow =
            serde_json::from_value(shadow(json!({ "overflowCasts": true }))).unwrap();
        assert!(overflowing.overflow_casts);
    }

    #[test]
    fn refuses_unknown_fields_and_unresolved_colours_when_resolved() {
        assert!(serde_json::from_value::<SourceShadow>(shadow(json!({ "angle": 90 }))).is_err());
        let theme = json!({ "kind": "theme", "slot": "accent1", "mods": [] });
        assert!(serde_json::from_value::<SourceShadow>(shadow(json!({ "color": theme }))).is_ok());
        assert!(
            serde_json::from_value::<ResolvedShadow>(shadow(json!({ "color": theme }))).is_err()
        );
    }

    #[test]
    fn bounds_sigma_and_refuses_a_shadow_that_paints_nothing() {
        assert!(issues(shadow(json!({}))).is_empty());
        assert!(issues(shadow(json!({ "sigma": 500 }))).is_empty());
        assert_eq!(
            issues(shadow(json!({ "sigma": 500.001 }))),
            ["IR0008 sigma"]
        );
        assert_eq!(issues(shadow(json!({ "sigma": -1 }))), ["IR0008 sigma"]);
        assert_eq!(
            issues(shadow(json!({ "color": black(0.0) }))),
            ["IR0008 color.a"]
        );
    }

    fn offset(shadow: &SourceShadow, axes: &Axes) -> (f64, f64) {
        let (x, y) = shadow.offset_in(axes);
        ((x * 1000.0).round() / 1000.0, (y * 1000.0).round() / 1000.0)
    }

    #[test]
    fn takes_a_page_offset_into_the_carriers_turned_axes() {
        let down: SourceShadow = serde_json::from_value(shadow(json!({}))).unwrap();
        assert_eq!(offset(&down, &Axes::PAGE), (0.0, 3.287));
        // A box turned 90° clockwise draws its local +x downwards: the page's down is its +x.
        assert_eq!(
            offset(&down, &Axes::PAGE.then(90.0, false, false)),
            (3.287, 0.0)
        );
        assert_eq!(
            offset(&down, &Axes::PAGE.then(180.0, false, false)),
            (0.0, -3.287)
        );
        let right: SourceShadow =
            serde_json::from_value(shadow(json!({ "dx": 20, "dy": 0 }))).unwrap();
        assert_eq!(
            offset(&right, &Axes::PAGE.then(30.0, false, false)),
            (17.321, -10.0)
        );
        // A mirrored box draws its local +x leftwards.
        assert_eq!(
            offset(&right, &Axes::PAGE.then(0.0, true, false)),
            (-20.0, 0.0)
        );
        assert_eq!(
            offset(&down, &Axes::PAGE.then(0.0, false, true)),
            (0.0, -3.287)
        );
        // A child turned 30° in a group turned 60° is turned 90° on the page.
        let nested = Axes::PAGE.then(60.0, false, false).then(30.0, false, false);
        assert_eq!(offset(&down, &nested), (3.287, 0.0));
        assert_eq!(nested.turn(), 90.0);
    }

    #[test]
    fn turns_a_shadow_that_rotates_with_its_shape_by_every_rotation_above_it() {
        let turned: SourceShadow = serde_json::from_value(shadow(json!({
            "dx": 20, "dy": 0, "rotateWithShape": true
        })))
        .unwrap();
        // In its own axes the offset is the one written, whatever the turn.
        assert_eq!(
            offset(&turned, &Axes::PAGE.then(30.0, false, false)),
            (20.0, 0.0)
        );
        let nested = Axes::PAGE.then(60.0, false, false).then(30.0, false, false);
        assert_eq!(offset(&turned, &nested), (20.0, 0.0));
        // Flips do not turn it.
        assert_eq!(
            offset(&turned, &Axes::PAGE.then(0.0, true, false)),
            (-20.0, 0.0)
        );
    }
}
