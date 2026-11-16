//! Geometry: presets with adjustments and custom paths in the source phase, frame-local paths once lowered.

use std::fmt;

use indexmap::IndexMap;
use serde::de::{self, SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::de::{literal_tag, self_tagged_union};
use crate::diagnostics::codes;
use crate::presets::PRESET_NAMES;
use crate::units::{PathUnit, Pt};
use crate::validate::{Validate, Validator};

/// The most segments one path may hold (the frontends' per-shape limit).
pub const MAX_PATH_SEGMENTS: usize = 100_000;

/// A path command: move, line, quadratic, cubic, close; arcs arrive as cubics. `C` is the coordinate: frame-local
/// points ([`Segment`]) or a custom path's own units ([`PathUnitSegment`]). Serialises as `["M", x, y]` … `["Z"]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seg<C> {
    M(C, C),
    L(C, C),
    Q(C, C, C, C),
    C(C, C, C, C, C, C),
    Z,
}

pub type Segment = Seg<Pt>;
pub type PathUnitSegment = Seg<PathUnit>;

impl<C: Copy> Seg<C> {
    pub fn command(&self) -> &'static str {
        match self {
            Seg::M(..) => "M",
            Seg::L(..) => "L",
            Seg::Q(..) => "Q",
            Seg::C(..) => "C",
            Seg::Z => "Z",
        }
    }

    /// The coordinates, in order.
    pub fn coords(&self) -> Vec<C> {
        match *self {
            Seg::M(x, y) | Seg::L(x, y) => vec![x, y],
            Seg::Q(a, b, c, d) => vec![a, b, c, d],
            Seg::C(a, b, c, d, e, f) => vec![a, b, c, d, e, f],
            Seg::Z => vec![],
        }
    }
}

impl<C: Serialize + Copy> Serialize for Seg<C> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let coords = self.coords();
        let mut seq = serializer.serialize_seq(Some(coords.len() + 1))?;
        seq.serialize_element(self.command())?;
        for coord in &coords {
            seq.serialize_element(coord)?;
        }
        seq.end()
    }
}

struct SegVisitor<C>(std::marker::PhantomData<C>);

impl<'de, C: Deserialize<'de> + Copy> Visitor<'de> for SegVisitor<C> {
    type Value = Seg<C>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "a path segment: [\"M\", x, y], [\"L\", x, y], [\"Q\", …4], [\"C\", …6] or [\"Z\"]",
        )
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Seg<C>, A::Error> {
        let command: String = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let arity = match command.as_str() {
            "M" | "L" => 2,
            "Q" => 4,
            "C" => 6,
            "Z" => 0,
            _ => {
                return Err(de::Error::invalid_value(
                    de::Unexpected::Str(&command),
                    &self,
                ));
            }
        };
        let mut coords = Vec::with_capacity(arity);
        for i in 0..arity {
            let coord: C = seq
                .next_element()?
                .ok_or_else(|| de::Error::invalid_length(i + 1, &self))?;
            coords.push(coord);
        }
        if seq.next_element::<de::IgnoredAny>()?.is_some() {
            return Err(de::Error::invalid_length(arity + 2, &self));
        }
        Ok(match (command.as_str(), coords.as_slice()) {
            ("M", &[x, y]) => Seg::M(x, y),
            ("L", &[x, y]) => Seg::L(x, y),
            ("Q", &[a, b, c, d]) => Seg::Q(a, b, c, d),
            ("C", &[a, b, c, d, e, f]) => Seg::C(a, b, c, d, e, f),
            _ => Seg::Z,
        })
    }
}

impl<'de, C: Deserialize<'de> + Copy> Deserialize<'de> for Seg<C> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_seq(SegVisitor(std::marker::PhantomData))
    }
}

impl<C: Validate + Copy> Validate for Seg<C> {
    fn validate(&self, v: &mut Validator) {
        for (i, coord) in self.coords().iter().enumerate() {
            v.at(i + 1, |v| coord.validate(v));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FillRule {
    Nonzero,
    Evenodd,
}

/// Marks a path lower-geometry added for an arrowhead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PathRole {
    #[serde(rename = "line-end")]
    LineEnd,
}

/// A frame-local path, what every geometry lowers to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Path {
    pub segments: Vec<Segment>,
    pub fill: bool,
    pub stroke: bool,
    pub fill_rule: FillRule,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub role: Option<PathRole>,
}

impl Validate for Path {
    fn validate(&self, v: &mut Validator) {
        v.at("segments", |v| {
            v.array(self.segments.len(), 0, MAX_PATH_SEGMENTS);
            self.segments.validate(v);
        });
    }
}

/// A custom path in its own units; lower-geometry scales them to the frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CustomPath {
    pub w: PathUnit,
    pub h: PathUnit,
    pub fill: bool,
    pub stroke: bool,
    pub fill_rule: FillRule,
    pub segments: Vec<PathUnitSegment>,
}

impl Validate for CustomPath {
    fn validate(&self, v: &mut Validator) {
        v.at("w", |v| {
            self.w.validate(v);
            v.min(self.w.to_f64(), 0.0, false);
        });
        v.at("h", |v| {
            self.h.validate(v);
            v.min(self.h.to_f64(), 0.0, false);
        });
        v.at("segments", |v| {
            v.array(self.segments.len(), 0, MAX_PATH_SEGMENTS);
            self.segments.validate(v);
        });
    }
}

/// An ECMA-376 preset shape name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PresetName(u8);

impl PresetName {
    pub fn parse(name: &str) -> Option<PresetName> {
        PRESET_NAMES
            .binary_search(&name)
            .ok()
            .and_then(|i| u8::try_from(i).ok())
            .map(PresetName)
    }

    pub fn as_str(self) -> &'static str {
        PRESET_NAMES.get(usize::from(self.0)).copied().unwrap_or("")
    }

    /// A connector preset (`/Connector\d$/`): drawn as a line, its feature is `geometry.connector`.
    pub fn is_connector(self) -> bool {
        let name = self.as_str().as_bytes();
        matches!(name.split_last(), Some((last, head)) if last.is_ascii_digit() && head.ends_with(b"Connector"))
    }
}

impl fmt::Display for PresetName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for PresetName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PresetName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        PresetName::parse(&name).ok_or_else(|| {
            de::Error::custom(format_args!(
                "Invalid option: {name:?} is not a preset shape"
            ))
        })
    }
}

literal_tag!(PresetTag = "preset");
literal_tag!(CustomTag = "custom");
literal_tag!(PathsTag = "paths");

/// A preset shape with its adjustment values (`avLst` guide name → value); source phase only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresetGeometry {
    pub kind: PresetTag,
    pub name: PresetName,
    #[serde(deserialize_with = "crate::de::js_record")]
    pub adjust: IndexMap<String, f64>,
}

/// `^[A-Za-z][A-Za-z0-9]{0,31}$`
fn is_guide_name(key: &str) -> bool {
    let bytes = key.as_bytes();
    matches!(bytes.first(), Some(b) if b.is_ascii_alphabetic())
        && bytes.len() <= 32
        && bytes.iter().all(u8::is_ascii_alphanumeric)
}

impl Validate for PresetGeometry {
    fn validate(&self, v: &mut Validator) {
        v.at("adjust", |v| {
            for (key, value) in &self.adjust {
                v.at(key.clone(), |v| {
                    if !is_guide_name(key) {
                        v.invalid(codes::IR0001, "Invalid key in record");
                    }
                    v.min(*value, -1e9, false);
                    v.max(*value, 1e9);
                });
            }
        });
    }
}

/// Paths in their own units; source phase only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomGeometry {
    pub kind: CustomTag,
    pub paths: Vec<CustomPath>,
}

impl Validate for CustomGeometry {
    fn validate(&self, v: &mut Validator) {
        v.at("paths", |v| {
            v.array(self.paths.len(), 1, 256);
            self.paths.validate(v);
        });
    }
}

/// The only resolved form: frame-local paths, with the preset kept as provenance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathsGeometry {
    pub kind: PathsTag,
    pub paths: Vec<Path>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub preset: Option<PresetName>,
}

impl Validate for PathsGeometry {
    fn validate(&self, v: &mut Validator) {
        v.at("paths", |v| {
            v.array(self.paths.len(), 0, 256);
            self.paths.validate(v);
        });
    }
}

self_tagged_union! {
    /// A geometry in the source phase.
    pub enum Geometry ("kind") {
        Preset(PresetGeometry) = "preset",
        Custom(CustomGeometry) = "custom",
        Paths(PathsGeometry) = "paths",
    }
}

impl Geometry {
    pub fn kind(&self) -> &'static str {
        match self {
            Geometry::Preset(_) => "preset",
            Geometry::Custom(_) => "custom",
            Geometry::Paths(_) => "paths",
        }
    }
}

impl Validate for Geometry {
    fn validate(&self, v: &mut Validator) {
        match self {
            Geometry::Preset(g) => g.validate(v),
            Geometry::Custom(g) => g.validate(v),
            Geometry::Paths(g) => g.validate(v),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lists_the_presets_once_each_sorted_by_code_unit() {
        assert_eq!(PRESET_NAMES.len(), 187);
        assert!(PRESET_NAMES.windows(2).all(|w| w[0] < w[1]));
        let round = PresetName::parse("roundRect").unwrap();
        assert_eq!(round.as_str(), "roundRect");
        assert!(PresetName::parse("squircle").is_none());
        assert!(PresetName::parse("bentConnector3").unwrap().is_connector());
        assert!(!round.is_connector());
    }

    #[test]
    fn accepts_presets_with_adjustments_and_refuses_unknown_names() {
        let geometry: Geometry = serde_json::from_value(
            json!({ "kind": "preset", "name": "roundRect", "adjust": { "adj": 16667 } }),
        )
        .unwrap();
        assert!(crate::validate::validate(&geometry).is_empty());
        assert!(
            serde_json::from_value::<Geometry>(
                json!({ "kind": "preset", "name": "squircle", "adjust": {} })
            )
            .is_err()
        );
        let bad_key: Geometry = serde_json::from_value(
            json!({ "kind": "preset", "name": "rect", "adjust": { "1adj": 1 } }),
        )
        .unwrap();
        assert_eq!(crate::validate::validate(&bad_key)[0].code, codes::IR0001);
    }

    #[test]
    fn keeps_custom_paths_in_their_own_units() {
        let custom = json!({ "kind": "custom", "paths": [{
            "w": 9144000, "h": 6858000, "fill": true, "stroke": true, "fillRule": "nonzero",
            "segments": [["M", 0, 0], ["L", 9144000, 6858000], ["C", 1, 2, 3, 4, 5, 6], ["Z"]]
        }] });
        let geometry: Geometry = serde_json::from_value(custom.clone()).unwrap();
        assert!(crate::validate::validate(&geometry).is_empty());
        assert_eq!(
            crate::canonical::to_json(&geometry).unwrap(),
            slidock_canon::value_to_canonical_json(&custom, slidock_canon::Layout::Compact)
                .unwrap()
        );
    }

    #[test]
    fn checks_segment_arity() {
        let paths = |segments: serde_json::Value| {
            serde_json::from_value::<Geometry>(json!({ "kind": "paths", "paths": [{
                "segments": segments, "fill": true, "stroke": false, "fillRule": "evenodd"
            }] }))
        };
        assert!(paths(json!([["M", 0, 0], ["Q", 1, 1, 2, 0], ["Z"]])).is_ok());
        assert!(paths(json!([["M", 0]])).is_err());
        assert!(paths(json!([["M", 0, 0, 1]])).is_err());
        assert!(paths(json!([["A", 1, 1, 0, 0, 1, 2, 2]])).is_err());
    }
}
