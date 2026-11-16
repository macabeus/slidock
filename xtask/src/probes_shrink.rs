//! The shrink probe decks (`probes/out/shrink-probe.pptx`, `.odp`): one slide of text boxes that shrink their text
//! on overflow and store no scale (a bare `a:normAutofit`, ODF's `draw:fit-to-size="shrink-to-fit"`), so the
//! compiler fits each by PowerPoint's steps, which LibreOffice takes. Every box is Liberation Sans with 7.2 pt side
//! and 3.6 pt top and bottom insets; each names what it pins:
//!
//! - "Fits": a short line with room to spare keeps its size.
//! - "Line spacing": one 28 pt line whose box is a little shorter than its line: the first step, line spacing down
//!   10 % at full size, fits it.
//! - "List": five 24 pt paragraphs in a box about two thirds of their height: the font steps down.
//! - "Wrapping": a paragraph that wraps in its narrow box: its lines break again at each scale.
//! - "Overflow": more text than the smallest step fits: it takes the last step, 25 % with line spacing down 20 %.
//! - "Stored" (PPTX only): `fontScale="62500" lnSpcReduction="20000"`, kept as stored.
//! - "Spacing stored" (PPTX only): `lnSpcReduction="10000"` alone, a stored full-size scale.

use std::fmt::Write as _;
use std::path::Path;

use crate::probes_pictures::probes_zip::{sha256_hex, write_zip};
use crate::probes_pictures::{odp_package, pptx_package};

const USAGE: &str = "usage: cargo xtask probes-shrink [--check]";

/// EMU per point.
const EMU: i64 = 12_700;

/// One text box: its name, its box on a 960 × 540 pt slide, its size in points, its paragraphs and (PPTX only) the
/// `normAutofit` attributes it stores.
struct Box {
    name: &'static str,
    frame: (i32, i32, i32, i32),
    size: u32,
    paragraphs: &'static [&'static str],
    stored: Option<&'static str>,
}

const LOREM: &str = "Text that shrinks to fit its box takes the largest step of its app at which every line fits, and \
the smallest when none does, so this paragraph runs on well past the room its box has for it.";

const BOXES: [Box; 7] = [
    Box {
        name: "Fits",
        frame: (40, 30, 420, 60),
        size: 24,
        paragraphs: &["Short title"],
        stored: None,
    },
    Box {
        name: "Line spacing",
        frame: (500, 30, 420, 38),
        size: 28,
        paragraphs: &["Plain grid"],
        stored: None,
    },
    Box {
        name: "List",
        frame: (40, 120, 420, 110),
        size: 24,
        paragraphs: &[
            "First item",
            "Second item",
            "Third item",
            "Fourth item",
            "Fifth item",
        ],
        stored: None,
    },
    Box {
        name: "Wrapping",
        frame: (500, 120, 200, 75),
        size: 20,
        paragraphs: &["Each line of this paragraph breaks again at every step the box tries"],
        stored: None,
    },
    Box {
        name: "Overflow",
        frame: (740, 120, 180, 40),
        size: 18,
        paragraphs: &[LOREM, LOREM],
        stored: None,
    },
    Box {
        name: "Stored",
        frame: (40, 300, 420, 60),
        size: 24,
        paragraphs: &["A stored scale", "is kept"],
        stored: Some(" fontScale=\"62500\" lnSpcReduction=\"20000\""),
    },
    Box {
        name: "Spacing stored",
        frame: (500, 300, 420, 60),
        size: 24,
        paragraphs: &["Line spacing stored alone"],
        stored: Some(" lnSpcReduction=\"10000\""),
    },
];

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
                    sha256_hex(&bytes)
                );
            }
            Ok(())
        }
        Some("--check") => {
            let stale: Vec<String> = generate()
                .into_iter()
                .filter(|(name, bytes)| std::fs::read(root.join(name)).ok().as_ref() != Some(bytes))
                .map(|(name, _)| name)
                .collect();
            if stale.is_empty() {
                println!("probes/out shrink files are up to date");
                Ok(())
            } else {
                Err(format!(
                    "stale (run cargo xtask probes-shrink): {}",
                    stale.join(", ")
                ))
            }
        }
        _ => Err(USAGE.to_owned()),
    }
}

/// Every generated file: (name under `probes/out/`, bytes).
pub fn generate() -> Vec<(String, Vec<u8>)> {
    vec![
        ("shrink-probe.pptx".to_owned(), write_zip(&pptx())),
        ("shrink-probe.odp".to_owned(), write_zip(&odp())),
    ]
}

/// A point in EMU.
fn emu(points: i32) -> i64 {
    i64::from(points) * EMU
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;")
}

fn pptx() -> Vec<crate::probes_pictures::probes_zip::Entry> {
    let mut tree = String::new();
    for (n, text_box) in BOXES.iter().enumerate() {
        let (x, y, w, h) = text_box.frame;
        let mut paragraphs = String::new();
        for paragraph in text_box.paragraphs {
            let _ = write!(
                paragraphs,
                "<a:p><a:r><a:rPr lang=\"en-US\" sz=\"{}\"><a:solidFill><a:srgbClr val=\"04455E\"/></a:solidFill><a:latin typeface=\"Liberation Sans\"/></a:rPr><a:t>{}</a:t></a:r></a:p>",
                text_box.size * 100,
                escape(paragraph)
            );
        }
        let _ = write!(
            tree,
            "<p:sp><p:nvSpPr><p:cNvPr id=\"{}\" name=\"{}\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/><a:ln w=\"6350\"><a:solidFill><a:srgbClr val=\"FE7867\"/></a:solidFill></a:ln></p:spPr><p:txBody><a:bodyPr wrap=\"square\" lIns=\"91440\" tIns=\"45720\" rIns=\"91440\" bIns=\"45720\" anchor=\"t\"><a:normAutofit{}/></a:bodyPr><a:lstStyle/>{paragraphs}</p:txBody></p:sp>",
            n + 2,
            text_box.name,
            emu(x),
            emu(y),
            emu(w),
            emu(h),
            text_box.stored.unwrap_or("")
        );
    }
    pptx_package("", &tree, Vec::new(), Vec::new())
}

fn odp() -> Vec<crate::probes_pictures::probes_zip::Entry> {
    let automatic = "<style:style style:name=\"grShrink\" style:family=\"graphic\"><style:graphic-properties draw:stroke=\"solid\" svg:stroke-width=\"0.5pt\" svg:stroke-color=\"#fe7867\" draw:fill=\"none\" draw:textarea-vertical-align=\"top\" draw:auto-grow-height=\"false\" draw:fit-to-size=\"shrink-to-fit\" style:shrink-to-fit=\"true\" fo:padding-top=\"3.6pt\" fo:padding-bottom=\"3.6pt\" fo:padding-left=\"7.2pt\" fo:padding-right=\"7.2pt\" fo:wrap-option=\"wrap\"/></style:style>";
    let mut drawables = String::new();
    for text_box in BOXES.iter().filter(|text_box| text_box.stored.is_none()) {
        let (x, y, w, h) = text_box.frame;
        let mut paragraphs = String::new();
        for paragraph in text_box.paragraphs {
            let _ = write!(
                paragraphs,
                "<text:p><text:span text:style-name=\"T{}\">{}</text:span></text:p>",
                text_box.size,
                escape(paragraph)
            );
        }
        let _ = write!(
            drawables,
            "<draw:frame xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" draw:style-name=\"grShrink\" draw:name=\"{}\" svg:x=\"{x}pt\" svg:y=\"{y}pt\" svg:width=\"{w}pt\" svg:height=\"{h}pt\"><draw:text-box>{paragraphs}</draw:text-box></draw:frame>",
            text_box.name
        );
    }
    let mut spans = String::from(automatic);
    for size in [18, 20, 24, 28] {
        let _ = write!(
            spans,
            "<style:style style:name=\"T{size}\" style:family=\"text\"><style:text-properties fo:font-size=\"{size}pt\" style:font-name=\"Liberation Sans\" fo:font-family=\"'Liberation Sans'\" fo:color=\"#04455e\"/></style:style>"
        );
    }
    odp_package("", &spans, "", "Shrink", &drawables, Vec::new())
}
