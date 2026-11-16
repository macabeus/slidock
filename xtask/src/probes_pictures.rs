//! The picture probe decks (`probes/out/picture-probe.pptx`, `.odp`): one slide of vector pictures, each an SVG
//! original kept beside a small PNG preview the way PowerPoint (`a:blip` plus `asvg:svgBlip`) and LibreOffice (an SVG
//! `draw:image` before its PNG) keep them. Every drawing is our own. Each picture names what it pins:
//!
//! - "Stretched vector": a square SVG in a wide frame. A picture fills its frame, so the SVG is written to stretch.
//! - "Sliced vector": an SVG whose root covers its viewport (`xMinYMax slice`): what its viewport shows is its lower
//!   half, which is what the picture shows.
//! - "Plain vector": an SVG in its own proportions.
//! - "Escaped url vector": an SVG whose `fill` is a CSS-escaped `url(…)` to another site. The allow-list refuses it, so
//!   the preview is drawn, and reported.
//!
//! The picture-labels probe decks (`probes/out/picture-labels-probe.pptx`, `.odp`): one slide of vector pictures
//! that draw text, as diagramming tools export them (Excalidraw: a public DOCTYPE, a style sheet of an embedded font):
//!
//! - "Labelled chart vector": a chart whose labels are drawn in an embedded font (Pacifico, a subset under the OFL,
//!   `licenses/pacifico-OFL.txt`), with a class rule, `xml:space`, empty metadata and an empty mask. It is drawn from
//!   its SVG, the DOCTYPE dropped.
//! - "Preserved space vector": a drawing whose root says `xml:space="preserve"`, drawn from its SVG.
//! - "Remote font vector": a style sheet whose font loads from another site. The allow-list refuses it, so the
//!   preview is drawn, and reported.
//! - "Internal subset vector": a DOCTYPE with an internal subset, where entities are declared (this one is empty, so
//!   the file still reads as an SVG). Refused the same way.
//! - "Fallback font vector": the chart with its labels in a family its sheet does not embed (Probe Print). Inside an
//!   `<img>` they would be drawn in the viewer's fallback font, so it is refused the same way.
//! - "Open quote vector": the chart whose font's `format()` hint opens a quote it does not close, which a browser
//!   reads to the end of the sheet (dropping the axis rule after it). Refused the same way.
//!
//! The picture-types probe decks (`probes/out/picture-types-probe.pptx`, `.odp`): one slide of raster pictures whose
//! files are ISO base media, typed by their `ftyp` brands (`probes/pictures/make-media.sh` writes the two cards):
//!
//! - "Off-slide omitted picture": a HEIC card, which the output omits (MID5601), in a frame that starts above and left
//!   of the slide. Its marker's label sits at the frame's centre, on the slide.
//! - "AVIF under mif1": an AVIF card whose major brand is the structural `mif1`; its compatible brands name AVIF, so it
//!   is drawn.
//! - "HEIF image sequence": a file of major brand `hevc` (an HEVC image sequence): image/heic, omitted.
//! - "Unknown ISO brand": a file whose brands name nothing known: application/octet-stream, omitted.
//!
//! The HEIC probe decks (`probes/out/heic-probe.pptx`, `.odp`): one slide of HEIC photos as an iPhone writes them
//! (`probes/heif/make-media.sh`, Apple's encoder): the compiler omits them (MID5601), and the web app has the browser
//! decode them where it can (MID5608).
//!
//! - "Grid photo": a 1100 × 600 grid of 3 × 2 HEVC tiles, tagged Display P3.
//! - "Turned grid photo": the same grid turned a quarter anticlockwise by its `irot` property: 600 × 1100.
//! - "Single photo": a 64 × 48 image of one HEVC picture.
//! - "Mirrored photo": the single photo with its `irot` (of no turn) retyped as an `imir` of axis 0: top and bottom
//!   swap.
//! - "Cropped photo": a 61 × 45 photo (`odd.heic`: the encoder's 62 × 46 picture and its clean aperture) with the
//!   aperture moved to a 40 × 30 rectangle 4 pixels in and 3 down.

use std::fmt::Write as _;
use std::path::Path;

// The motion probes' zip writer, as its own copy: the motion generator keeps its module private.
#[path = "probes_zip.rs"]
#[allow(
    dead_code,
    clippy::duplicate_mod,
    reason = "the picture probes use the writer only"
)]
pub(crate) mod probes_zip;

use probes_zip::Entry;

const USAGE: &str = "usage: cargo xtask probes-pictures [--check]";

/// What a picture shows: an SVG original beside a PNG preview of one colour, or one raster file.
enum Shows {
    Vector {
        svg: fn() -> String,
        preview: [u8; 3],
    },
    Raster {
        ext: &'static str,
        mime: &'static str,
        bytes: fn() -> Vec<u8>,
    },
}

/// One picture: its name, its frame on a 960 × 540 pt slide and what it shows.
struct Picture {
    name: &'static str,
    frame: (i32, i32, u32, u32),
    shows: Shows,
}

const VECTORS: [Picture; 4] = [
    Picture {
        name: "Stretched vector",
        frame: (40, 40, 880, 160),
        shows: Shows::Vector {
            svg: || {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="100" height="100"><title>Stretched vector</title><rect width="100" height="100" fill="#FE7867"/><rect x="10" y="10" width="80" height="80" fill="#04455E"/></svg>"##.to_owned()
            },
            preview: [0xFE, 0x78, 0x67],
        },
    },
    Picture {
        name: "Sliced vector",
        frame: (40, 240, 400, 200),
        shows: Shows::Vector {
            svg: || {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="200" height="100" preserveAspectRatio="xMinYMax slice"><title>Sliced vector</title><rect width="100" height="50" fill="#04455E"/><rect y="50" width="100" height="50" fill="#FE7867"/><rect x="45" y="50" width="10" height="50" fill="#F4F1EA"/></svg>"##.to_owned()
            },
            preview: [0xFE, 0x78, 0x67],
        },
    },
    Picture {
        name: "Plain vector",
        frame: (480, 240, 200, 100),
        shows: Shows::Vector {
            svg: || {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100" width="200" height="100"><title>Plain vector</title><path d="M 0 0 H 200 V 100 H 0 Z M 9 9 V 91 H 191 V 9 Z" fill="#FE7867" fill-rule="evenodd"/></svg>"##.to_owned()
            },
            preview: [0xFE, 0x78, 0x67],
        },
    },
    Picture {
        name: "Escaped url vector",
        frame: (720, 240, 200, 100),
        shows: Shows::Vector {
            svg: || {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100" width="200" height="100"><title>Escaped url vector</title><rect width="200" height="100" fill="#04455E"/><rect width="200" height="100" fill="u\72l(https://probe.invalid/leak#a)"/></svg>"##.to_owned()
            },
            preview: [0x04, 0x45, 0x5E],
        },
    },
];

/// A subset of Pacifico (about fifty characters, among them `Cobras 100`), as WOFF2.
const PACIFICO: &[u8] =
    include_bytes!("../../crates/slidock-fonts/tests/data/container/pacifico.woff2");
const PACIFICO_OFL: &[u8] =
    include_bytes!("../../crates/slidock-fonts/tests/data/container/OFL-pacifico.txt");

/// Standard base64, padded.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | (u32::from(b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

const PUBLIC_DOCTYPE: &str = r#"<?xml version="1.0" standalone="no"?><!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">"#;

/// A chart labelled in a font family, with a style sheet that embeds Probe Hand from a source, as Excalidraw writes one.
fn labelled_chart(font_source: &str, family: &str) -> String {
    format!(
        r##"{PUBLIC_DOCTYPE}<svg xmlns="http://www.w3.org/2000/svg" version="1.1" viewBox="0 0 400 200" width="400px" height="200px"><!-- svg-source:probe --><metadata/><defs><style class="style-fonts">
      @font-face {{ font-family: Probe Hand; src: {font_source}; }}
      .axis {{ stroke: #04455E; stroke-width: 4 }}</style></defs><rect width="400" height="200" fill="#F4F1EA"/><g class="axis" stroke-linecap="round"><path d="M 40 170 H 380 M 40 170 V 20" fill="none"/></g><mask/><path d="M 40 170 L 160 140 L 260 110 L 380 90" stroke="#FE7867" stroke-width="6" fill="none"/><text x="60" y="80" font-family="{family}, sans-serif" font-size="48px" fill="#1E1E1E" style="white-space: pre;" xml:space="preserve" dominant-baseline="alphabetic">Cobras <tspan fill="#FE7867">100</tspan></text></svg>"##
    )
}

/// Probe Hand's source: Pacifico embedded as a `data:` URL, then a hint.
fn embedded_hand(hint: &str) -> String {
    format!("url(data:font/woff2;base64,{}){hint}", base64(PACIFICO))
}

const LABELLED: [Picture; 6] = [
    Picture {
        name: "Labelled chart vector",
        frame: (40, 40, 560, 280),
        shows: Shows::Vector {
            svg: || labelled_chart(&embedded_hand(""), "Probe Hand"),
            preview: [0xF4, 0xF1, 0xEA],
        },
    },
    Picture {
        name: "Preserved space vector",
        frame: (640, 40, 280, 140),
        shows: Shows::Vector {
            svg: || {
                r##"<svg xmlns="http://www.w3.org/2000/svg" xml:space="preserve" viewBox="0 0 200 100" width="200" height="100"><title>Preserved space vector</title><path d="M 0 0 H 200 V 100 H 0 Z M 9 9 V 91 H 191 V 9 Z" fill="#04455E" fill-rule="evenodd"/></svg>"##.to_owned()
            },
            preview: [0x04, 0x45, 0x5E],
        },
    },
    Picture {
        name: "Remote font vector",
        frame: (40, 360, 280, 140),
        shows: Shows::Vector {
            svg: || labelled_chart("url(https://probe.invalid/hand.woff2)", "Probe Hand"),
            preview: [0xF4, 0xF1, 0xEA],
        },
    },
    Picture {
        name: "Internal subset vector",
        frame: (640, 360, 280, 140),
        shows: Shows::Vector {
            svg: || {
                r##"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd" [ ]><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100" width="200" height="100"><title>Internal subset vector</title><rect width="200" height="100" fill="#FE7867"/></svg>"##.to_owned()
            },
            preview: [0xFE, 0x78, 0x67],
        },
    },
    Picture {
        name: "Fallback font vector",
        frame: (340, 360, 280, 140),
        shows: Shows::Vector {
            svg: || labelled_chart(&embedded_hand(""), "Probe Print"),
            preview: [0xF4, 0xF1, 0xEA],
        },
    },
    Picture {
        name: "Open quote vector",
        frame: (640, 200, 280, 140),
        shows: Shows::Vector {
            svg: || labelled_chart(&embedded_hand(" format('woff2)"), "Probe Hand"),
            preview: [0xF4, 0xF1, 0xEA],
        },
    },
];

const CARD_HEIC: &[u8] = include_bytes!("../../probes/pictures/media/card.heic");
const CARD_AVIF: &[u8] = include_bytes!("../../probes/pictures/media/card.avif");

/// The AVIF card with `mif1` as its major brand, as some encoders write it; its compatible brands still name AVIF.
fn avif_under_mif1() -> Vec<u8> {
    let mut bytes = CARD_AVIF.to_vec();
    assert_eq!(
        &bytes[4..12],
        b"ftypavif",
        "card.avif starts with its ftyp box"
    );
    assert!(
        bytes[16..28].windows(4).any(|brand| brand == b"avif"),
        "card.avif lists avif among its compatible brands"
    );
    bytes[8..12].copy_from_slice(b"mif1");
    bytes
}

/// An `ftyp` box of these brands, then an empty `free` box: a file typed by its brands alone.
fn iso_head(major: &[u8; 4], compatible: &[&[u8; 4]]) -> Vec<u8> {
    // Far under 4 GiB.
    #[allow(clippy::cast_possible_truncation)]
    let size = (16 + 4 * compatible.len()) as u32;
    let mut out = size.to_be_bytes().to_vec();
    out.extend_from_slice(b"ftyp");
    out.extend_from_slice(major);
    out.extend_from_slice(&[0; 4]);
    for brand in compatible {
        out.extend_from_slice(*brand);
    }
    out.extend_from_slice(&8u32.to_be_bytes());
    out.extend_from_slice(b"free");
    out
}

const RASTERS: [Picture; 4] = [
    Picture {
        name: "Off-slide omitted picture",
        frame: (-160, -120, 560, 400),
        shows: Shows::Raster {
            ext: "heic",
            mime: "image/heic",
            bytes: || CARD_HEIC.to_vec(),
        },
    },
    Picture {
        name: "AVIF under mif1",
        frame: (480, 40, 320, 180),
        shows: Shows::Raster {
            ext: "avif",
            mime: "image/avif",
            bytes: avif_under_mif1,
        },
    },
    Picture {
        name: "HEIF image sequence",
        frame: (480, 280, 200, 112),
        shows: Shows::Raster {
            ext: "heics",
            mime: "image/heic-sequence",
            bytes: || iso_head(b"hevc", &[b"msf1", b"hevc"]),
        },
    },
    Picture {
        name: "Unknown ISO brand",
        frame: (720, 280, 200, 112),
        shows: Shows::Raster {
            ext: "bin",
            mime: "application/octet-stream",
            bytes: || iso_head(b"zzzz", &[b"zzzz"]),
        },
    },
];

const GRID_HEIC: &[u8] = include_bytes!("../../probes/heif/grid.heic");
const SINGLE_HEIC: &[u8] = include_bytes!("../../probes/heif/single.heic");
const ODD_HEIC: &[u8] = include_bytes!("../../probes/heif/odd.heic");

/// The single photo mirrored top to bottom: its `irot` of no turn retyped as an `imir` of axis 0, of the same size.
fn mirrored_single() -> Vec<u8> {
    let mut bytes = SINGLE_HEIC.to_vec();
    let at = bytes
        .windows(4)
        .position(|window| window == b"irot")
        .expect("single.heic has an irot property");
    assert_eq!(bytes[at + 4], 0, "single.heic is not turned");
    bytes[at..at + 4].copy_from_slice(b"imir");
    bytes
}

/// The odd photo with its clean aperture moved to 40 × 30 pixels from (4, 3) of its 62 × 46 picture: the
/// aperture's centre is 7 left of the picture's and 5 up.
fn cropped_odd() -> Vec<u8> {
    let mut bytes = ODD_HEIC.to_vec();
    let at = bytes
        .windows(4)
        .position(|window| window == b"clap")
        .expect("odd.heic has a clap property");
    let fields: [u32; 8] = [40, 1, 30, 1, (-7i32) as u32, 1, (-5i32) as u32, 1];
    for (index, field) in fields.iter().enumerate() {
        let start = at + 4 + 4 * index;
        bytes[start..start + 4].copy_from_slice(&field.to_be_bytes());
    }
    bytes
}

/// The grid with its `irot` property turned a quarter anticlockwise.
fn turned_grid() -> Vec<u8> {
    let mut bytes = GRID_HEIC.to_vec();
    let at = bytes
        .windows(4)
        .position(|window| window == b"irot")
        .expect("grid.heic has an irot property");
    assert_eq!(bytes[at + 4], 0, "grid.heic is not turned");
    bytes[at + 4] = 1;
    bytes
}

const HEIC_PHOTOS: [Picture; 5] = [
    Picture {
        name: "Grid photo",
        frame: (40, 40, 440, 240),
        shows: Shows::Raster {
            ext: "heic",
            mime: "image/heic",
            bytes: || GRID_HEIC.to_vec(),
        },
    },
    Picture {
        name: "Turned grid photo",
        frame: (520, 40, 180, 330),
        shows: Shows::Raster {
            ext: "heic",
            mime: "image/heic",
            bytes: turned_grid,
        },
    },
    Picture {
        name: "Single photo",
        frame: (40, 320, 240, 180),
        shows: Shows::Raster {
            ext: "heic",
            mime: "image/heic",
            bytes: || SINGLE_HEIC.to_vec(),
        },
    },
    Picture {
        name: "Mirrored photo",
        frame: (300, 380, 200, 150),
        shows: Shows::Raster {
            ext: "heic",
            mime: "image/heic",
            bytes: mirrored_single,
        },
    },
    Picture {
        name: "Cropped photo",
        frame: (720, 40, 200, 150),
        shows: Shows::Raster {
            ext: "heic",
            mime: "image/heic",
            bytes: cropped_odd,
        },
    },
];

const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const NS_ASVG: &str = "http://schemas.microsoft.com/office/drawing/2016/SVG/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PPTX_DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const ODF_DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";
/// EMU per point.
const EMU: u32 = 12_700;

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
            let stale: Vec<String> = generate()
                .into_iter()
                .filter(|(name, bytes)| std::fs::read(root.join(name)).ok().as_ref() != Some(bytes))
                .map(|(name, _)| name)
                .collect();
            if stale.is_empty() {
                println!("probes/out picture files are up to date");
                Ok(())
            } else {
                Err(format!(
                    "stale (run cargo xtask probes-pictures): {}",
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
        (
            "picture-probe.pptx".to_owned(),
            probes_zip::write_zip(&pptx(&VECTORS)),
        ),
        (
            "picture-probe.odp".to_owned(),
            probes_zip::write_zip(&odp(&VECTORS)),
        ),
        (
            "picture-labels-probe.pptx".to_owned(),
            probes_zip::write_zip(&pptx(&LABELLED)),
        ),
        (
            "picture-labels-probe.odp".to_owned(),
            probes_zip::write_zip(&odp(&LABELLED)),
        ),
        (
            "licenses/pacifico-OFL.txt".to_owned(),
            PACIFICO_OFL.to_vec(),
        ),
        (
            "picture-types-probe.pptx".to_owned(),
            probes_zip::write_zip(&pptx(&RASTERS)),
        ),
        (
            "picture-types-probe.odp".to_owned(),
            probes_zip::write_zip(&odp(&RASTERS)),
        ),
        (
            "heic-probe.pptx".to_owned(),
            probes_zip::write_zip(&pptx(&HEIC_PHOTOS)),
        ),
        (
            "heic-probe.odp".to_owned(),
            probes_zip::write_zip(&odp(&HEIC_PHOTOS)),
        ),
    ]
}

fn xml(name: &str, text: String) -> Entry {
    Entry {
        name: name.to_owned(),
        data: text.into_bytes(),
        stored: false,
    }
}

fn stored(name: String, data: Vec<u8>) -> Entry {
    Entry {
        name,
        data,
        stored: true,
    }
}

/// A 16 × 9 PNG of one colour: RGB, 8 bits, each row unfiltered, zlib around the deflate stream.
fn png(rgb: [u8; 3]) -> Vec<u8> {
    let (width, height) = (16u32, 9u32);
    // Each row: filter type 0, then its pixels.
    let row: Vec<u8> = std::iter::once(0)
        .chain(rgb.repeat(width as usize))
        .collect();
    let raw = row.repeat(height as usize);
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    let mut zlib = vec![0x78, 0x01];
    zlib.extend_from_slice(&probes_zip::deflate(&raw));
    zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [(&b"IHDR"[..], ihdr), (b"IDAT", zlib), (b"IEND", Vec::new())] {
        // A chunk is far under 4 GiB.
        #[allow(clippy::cast_possible_truncation)]
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(&data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&probes_zip::crc32(&body).to_be_bytes());
    }
    out
}

fn rels(list: &[(String, &str, String)]) -> String {
    let mut out = format!(
        "{PPTX_DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">"
    );
    for (id, kind, target) in list {
        let _ = write!(
            out,
            "<Relationship Id=\"{id}\" Type=\"{REL}/{kind}\" Target=\"{target}\"/>"
        );
    }
    out.push_str("</Relationships>");
    out
}

fn rel(id: &str, kind: &'static str, target: &str) -> (String, &'static str, String) {
    (id.to_owned(), kind, target.to_owned())
}

const GROUP: &str = "<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>";

/// The `Default` content types of the media parts: PNG and SVG, then each raster's own.
fn media_defaults(pictures: &[Picture]) -> String {
    let mut out = "<Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"svg\" ContentType=\"image/svg+xml\"/>".to_owned();
    let mut written: Vec<&str> = Vec::new();
    for picture in pictures {
        if let Shows::Raster { ext, mime, .. } = picture.shows
            && !written.contains(&ext)
        {
            written.push(ext);
            let _ = write!(out, "<Default Extension=\"{ext}\" ContentType=\"{mime}\"/>");
        }
    }
    out
}

fn pptx(pictures: &[Picture]) -> Vec<Entry> {
    let mut tree = String::new();
    let mut slide_rels = Vec::new();
    for (n, picture) in pictures.iter().enumerate() {
        let (x, y, w, h) = picture.frame;
        let blip = match picture.shows {
            Shows::Vector { .. } => {
                let (png_id, svg_id) = (format!("rIdPng{n}"), format!("rIdSvg{n}"));
                slide_rels.push(rel(&png_id, "image", &format!("../media/image{n}.png")));
                slide_rels.push(rel(&svg_id, "image", &format!("../media/image{n}.svg")));
                format!(
                    "<a:blip r:embed=\"{png_id}\"><a:extLst><a:ext uri=\"{{96DAC541-7B7A-43D3-8B79-37D633B846F1}}\"><asvg:svgBlip xmlns:asvg=\"{NS_ASVG}\" r:embed=\"{svg_id}\"/></a:ext></a:extLst></a:blip>"
                )
            }
            Shows::Raster { ext, .. } => {
                let id = format!("rIdImg{n}");
                slide_rels.push(rel(&id, "image", &format!("../media/image{n}.{ext}")));
                format!("<a:blip r:embed=\"{id}\"/>")
            }
        };
        let _ = write!(
            tree,
            "<p:pic><p:nvPicPr><p:cNvPr id=\"{}\" name=\"{}\"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill>{blip}<a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>",
            n + 2,
            picture.name,
            i64::from(x) * i64::from(EMU),
            i64::from(y) * i64::from(EMU),
            w * EMU,
            h * EMU
        );
    }
    let mut media = Vec::new();
    for (n, picture) in pictures.iter().enumerate() {
        match picture.shows {
            Shows::Vector { svg, preview } => {
                media.push(stored(format!("ppt/media/image{n}.png"), png(preview)));
                media.push(stored(
                    format!("ppt/media/image{n}.svg"),
                    svg().into_bytes(),
                ));
            }
            Shows::Raster { ext, bytes, .. } => {
                media.push(stored(format!("ppt/media/image{n}.{ext}"), bytes()));
            }
        }
    }
    pptx_package(&media_defaults(pictures), &tree, slide_rels, media)
}

/// A one-slide PPTX around a slide tree: the package, a blank master and layout, the probe theme, the slide with its
/// relationships (after the layout's) and the media parts.
pub(crate) fn pptx_package(
    defaults: &str,
    tree: &str,
    slide_rels: Vec<(String, &'static str, String)>,
    media: Vec<Entry>,
) -> Vec<Entry> {
    let ct = "application/vnd.openxmlformats-officedocument.presentationml";
    let types = format!(
        "{PPTX_DECL}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>{defaults}<Override PartName=\"/ppt/presentation.xml\" ContentType=\"{ct}.presentation.main+xml\"/><Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"{ct}.slideMaster+xml\"/><Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"{ct}.slideLayout+xml\"/><Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/><Override PartName=\"/ppt/slides/slide1.xml\" ContentType=\"{ct}.slide+xml\"/></Types>"
    );
    let mut entries = vec![
        xml("[Content_Types].xml", types),
        xml(
            "_rels/.rels",
            rels(&[rel("rId1", "officeDocument", "ppt/presentation.xml")]),
        ),
        xml(
            "ppt/presentation.xml",
            format!(
                "{PPTX_DECL}<p:presentation xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\"><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst><p:sldIdLst><p:sldId id=\"256\" r:id=\"rId3\"/></p:sldIdLst><p:sldSz cx=\"12192000\" cy=\"6858000\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"
            ),
        ),
        xml(
            "ppt/_rels/presentation.xml.rels",
            rels(&[
                rel("rId1", "slideMaster", "slideMasters/slideMaster1.xml"),
                rel("rId2", "theme", "theme/theme1.xml"),
                rel("rId3", "slide", "slides/slide1.xml"),
            ]),
        ),
        xml(
            "ppt/slideMasters/slideMaster1.xml",
            format!(
                "{PPTX_DECL}<p:sldMaster xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\"><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"F4F1EA\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree>{GROUP}</p:spTree></p:cSld><p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/><p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst></p:sldMaster>"
            ),
        ),
        xml(
            "ppt/slideMasters/_rels/slideMaster1.xml.rels",
            rels(&[
                rel("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"),
                rel("rId2", "theme", "../theme/theme1.xml"),
            ]),
        ),
        xml(
            "ppt/slideLayouts/slideLayout1.xml",
            format!(
                "{PPTX_DECL}<p:sldLayout xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\" type=\"blank\" preserve=\"1\"><p:cSld name=\"Blank\"><p:spTree>{GROUP}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
            ),
        ),
        xml(
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            rels(&[rel(
                "rId1",
                "slideMaster",
                "../slideMasters/slideMaster1.xml",
            )]),
        ),
        xml("ppt/theme/theme1.xml", theme()),
    ];
    let mut rels_list = vec![rel(
        "rId1",
        "slideLayout",
        "../slideLayouts/slideLayout1.xml",
    )];
    rels_list.extend(slide_rels);
    entries.push(xml(
        "ppt/slides/slide1.xml",
        format!(
            "{PPTX_DECL}<p:sld xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\"><p:cSld><p:spTree>{GROUP}{tree}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
        ),
    ));
    entries.push(xml("ppt/slides/_rels/slide1.xml.rels", rels(&rels_list)));
    entries.extend(media);
    entries
}

fn theme() -> String {
    let mut colors = String::new();
    for (name, value) in [
        ("dk1", "000000"),
        ("lt1", "FFFFFF"),
        ("dk2", "1F2937"),
        ("lt2", "F4F1EA"),
        ("accent1", "FE7867"),
        ("accent2", "04455E"),
        ("accent3", "2E7D32"),
        ("accent4", "6A1B9A"),
        ("accent5", "F9A825"),
        ("accent6", "00838F"),
        ("hlink", "1565C0"),
        ("folHlink", "6A1B9A"),
    ] {
        let _ = write!(colors, "<a:{name}><a:srgbClr val=\"{value}\"/></a:{name}>");
    }
    let font = "<a:latin typeface=\"Liberation Sans\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/>";
    let fill = "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>";
    let line = "<a:ln w=\"6350\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln>";
    format!(
        "{PPTX_DECL}<a:theme xmlns:a=\"{NS_A}\" name=\"Picture probe\"><a:themeElements><a:clrScheme name=\"Picture probe\">{colors}</a:clrScheme><a:fontScheme name=\"Picture probe\"><a:majorFont>{font}</a:majorFont><a:minorFont>{font}</a:minorFont></a:fontScheme><a:fmtScheme name=\"Picture probe\"><a:fillStyleLst>{fill}{fill}{fill}</a:fillStyleLst><a:lnStyleLst>{line}{line}{line}</a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst>{fill}{fill}{fill}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"
    )
}

const ODF_NAMESPACES: &str = concat!(
    " xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"",
    " xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\"",
    " xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\"",
    " xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\"",
    " xmlns:xlink=\"http://www.w3.org/1999/xlink\"",
    " xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\"",
    " xmlns:presentation=\"urn:oasis:names:tc:opendocument:xmlns:presentation:1.0\""
);

fn odp(pictures: &[Picture]) -> Vec<Entry> {
    let mut files = String::new();
    let mut frames = String::new();
    for (n, picture) in pictures.iter().enumerate() {
        let parts: &[(&str, &str)] = match picture.shows {
            Shows::Vector { .. } => &[("svg", "image/svg+xml"), ("png", "image/png")],
            Shows::Raster { ext, mime, .. } => &[(ext, mime)],
        };
        let mut images = String::new();
        for (kind, mime) in parts {
            let _ = write!(
                files,
                "<manifest:file-entry manifest:full-path=\"Pictures/image{n}.{kind}\" manifest:media-type=\"{mime}\"/>"
            );
            let _ = write!(
                images,
                "<draw:image xlink:href=\"Pictures/image{n}.{kind}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\" draw:mime-type=\"{mime}\"/>"
            );
        }
        let (x, y, w, h) = picture.frame;
        let _ = write!(
            frames,
            "<draw:frame draw:style-name=\"grPicture\" draw:name=\"{}\" svg:x=\"{x}pt\" svg:y=\"{y}pt\" svg:width=\"{w}pt\" svg:height=\"{h}pt\">{images}</draw:frame>",
            picture.name,
        );
    }
    let mut media = Vec::new();
    for (n, picture) in pictures.iter().enumerate() {
        match picture.shows {
            Shows::Vector { svg, preview } => {
                media.push(stored(format!("Pictures/image{n}.svg"), svg().into_bytes()));
                media.push(stored(format!("Pictures/image{n}.png"), png(preview)));
            }
            Shows::Raster { ext, bytes, .. } => {
                media.push(stored(format!("Pictures/image{n}.{ext}"), bytes()));
            }
        }
    }
    odp_package(
        &files,
        "<style:style style:name=\"grPicture\" style:family=\"graphic\"><style:graphic-properties draw:fill=\"none\" draw:stroke=\"none\"/></style:style>",
        "",
        "Pictures",
        &frames,
        media,
    )
}

/// A one-page ODP: the manifest (its own parts, then `files`), the content (the automatic styles and one page of
/// `drawables`), the styles (`styles`, a 960 × 540 pt page on a plain master) and the media parts.
pub(crate) fn odp_package(
    files: &str,
    automatic_styles: &str,
    styles: &str,
    page: &str,
    drawables: &str,
    media: Vec<Entry>,
) -> Vec<Entry> {
    let manifest = format!(
        "{ODF_DECL}<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\"><manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.3\" manifest:media-type=\"application/vnd.oasis.opendocument.presentation\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/><manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/>{files}</manifest:manifest>"
    );
    let content = format!(
        "{ODF_DECL}<office:document-content{ODF_NAMESPACES} office:version=\"1.3\"><office:automatic-styles>{automatic_styles}</office:automatic-styles><office:body><office:presentation><draw:page draw:name=\"{page}\" draw:master-page-name=\"Probe\">{drawables}</draw:page></office:presentation></office:body></office:document-content>"
    );
    let office_styles = if styles.is_empty() {
        "<office:styles/>".to_owned()
    } else {
        format!("<office:styles>{styles}</office:styles>")
    };
    let styles = format!(
        "{ODF_DECL}<office:document-styles{ODF_NAMESPACES} office:version=\"1.3\">{office_styles}<office:automatic-styles><style:page-layout style:name=\"PM1\"><style:page-layout-properties fo:margin-top=\"0pt\" fo:margin-bottom=\"0pt\" fo:margin-left=\"0pt\" fo:margin-right=\"0pt\" fo:page-width=\"960pt\" fo:page-height=\"540pt\" style:print-orientation=\"landscape\"/></style:page-layout><style:style style:name=\"Mdp1\" style:family=\"drawing-page\"><style:drawing-page-properties draw:fill=\"solid\" draw:fill-color=\"#F4F1EA\"/></style:style></office:automatic-styles><office:master-styles><style:master-page style:name=\"Probe\" style:page-layout-name=\"PM1\" draw:style-name=\"Mdp1\"/></office:master-styles></office:document-styles>"
    );
    let mut entries = vec![
        stored(
            "mimetype".to_owned(),
            b"application/vnd.oasis.opendocument.presentation".to_vec(),
        ),
        xml("META-INF/manifest.xml", manifest),
        xml("content.xml", content),
        xml("styles.xml", styles),
    ];
    entries.extend(media);
    entries
}
