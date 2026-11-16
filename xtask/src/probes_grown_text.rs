//! The grown-text probe deck (`probes/out/grown-text-probe.pptx`), made to be imported into Keynote: text boxes that
//! size to their text (`wrap="none"` with `a:spAutoFit`) inside groups, where the group's coordinates and the slide's
//! put the box's anchor at different distances from the slide's edges. A text box Keynote sizes to its text wraps at
//! the slide's edges, measured on the slide, not in its group. Every box is 24 pt Liberation Sans on a
//! 960 × 540 pt slide, its stored frame 40 pt wide and each group as wide as its arrow, as Keynote keeps them when a
//! grouped label's text grows past its group; each names what it pins:
//!
//! - "Centred in a group": centred on 680 of the slide, 120 into its group of an arrow and the label: one line about
//!   400 pt wide, under the slide's 560 and over the group's 240.
//! - "Centred, no group": the same label at the same place outside any group.
//! - "Left in a group": left-aligned at 710 of the slide, 10 into its group: about 360 pt of text past the slide's
//!   250 pt to the right edge, which wraps it there.
//! - "Right in a group": right-aligned at 500 of the slide, 200 into its group: about 270 pt of text, one line.
//! - "Centred in nested groups": centred on 340 of the slide, 100 into the inner group and 300 into the
//!   outer: about 410 pt of text, one line.

use std::fmt::Write as _;
use std::path::Path;

use crate::probes_pictures::pptx_package;
use crate::probes_pictures::probes_zip::{Entry, sha256_hex, write_zip};

const USAGE: &str = "usage: cargo xtask probes-grown-text [--check]";

/// EMU per point.
const EMU: i64 = 12_700;

const NAME: &str = "grown-text-probe.pptx";

/// A drawable of the probe, in slide points.
enum Item {
    /// A text box sized to its text: its name, its box, its alignment (`l`, `ctr`, `r`) and its text.
    Label {
        name: &'static str,
        frame: (i32, i32, i32, i32),
        align: &'static str,
        text: &'static str,
    },
    /// A filled right arrow.
    Arrow { frame: (i32, i32, i32, i32) },
    /// A group of items, its child space its own frame.
    Group {
        name: &'static str,
        frame: (i32, i32, i32, i32),
        children: Vec<Item>,
    },
}

fn items() -> Vec<Item> {
    vec![
        Item::Group {
            name: "Centred in a group",
            frame: (560, 40, 240, 100),
            children: vec![
                Item::Arrow {
                    frame: (560, 40, 240, 40),
                },
                Item::Label {
                    name: "Centred label",
                    frame: (660, 90, 40, 36),
                    align: "ctr",
                    text: "A centred label wider than its group",
                },
            ],
        },
        Item::Label {
            name: "Centred, no group",
            frame: (660, 180, 40, 36),
            align: "ctr",
            text: "A centred label wider than its group",
        },
        Item::Group {
            name: "Left in a group",
            frame: (700, 280, 200, 80),
            children: vec![
                Item::Arrow {
                    frame: (700, 280, 200, 30),
                },
                Item::Label {
                    name: "Left label",
                    frame: (710, 320, 40, 36),
                    align: "l",
                    text: "Left label running past the edge",
                },
            ],
        },
        Item::Group {
            name: "Right in a group",
            frame: (300, 280, 300, 80),
            children: vec![
                Item::Arrow {
                    frame: (300, 280, 300, 30),
                },
                Item::Label {
                    name: "Right label",
                    frame: (460, 320, 40, 36),
                    align: "r",
                    text: "A right label grows left",
                },
            ],
        },
        Item::Group {
            name: "Centred in nested groups",
            frame: (40, 400, 600, 120),
            children: vec![
                Item::Arrow {
                    frame: (40, 400, 600, 30),
                },
                Item::Group {
                    name: "Inner group",
                    frame: (240, 440, 200, 80),
                    children: vec![
                        Item::Arrow {
                            frame: (240, 440, 200, 30),
                        },
                        Item::Label {
                            name: "Nested label",
                            frame: (320, 480, 40, 36),
                            align: "ctr",
                            text: "A nested centred label, two groups deep",
                        },
                    ],
                },
            ],
        },
    ]
}

pub fn run(args: &[String]) -> Result<(), String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../probes/out")
        .join(NAME);
    let bytes = write_zip(&pptx());
    match args.first().map(String::as_str) {
        None => {
            std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            println!(
                "wrote probes/out/{NAME} ({} bytes, sha256 {})",
                bytes.len(),
                sha256_hex(&bytes)
            );
            Ok(())
        }
        Some("--check") if std::fs::read(&path).ok() == Some(bytes) => {
            println!("probes/out/{NAME} is up to date");
            Ok(())
        }
        Some("--check") => Err(format!("stale (run cargo xtask probes-grown-text): {NAME}")),
        _ => Err(USAGE.to_owned()),
    }
}

/// A point in EMU.
fn emu(points: i32) -> i64 {
    i64::from(points) * EMU
}

fn xfrm(frame: (i32, i32, i32, i32)) -> String {
    let (x, y, w, h) = frame;
    format!(
        "<a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/>",
        emu(x),
        emu(y),
        emu(w),
        emu(h)
    )
}

fn write_item(out: &mut String, item: &Item, id: &mut usize) {
    *id += 1;
    match item {
        Item::Label {
            name,
            frame,
            align,
            text,
        } => {
            let _ = write!(
                out,
                "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm>{}</a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr wrap=\"none\" lIns=\"50800\" tIns=\"50800\" rIns=\"50800\" bIns=\"50800\" anchor=\"t\"><a:spAutoFit/></a:bodyPr><a:lstStyle/><a:p><a:pPr algn=\"{align}\"/><a:r><a:rPr lang=\"en-US\" sz=\"2400\"><a:solidFill><a:srgbClr val=\"000000\"/></a:solidFill><a:latin typeface=\"Liberation Sans\"/></a:rPr><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>",
                xfrm(*frame)
            );
        }
        Item::Arrow { frame } => {
            let _ = write!(
                out,
                "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Arrow {id}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm>{}</a:xfrm><a:prstGeom prst=\"rightArrow\"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val=\"04455E\"/></a:solidFill><a:ln><a:noFill/></a:ln></p:spPr></p:sp>",
                xfrm(*frame)
            );
        }
        Item::Group {
            name,
            frame,
            children,
        } => {
            let (x, y, w, h) = *frame;
            let _ = write!(
                out,
                "<p:grpSp><p:nvGrpSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm>{}<a:chOff x=\"{}\" y=\"{}\"/><a:chExt cx=\"{}\" cy=\"{}\"/></a:xfrm></p:grpSpPr>",
                xfrm(*frame),
                emu(x),
                emu(y),
                emu(w),
                emu(h)
            );
            for child in children {
                write_item(out, child, id);
            }
            out.push_str("</p:grpSp>");
        }
    }
}

fn pptx() -> Vec<Entry> {
    let mut tree = String::new();
    let mut id = 1;
    for item in &items() {
        write_item(&mut tree, item, &mut id);
    }
    pptx_package("", &tree, Vec::new(), Vec::new())
}
