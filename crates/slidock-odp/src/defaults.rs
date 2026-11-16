//! The frontend's defaults, under every style chain: what an ODF consumer assumes until a style states a value. They
//! exist twice, in ODF's terms (the root layer of style resolution) and as the presentation's IR defaults, and state
//! the same values: 18 pt black Liberation Sans, flush left, single spacing, LibreOffice's 1.27 cm tab grid and its
//! text frame padding of 0.25 cm by 0.125 cm, top-anchored and wrapped.

use slidock_ir::color::Color;
use slidock_ir::text::{
    Align, Anchor, Autofit, BodyProps, Bullet, Caps, Decoration, DefaultTextStyles, FamilyFont,
    FontRef, Insets, LevelDefaults, LineBasis, LineSpacing, Overflow, ParaProps, RunProps, Spacing,
    Wrap,
};

use crate::names::qn;
use crate::styles::{Props, props_of};
use crate::units::{pt, ratio_of, rgb, size, unit_ratio};

/// What "single" line spacing means in the presentations this frontend reads: calibrated against the Google exports'
/// oracle PDFs, where 100% and 115% pitch at 1.2 × and 1.38 × the size, like PowerPoint's.
pub(crate) const LINE_BASIS: LineBasis = LineBasis::Size1_2;

/// The root layer of style resolution.
pub(crate) fn root_props<'a>() -> Props<'a> {
    props_of(
        &[
            (qn!("draw:fill"), "none"),
            (qn!("draw:stroke"), "none"),
            (qn!("fo:padding-left"), "0.25cm"),
            (qn!("fo:padding-right"), "0.25cm"),
            (qn!("fo:padding-top"), "0.125cm"),
            (qn!("fo:padding-bottom"), "0.125cm"),
            (qn!("draw:textarea-vertical-align"), "top"),
            (qn!("fo:wrap-option"), "wrap"),
        ],
        &[
            (qn!("fo:text-align"), "start"),
            (qn!("fo:margin-left"), "0cm"),
            (qn!("fo:margin-right"), "0cm"),
            (qn!("fo:text-indent"), "0cm"),
            (qn!("fo:margin-top"), "0cm"),
            (qn!("fo:margin-bottom"), "0cm"),
            (qn!("fo:line-height"), "100%"),
            (qn!("style:tab-stop-distance"), "1.27cm"),
        ],
        &[
            (qn!("fo:font-family"), "Liberation Sans"),
            (qn!("fo:font-size"), "18pt"),
            (qn!("fo:font-weight"), "normal"),
            (qn!("fo:font-style"), "normal"),
            (qn!("fo:color"), "#000000"),
        ],
    )
}

fn para() -> ParaProps {
    ParaProps {
        align: Align::Left,
        margin_left: pt(0.0),
        margin_right: pt(0.0),
        indent: pt(0.0),
        space_before: Spacing::Pt { value: pt(0.0) },
        space_after: Spacing::Pt { value: pt(0.0) },
        line_spacing: LineSpacing::Multiple {
            value: ratio_of(1.0),
            basis: LINE_BASIS,
        },
        tab_size: size(36.0),
        bullet: Bullet::None {},
        rtl: false,
        line_height: None,
        space_before_pt: None,
        space_after_pt: None,
        leading_skew: None,
        line_tail_pt: None,
    }
}

fn run() -> RunProps {
    RunProps {
        font: FontRef::Family(FamilyFont::new("Liberation Sans")),
        size: size(18.0),
        weight: 400,
        italic: false,
        underline: Decoration::None,
        strike: Decoration::None,
        color: Color::Rgb(rgb(0.0, 0.0, 0.0, 1.0)),
        highlight: None,
        baseline: ratio_of(0.0),
        caps: Caps::None,
        letter_spacing: pt(0.0),
        link: None,
        synthesize_bold: true,
        // The size a `style:text-position` that omits one falls back to, as LibreOffice draws it.
        script_scale: unit_ratio(0.58),
        shadow: None,
    }
}

/// Every level's defaults.
pub(crate) fn default_text() -> DefaultTextStyles {
    let level = || LevelDefaults {
        para: para(),
        run: run(),
    };
    DefaultTextStyles {
        l0: level(),
        l1: level(),
        l2: level(),
        l3: level(),
        l4: level(),
        l5: level(),
        l6: level(),
        l7: level(),
        l8: level(),
    }
}

pub(crate) fn default_body() -> BodyProps<Autofit> {
    BodyProps {
        insets: Insets {
            l: pt(7.087),
            t: pt(3.543),
            r: pt(7.087),
            b: pt(3.543),
        },
        anchor: Anchor::Top,
        wrap: Wrap::Square,
        autofit: Autofit::None {},
        overflow: Overflow::Spread,
        max_width: None,
    }
}
