//! The face a browser picks from a family: the CSS Fonts 4 font matching algorithm (§5.2), which narrows the faces
//! by stretch, then by style, then by weight. Decks never set `font-stretch`, so the stretch wanted is always
//! `normal`: a catalog family keeps only the faces at the width [`stretch_rank`] ranks first
//! ([`crate::FontCatalog::build`]), and [`css_match`] picks by style and weight among them. Binding uses it so that
//! the face whose metrics the compiler reads is the face the browser draws.

use slidock_ir::fonts::FaceStyle;

use crate::catalog::{CatalogFace, CatalogFamily};

/// The weights a face covers: one weight for a static face, a range for a variable one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeightRange {
    pub min: u32,
    pub max: u32,
}

impl WeightRange {
    pub fn point(weight: u32) -> WeightRange {
        WeightRange {
            min: weight,
            max: weight,
        }
    }

    /// The range between two weights, in either order.
    pub fn between(a: u32, b: u32) -> WeightRange {
        WeightRange {
            min: a.min(b),
            max: a.max(b),
        }
    }
}

/// `OS/2.usWidthClass` of a face of normal width (100 %).
pub const NORMAL_WIDTH: u16 = 5;

/// Where a width falls in the stretch search order: lower is checked first.
pub type StretchRank = (u8, u16);

/// Where a face's width (`OS/2.usWidthClass`: 1 ultra-condensed, 50 %, to 9 ultra-expanded, 200 %) falls in the
/// search order for the `desired` one:
///
/// - `desired` normal or narrower: narrower widths descending, then wider ones ascending.
/// - `desired` wider than normal: wider widths ascending, then narrower ones descending.
///
/// A width outside 1–9 is unknown and comes after every known width, so it is matched only in a family that has no
/// face of a known width. The classes are ordered as the percentages they stand for, so their order is the order of
/// CSS stretch values.
pub fn stretch_rank(width: u16, desired: u16) -> StretchRank {
    if !(1..=9).contains(&width) {
        return (3, 0);
    }
    if width == desired {
        return (0, 0);
    }
    let wider = width > desired;
    let distance = width.abs_diff(desired);
    let tier = match (desired <= NORMAL_WIDTH, wider) {
        (true, false) | (false, true) => 1,
        (true, true) | (false, false) => 2,
    };
    (tier, distance)
}

/// What a face's `@font-face` rule states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceDescriptor {
    pub weight: WeightRange,
    pub style: FaceStyle,
}

impl From<&CatalogFace> for FaceDescriptor {
    /// A variable face covers its axis range when it was read, else its own weight.
    fn from(face: &CatalogFace) -> Self {
        FaceDescriptor {
            weight: face
                .variation
                .as_ref()
                .map_or(WeightRange::point(face.weight), |variation| variation.range),
            style: face.style,
        }
    }
}

/// Where a face's weight falls in the search order for `desired`: lower is checked first, then nearer.
///
/// - `desired` in 400–500: weights from `desired` up to 500 ascending, then lighter ones descending, then those
///   above 500 ascending.
/// - below 400: lighter or equal weights descending, then heavier ones ascending.
/// - above 500: heavier or equal weights ascending, then lighter ones descending.
///
/// A range that holds `desired` matches it exactly; otherwise its nearest end counts.
fn weight_rank(weight: WeightRange, desired: u32) -> (u8, u32) {
    if (weight.min..=weight.max).contains(&desired) {
        return (0, 0);
    }
    let heavier = weight.min > desired;
    let distance = if heavier {
        weight.min - desired
    } else {
        desired - weight.max
    };
    let tier = match (desired, heavier) {
        (400..=500, true) if weight.min <= 500 => 1,
        (400..=500, false) => 2,
        (400..=500, true) => 3,
        (..400, false) | (501.., true) => 1,
        (..400, true) | (501.., false) => 2,
    };
    (tier, distance)
}

/// The index of the face the browser picks, or `None` when there are no faces.
///
/// Style narrows first: an italic request takes the italic faces if there are any, else the normal ones, and a
/// normal request the reverse. Weight then picks within them by [`weight_rank`]; of equal faces the first wins.
pub fn css_match<I>(faces: I, weight: u32, style: FaceStyle) -> Option<usize>
where
    I: IntoIterator<Item = FaceDescriptor>,
    I::IntoIter: Clone,
{
    let faces = faces.into_iter();
    let wanted = if faces.clone().any(|face| face.style == style) {
        style
    } else {
        match style {
            FaceStyle::Normal => FaceStyle::Italic,
            FaceStyle::Italic => FaceStyle::Normal,
        }
    };
    faces
        .enumerate()
        .filter(|(_, face)| face.style == wanted)
        .min_by_key(|(_, face)| weight_rank(face.weight, weight))
        .map(|(index, _)| index)
}

impl CatalogFamily {
    /// The face of this family the browser draws for a weight and style.
    pub fn css_match(&self, weight: u32, style: FaceStyle) -> Option<&CatalogFace> {
        let index = css_match(self.faces.iter().map(FaceDescriptor::from), weight, style)?;
        self.faces.get(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statics(faces: &[(u32, FaceStyle)]) -> Vec<FaceDescriptor> {
        faces
            .iter()
            .map(|&(weight, style)| FaceDescriptor {
                weight: WeightRange::point(weight),
                style,
            })
            .collect()
    }

    /// The weight picked from upright static faces of these weights.
    fn pick(weights: &[u32], desired: u32) -> Option<u32> {
        let faces: Vec<_> = weights.iter().map(|&w| (w, FaceStyle::Normal)).collect();
        css_match(statics(&faces), desired, FaceStyle::Normal).map(|i| weights[i])
    }

    #[test]
    fn bold_from_semibold_and_black_takes_black() {
        // Nearest-weight matching would pick 600; CSS checks heavier weights first above 500.
        assert_eq!(pick(&[600, 900], 700), Some(900));
        assert_eq!(pick(&[400, 600, 900], 700), Some(900));
        assert_eq!(pick(&[400, 600], 700), Some(600));
    }

    #[test]
    fn exact_weights_win() {
        for desired in [100, 400, 450, 500, 700, 900] {
            assert_eq!(
                pick(&[100, 400, 450, 500, 700, 900], desired),
                Some(desired)
            );
        }
    }

    #[test]
    fn between_400_and_500_looks_up_to_500_then_lighter_then_heavier() {
        assert_eq!(pick(&[300, 500], 400), Some(500));
        assert_eq!(pick(&[300, 450, 500], 400), Some(450));
        assert_eq!(pick(&[300, 600], 400), Some(300));
        assert_eq!(pick(&[200, 300, 600], 450), Some(300));
        assert_eq!(pick(&[600, 700], 500), Some(600));
        assert_eq!(pick(&[400, 600], 500), Some(400));
    }

    #[test]
    fn below_400_looks_lighter_first() {
        assert_eq!(pick(&[200, 400], 300), Some(200));
        assert_eq!(pick(&[100, 200, 400], 300), Some(200));
        assert_eq!(pick(&[400, 500], 300), Some(400));
    }

    #[test]
    fn above_500_looks_heavier_first() {
        assert_eq!(pick(&[500, 700], 600), Some(700));
        assert_eq!(pick(&[400, 500], 600), Some(500));
        assert_eq!(pick(&[800, 900], 700), Some(800));
    }

    #[test]
    fn style_narrows_before_weight() {
        use FaceStyle::{Italic, Normal};
        let faces = statics(&[(700, Normal), (400, Italic)]);
        assert_eq!(css_match(faces.clone(), 700, Italic), Some(1));
        assert_eq!(css_match(faces.clone(), 400, Normal), Some(0));
        let upright = statics(&[(400, Normal), (700, Normal)]);
        assert_eq!(css_match(upright, 700, Italic), Some(1));
        let italics = statics(&[(400, Italic), (700, Italic)]);
        assert_eq!(css_match(italics, 400, Normal), Some(0));
        assert_eq!(css_match(Vec::new(), 400, Normal), None);
    }

    #[test]
    fn a_range_matches_inside_and_by_its_nearest_end_outside() {
        let variable = |min, max| FaceDescriptor {
            weight: WeightRange::between(min, max),
            style: FaceStyle::Normal,
        };
        let point = |weight| FaceDescriptor {
            weight: WeightRange::point(weight),
            style: FaceStyle::Normal,
        };
        assert_eq!(
            css_match([point(400), variable(600, 800)], 700, FaceStyle::Normal),
            Some(1)
        );
        assert_eq!(
            css_match([variable(100, 300), point(900)], 700, FaceStyle::Normal),
            Some(1)
        );
        assert_eq!(
            css_match([variable(100, 300), point(350)], 700, FaceStyle::Normal),
            Some(1)
        );
        assert_eq!(
            css_match([variable(450, 600), point(400)], 420, FaceStyle::Normal),
            Some(0)
        );
        assert_eq!(
            css_match([variable(520, 600), point(300)], 420, FaceStyle::Normal),
            Some(1)
        );
        assert_eq!(
            WeightRange::between(800, 200),
            WeightRange { min: 200, max: 800 }
        );
    }

    #[test]
    fn normal_stretch_takes_normal_then_narrower_descending_then_wider_ascending() {
        let order = |desired: u16| {
            let mut widths: Vec<u16> = (0..=10).collect();
            widths.sort_by_key(|&width| (stretch_rank(width, desired), width));
            widths
        };
        assert_eq!(order(NORMAL_WIDTH), [5, 4, 3, 2, 1, 6, 7, 8, 9, 0, 10]);
        assert_eq!(order(3), [3, 2, 1, 4, 5, 6, 7, 8, 9, 0, 10]);
        assert_eq!(order(7), [7, 8, 9, 6, 5, 4, 3, 2, 1, 0, 10]);
        // Condensed (3) comes before semi-expanded (6), though it is further from normal.
        assert!(stretch_rank(3, NORMAL_WIDTH) < stretch_rank(6, NORMAL_WIDTH));
        assert_eq!(
            stretch_rank(0, NORMAL_WIDTH),
            stretch_rank(12, NORMAL_WIDTH)
        );
    }

    #[test]
    fn the_first_of_equal_faces_wins() {
        let faces = statics(&[(400, FaceStyle::Normal), (400, FaceStyle::Normal)]);
        assert_eq!(css_match(faces, 400, FaceStyle::Normal), Some(0));
    }
}
