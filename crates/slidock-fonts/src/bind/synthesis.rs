//! What a browser fakes when the face it matched lacks the weight or style a run asks for (CSS Fonts 4 §5.2 and
//! `font-synthesis`): bold when the run asks for 600 or more and the face's heaviest weight is below 600, oblique
//! when the run asks for italic and the face is upright. With true descriptors on every shipped face, a bold run
//! drawn with a regular file is emboldened exactly where the IR's `synthesizeBold` lets it be (the backend prints
//! `font-synthesis-weight:none` elsewhere), as PowerPoint and Google Slides do; Keynote never fakes bold.

use slidock_ir::fonts::FaceStyle;

use super::FaceDescriptor;

/// The weight from which a run counts as bold.
const BOLD: u32 = 600;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Synthesis {
    pub bold: bool,
    pub italic: bool,
}

/// What the browser synthesizes drawing a run of `weight` and `style` with `face`. `bold_allowed` is the run's
/// `synthesizeBold`; italic synthesis is always allowed (the IR has no flag for it, and PowerPoint fakes italic
/// too).
pub fn synthesis(
    face: FaceDescriptor,
    weight: u32,
    style: FaceStyle,
    bold_allowed: bool,
) -> Synthesis {
    Synthesis {
        bold: bold_allowed && weight >= BOLD && face.weight.max < BOLD,
        italic: style == FaceStyle::Italic && face.style == FaceStyle::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind::WeightRange;

    fn face(min: u32, max: u32, style: FaceStyle) -> FaceDescriptor {
        FaceDescriptor {
            weight: WeightRange::between(min, max),
            style,
        }
    }

    #[test]
    fn bold_is_faked_only_from_faces_lighter_than_600() {
        let regular = face(400, 400, FaceStyle::Normal);
        assert!(synthesis(regular, 700, FaceStyle::Normal, true).bold);
        assert!(synthesis(regular, 600, FaceStyle::Normal, true).bold);
        assert!(!synthesis(regular, 500, FaceStyle::Normal, true).bold);
        assert!(!synthesis(regular, 700, FaceStyle::Normal, false).bold);
        assert!(
            !synthesis(
                face(600, 600, FaceStyle::Normal),
                900,
                FaceStyle::Normal,
                true
            )
            .bold
        );
        assert!(
            !synthesis(
                face(200, 700, FaceStyle::Normal),
                900,
                FaceStyle::Normal,
                true
            )
            .bold
        );
        assert!(
            synthesis(
                face(100, 500, FaceStyle::Normal),
                700,
                FaceStyle::Normal,
                true
            )
            .bold
        );
    }

    #[test]
    fn italic_is_faked_only_from_upright_faces() {
        let upright = face(400, 400, FaceStyle::Normal);
        let italic = face(400, 400, FaceStyle::Italic);
        assert!(synthesis(upright, 400, FaceStyle::Italic, false).italic);
        assert!(!synthesis(italic, 400, FaceStyle::Italic, false).italic);
        assert!(!synthesis(italic, 400, FaceStyle::Normal, false).italic);
    }
}
