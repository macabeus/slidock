//! The photo probe deck (`probes/out/photo-probe.pptx`): one 960 × 540 pt slide of raster pictures larger than it
//! draws them, for the download's picture compression (`slidock::images`). The media are our own
//! (`probes/photos/make-media.sh`); each picture names what it pins:
//!
//! - "Photo" and "Photo again": one 2400 × 1600 JPEG drawn at 240 × 160 and at 360 × 240 pt. The larger use sets
//!   its size.
//! - "Cropped picture": a 1000 × 750 PNG with a quarter cut off each side, drawn at 100 × 75 pt: the whole image is
//!   drawn at twice its frame.
//! - "Logo with alpha": a 1200 × 1200 PNG with transparency at 120 × 120 pt.
//! - "Turned photo": a JPEG stored 1600 × 1200 with EXIF orientation 6, which shows 1200 × 1600, at 150 × 200 pt.
//! - "Filled ellipse": an ellipse whose image fill is an 800 × 800 PNG, at 80 × 80 pt.
//! - "Animated GIF": two frames, kept as it is.
//! - "Plain vector": an SVG beside its PNG preview, kept as it is.
//! - The slide's background: a 3000 × 1688 JPEG, smaller than what a full slide keeps, so it is not scaled.

use std::fmt::Write as _;
use std::path::Path;

use crate::probes_pictures::pptx_package;
use crate::probes_pictures::probes_zip::{Entry, sha256_hex, write_zip};

const USAGE: &str = "usage: cargo xtask probes-photos [--check]";

/// EMU per point.
const EMU: u32 = 12_700;

const NS_ASVG: &str = "http://schemas.microsoft.com/office/drawing/2016/SVG/main";

const PHOTO: &[u8] = include_bytes!("../../probes/photos/media/photo.jpg");
const CROP: &[u8] = include_bytes!("../../probes/photos/media/crop.png");
const LOGO: &[u8] = include_bytes!("../../probes/photos/media/logo.png");
const TURNED: &[u8] = include_bytes!("../../probes/photos/media/turned.jpg");
const FILL: &[u8] = include_bytes!("../../probes/photos/media/fill.png");
const BACKGROUND: &[u8] = include_bytes!("../../probes/photos/media/bg.jpg");
const ANIMATED: &[u8] = include_bytes!("../../probes/photos/media/anim.gif");

const VECTOR: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100" width="200" height="100"><title>Plain vector</title><path d="M 0 0 H 200 V 100 H 0 Z M 9 9 V 91 H 191 V 9 Z" fill="#FE7867" fill-rule="evenodd"/></svg>"##;

const PREVIEW: &[u8] = include_bytes!("../../probes/photos/media/preview.png");

/// What an item draws.
enum Draws {
    /// A picture; its crop (`a:srcRect`, in thousandths of a percent from each side), when cut.
    Picture { crop: Option<u32> },
    /// An ellipse filled with the image.
    Fill,
    /// A picture of an SVG beside its PNG preview.
    Vector,
}

struct Item {
    name: &'static str,
    /// x, y, w, h in points.
    frame: (u32, u32, u32, u32),
    /// The media part's name and bytes.
    file: (&'static str, &'static [u8]),
    draws: Draws,
}

const ITEMS: [Item; 8] = [
    Item {
        name: "Photo",
        frame: (40, 40, 240, 160),
        file: ("photo.jpg", PHOTO),
        draws: Draws::Picture { crop: None },
    },
    Item {
        name: "Photo again",
        frame: (300, 40, 360, 240),
        file: ("photo.jpg", PHOTO),
        draws: Draws::Picture { crop: None },
    },
    Item {
        name: "Cropped picture",
        frame: (700, 40, 100, 75),
        file: ("crop.png", CROP),
        draws: Draws::Picture { crop: Some(25_000) },
    },
    Item {
        name: "Logo with alpha",
        frame: (40, 300, 120, 120),
        file: ("logo.png", LOGO),
        draws: Draws::Picture { crop: None },
    },
    Item {
        name: "Turned photo",
        frame: (200, 300, 150, 200),
        file: ("turned.jpg", TURNED),
        draws: Draws::Picture { crop: None },
    },
    Item {
        name: "Filled ellipse",
        frame: (400, 300, 80, 80),
        file: ("fill.png", FILL),
        draws: Draws::Fill,
    },
    Item {
        name: "Animated GIF",
        frame: (520, 300, 96, 96),
        file: ("anim.gif", ANIMATED),
        draws: Draws::Picture { crop: None },
    },
    Item {
        name: "Plain vector",
        frame: (700, 300, 200, 100),
        file: ("vector.png", PREVIEW),
        draws: Draws::Vector,
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
                println!("probes/out photo files are up to date");
                Ok(())
            } else {
                Err(format!(
                    "stale (run cargo xtask probes-photos): {}",
                    stale.join(", ")
                ))
            }
        }
        _ => Err(USAGE.to_owned()),
    }
}

/// Every generated file: (name under `probes/out/`, bytes).
pub fn generate() -> Vec<(String, Vec<u8>)> {
    vec![("photo-probe.pptx".to_owned(), write_zip(&pptx()))]
}

fn xfrm((x, y, w, h): (u32, u32, u32, u32)) -> String {
    format!(
        "<a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>",
        x * EMU,
        y * EMU,
        w * EMU,
        h * EMU
    )
}

fn pptx() -> Vec<Entry> {
    let mut tree = String::new();
    let mut rels: Vec<(String, &'static str, String)> = Vec::new();
    let mut media: Vec<Entry> = Vec::new();
    let mut embed = |file: &'static str, bytes: &'static [u8], rels: &mut Vec<_>| -> String {
        let id = format!("rId{}", file.replace('.', ""));
        if !rels.iter().any(|(known, _, _)| known == &id) {
            rels.push((id.clone(), "image", format!("../media/{file}")));
            media.push(Entry {
                name: format!("ppt/media/{file}"),
                data: bytes.to_vec(),
                stored: true,
            });
        }
        id
    };
    for (n, item) in ITEMS.iter().enumerate() {
        let id = n + 2;
        let (file, bytes) = item.file;
        let blip = embed(file, bytes, &mut rels);
        match item.draws {
            Draws::Picture { crop } => {
                let crop = crop
                    .map(|cut| {
                        format!("<a:srcRect l=\"{cut}\" t=\"{cut}\" r=\"{cut}\" b=\"{cut}\"/>")
                    })
                    .unwrap_or_default();
                let _ = write!(
                    tree,
                    "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"{}\"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed=\"{blip}\"/>{crop}<a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>",
                    item.name,
                    xfrm(item.frame)
                );
            }
            Draws::Fill => {
                let _ = write!(
                    tree,
                    "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr>{}<a:prstGeom prst=\"ellipse\"><a:avLst/></a:prstGeom><a:blipFill><a:blip r:embed=\"{blip}\"/><a:stretch><a:fillRect/></a:stretch></a:blipFill></p:spPr></p:sp>",
                    item.name,
                    xfrm(item.frame)
                );
            }
            Draws::Vector => {
                let svg = embed("vector.svg", VECTOR.as_bytes(), &mut rels);
                let _ = write!(
                    tree,
                    "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"{}\"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed=\"{blip}\"><a:extLst><a:ext uri=\"{{96DAC541-7B7A-43D3-8B79-37D633B846F1}}\"><asvg:svgBlip xmlns:asvg=\"{NS_ASVG}\" r:embed=\"{svg}\"/></a:ext></a:extLst></a:blip><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>",
                    item.name,
                    xfrm(item.frame)
                );
            }
        }
    }
    let background = embed("bg.jpg", BACKGROUND, &mut rels);
    let defaults = "<Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"jpg\" ContentType=\"image/jpeg\"/><Default Extension=\"gif\" ContentType=\"image/gif\"/><Default Extension=\"svg\" ContentType=\"image/svg+xml\"/>";
    let mut entries = pptx_package(defaults, &tree, rels, media);
    // The slide's background, which the shared package writes none of.
    for entry in &mut entries {
        if entry.name == "ppt/slides/slide1.xml" {
            let text = String::from_utf8_lossy(&entry.data).replacen(
                "<p:cSld><p:spTree>",
                &format!(
                    "<p:cSld><p:bg><p:bgPr><a:blipFill><a:blip r:embed=\"{background}\"/><a:stretch><a:fillRect/></a:stretch></a:blipFill><a:effectLst/></p:bgPr></p:bg><p:spTree>"
                ),
                1,
            );
            entry.data = text.into_bytes();
        }
    }
    entries
}
