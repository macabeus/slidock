//! `TSD.GeometryArchive.flags`, the bits of `TSDGeometry`: whether the stored width and height are the box's own
//! (Keynote sizes a cleared one to its text), and whether the drawable is mirrored left to right or top to bottom
//! about its centre, before its angle turns it. A shape's path source carries flips of its own, which mirror the
//! outline only; these mirror the whole drawable (a group, a picture, a movie, a shape with its text, or a mask).
//! Each bit is read on its own, so a bit slidock does not know changes nothing.
//!
//! A group's children are in the group's coordinates; [`SlideMap`] maps a container's coordinates onto the slide.

use slidock_ir::frame::Frame;
use slidock_math::{cos, sin};

use crate::proto::tsd;

pub const WIDTH_VALID: u32 = 1;
pub const HEIGHT_VALID: u32 = 2;
pub const HORIZONTAL_FLIP: u32 = 4;
pub const VERTICAL_FLIP: u32 = 8;

/// The drawable's mirrors: left to right, then top to bottom.
pub fn flips(geometry: Option<&tsd::GeometryArchive>) -> (bool, bool) {
    let flags = geometry.and_then(|g| g.flags).unwrap_or(0);
    (flags & HORIZONTAL_FLIP != 0, flags & VERTICAL_FLIP != 0)
}

/// Where a container's coordinates land on the slide: `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlideMap {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl SlideMap {
    /// The slide's own coordinates.
    pub const SLIDE: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// The map of a group's children, the group's frame being in this map's coordinates: the group
    /// mirrors them about its centre, turns them clockwise about it by its rotation, and moves them to its position.
    #[must_use]
    pub fn within(self, frame: &Frame) -> Self {
        let (x, y) = (frame.x.to_f64(), frame.y.to_f64());
        if frame.rotation.steps() == 0 && !frame.flip_h && !frame.flip_v {
            return self.then(Self {
                e: x,
                f: y,
                ..Self::SLIDE
            });
        }
        let (cx, cy) = (frame.w.to_f64() / 2.0, frame.h.to_f64() / 2.0);
        let radians = frame.rotation.to_f64() * std::f64::consts::PI / 180.0;
        let (cos, sin) = (cos(radians), sin(radians));
        let sx = if frame.flip_h { -1.0 } else { 1.0 };
        let sy = if frame.flip_v { -1.0 } else { 1.0 };
        let (a, b, c, d) = (cos * sx, sin * sx, -sin * sy, cos * sy);
        self.then(Self {
            a,
            b,
            c,
            d,
            e: x + cx - (a * cx + c * cy),
            f: y + cy - (b * cx + d * cy),
        })
    }

    /// `inner`'s coordinates through `inner`, then this map.
    fn then(self, inner: Self) -> Self {
        Self {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            e: self.a * inner.e + self.c * inner.f + self.e,
            f: self.b * inner.e + self.d * inner.f + self.f,
        }
    }

    /// The slide x of a point in this map's coordinates.
    pub fn x(self, x: f64, y: f64) -> f64 {
        self.a * x + self.c * y + self.e
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(flags: Option<u32>) -> tsd::GeometryArchive {
        tsd::GeometryArchive {
            flags,
            ..tsd::GeometryArchive::default()
        }
    }

    #[test]
    fn reads_the_flips_from_their_own_bits_and_nothing_else() {
        assert_eq!(flips(None), (false, false));
        assert_eq!(flips(Some(&with(None))), (false, false));
        assert_eq!(flips(Some(&with(Some(3)))), (false, false));
        assert_eq!(flips(Some(&with(Some(7)))), (true, false));
        assert_eq!(flips(Some(&with(Some(11)))), (false, true));
        assert_eq!(flips(Some(&with(Some(12)))), (true, true));
        assert_eq!(flips(Some(&with(Some(0x13)))), (false, false));
    }

    fn frame(x: f64, y: f64, rotation: f64, flip_h: bool, flip_v: bool) -> Frame {
        use slidock_ir::units::{Angle, Pt, Size};
        Frame {
            x: Pt::exact(x).unwrap(),
            y: Pt::exact(y).unwrap(),
            w: Size::exact(200.0).unwrap(),
            h: Size::exact(100.0).unwrap(),
            rotation: Angle::exact(rotation).unwrap(),
            flip_h,
            flip_v,
        }
    }

    #[test]
    fn maps_a_groups_children_through_its_mirror_turn_and_position() {
        let at = |map: SlideMap| (map.x(150.0, 50.0) * 1e9).round() / 1e9;
        assert_eq!(at(SlideMap::SLIDE), 150.0);
        let moved = SlideMap::SLIDE.within(&frame(1000.0, 7.0, 0.0, false, false));
        assert_eq!(at(moved), 1150.0);
        // Two groups add their positions.
        assert_eq!(
            at(moved.within(&frame(30.0, 0.0, 0.0, false, false))),
            1180.0
        );
        // Mirrored left to right about the centre (100, 50): 150 lands at 50.
        assert_eq!(
            at(SlideMap::SLIDE.within(&frame(1000.0, 0.0, 0.0, true, false))),
            1050.0
        );
        // Turned 90° clockwise about the centre: (50, 0) from it points down, so x stays the centre's.
        assert_eq!(
            at(SlideMap::SLIDE.within(&frame(1000.0, 0.0, 90.0, false, false))),
            1100.0
        );
        // Mirrored top to bottom, then turned 90°: a point below the centre goes above it, then to its right.
        let mirrored = SlideMap::SLIDE.within(&frame(0.0, 0.0, 90.0, false, true));
        assert_eq!((mirrored.x(100.0, 80.0) * 1e9).round() / 1e9, 130.0);
    }
}
