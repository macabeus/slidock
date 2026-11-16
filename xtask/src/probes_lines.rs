//! The line-ends probe decks (`probes/out/line-ends-probe.pptx`, `.odp`): one slide of arrows whose ends sit on
//! curves or past a short last segment, where an end's direction and the line's cut depend on more than the last
//! control point. Every drawing is our own; each names what it pins:
//!
//! - "Hooked arrow": a cubic that turns back on itself, with a large triangle at its end. The triangle points along
//!   the chord under it, and the line stops at the middle of its base.
//! - "Arc arrow": a quadratic with a large stealth at its start and a large triangle at its end. The line starts at
//!   the stealth's notch and stops at the triangle's base.
//! - "Short last segment arrow": a line, then a turn shorter than its triangle. The line stops on the first segment,
//!   the triangle's length from the tip.
//! - "Curved connector" (PPTX only; ODP's curved connector is declined, OD9110): `curvedConnector3` with a large
//!   triangle at its end.
//! - "Short bend, two heads" and "Short curve, two heads": paths shorter than one of their two triangles. Both
//!   triangles are drawn, each pointing from the far end of the path to its tip.
//! - "Bend under two heads": a path longer than one triangle and shorter than two. Each triangle points along the
//!   whole path, not along what the other one's cut leaves of it.
//! - "Short bent connector" (PPTX only): `bentConnector2` shorter than one of its two triangles.

use std::fmt::Write as _;
use std::path::Path;

use crate::probes_pictures::probes_zip::{sha256_hex, write_zip};
use crate::probes_pictures::{odp_package, pptx_package};

const USAGE: &str = "usage: cargo xtask probes-lines [--check]";

/// EMU per point.
const EMU: i64 = 12_700;

/// The stroke every arrow has, in points.
const WIDTH: u32 = 4;

/// What an end is: OOXML's preset name (large width and length), LibreOffice's marker for it.
#[derive(Clone, Copy)]
enum End {
    Triangle,
    Stealth,
}

impl End {
    fn ooxml(self) -> &'static str {
        match self {
            End::Triangle => "triangle",
            End::Stealth => "stealth",
        }
    }

    /// The name LibreOffice gives OOXML's large-by-large end (the 9th size class), encoded as a style name.
    fn marker(self) -> &'static str {
        match self {
            End::Triangle => "msArrowEnd_20_9",
            End::Stealth => "msArrowStealthEnd_20_9",
        }
    }
}

/// A path command in the drawing's own points (its box's origin at the top left).
enum Cmd {
    M(i32, i32),
    L(i32, i32),
    Q(i32, i32, i32, i32),
    C(i32, i32, i32, i32, i32, i32),
}

/// What an arrow is drawn as: a path of our own, or (PPTX only) a preset connector.
enum Drawn {
    Path(&'static [Cmd]),
    Connector(&'static str),
}

/// One arrow: its name, its box on a 960 × 540 pt slide, its drawing and its ends.
struct Arrow {
    name: &'static str,
    frame: (i32, i32, i32, i32),
    drawn: Drawn,
    head: Option<End>,
    tail: Option<End>,
}

const ARROWS: [Arrow; 8] = [
    Arrow {
        name: "Hooked arrow",
        frame: (60, 60, 200, 120),
        drawn: Drawn::Path(&[Cmd::M(0, 0), Cmd::C(200, 0, 200, 120, 100, 120)]),
        head: None,
        tail: Some(End::Triangle),
    },
    Arrow {
        name: "Arc arrow",
        frame: (360, 60, 240, 160),
        drawn: Drawn::Path(&[Cmd::M(0, 160), Cmd::Q(0, 0, 240, 0)]),
        head: Some(End::Stealth),
        tail: Some(End::Triangle),
    },
    Arrow {
        name: "Short last segment arrow",
        frame: (60, 320, 300, 8),
        drawn: Drawn::Path(&[Cmd::M(0, 0), Cmd::L(300, 0), Cmd::L(300, 8)]),
        head: None,
        tail: Some(End::Triangle),
    },
    Arrow {
        name: "Curved connector",
        frame: (420, 300, 240, 160),
        drawn: Drawn::Connector("curvedConnector3"),
        head: None,
        tail: Some(End::Triangle),
    },
    Arrow {
        name: "Short bend, two heads",
        frame: (760, 80, 10, 8),
        drawn: Drawn::Path(&[Cmd::M(0, 0), Cmd::L(10, 0), Cmd::L(10, 8)]),
        head: Some(End::Triangle),
        tail: Some(End::Triangle),
    },
    Arrow {
        name: "Short curve, two heads",
        frame: (760, 200, 12, 8),
        drawn: Drawn::Path(&[Cmd::M(0, 8), Cmd::Q(0, 0, 12, 0)]),
        head: Some(End::Triangle),
        tail: Some(End::Triangle),
    },
    Arrow {
        name: "Bend under two heads",
        frame: (760, 320, 24, 6),
        drawn: Drawn::Path(&[Cmd::M(0, 0), Cmd::L(24, 0), Cmd::L(24, 6)]),
        head: Some(End::Triangle),
        tail: Some(End::Triangle),
    },
    Arrow {
        name: "Short bent connector",
        frame: (760, 440, 10, 8),
        drawn: Drawn::Connector("bentConnector2"),
        head: Some(End::Triangle),
        tail: Some(End::Triangle),
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
                println!("probes/out line-ends files are up to date");
                Ok(())
            } else {
                Err(format!(
                    "stale (run cargo xtask probes-lines): {}",
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
        ("line-ends-probe.pptx".to_owned(), write_zip(&pptx())),
        ("line-ends-probe.odp".to_owned(), write_zip(&odp())),
    ]
}

/// A point in EMU.
fn emu(points: i32) -> i64 {
    i64::from(points) * EMU
}

fn pptx() -> Vec<crate::probes_pictures::probes_zip::Entry> {
    let mut tree = String::new();
    for (n, arrow) in ARROWS.iter().enumerate() {
        let (x, y, w, h) = arrow.frame;
        let xfrm = format!(
            "<a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>",
            emu(x),
            emu(y),
            emu(w),
            emu(h)
        );
        let mut ends = String::new();
        if let Some(end) = arrow.head {
            let _ = write!(
                ends,
                "<a:headEnd type=\"{}\" w=\"lg\" len=\"lg\"/>",
                end.ooxml()
            );
        }
        if let Some(end) = arrow.tail {
            let _ = write!(
                ends,
                "<a:tailEnd type=\"{}\" w=\"lg\" len=\"lg\"/>",
                end.ooxml()
            );
        }
        let line = format!(
            "<a:noFill/><a:ln w=\"{}\"><a:solidFill><a:srgbClr val=\"04455E\"/></a:solidFill>{ends}</a:ln>",
            emu(i32::try_from(WIDTH).unwrap_or(1))
        );
        let id = n + 2;
        match arrow.drawn {
            Drawn::Path(cmds) => {
                // The path's units are hundredths of a point, in a path box of the frame's size.
                let pt = |v: i32| v * 100;
                let mut path = String::new();
                for cmd in cmds {
                    let _ = match *cmd {
                        Cmd::M(a, b) => write!(
                            path,
                            "<a:moveTo><a:pt x=\"{}\" y=\"{}\"/></a:moveTo>",
                            pt(a),
                            pt(b)
                        ),
                        Cmd::L(a, b) => write!(
                            path,
                            "<a:lnTo><a:pt x=\"{}\" y=\"{}\"/></a:lnTo>",
                            pt(a),
                            pt(b)
                        ),
                        Cmd::Q(a, b, c, d) => write!(
                            path,
                            "<a:quadBezTo><a:pt x=\"{}\" y=\"{}\"/><a:pt x=\"{}\" y=\"{}\"/></a:quadBezTo>",
                            pt(a),
                            pt(b),
                            pt(c),
                            pt(d)
                        ),
                        Cmd::C(a, b, c, d, e, f) => write!(
                            path,
                            "<a:cubicBezTo><a:pt x=\"{}\" y=\"{}\"/><a:pt x=\"{}\" y=\"{}\"/><a:pt x=\"{}\" y=\"{}\"/></a:cubicBezTo>",
                            pt(a),
                            pt(b),
                            pt(c),
                            pt(d),
                            pt(e),
                            pt(f)
                        ),
                    };
                }
                let _ = write!(
                    tree,
                    "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr>{xfrm}<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l=\"0\" t=\"0\" r=\"r\" b=\"b\"/><a:pathLst><a:path w=\"{}\" h=\"{}\" fill=\"none\">{path}</a:path></a:pathLst></a:custGeom>{line}</p:spPr></p:sp>",
                    arrow.name,
                    pt(w),
                    pt(h)
                );
            }
            Drawn::Connector(preset) => {
                let _ = write!(
                    tree,
                    "<p:cxnSp><p:nvCxnSpPr><p:cNvPr id=\"{id}\" name=\"{}\"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr><p:spPr>{xfrm}<a:prstGeom prst=\"{preset}\"><a:avLst/></a:prstGeom>{line}</p:spPr></p:cxnSp>",
                    arrow.name
                );
            }
        }
    }
    pptx_package("", &tree, Vec::new(), Vec::new())
}

fn odp() -> Vec<crate::probes_pictures::probes_zip::Entry> {
    let mut automatic = String::new();
    let mut drawables = String::new();
    for (n, arrow) in ARROWS.iter().enumerate() {
        let Drawn::Path(cmds) = arrow.drawn else {
            continue;
        };
        let mut markers = String::new();
        if let Some(end) = arrow.head {
            let _ = write!(
                markers,
                " draw:marker-start=\"{}\" draw:marker-start-width=\"{}pt\"",
                end.marker(),
                5 * WIDTH
            );
        }
        if let Some(end) = arrow.tail {
            let _ = write!(
                markers,
                " draw:marker-end=\"{}\" draw:marker-end-width=\"{}pt\"",
                end.marker(),
                5 * WIDTH
            );
        }
        let _ = write!(
            automatic,
            "<style:style style:name=\"gr{n}\" style:family=\"graphic\"><style:graphic-properties draw:stroke=\"solid\" svg:stroke-width=\"{WIDTH}pt\" svg:stroke-color=\"#04455e\" draw:fill=\"none\"{markers}/></style:style>"
        );
        let mut d = String::new();
        for cmd in cmds {
            let _ = match *cmd {
                Cmd::M(a, b) => write!(d, "M{a} {b}"),
                Cmd::L(a, b) => write!(d, "L{a} {b}"),
                Cmd::Q(a, b, c, e) => write!(d, "Q{a} {b} {c} {e}"),
                Cmd::C(a, b, c, e, f, g) => write!(d, "C{a} {b} {c} {e} {f} {g}"),
            };
        }
        let (x, y, w, h) = arrow.frame;
        let _ = write!(
            drawables,
            "<draw:path draw:style-name=\"gr{n}\" draw:name=\"{}\" svg:x=\"{x}pt\" svg:y=\"{y}pt\" svg:width=\"{w}pt\" svg:height=\"{h}pt\" svg:viewBox=\"0 0 {w} {h}\" svg:d=\"{d}\"/>",
            arrow.name
        );
    }
    // LibreOffice's own outlines of the two markers (only their names are read).
    let styles = "<draw:marker draw:name=\"msArrowEnd_20_9\" draw:display-name=\"msArrowEnd 9\" svg:viewBox=\"0 0 210 210\" svg:d=\"M105 0L210 210H0Z\"/><draw:marker draw:name=\"msArrowStealthEnd_20_9\" draw:display-name=\"msArrowStealthEnd 9\" svg:viewBox=\"0 0 210 210\" svg:d=\"M105 0L210 210L105 105L0 210Z\"/>";
    odp_package("", &automatic, styles, "Line ends", &drawables, Vec::new())
}
