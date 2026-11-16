//! A theme part: the twelve scheme colours and the major and minor fonts go into the IR; the format scheme (fill, line,
//! effect and background style lists) stays here, for the style references the frontend resolves itself.

use std::rc::Rc;

use slidock_ir::color::{Color, RgbColor, SchemeColors, SchemeSlot};
use slidock_ir::ids::ThemeId;
use slidock_ir::origin::Origin;
use slidock_ir::presentation::{MajorMinor, Theme, ThemeFonts};

use crate::clip;
use crate::color::{color_element, hex_color, plain_color, read_color, rgb};
use crate::report::Report;
use crate::xml::{El, Element, Ns};

/// The style lists `p:style` and `p:bgRef` index, as written (their colours hold `phClr`).
#[derive(Clone, Debug, Default)]
pub(crate) struct FormatScheme {
    pub fills: Vec<Rc<Element>>,
    pub lines: Vec<Rc<Element>>,
    pub effects: Vec<Rc<Element>>,
    pub backgrounds: Vec<Rc<Element>>,
}

#[derive(Clone, Debug)]
pub(crate) struct ThemeData {
    pub theme: Theme,
    pub formats: FormatScheme,
}

/// The scheme slots in their written order, with their element names.
const SLOTS: [(SchemeSlot, &str); 12] = [
    (SchemeSlot::Dk1, "dk1"),
    (SchemeSlot::Lt1, "lt1"),
    (SchemeSlot::Dk2, "dk2"),
    (SchemeSlot::Lt2, "lt2"),
    (SchemeSlot::Accent1, "accent1"),
    (SchemeSlot::Accent2, "accent2"),
    (SchemeSlot::Accent3, "accent3"),
    (SchemeSlot::Accent4, "accent4"),
    (SchemeSlot::Accent5, "accent5"),
    (SchemeSlot::Accent6, "accent6"),
    (SchemeSlot::Hlink, "hlink"),
    (SchemeSlot::FolHlink, "folHlink"),
];

/// Office's default scheme, for a slot a theme leaves out.
fn office(slot: SchemeSlot) -> &'static str {
    match slot {
        SchemeSlot::Dk1 => "000000",
        SchemeSlot::Lt1 => "FFFFFF",
        SchemeSlot::Dk2 => "44546A",
        SchemeSlot::Lt2 => "E7E6E6",
        SchemeSlot::Accent1 => "4472C4",
        SchemeSlot::Accent2 => "ED7D31",
        SchemeSlot::Accent3 => "A5A5A5",
        SchemeSlot::Accent4 => "FFC000",
        SchemeSlot::Accent5 => "5B9BD5",
        SchemeSlot::Accent6 => "70AD47",
        SchemeSlot::Hlink => "0563C1",
        SchemeSlot::FolHlink => "954F72",
    }
}

fn fonts(element: Option<&Element>) -> ThemeFonts {
    let typeface = |local: &str| {
        element
            .child(Ns::A, local)
            .attr("typeface")
            .map(|face| clip(face, 256))
    };
    ThemeFonts {
        latin: typeface("latin").unwrap_or_default(),
        ea: typeface("ea").filter(|face| !face.is_empty()),
        cs: typeface("cs").filter(|face| !face.is_empty()),
    }
}

/// A theme as read, and the scheme slots it leaves out: they take Office's defaults. Scheme colours are literals,
/// so their modifiers apply here.
pub(crate) fn read_theme(
    root: &Element,
    id: ThemeId,
    part: &str,
    report: &mut dyn Report,
) -> (ThemeData, Vec<&'static str>) {
    let elements = root.child(Ns::A, "themeElements");
    let scheme = elements.child(Ns::A, "clrScheme");
    let mut missing = Vec::new();
    let mut colors: Vec<RgbColor> = Vec::with_capacity(SLOTS.len());
    for (slot, name) in SLOTS {
        let color = color_element(scheme.child(Ns::A, name))
            .map(|element| plain_color(read_color(element, report)));
        match color {
            Some(Color::Rgb(color)) => colors.push(color),
            _ => {
                missing.push(name);
                colors
                    .push(hex_color(Some(office(slot))).unwrap_or_else(|| rgb(0.0, 0.0, 0.0, 1.0)));
            }
        }
    }
    let mut colors = colors.into_iter();
    let mut next = || colors.next().unwrap_or_else(|| rgb(0.0, 0.0, 0.0, 1.0));
    let colors = SchemeColors {
        dk1: next(),
        lt1: next(),
        dk2: next(),
        lt2: next(),
        accent1: next(),
        accent2: next(),
        accent3: next(),
        accent4: next(),
        accent5: next(),
        accent6: next(),
        hlink: next(),
        fol_hlink: next(),
    };
    let font_scheme = elements.child(Ns::A, "fontScheme");
    let format = elements.child(Ns::A, "fmtScheme");
    let list = |name: &str| -> Vec<Rc<Element>> {
        format
            .and_then(|format| format.child(Ns::A, name))
            .map(|list| list.kids().cloned().collect())
            .unwrap_or_default()
    };
    let theme = Theme {
        id,
        origin: Origin::source(part),
        name: root
            .attr("name")
            .filter(|name| !name.is_empty())
            .map(|name| clip(name, 256)),
        colors,
        fonts: MajorMinor {
            major: fonts(font_scheme.child(Ns::A, "majorFont")),
            minor: fonts(font_scheme.child(Ns::A, "minorFont")),
        },
        ext: None,
    };
    let data = ThemeData {
        theme,
        formats: FormatScheme {
            fills: list("fillStyleLst"),
            lines: list("lnStyleLst"),
            effects: list("effectStyleLst"),
            backgrounds: list("bgFillStyleLst"),
        },
    };
    (data, missing)
}
