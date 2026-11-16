//! The callout probe decks: 960 × 540 pt slides, four callouts per slide (each its own fill), each slide named by a
//! 12 pt Liberation Sans label in its top-left corner. Every drawing is our own. Unless a case says otherwise a callout
//! is a 180 × 100 pt body with a 2 pt #1F2937 line.
//!
//! `callout-probe.pptx`, ECMA-376's callout presets with their adjustments (`adj1`/`adj2` place the tail's tip, in
//! 1/100000 of the width and height from the centre; LibreOffice's PDF is the oracle):
//!
//! - P01–P02: `wedgeRectCallout` with its tip left, right, above and below; inside the body, far outside, and on both
//!   diagonals (at the corners).
//! - P03–P05: `wedgeRoundRectCallout` at corner radii 0, 16667, 35000 and 50000; tips on the four sides; inside, far
//!   outside and on the diagonals.
//! - P06: `wedgeEllipseCallout` with its tip left, above right, inside and far below.
//! - P07: `cloudCallout` with its tip below left, above right, inside and far right.
//! - P08–P10: the line callouts: `borderCallout1`–`3`, `callout1`–`3`, `accentCallout1`–`3` and
//!   `accentBorderCallout1`–`3`.
//! - P11: turned 30°, 135°, 90° and 315°.
//! - P12: mirrored horizontally, vertically and both; an ellipse callout mirrored horizontally.
//! - P13: text in the body; an outer shadow; a fill at 50 %; an 8 pt line.
//! - P14: each wedge callout and the cloud with its default adjustments.
//!
//! `callout-probe.odp`, LibreOffice's own callout types (`rectangular-callout`, `round-rectangular-callout`,
//! `round-callout`, `cloud-callout`, `line-callout-1`–`3`) as LibreOffice writes a shape: its `draw:type`, its
//! `draw:modifiers` (the tip, in 21600ths of the box) and the enhanced path and equations that draw it, here of our
//! own making, so every path command and formula function is drawn by LibreOffice from the file:
//!
//! - O01–O02: rectangular callouts with their tips on the four sides; inside, far outside and past two corners.
//! - O03–O04: rounded ones (`X`/`Y` quarter ellipses) likewise.
//! - O05: round callouts drawn with each arc command (`W`, `B`, `A`, `V`).
//! - O06: cloud callouts (`C` bumps, `U` bubbles toward the tip, unfilled `F` strokes).
//! - O07: line callouts (an unfilled leader, an `S` accent bar).
//! - O08: the other commands and functions: `X` and `Y` alternating, `T` both ways, `Q`, and `min`, `max`, `sqrt`,
//!   `sin`, `cos`, `tan`, `atan`, `width`, `height`, `right`, `bottom`, `left`, `logwidth`.
//! - O09: turned 30° and 135°, mirrored horizontally and vertically.
//! - O10: text in the body.

use std::fmt::Write as _;
use std::path::Path;

use crate::probes_pictures::probes_zip::{self, Entry};
use crate::probes_shadows::{
    LABEL, OPAQUE, OdpDeck, OdpStyles, PARAGRAPH, PptxDeck, PptxSlide, SLIDE, Shadow, emu, escape,
    odf_color, odf_label, odp_package, outer, page_style, placement, pptx_package, solid,
    standard_styles, text_lines, text_run, xfrm,
};

const USAGE: &str = "usage: cargo xtask probes-callouts [--check]";

const INK: [u8; 3] = [0x1F, 0x29, 0x37];
/// The four fills of a slide's callouts, in slot order.
const FILLS: [[u8; 3]; 4] = [
    [0x4F, 0x9D, 0xDE],
    [0xF9, 0xA8, 0x25],
    [0x6A, 0xBF, 0x69],
    [0xE0, 0x6C, 0x9F],
];
const BLACK: [u8; 3] = [0, 0, 0];

/// The four slots of a slide: the top left of each 180 × 100 pt body.
const SLOTS: [(f64, f64); 4] = [
    (130.0, 110.0),
    (620.0, 110.0),
    (130.0, 340.0),
    (620.0, 340.0),
];
const SIZE: (f64, f64) = (180.0, 100.0);

pub fn run(args: &[String]) -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../probes/out");
    match args.first().map(String::as_str) {
        None => {
            for (name, bytes) in generate() {
                let path = root.join(&name);
                std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
                println!(
                    "wrote probes/out/{name} ({} bytes, sha256 {})",
                    bytes.len(),
                    probes_zip::sha256_hex(&bytes)
                );
            }
            Ok(())
        }
        Some("--check") => {
            let stale = stale(&root);
            if stale.is_empty() {
                println!("probes/out callout files are up to date");
                Ok(())
            } else {
                Err(format!(
                    "stale (run cargo xtask probes-callouts): {}",
                    stale.join(", ")
                ))
            }
        }
        _ => Err(USAGE.to_owned()),
    }
}

/// The generated files that differ from the ones under `root`.
fn stale(root: &Path) -> Vec<String> {
    generate()
        .into_iter()
        .filter(|(name, bytes)| std::fs::read(root.join(name)).ok().as_ref() != Some(bytes))
        .map(|(name, _)| name)
        .collect()
}

/// Every generated file: (name under `probes/out/`, bytes).
pub fn generate() -> Vec<(String, Vec<u8>)> {
    vec![
        (
            "callout-probe.pptx".to_owned(),
            probes_zip::write_zip(&pptx()),
        ),
        (
            "callout-probe.odp".to_owned(),
            probes_zip::write_zip(&odp()),
        ),
    ]
}

// ---------------------------------------------------------------------------------------------------------------------
// PPTX

/// One PPTX callout: its preset and adjustments (none: the preset's defaults) and how it differs from the plain case.
#[derive(Clone, Copy)]
struct Callout {
    preset: &'static str,
    adjust: &'static [(&'static str, i32)],
    rot: i32,
    flip_h: bool,
    flip_v: bool,
    text: Option<&'static str>,
    shadow: bool,
    alpha: u32,
    line: f64,
}

const fn callout(preset: &'static str, adjust: &'static [(&'static str, i32)]) -> Callout {
    Callout {
        preset,
        adjust,
        rot: 0,
        flip_h: false,
        flip_v: false,
        text: None,
        shadow: false,
        alpha: OPAQUE,
        line: 2.0,
    }
}

const fn turned(preset: &'static str, adjust: &'static [(&'static str, i32)], rot: i32) -> Callout {
    let mut c = callout(preset, adjust);
    c.rot = rot;
    c
}

const fn mirrored(
    preset: &'static str,
    adjust: &'static [(&'static str, i32)],
    flip_h: bool,
    flip_v: bool,
) -> Callout {
    let mut c = callout(preset, adjust);
    c.flip_h = flip_h;
    c.flip_v = flip_v;
    c
}

const RECT: &str = "wedgeRectCallout";
const ROUND: &str = "wedgeRoundRectCallout";
const ELLIPSE: &str = "wedgeEllipseCallout";
const CLOUD: &str = "cloudCallout";
const TAIL: &[(&str, i32)] = &[("adj1", -20833), ("adj2", 62500), ("adj3", 16667)];

fn pptx_slides() -> Vec<(&'static str, [Callout; 4])> {
    vec![
        (
            "P01 wedgeRectCallout: tip left, right, above, below",
            [
                callout(RECT, &[("adj1", -75000), ("adj2", 10000)]),
                callout(RECT, &[("adj1", 78000), ("adj2", -25000)]),
                callout(RECT, &[("adj1", -10000), ("adj2", -95000)]),
                callout(RECT, &[("adj1", 25000), ("adj2", 90000)]),
            ],
        ),
        (
            "P02 wedgeRectCallout: tip inside, far outside, on both diagonals",
            [
                callout(RECT, &[("adj1", 10000), ("adj2", -15000)]),
                callout(RECT, &[("adj1", 90000), ("adj2", 160000)]),
                callout(RECT, &[("adj1", -70000), ("adj2", -70000)]),
                callout(RECT, &[("adj1", 65000), ("adj2", 65000)]),
            ],
        ),
        (
            "P03 wedgeRoundRectCallout: corner radius 0, 16667, 35000, 50000",
            [
                callout(ROUND, &[("adj1", -20833), ("adj2", 62500), ("adj3", 0)]),
                callout(ROUND, TAIL),
                callout(ROUND, &[("adj1", -20833), ("adj2", 62500), ("adj3", 35000)]),
                callout(ROUND, &[("adj1", -20833), ("adj2", 62500), ("adj3", 50000)]),
            ],
        ),
        (
            "P04 wedgeRoundRectCallout: tip left, right, above, below",
            [
                callout(ROUND, &[("adj1", -75000), ("adj2", 10000), ("adj3", 16667)]),
                callout(ROUND, &[("adj1", 78000), ("adj2", -25000), ("adj3", 16667)]),
                callout(
                    ROUND,
                    &[("adj1", -10000), ("adj2", -95000), ("adj3", 16667)],
                ),
                callout(ROUND, &[("adj1", 25000), ("adj2", 90000), ("adj3", 16667)]),
            ],
        ),
        (
            "P05 wedgeRoundRectCallout: tip inside, far outside, on both diagonals",
            [
                callout(ROUND, &[("adj1", 10000), ("adj2", -15000), ("adj3", 16667)]),
                callout(ROUND, &[("adj1", 90000), ("adj2", 160000), ("adj3", 16667)]),
                callout(
                    ROUND,
                    &[("adj1", -70000), ("adj2", -70000), ("adj3", 16667)],
                ),
                callout(ROUND, &[("adj1", 65000), ("adj2", 65000), ("adj3", 30000)]),
            ],
        ),
        (
            "P06 wedgeEllipseCallout: tip left, above right, inside, far below",
            [
                callout(ELLIPSE, &[("adj1", -80000), ("adj2", 20000)]),
                callout(ELLIPSE, &[("adj1", 60000), ("adj2", -75000)]),
                callout(ELLIPSE, &[("adj1", 15000), ("adj2", 10000)]),
                callout(ELLIPSE, &[("adj1", -5000), ("adj2", 150000)]),
            ],
        ),
        (
            "P07 cloudCallout: tip below left, above right, inside, far right",
            [
                callout(CLOUD, &[("adj1", -45000), ("adj2", 85000)]),
                callout(CLOUD, &[("adj1", 55000), ("adj2", -80000)]),
                callout(CLOUD, &[("adj1", 10000), ("adj2", 15000)]),
                callout(CLOUD, &[("adj1", 120000), ("adj2", 20000)]),
            ],
        ),
        (
            "P08 borderCallout1, borderCallout2, borderCallout3, callout1",
            [
                callout(
                    "borderCallout1",
                    &[
                        ("adj1", 18750),
                        ("adj2", -8333),
                        ("adj3", 112500),
                        ("adj4", -38333),
                    ],
                ),
                callout(
                    "borderCallout2",
                    &[
                        ("adj1", 18750),
                        ("adj2", -8333),
                        ("adj3", 18750),
                        ("adj4", -16667),
                        ("adj5", 112500),
                        ("adj6", -46667),
                    ],
                ),
                callout(
                    "borderCallout3",
                    &[
                        ("adj1", 18750),
                        ("adj2", 108333),
                        ("adj3", 18750),
                        ("adj4", 116667),
                        ("adj5", 100000),
                        ("adj6", 116667),
                        ("adj7", 130000),
                        ("adj8", 105000),
                    ],
                ),
                callout(
                    "callout1",
                    &[
                        ("adj1", 18750),
                        ("adj2", -8333),
                        ("adj3", -40000),
                        ("adj4", -20000),
                    ],
                ),
            ],
        ),
        (
            "P09 accentCallout1, accentCallout2, accentCallout3, callout2",
            [
                callout(
                    "accentCallout1",
                    &[
                        ("adj1", 18750),
                        ("adj2", -8333),
                        ("adj3", 112500),
                        ("adj4", -38333),
                    ],
                ),
                callout("accentCallout2", &[]),
                callout("accentCallout3", &[]),
                callout("callout2", &[]),
            ],
        ),
        (
            "P10 accentBorderCallout1, accentBorderCallout2, accentBorderCallout3, callout3",
            [
                callout("accentBorderCallout1", &[]),
                callout("accentBorderCallout2", &[]),
                callout("accentBorderCallout3", &[]),
                callout("callout3", &[]),
            ],
        ),
        (
            "P11 turned 30, 135, 90 and 315 degrees",
            [
                turned(ROUND, TAIL, 30),
                turned(RECT, &[("adj1", -75000), ("adj2", 10000)], 135),
                turned(ELLIPSE, &[("adj1", 60000), ("adj2", -75000)], 90),
                turned(CLOUD, &[("adj1", -45000), ("adj2", 85000)], 315),
            ],
        ),
        (
            "P12 mirrored horizontally, vertically, both; ellipse mirrored horizontally",
            [
                mirrored(ROUND, TAIL, true, false),
                mirrored(ROUND, TAIL, false, true),
                mirrored(ROUND, TAIL, true, true),
                mirrored(ELLIPSE, &[("adj1", 60000), ("adj2", -75000)], true, false),
            ],
        ),
        (
            "P13 text in the body, an outer shadow, a fill at 50 percent, an 8 pt line",
            [
                Callout {
                    text: Some("Text in the body"),
                    ..callout(ROUND, TAIL)
                },
                Callout {
                    shadow: true,
                    ..callout(RECT, &[("adj1", 60000), ("adj2", 80000)])
                },
                Callout {
                    alpha: 50_000,
                    ..callout(ELLIPSE, &[("adj1", -60000), ("adj2", 80000)])
                },
                Callout {
                    line: 8.0,
                    ..callout(ROUND, &[("adj1", 25000), ("adj2", 90000), ("adj3", 16667)])
                },
            ],
        ),
        (
            "P14 default adjustments: wedgeRectCallout, wedgeRoundRectCallout, wedgeEllipseCallout, cloudCallout",
            [
                callout(RECT, &[]),
                callout(ROUND, &[]),
                callout(ELLIPSE, &[]),
                callout(CLOUD, &[]),
            ],
        ),
    ]
}

/// A shadow 12 pt away at 45°, black at 50 %, unblurred (LibreOffice prints it as vectors).
const SHADOW: Shadow = Shadow {
    dist: 12.0,
    dir: 45,
    blur: 0.0,
    color: BLACK,
    alpha: 50_000,
};

fn pptx_callout(slide: &mut PptxSlide, slot: usize, c: &Callout) -> String {
    let id = slide.id();
    let (x, y) = SLOTS.get(slot).copied().unwrap_or_default();
    let fill = FILLS.get(slot).copied().unwrap_or(INK);
    let mut gd = String::new();
    for (name, value) in c.adjust {
        let _ = write!(gd, "<a:gd name=\"{name}\" fmla=\"val {value}\"/>");
    }
    let effect = if c.shadow {
        format!("<a:effectLst>{}</a:effectLst>", outer(SHADOW))
    } else {
        String::new()
    };
    let body = c.text.map_or_else(String::new, |text| {
        format!(
            "<p:txBody><a:bodyPr wrap=\"square\" lIns=\"91440\" tIns=\"45720\" rIns=\"91440\" bIns=\"45720\" anchor=\"ctr\" rtlCol=\"0\"><a:noAutofit/></a:bodyPr><a:lstStyle/><a:p><a:pPr algn=\"ctr\"/>{}</a:p></p:txBody>",
            text_run(text, 18, INK, "")
        )
    });
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{} {}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr>{}<a:prstGeom prst=\"{}\"><a:avLst>{gd}</a:avLst></a:prstGeom>{}<a:ln w=\"{}\">{}</a:ln>{effect}</p:spPr>{body}</p:sp>",
        c.preset,
        slot + 1,
        xfrm((x, y, SIZE.0, SIZE.1), c.rot, c.flip_h, c.flip_v),
        c.preset,
        solid(fill, c.alpha),
        emu(c.line),
        solid(INK, OPAQUE)
    )
}

fn pptx() -> Vec<Entry> {
    let mut written = Vec::new();
    for (label, callouts) in pptx_slides() {
        let mut slide = PptxSlide::new(1, None);
        slide.label(label, LABEL);
        for (slot, c) in callouts.iter().enumerate() {
            let xml = pptx_callout(&mut slide, slot, c);
            slide.tree.push_str(&xml);
        }
        written.push(slide);
    }
    pptx_package(&PptxDeck {
        theme_effects: &"<a:effectStyle><a:effectLst/></a:effectStyle>".repeat(3),
        master_tree: "",
        layouts: vec![String::new()],
        slides: written,
        media: Vec::new(),
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// ODP

/// The tip's offsets from the centre, the sides it may lie beyond, and the points of each side's slot: the tip
/// when the tail leaves that side, else the side's own point (a straight edge).
const SIDES: &[&str] = &[
    "$0 -10800",
    "$1 -10800",
    "abs(?f0 )-abs(?f1 )",
    "if(?f2 ,-1,-?f1 )",
    "if(?f2 ,?f0 ,-1)",
    "if(?f2 ,-1,?f1 )",
    "if(?f2 ,-?f0 ,-1)",
    "if(?f3 ,$0 ,8100)",
    "if(?f3 ,$1 ,0)",
    "if(?f4 ,$0 ,21600)",
    "if(?f4 ,$1 ,8100)",
    "if(?f5 ,$0 ,13500)",
    "if(?f5 ,$1 ,21600)",
    "if(?f6 ,$0 ,0)",
    "if(?f6 ,$1 ,13500)",
];

const RECT_PATH: &str = "M 0 0 L 5400 0 ?f7 ?f8 10800 0 21600 0 21600 5400 ?f9 ?f10 21600 10800 21600 21600 16200 21600 ?f11 ?f12 10800 21600 0 21600 0 16200 ?f13 ?f14 0 10800 Z N";

const ROUND_PATH: &str = "M 3600 0 L 5400 0 ?f7 ?f8 10800 0 18000 0 X 21600 3600 L 21600 5400 ?f9 ?f10 21600 10800 21600 18000 Y 18000 21600 L 16200 21600 ?f11 ?f12 10800 21600 3600 21600 X 0 18000 L 0 16200 ?f13 ?f14 0 10800 0 3600 Y 3600 0 Z N";

/// The tip's direction, and the ellipse points 12° either side of it, where the tail meets the body.
const ROUND_CALLOUT: &[&str] = &[
    "$0 -10800",
    "$1 -10800",
    "atan2(?f1 ,?f0 )",
    "?f2 +0.21",
    "?f2 -0.21",
    "10800+10800*cos(?f3 )",
    "10800+10800*sin(?f3 )",
    "10800+10800*cos(?f4 )",
    "10800+10800*sin(?f4 )",
];

/// The cloud's bubbles, on the way from its centre to the tip.
const CLOUD_BUBBLES: &[&str] = &[
    "$0 -10800",
    "$1 -10800",
    "10800+?f0 *0.6",
    "10800+?f1 *0.6",
    "10800+?f0 *0.78",
    "10800+?f1 *0.78",
    "10800+?f0 *0.93",
    "10800+?f1 *0.93",
];

const CLOUD_PATH: &str = "M 10800 2600 C 13420 -742 18802 1685 17836 5687 C 22069 5522 23399 10977 19574 12625 C 22233 15760 18508 20135 14705 18188 C 13787 22264 7813 22264 6895 18188 C 3092 20135 -633 15760 2026 12625 C -1799 10977 -469 5522 3764 5687 C 2798 1685 8180 -742 10800 2600 Z N U ?f2 ?f3 1500 1500 0 360 Z N U ?f4 ?f5 1000 1000 0 360 Z N U ?f6 ?f7 600 600 0 360 Z N M 5200 6000 C 5900 5300 7100 5100 8000 5500 F N M 13000 14800 C 14100 15400 15300 15200 16200 14300 F N";

/// The formulas of the commands slide: `min`, `max`, `sqrt`, `sin`, `cos`, `tan`, `atan` and the box's constants.
const FUNCTIONS: &[&str] = &[
    "min(width /4,8000)",
    "max(height /3,1000)",
    "sqrt(right *bottom )/2",
    "10800+8000*sin(pi /6)",
    "10800+8000*cos(pi /3)",
    "atan(1)*4/pi *21600",
    "tan(pi /4)*left +logwidth /logwidth *1000",
    "-?f6 +?f5 ",
];

/// One ODP shape: its type, modifiers, path and equations, and how it is placed.
#[derive(Clone, Copy)]
struct Drawn {
    kind: &'static str,
    modifiers: &'static str,
    path: &'static str,
    equations: &'static [&'static str],
    rot: i32,
    mirror_h: bool,
    mirror_v: bool,
    text: Option<&'static str>,
}

const fn drawn(
    kind: &'static str,
    modifiers: &'static str,
    path: &'static str,
    equations: &'static [&'static str],
) -> Drawn {
    Drawn {
        kind,
        modifiers,
        path,
        equations,
        rot: 0,
        mirror_h: false,
        mirror_v: false,
        text: None,
    }
}

const fn rect(modifiers: &'static str) -> Drawn {
    drawn("rectangular-callout", modifiers, RECT_PATH, SIDES)
}

const fn round_rect(modifiers: &'static str) -> Drawn {
    drawn("round-rectangular-callout", modifiers, ROUND_PATH, SIDES)
}

const fn cloud(modifiers: &'static str) -> Drawn {
    drawn("cloud-callout", modifiers, CLOUD_PATH, CLOUD_BUBBLES)
}

fn odp_slides() -> Vec<(&'static str, [Drawn; 4])> {
    vec![
        (
            "O01 rectangular-callout: tip left, right, above, below",
            [
                rect("-4000 12000"),
                rect("26000 7000"),
                rect("9000 -6000"),
                rect("14000 27000"),
            ],
        ),
        (
            "O02 rectangular-callout: tip inside, far outside, past two corners",
            [
                rect("12500 9000"),
                rect("18000 45000"),
                rect("-5000 -4000"),
                rect("27000 26000"),
            ],
        ),
        (
            "O03 round-rectangular-callout: tip left, right, above, below",
            [
                round_rect("-4000 12000"),
                round_rect("26000 7000"),
                round_rect("9000 -6000"),
                round_rect("14000 27000"),
            ],
        ),
        (
            "O04 round-rectangular-callout: tip inside, far outside, past two corners",
            [
                round_rect("12500 9000"),
                round_rect("18000 45000"),
                round_rect("-5000 -4000"),
                round_rect("27000 26000"),
            ],
        ),
        (
            "O05 round-callout drawn with W, B, A and V",
            [
                drawn(
                    "round-callout",
                    "-3000 25000",
                    "W 0 0 21600 21600 ?f5 ?f6 ?f7 ?f8 L $0 $1 Z N",
                    ROUND_CALLOUT,
                ),
                drawn(
                    "round-callout",
                    "27000 -3000",
                    "B 0 0 21600 21600 ?f7 ?f8 ?f5 ?f6 L $0 $1 Z N",
                    ROUND_CALLOUT,
                ),
                drawn(
                    "round-callout",
                    "-5000 4000",
                    "M $0 $1 A 0 0 21600 21600 ?f7 ?f8 ?f5 ?f6 Z N",
                    ROUND_CALLOUT,
                ),
                drawn(
                    "round-callout",
                    "12000 14000",
                    "V 0 0 21600 21600 ?f5 ?f6 ?f7 ?f8 L $0 $1 Z N",
                    ROUND_CALLOUT,
                ),
            ],
        ),
        (
            "O06 cloud-callout: tip below left, above right, inside, far right",
            [
                cloud("2000 30000"),
                cloud("23000 -6000"),
                cloud("13000 12500"),
                cloud("40000 13000"),
            ],
        ),
        (
            "O07 line-callout-1, line-callout-2, line-callout-3, an accent bar",
            [
                drawn(
                    "line-callout-1",
                    "-1800 4000 -8000 30000",
                    "M 0 0 L 21600 0 21600 21600 0 21600 Z N M $0 $1 L $2 $3 N",
                    &[],
                ),
                drawn(
                    "line-callout-2",
                    "-1800 4000 -5000 4000 -9000 28000",
                    "M 0 0 L 21600 0 21600 21600 0 21600 Z N M $0 $1 L $2 $3 $4 $5 F N",
                    &[],
                ),
                drawn(
                    "line-callout-3",
                    "23400 17600 27000 17600 27000 -4000 31000 -9000",
                    "M 0 0 L 21600 0 21600 21600 0 21600 Z N M $0 $1 L $2 $3 $4 $5 $6 $7 F N",
                    &[],
                ),
                drawn(
                    "line-callout-2",
                    "-1800 10800 -9000 30000",
                    "M 0 0 L 21600 0 21600 21600 0 21600 Z N M -1400 0 L -500 0 -500 21600 -1400 21600 Z S N M $0 $1 L $2 $3 F N",
                    &[],
                ),
            ],
        ),
        (
            "O08 X and Y alternating, T both ways, Q, and formula functions",
            [
                drawn(
                    "non-primitive",
                    "",
                    "M 0 10800 X 10800 0 21600 10800 10800 21600 0 10800 Z N M 10800 3000 Y 18600 10800 10800 18600 3000 10800 10800 3000 Z N",
                    &[],
                ),
                drawn(
                    "non-primitive",
                    "",
                    "M 21600 21600 L 21600 10800 T 10800 10800 10800 8000 0 180 L 0 21600 Z N",
                    &[],
                ),
                drawn(
                    "non-primitive",
                    "",
                    "M 0 21600 L 0 10800 T 10800 10800 10800 8000 180 0 L 21600 21600 Z N",
                    &[],
                ),
                drawn(
                    "non-primitive",
                    "",
                    "M ?f0 0 L ?f5 ?f1 ?f7 ?f3 ?f2 21600 ?f6 ?f4 Z N M 2000 21600 Q 10800 9000 19600 21600 Z N",
                    FUNCTIONS,
                ),
            ],
        ),
        (
            "O09 turned 30 and 135 degrees, mirrored horizontally and vertically",
            [
                Drawn {
                    rot: 30,
                    ..round_rect("14000 27000")
                },
                Drawn {
                    rot: 135,
                    ..rect("-4000 12000")
                },
                Drawn {
                    mirror_h: true,
                    ..round_rect("-4000 12000")
                },
                Drawn {
                    mirror_v: true,
                    ..round_rect("9000 -6000")
                },
            ],
        ),
        (
            "O10 text in the body",
            [
                Drawn {
                    text: Some("Text in the body"),
                    ..round_rect("14000 27000")
                },
                Drawn {
                    text: Some("Text in the body"),
                    ..rect("26000 7000")
                },
                Drawn {
                    text: Some("Text"),
                    ..drawn(
                        "round-callout",
                        "-3000 25000",
                        "W 0 0 21600 21600 ?f5 ?f6 ?f7 ?f8 L $0 $1 Z N",
                        ROUND_CALLOUT,
                    )
                },
                Drawn {
                    text: Some("Text"),
                    ..cloud("2000 30000")
                },
            ],
        ),
    ]
}

fn odp_shape(styles: &mut OdpStyles, slot: usize, d: &Drawn) -> String {
    let (x, y) = SLOTS.get(slot).copied().unwrap_or_default();
    let fill = FILLS.get(slot).copied().unwrap_or(INK);
    let style = styles.graphic(
        None,
        &format!(
            "draw:fill=\"solid\" draw:fill-color=\"{}\" draw:stroke=\"solid\" svg:stroke-width=\"2pt\" svg:stroke-color=\"{}\" draw:shadow=\"hidden\" draw:textarea-vertical-align=\"middle\" draw:textarea-horizontal-align=\"center\" draw:auto-grow-height=\"false\" draw:auto-grow-width=\"false\" fo:padding-left=\"7.2pt\" fo:padding-right=\"7.2pt\" fo:padding-top=\"3.6pt\" fo:padding-bottom=\"3.6pt\"",
            odf_color(fill),
            odf_color(INK)
        ),
        "",
    );
    let text = d
        .text
        .map_or_else(String::new, |text| text_lines(styles, &[text], 18, INK));
    let mut equations = String::new();
    for (k, formula) in d.equations.iter().enumerate() {
        let _ = write!(
            equations,
            "<draw:equation draw:name=\"f{k}\" draw:formula=\"{formula}\"/>"
        );
    }
    let mut attrs = String::new();
    if !d.modifiers.is_empty() {
        let _ = write!(attrs, " draw:modifiers=\"{}\"", d.modifiers);
    }
    if d.mirror_h {
        attrs.push_str(" draw:mirror-horizontal=\"true\"");
    }
    if d.mirror_v {
        attrs.push_str(" draw:mirror-vertical=\"true\"");
    }
    format!(
        "<draw:custom-shape draw:style-name=\"{style}\" draw:text-style-name=\"Pfi\" draw:name=\"{} {}\" {}>{text}<draw:enhanced-geometry svg:viewBox=\"0 0 21600 21600\" draw:type=\"{}\"{attrs} draw:enhanced-path=\"{}\">{equations}</draw:enhanced-geometry></draw:custom-shape>",
        escape(d.kind),
        slot + 1,
        placement((x, y, SIZE.0, SIZE.1), d.rot),
        d.kind,
        d.path
    )
}

fn odp() -> Vec<Entry> {
    let mut styles = OdpStyles::default();
    styles.raw(PARAGRAPH);
    styles.raw(&page_style("dpSlide", SLIDE));
    let mut pages = String::new();
    for (k, (label, shapes)) in odp_slides().iter().enumerate() {
        let mut body = odf_label(&mut styles, label, LABEL);
        for (slot, d) in shapes.iter().enumerate() {
            body.push_str(&odp_shape(&mut styles, slot, d));
        }
        let _ = write!(
            pages,
            "<draw:page draw:name=\"O{:02}\" draw:style-name=\"dpSlide\" draw:master-page-name=\"Probe\">{body}</draw:page>",
            k + 1
        );
    }
    odp_package(&OdpDeck {
        automatic: &styles.xml,
        common: &standard_styles(""),
        master_automatic: "",
        masters: "",
        pages: &pages,
        media: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_decks_are_the_generators() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../probes/out");
        assert_eq!(stale(&root), Vec::<String>::new());
    }

    #[test]
    fn each_slide_is_labelled_in_order() {
        for (k, (label, _)) in pptx_slides().iter().enumerate() {
            assert!(label.starts_with(&format!("P{:02} ", k + 1)), "{label}");
        }
        for (k, (label, _)) in odp_slides().iter().enumerate() {
            assert!(label.starts_with(&format!("O{:02} ", k + 1)), "{label}");
        }
        let content = odp();
        let content = content.iter().find(|e| e.name == "content.xml").unwrap();
        let content = std::str::from_utf8(&content.data).unwrap();
        assert_eq!(content.matches("<draw:page ").count(), odp_slides().len());
        assert_eq!(
            content.matches("<draw:custom-shape ").count(),
            odp_slides().len() * 5
        );
    }
}
