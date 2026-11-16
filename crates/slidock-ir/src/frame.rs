//! Element boxes.

use serde::{Deserialize, Serialize};

use crate::units::{Angle, Pt, Size};
use crate::validate::{Validate, Validator};

/// An element's box. `x`/`y` are the top-left of the unrotated box, relative to the parent container (the
/// slide, layout or master) or the parent group's box (unscaled). Rotation turns about the box centre, clockwise;
/// flips act on the box before rotation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Frame {
    pub x: Pt,
    pub y: Pt,
    pub w: Size,
    pub h: Size,
    pub rotation: Angle,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl Validate for Frame {
    fn validate(&self, v: &mut Validator) {
        v.field("x", &self.x);
        v.field("y", &self.y);
        v.field("w", &self.w);
        v.field("h", &self.h);
        v.field("rotation", &self.rotation);
    }
}

/// The coordinate space a group gives its children (OOXML `chOff`/`chExt`); source phase only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildSpace {
    pub x: Pt,
    pub y: Pt,
    pub w: Size,
    pub h: Size,
}

impl Validate for ChildSpace {
    fn validate(&self, v: &mut Validator) {
        v.field("x", &self.x);
        v.field("y", &self.y);
        v.field("w", &self.w);
        v.field("h", &self.h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn frame() -> serde_json::Value {
        json!({ "x": 10, "y": 20, "w": 300, "h": 40.5, "rotation": 0, "flipH": false, "flipV": false })
    }

    #[test]
    fn accepts_a_canonical_frame() {
        let parsed: Frame = serde_json::from_value(frame()).unwrap();
        assert!(crate::validate::validate(&parsed).is_empty());
        assert_eq!(
            crate::canonical::to_json(&parsed).unwrap(),
            slidock_canon::value_to_canonical_json(&frame(), slidock_canon::Layout::Compact)
                .unwrap()
        );
    }

    #[test]
    fn refuses_negative_sizes_unnormalised_rotation_and_shears() {
        let mut negative = frame();
        negative["w"] = json!(-1);
        let parsed: Frame = serde_json::from_value(negative).unwrap();
        assert_eq!(crate::validate::validate(&parsed).len(), 1);
        let mut turned = frame();
        turned["rotation"] = json!(360);
        let parsed: Frame = serde_json::from_value(turned).unwrap();
        assert_eq!(crate::validate::validate(&parsed).len(), 1);
        let mut sheared = frame();
        sheared["matrix"] = json!([1, 0, 0.2, 1, 0, 0]);
        assert!(serde_json::from_value::<Frame>(sheared).is_err());
    }
}
