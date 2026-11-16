//! The shadow probe decks: 960 × 540 pt slides, one case per slide, each named by a 12 pt Liberation Sans label in its
//! top-left corner. Every drawing is our own: the photo, the clip and its poster come from
//! `probes/shadows/make-media.sh`; the alpha PNGs are drawn here. Unless a case says otherwise the object is a
//! 240 × 160 pt white rectangle in the middle of a #F2F2F2 slide, and its shadow is black at 50 %, 20 pt away at 45°
//! (clockwise from +x), unblurred: LibreOffice prints an unblurred shadow as vectors, so its offset and colour can be
//! read from its PDF to about 0.01 pt.
//!
//! The PPTX writes every shadow as Keynote's PPTX export does (`sx`/`sy` 100 %, `kx`/`ky` 0, `algn="b"`,
//! `rotWithShape="0"`). The ODP twin writes the same numbers as `draw:shadow-offset-x`/`-y` (pt), `draw:shadow-color`,
//! `draw:shadow-opacity` and `loext:shadow-blur` (pt), on styles that set every graphic property they use.
//!
//! `shadow-probe.pptx` and `shadow-probe.odp`, twins (LibreOffice and Keynote oracles):
//!
//! - A01–A08: direction 0, 45, …, 315.
//! - A09–A11: distance 0 (blur 10), 3.287 and 60.
//! - A12–A17: blur 3, 5, 14, 25, 50 and 100 (direction 90).
//! - A18–A20: red at 100 %; blue at 25 %; white at 53.314 % on #202040 (retro's set B: direction 90, distance 3.287,
//!   blur 14).
//! - A21: retro's set A (black 53.314 %, direction 90, distance 3.287, blur 14) on the 1000 × 600 photo.
//! - A22–A25: an ellipse; a custom star; a stroke-only rectangle (8 pt line); a straight 8 pt line ending in a
//!   triangle.
//! - A26: a text box with no fill and no line, "Shadow" at 72 pt: the glyphs cast the shadow.
//! - A27–A28: an opaque filled text box; the same with text overflowing its bottom.
//! - A29: a fill at 50 % under a shadow at 100 %.
//! - A30–A35: the photo; a PNG of a disc with a hole; the photo with an 8 pt border; cropped; in a round-rect
//!   geometry (ODP: a custom shape with the photo as its bitmap fill); at 50 % opacity.
//! - A36: a video with its poster.
//! - A37–A39: the rectangle turned 30°, 90° and 180° (direction 90).
//! - A40–A41: a right triangle flipped horizontally; vertically (direction 0).
//! - A42: the photo turned 30°.
//! - A43: a group of two shadowed rectangles, the group turned 30° (no group shadow).
//! - A44: two overlapping shadowed rectangles: the second's shadow falls on the first.
//! - A45: a shadow crossing the slide's right edge (direction 0, distance 60, blur 25).
//! - A46: 48 small shadowed rectangles at blur 25 (cost).
//!
//! `shadow-probe-odp.odp`, LibreOffice-only semantics. Its own "standard" graphic style holds a red shadow at 50 %,
//! 0.5 cm right and 1 cm down (hidden), and "objectwithshadow" LibreOffice's own (0.2 cm, #808080, visible):
//!
//! - B01: an unparented style with only `draw:shadow="visible"` (the "standard" under it gives the rest).
//! - B02: the same style parented to "standard" by name.
//! - B03: a style parented to "objectwithshadow", its colour blue.
//! - B04: a title placeholder whose presentation style says only visible and blur 0.5 cm.
//! - B05: a subtitle placeholder whose master style ("Probe-subtitle") holds a blue shadow 0.3 cm away.
//! - B06–B08: a fill at 50 % under a shadow at 50 % (LibreOffice's equal-transparency rule); at 40 %; a shadow at 0 %.
//! - B09: `fo:text-shadow` on black text at 12, 24, 48 and 96 pt.
//! - B10: the same on red text, the value `5pt 5pt 3pt #00ff00`.
//! - B11: a character shadow on text turned 30°.
//! - B12: a 2 × 2 table frame, filled on its diagonal, with a shadow.
//! - B13: a `draw:g` with a shadow style around two rectangles without one.
//! - B14: a bent connector ending in an arrow.
//! - B15: a PNG whose alpha ramps from 0 on the left to 1 on the right.
//! - B16: a shadowed rectangle on the slide's master page.
//!
//! `shadow-probe-ooxml.pptx`, OOXML-only (LibreOffice's PDF is a reference, never the truth):
//!
//! - C01: `effectRef idx="2"` (the theme's black shadow). C02: the same with an empty own `a:effectLst`. C03: the same
//!   with an own green shadow. C04: `effectRef idx="3"`, whose theme shadow is `phClr`, with a blue reference colour.
//!   C05: a title placeholder that inherits its layout placeholder's shadow.
//! - C06: `innerShdw`. C07–C09: `prstShdw` shdw1, shdw13 and shdw17. C10: an outer and an inner shadow.
//! - C11: `sx`/`sy` 50 % aligned top-left. C12: `sx` 150 % aligned top. C13: `sy` 50 % only. C14: `kx` 20°.
//! - C15: `rotWithShape="1"` on a rectangle turned 30°. C16: `rotWithShape` absent on the same. C17: a right triangle
//!   flipped horizontally with `rotWithShape="1"`.
//! - C18: a group's own shadow. C19: a table's shadow. C20: a run's shadow (`a:rPr`). C21: a run shadow inherited from
//!   the body's `a:lstStyle`. C22: a shadow in an `a:effectDag`.
//! - C23: a fill at 50 % under a shadow at 50 %. C24: the photo at `alphaModFix` 50 % with a shadow. C25: a shape with
//!   no fill, no line and no text, with a shadow (draws nothing).

use std::fmt::Write as _;
use std::path::Path;

use crate::probes_pictures::probes_zip::{self, Entry};

const USAGE: &str = "usage: cargo xtask probes-shadows [--check]";

const PHOTO: &[u8] = include_bytes!("../../probes/shadows/media/photo.jpg");
const CLIP: &[u8] = include_bytes!("../../probes/shadows/media/clip.mp4");
const POSTER: &[u8] = include_bytes!("../../probes/shadows/media/poster.png");

/// EMU per point.
const EMU: f64 = 12_700.0;

const WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];
pub(crate) const BLACK: [u8; 3] = [0, 0, 0];
const INK: [u8; 3] = [0x1F, 0x29, 0x37];
const BLUE: [u8; 3] = [0x2F, 0x6F, 0xDF];
const AMBER: [u8; 3] = [0xF9, 0xA8, 0x25];
pub(crate) const SLIDE: [u8; 3] = [0xF2, 0xF2, 0xF2];
const DARK: [u8; 3] = [0x20, 0x20, 0x40];
pub(crate) const LABEL: [u8; 3] = [0x33, 0x33, 0x33];

/// Opacity in OOXML's thousandths of a percent.
pub(crate) const OPAQUE: u32 = 100_000;
/// Keynote's opacity in the user's deck (0.53314), as its PPTX export writes it.
const RETRO_ALPHA: u32 = 53_314;

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
                println!("probes/out shadow files are up to date");
                Ok(())
            } else {
                Err(format!(
                    "stale (run cargo xtask probes-shadows): {}",
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
    let twins = twin_slides();
    vec![
        (
            "shadow-probe.pptx".to_owned(),
            probes_zip::write_zip(&twin_pptx(&twins)),
        ),
        (
            "shadow-probe.odp".to_owned(),
            probes_zip::write_zip(&twin_odp(&twins)),
        ),
        (
            "shadow-probe-odp.odp".to_owned(),
            probes_zip::write_zip(&odp_only()),
        ),
        (
            "shadow-probe-ooxml.pptx".to_owned(),
            probes_zip::write_zip(&ooxml_only()),
        ),
    ]
}

// ---------------------------------------------------------------------------------------------------------------------
// The twin model

/// An outer shadow: distance (pt) at a direction (degrees clockwise from +x), blur (pt), colour and opacity.
#[derive(Clone, Copy)]
pub(crate) struct Shadow {
    pub(crate) dist: f64,
    pub(crate) dir: i32,
    pub(crate) blur: f64,
    pub(crate) color: [u8; 3],
    pub(crate) alpha: u32,
}

const BASE: Shadow = Shadow {
    dist: 20.0,
    dir: 45,
    blur: 0.0,
    color: BLACK,
    alpha: 50_000,
};

impl Shadow {
    /// The offset in page points, to 4 decimals.
    fn offset(self) -> (f64, f64) {
        let (cos, sin) = cos_sin(self.dir);
        (round4(self.dist * cos), round4(self.dist * sin))
    }
}

/// The cosine and sine of the probes' angles, exact to the last bit (no libm: the outputs are the same on every host).
pub(crate) fn cos_sin(degrees: i32) -> (f64, f64) {
    let half = std::f64::consts::FRAC_1_SQRT_2;
    let third = 3f64.sqrt() / 2.0;
    match degrees.rem_euclid(360) {
        0 => (1.0, 0.0),
        30 => (third, 0.5),
        45 => (half, half),
        90 => (0.0, 1.0),
        135 => (-half, half),
        180 => (-1.0, 0.0),
        225 => (-half, -half),
        270 => (0.0, -1.0),
        315 => (half, -half),
        other => panic!("no exact cosine for {other} degrees"),
    }
}

fn round4(value: f64) -> f64 {
    let rounded = (value * 10_000.0).round() / 10_000.0;
    if rounded == 0.0 { 0.0 } else { rounded }
}

/// A number with at most 4 decimals and no trailing zeros.
pub(crate) fn num(value: f64) -> String {
    let text = format!("{:.4}", round4(value));
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

pub(crate) fn emu(points: f64) -> i64 {
    // Far inside i64.
    #[allow(clippy::cast_possible_truncation)]
    let value = (points * EMU).round() as i64;
    value
}

pub(crate) fn hex(color: [u8; 3]) -> String {
    format!("{:02X}{:02X}{:02X}", color[0], color[1], color[2])
}

pub(crate) fn odf_color(color: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

/// A percentage of OOXML thousandths, at most 3 decimals.
pub(crate) fn percent(alpha: u32) -> String {
    format!("{}%", num(f64::from(alpha) / 1000.0))
}

#[derive(Clone, Copy, PartialEq)]
enum Geom {
    Rect,
    Ellipse,
    RightTriangle,
    Star,
}

#[derive(Clone, Copy)]
struct Stroke {
    width: f64,
    color: [u8; 3],
}

#[derive(Clone, Copy)]
enum Image {
    Photo,
    Disc,
}

impl Image {
    fn file(self) -> &'static str {
        match self {
            Image::Photo => "photo.jpg",
            Image::Disc => "disc.png",
        }
    }

    fn mime(self) -> &'static str {
        match self {
            Image::Photo => "image/jpeg",
            Image::Disc => "image/png",
        }
    }

    /// The image's own size in points (the photo is tagged 72 dpi; the PNG has no density, so 96 dpi).
    fn size(self) -> (f64, f64) {
        match self {
            Image::Photo => (1000.0, 600.0),
            Image::Disc => (180.0, 120.0),
        }
    }
}

struct Text {
    lines: &'static [&'static str],
    size: u32,
    color: [u8; 3],
}

enum Kind {
    Shape {
        geom: Geom,
        /// The fill colour and its opacity, or no fill.
        fill: Option<([u8; 3], u32)>,
        stroke: Option<Stroke>,
        text: Option<Text>,
    },
    /// A straight line from the frame's top-left to its bottom-right, ending in a triangle.
    Arrow(Stroke),
    Picture {
        image: Image,
        /// Left, top, right and bottom crop in OOXML thousandths of a percent.
        crop: Option<[u32; 4]>,
        border: Option<Stroke>,
        round: bool,
        alpha: Option<u32>,
    },
    Video,
    Group(Vec<Item>),
}

struct Item {
    name: String,
    frame: (f64, f64, f64, f64),
    rot: i32,
    flip_h: bool,
    flip_v: bool,
    kind: Kind,
    shadow: Option<Shadow>,
}

impl Item {
    fn new(name: &str, frame: (f64, f64, f64, f64), kind: Kind, shadow: Option<Shadow>) -> Item {
        Item {
            name: name.to_owned(),
            frame,
            rot: 0,
            flip_h: false,
            flip_v: false,
            kind,
            shadow,
        }
    }

    fn turned(mut self, rot: i32) -> Item {
        self.rot = rot;
        self
    }
}

struct TwinSlide {
    label: String,
    background: [u8; 3],
    items: Vec<Item>,
}

const CENTRE: (f64, f64, f64, f64) = (360.0, 190.0, 240.0, 160.0);

fn shape(geom: Geom, fill: Option<([u8; 3], u32)>, stroke: Option<Stroke>) -> Kind {
    Kind::Shape {
        geom,
        fill,
        stroke,
        text: None,
    }
}

fn rect(name: &str, frame: (f64, f64, f64, f64), fill: [u8; 3], shadow: Shadow) -> Item {
    Item::new(
        name,
        frame,
        shape(Geom::Rect, Some((fill, OPAQUE)), None),
        Some(shadow),
    )
}

fn picture(image: Image) -> Kind {
    Kind::Picture {
        image,
        crop: None,
        border: None,
        round: false,
        alpha: None,
    }
}

fn twin_slides() -> Vec<TwinSlide> {
    let mut slides = Vec::new();
    let mut add = |label: String, background: [u8; 3], items: Vec<Item>| {
        slides.push(TwinSlide {
            label,
            background,
            items,
        });
    };
    let n = |k: usize| format!("A{k:02}");
    for (k, dir) in [0, 45, 90, 135, 180, 225, 270, 315].into_iter().enumerate() {
        let shadow = Shadow { dir, ..BASE };
        add(
            format!("{} direction {dir}", n(k + 1)),
            SLIDE,
            vec![rect("Rectangle", CENTRE, WHITE, shadow)],
        );
    }
    for (k, (dist, blur)) in [(0.0, 10.0), (3.287, 0.0), (60.0, 0.0)]
        .into_iter()
        .enumerate()
    {
        let shadow = Shadow { dist, blur, ..BASE };
        let blurred = if blur > 0.0 {
            format!(", blur {}", num(blur))
        } else {
            String::new()
        };
        add(
            format!("{} distance {}{blurred}", n(k + 9), num(dist)),
            SLIDE,
            vec![rect("Rectangle", CENTRE, WHITE, shadow)],
        );
    }
    for (k, blur) in [3.0, 5.0, 14.0, 25.0, 50.0, 100.0].into_iter().enumerate() {
        let shadow = Shadow {
            dir: 90,
            blur,
            ..BASE
        };
        add(
            format!("{} blur {} (direction 90)", n(k + 12), num(blur)),
            SLIDE,
            vec![rect("Rectangle", CENTRE, WHITE, shadow)],
        );
    }
    let retro = Shadow {
        dist: 3.287,
        dir: 90,
        blur: 14.0,
        color: BLACK,
        alpha: RETRO_ALPHA,
    };
    add(
        "A18 red at 100 %".to_owned(),
        SLIDE,
        vec![rect(
            "Rectangle",
            CENTRE,
            WHITE,
            Shadow {
                color: [0xFF, 0, 0],
                alpha: OPAQUE,
                ..BASE
            },
        )],
    );
    add(
        "A19 blue at 25 %".to_owned(),
        SLIDE,
        vec![rect(
            "Rectangle",
            CENTRE,
            WHITE,
            Shadow {
                color: [0, 0, 0xFF],
                alpha: 25_000,
                ..BASE
            },
        )],
    );
    add(
        "A20 white at 53.314 % on dark (retro set B)".to_owned(),
        DARK,
        vec![rect(
            "Rectangle",
            CENTRE,
            WHITE,
            Shadow {
                color: WHITE,
                ..retro
            },
        )],
    );
    add(
        "A21 retro set A on a 1000 x 600 photo".to_owned(),
        SLIDE,
        vec![Item::new(
            "Photo",
            (230.0, 120.0, 500.0, 300.0),
            picture(Image::Photo),
            Some(retro),
        )],
    );
    add(
        "A22 ellipse".to_owned(),
        SLIDE,
        vec![Item::new(
            "Ellipse",
            CENTRE,
            shape(Geom::Ellipse, Some((WHITE, OPAQUE)), None),
            Some(BASE),
        )],
    );
    add(
        "A23 custom star".to_owned(),
        SLIDE,
        vec![Item::new(
            "Star",
            (380.0, 170.0, 200.0, 200.0),
            shape(Geom::Star, Some((WHITE, OPAQUE)), None),
            Some(BASE),
        )],
    );
    add(
        "A24 stroke-only rectangle (8 pt line)".to_owned(),
        SLIDE,
        vec![Item::new(
            "Outline",
            CENTRE,
            shape(
                Geom::Rect,
                None,
                Some(Stroke {
                    width: 8.0,
                    color: INK,
                }),
            ),
            Some(BASE),
        )],
    );
    add(
        "A25 8 pt line with a triangle end".to_owned(),
        SLIDE,
        vec![Item::new(
            "Arrow",
            (300.0, 270.0, 360.0, 0.0),
            Kind::Arrow(Stroke {
                width: 8.0,
                color: INK,
            }),
            Some(BASE),
        )],
    );
    let text_box = |lines: &'static [&'static str], size: u32, fill: Option<[u8; 3]>| Kind::Shape {
        geom: Geom::Rect,
        fill: fill.map(|color| (color, OPAQUE)),
        stroke: None,
        text: Some(Text {
            lines,
            size,
            color: INK,
        }),
    };
    add(
        "A26 text box, no fill, no line (72 pt)".to_owned(),
        SLIDE,
        vec![Item::new(
            "Glyphs",
            (300.0, 200.0, 360.0, 120.0),
            text_box(&["Shadow"], 72, None),
            Some(BASE),
        )],
    );
    add(
        "A27 opaque filled text box".to_owned(),
        SLIDE,
        vec![Item::new(
            "Filled text",
            CENTRE,
            text_box(&["Filled", "text box"], 40, Some(WHITE)),
            Some(BASE),
        )],
    );
    add(
        "A28 opaque filled text box, text overflowing".to_owned(),
        SLIDE,
        vec![Item::new(
            "Overflowing text",
            CENTRE,
            text_box(&["One", "Two", "Three", "Four", "Five"], 40, Some(WHITE)),
            Some(BASE),
        )],
    );
    add(
        "A29 fill at 50 %, shadow at 100 %".to_owned(),
        SLIDE,
        vec![Item::new(
            "Translucent",
            CENTRE,
            shape(Geom::Rect, Some((BLUE, 50_000)), None),
            Some(Shadow {
                alpha: OPAQUE,
                ..BASE
            }),
        )],
    );
    let photo_frame = (360.0, 198.0, 240.0, 144.0);
    add(
        "A30 JPEG picture".to_owned(),
        SLIDE,
        vec![Item::new(
            "Photo",
            photo_frame,
            picture(Image::Photo),
            Some(BASE),
        )],
    );
    add(
        "A31 PNG with alpha (a disc with a hole)".to_owned(),
        SLIDE,
        vec![Item::new("Disc", CENTRE, picture(Image::Disc), Some(BASE))],
    );
    add(
        "A32 picture with an 8 pt border".to_owned(),
        SLIDE,
        vec![Item::new(
            "Bordered photo",
            photo_frame,
            Kind::Picture {
                image: Image::Photo,
                crop: None,
                border: Some(Stroke {
                    width: 8.0,
                    color: INK,
                }),
                round: false,
                alpha: None,
            },
            Some(BASE),
        )],
    );
    add(
        "A33 cropped picture".to_owned(),
        SLIDE,
        vec![Item::new(
            "Cropped photo",
            photo_frame,
            Kind::Picture {
                image: Image::Photo,
                crop: Some([20_000, 10_000, 10_000, 20_000]),
                border: None,
                round: false,
                alpha: None,
            },
            Some(BASE),
        )],
    );
    add(
        "A34 picture in a round rectangle".to_owned(),
        SLIDE,
        vec![Item::new(
            "Round photo",
            photo_frame,
            Kind::Picture {
                image: Image::Photo,
                crop: None,
                border: None,
                round: true,
                alpha: None,
            },
            Some(BASE),
        )],
    );
    add(
        "A35 picture at 50 % opacity".to_owned(),
        SLIDE,
        vec![Item::new(
            "Faded photo",
            photo_frame,
            Kind::Picture {
                image: Image::Photo,
                crop: None,
                border: None,
                round: false,
                alpha: Some(50_000),
            },
            Some(BASE),
        )],
    );
    add(
        "A36 video with a poster".to_owned(),
        SLIDE,
        vec![Item::new(
            "Clip",
            (320.0, 180.0, 320.0, 180.0),
            Kind::Video,
            Some(BASE),
        )],
    );
    for (k, rot) in [30, 90, 180].into_iter().enumerate() {
        let shadow = Shadow { dir: 90, ..BASE };
        add(
            format!("{} rectangle turned {rot} (direction 90)", n(k + 37)),
            SLIDE,
            vec![rect("Turned rectangle", CENTRE, WHITE, shadow).turned(rot)],
        );
    }
    for (k, (flip_h, flip_v)) in [(true, false), (false, true)].into_iter().enumerate() {
        let mut item = Item::new(
            "Triangle",
            CENTRE,
            shape(Geom::RightTriangle, Some((WHITE, OPAQUE)), None),
            Some(Shadow { dir: 0, ..BASE }),
        );
        item.flip_h = flip_h;
        item.flip_v = flip_v;
        let flip = if flip_h { "horizontally" } else { "vertically" };
        add(
            format!("{} right triangle flipped {flip} (direction 0)", n(k + 40)),
            SLIDE,
            vec![item],
        );
    }
    add(
        "A42 picture turned 30".to_owned(),
        SLIDE,
        vec![
            Item::new(
                "Turned photo",
                photo_frame,
                picture(Image::Photo),
                Some(BASE),
            )
            .turned(30),
        ],
    );
    add(
        "A43 group of two shadowed rectangles, turned 30".to_owned(),
        SLIDE,
        vec![
            Item::new(
                "Group",
                (300.0, 170.0, 360.0, 200.0),
                Kind::Group(vec![
                    rect("Left", (300.0, 170.0, 160.0, 100.0), WHITE, BASE),
                    rect("Right", (500.0, 270.0, 160.0, 100.0), AMBER, BASE),
                ]),
                None,
            )
            .turned(30),
        ],
    );
    add(
        "A44 overlapping shadows".to_owned(),
        SLIDE,
        vec![
            rect("Under", (300.0, 150.0, 240.0, 160.0), WHITE, BASE),
            rect("Over", (420.0, 230.0, 240.0, 160.0), AMBER, BASE),
        ],
    );
    add(
        "A45 shadow crossing the slide edge (direction 0, distance 60, blur 25)".to_owned(),
        SLIDE,
        vec![rect(
            "Edge",
            (700.0, 190.0, 240.0, 160.0),
            WHITE,
            Shadow {
                dist: 60.0,
                dir: 0,
                blur: 25.0,
                ..BASE
            },
        )],
    );
    let mut many = Vec::new();
    for row in 0..6u32 {
        for col in 0..8u32 {
            many.push(rect(
                &format!("Cell {}", row * 8 + col + 1),
                (
                    40.0 + 112.0 * f64::from(col),
                    70.0 + 76.0 * f64::from(row),
                    80.0,
                    50.0,
                ),
                WHITE,
                Shadow { blur: 25.0, ..BASE },
            ));
        }
    }
    add("A46 48 shapes at blur 25".to_owned(), SLIDE, many);
    slides
}

/// A five-pointed star filling its 200 × 200 box, in hundredths of a point (a regular star of outer radius 100 and
/// inner 40, stretched to touch every side: LibreOffice fits an ODF polygon's points to its frame).
const STAR: [(i32, i32); 10] = [
    (10_000, 0),
    (12_472, 7_478),
    (20_000, 7_640),
    (14_000, 12_422),
    (16_180, 20_000),
    (10_000, 15_478),
    (3_820, 20_000),
    (6_000, 12_422),
    (0, 7_640),
    (7_528, 7_478),
];

// ---------------------------------------------------------------------------------------------------------------------
// PPTX

const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const NS_P14: &str = "http://schemas.microsoft.com/office/powerpoint/2010/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const MEDIA_REL: &str = "http://schemas.microsoft.com/office/2007/relationships/media";
const PPTX_DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const GROUP: &str = "<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>";

fn xml_entry(name: &str, text: String) -> Entry {
    Entry {
        name: name.to_owned(),
        data: text.into_bytes(),
        stored: false,
    }
}

fn stored(name: &str, data: Vec<u8>) -> Entry {
    Entry {
        name: name.to_owned(),
        data,
        stored: true,
    }
}

fn rels(list: &[(String, &str, String)]) -> String {
    let mut out = format!(
        "{PPTX_DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">"
    );
    for (id, kind, target) in list {
        let kind = if *kind == "media" {
            MEDIA_REL.to_owned()
        } else {
            format!("{REL}/{kind}")
        };
        let _ = write!(
            out,
            "<Relationship Id=\"{id}\" Type=\"{kind}\" Target=\"{target}\"/>"
        );
    }
    out.push_str("</Relationships>");
    out
}

/// One slide's tree and relationships, as a writer fills them.
pub(crate) struct PptxSlide {
    pub(crate) tree: String,
    pub(crate) rels: Vec<(String, &'static str, String)>,
    pub(crate) next_id: u32,
    pub(crate) layout: usize,
    pub(crate) background: Option<[u8; 3]>,
}

impl PptxSlide {
    pub(crate) fn new(layout: usize, background: Option<[u8; 3]>) -> PptxSlide {
        PptxSlide {
            tree: String::new(),
            rels: vec![(
                "rId1".to_owned(),
                "slideLayout",
                format!("../slideLayouts/slideLayout{layout}.xml"),
            )],
            next_id: 1,
            layout,
            background,
        }
    }

    pub(crate) fn id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    /// The relationship to a target, added once.
    fn rel(&mut self, kind: &'static str, target: &str) -> String {
        if let Some((id, _, _)) = self.rels.iter().find(|(_, k, t)| *k == kind && t == target) {
            return id.clone();
        }
        let id = format!("rId{}", self.rels.len() + 1);
        self.rels.push((id.clone(), kind, target.to_owned()));
        id
    }

    /// The 12 pt label naming the case, in the top-left corner.
    pub(crate) fn label(&mut self, text: &str, color: [u8; 3]) {
        let id = self.id();
        let _ = write!(
            self.tree,
            "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Label\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/><a:ln><a:noFill/></a:ln></p:spPr><p:txBody><a:bodyPr wrap=\"none\" lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\" anchor=\"t\" rtlCol=\"0\"><a:noAutofit/></a:bodyPr><a:lstStyle/><a:p>{}</a:p></p:txBody></p:sp>",
            xfrm((24.0, 12.0, 720.0, 20.0), 0, false, false),
            text_run(text, 12, color, "")
        );
    }
}

pub(crate) fn xfrm(
    (x, y, w, h): (f64, f64, f64, f64),
    rot: i32,
    flip_h: bool,
    flip_v: bool,
) -> String {
    let mut attrs = String::new();
    if rot != 0 {
        let _ = write!(attrs, " rot=\"{}\"", rot * 60_000);
    }
    if flip_h {
        attrs.push_str(" flipH=\"1\"");
    }
    if flip_v {
        attrs.push_str(" flipV=\"1\"");
    }
    format!(
        "<a:xfrm{attrs}><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>",
        emu(x),
        emu(y),
        emu(w),
        emu(h)
    )
}

fn srgb(color: [u8; 3], alpha: u32) -> String {
    if alpha >= OPAQUE {
        format!("<a:srgbClr val=\"{}\"/>", hex(color))
    } else {
        format!(
            "<a:srgbClr val=\"{}\"><a:alpha val=\"{alpha}\"/></a:srgbClr>",
            hex(color)
        )
    }
}

pub(crate) fn solid(color: [u8; 3], alpha: u32) -> String {
    format!("<a:solidFill>{}</a:solidFill>", srgb(color, alpha))
}

pub(crate) fn text_run(text: &str, size: u32, color: [u8; 3], effects: &str) -> String {
    format!(
        "<a:r><a:rPr lang=\"en-US\" sz=\"{}\" dirty=\"0\">{}{effects}<a:latin typeface=\"Liberation Sans\"/></a:rPr><a:t>{}</a:t></a:r>",
        size * 100,
        solid(color, OPAQUE),
        escape(text)
    )
}

pub(crate) fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Keynote's export of an outer shadow: no scale, skew or turn with the shape, aligned bottom.
pub(crate) fn outer(shadow: Shadow) -> String {
    format!(
        "<a:outerShdw sx=\"100000\" sy=\"100000\" kx=\"0\" ky=\"0\" algn=\"b\" rotWithShape=\"0\" blurRad=\"{}\" dist=\"{}\" dir=\"{}\">{}</a:outerShdw>",
        emu(shadow.blur),
        emu(shadow.dist),
        shadow.dir.rem_euclid(360) * 60_000,
        srgb(shadow.color, shadow.alpha)
    )
}

fn effects(shadow: Option<Shadow>) -> String {
    shadow.map_or_else(String::new, |s| {
        format!("<a:effectLst>{}</a:effectLst>", outer(s))
    })
}

fn line(stroke: Option<Stroke>) -> String {
    stroke.map_or_else(
        || "<a:ln><a:noFill/></a:ln>".to_owned(),
        |s| {
            format!(
                "<a:ln w=\"{}\">{}</a:ln>",
                emu(s.width),
                solid(s.color, OPAQUE)
            )
        },
    )
}

fn star_path() -> String {
    let mut path = String::new();
    for (k, (x, y)) in STAR.iter().enumerate() {
        let verb = if k == 0 { "moveTo" } else { "lnTo" };
        let _ = write!(path, "<a:{verb}><a:pt x=\"{x}\" y=\"{y}\"/></a:{verb}>");
    }
    format!(
        "<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l=\"0\" t=\"0\" r=\"r\" b=\"b\"/><a:pathLst><a:path w=\"20000\" h=\"20000\">{path}<a:close/></a:path></a:pathLst></a:custGeom>"
    )
}

fn geometry(geom: Geom) -> String {
    let preset = match geom {
        Geom::Rect => "rect",
        Geom::Ellipse => "ellipse",
        Geom::RightTriangle => "rtTriangle",
        Geom::Star => return star_path(),
    };
    format!("<a:prstGeom prst=\"{preset}\"><a:avLst/></a:prstGeom>")
}

/// A text body anchored at the top, PowerPoint's default insets, no autofit, one paragraph per line.
fn text_body(text: &Text, effects: &str) -> String {
    let mut out = "<p:txBody><a:bodyPr wrap=\"none\" lIns=\"91440\" tIns=\"45720\" rIns=\"91440\" bIns=\"45720\" anchor=\"t\" rtlCol=\"0\"><a:noAutofit/></a:bodyPr><a:lstStyle/>".to_owned();
    for line in text.lines {
        let _ = write!(
            out,
            "<a:p>{}</a:p>",
            text_run(line, text.size, text.color, effects)
        );
    }
    out.push_str("</p:txBody>");
    out
}

fn pptx_item(slide: &mut PptxSlide, item: &Item) -> String {
    let id = slide.id();
    let name = escape(&item.name);
    let frame = xfrm(item.frame, item.rot, item.flip_h, item.flip_v);
    let effect = effects(item.shadow);
    match &item.kind {
        Kind::Shape {
            geom,
            fill,
            stroke,
            text,
        } => {
            let fill = fill.map_or_else(|| "<a:noFill/>".to_owned(), |(c, a)| solid(c, a));
            let text_box = if text.is_some() { " txBox=\"1\"" } else { "" };
            let body = text.as_ref().map_or_else(String::new, |t| text_body(t, ""));
            format!(
                "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvSpPr{text_box}/><p:nvPr/></p:nvSpPr><p:spPr>{frame}{}{fill}{}{effect}</p:spPr>{body}</p:sp>",
                geometry(*geom),
                line(*stroke)
            )
        }
        Kind::Arrow(stroke) => format!(
            "<p:cxnSp><p:nvCxnSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr><p:spPr>{frame}<a:prstGeom prst=\"line\"><a:avLst/></a:prstGeom><a:ln w=\"{}\">{}<a:tailEnd type=\"triangle\" w=\"med\" len=\"med\"/></a:ln>{effect}</p:spPr></p:cxnSp>",
            emu(stroke.width),
            solid(stroke.color, OPAQUE)
        ),
        Kind::Picture {
            image,
            crop,
            border,
            round,
            alpha,
        } => {
            let embed = slide.rel("image", &format!("../media/{}", image.file()));
            let alpha = alpha.map_or_else(String::new, |a| format!("<a:alphaModFix amt=\"{a}\"/>"));
            let crop = crop.map_or_else(String::new, |[l, t, r, b]| {
                format!("<a:srcRect l=\"{l}\" t=\"{t}\" r=\"{r}\" b=\"{b}\"/>")
            });
            let preset = if *round { "roundRect" } else { "rect" };
            let border = border.map_or_else(String::new, |s| line(Some(s)));
            format!(
                "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed=\"{embed}\">{alpha}</a:blip>{crop}<a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{frame}<a:prstGeom prst=\"{preset}\"><a:avLst/></a:prstGeom>{border}{effect}</p:spPr></p:pic>"
            )
        }
        Kind::Video => {
            let media = slide.rel("media", "../media/clip.mp4");
            let video = slide.rel("video", "../media/clip.mp4");
            let poster = slide.rel("image", "../media/poster.png");
            format!(
                "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr><a:videoFile r:link=\"{video}\"/><p:extLst><p:ext uri=\"{{DAA4B4D4-6D71-4841-9C94-3DE7FCFB9230}}\"><p14:media xmlns:p14=\"{NS_P14}\" r:embed=\"{media}\"/></p:ext></p:extLst></p:nvPr></p:nvPicPr><p:blipFill><a:blip r:embed=\"{poster}\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{frame}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>{effect}</p:spPr></p:pic>"
            )
        }
        Kind::Group(children) => {
            let (x, y, w, h) = item.frame;
            let rot = if item.rot == 0 {
                String::new()
            } else {
                format!(" rot=\"{}\"", item.rot * 60_000)
            };
            let children: String = children.iter().map(|c| pptx_item(slide, c)).collect();
            format!(
                "<p:grpSp><p:nvGrpSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm{rot}><a:off x=\"{0}\" y=\"{1}\"/><a:ext cx=\"{2}\" cy=\"{3}\"/><a:chOff x=\"{0}\" y=\"{1}\"/><a:chExt cx=\"{2}\" cy=\"{3}\"/></a:xfrm>{effect}</p:grpSpPr>{children}</p:grpSp>",
                emu(x),
                emu(y),
                emu(w),
                emu(h)
            )
        }
    }
}

fn twin_pptx(slides: &[TwinSlide]) -> Vec<Entry> {
    let mut written = Vec::new();
    for slide in slides {
        let mut out = PptxSlide::new(1, (slide.background != SLIDE).then_some(slide.background));
        out.label(&slide.label, label_color(slide.background));
        for item in &slide.items {
            let xml = pptx_item(&mut out, item);
            out.tree.push_str(&xml);
        }
        written.push(out);
    }
    pptx_package(&PptxDeck {
        theme_effects: &"<a:effectStyle><a:effectLst/></a:effectStyle>".repeat(3),
        master_tree: "",
        layouts: vec![String::new()],
        slides: written,
        media: vec![
            ("photo.jpg", PHOTO.to_vec()),
            ("disc.png", disc_png()),
            ("clip.mp4", CLIP.to_vec()),
            ("poster.png", POSTER.to_vec()),
        ],
    })
}

fn label_color(background: [u8; 3]) -> [u8; 3] {
    if background == DARK { WHITE } else { LABEL }
}

pub(crate) struct PptxDeck<'a> {
    /// The theme's three `a:effectStyle` elements.
    pub(crate) theme_effects: &'a str,
    /// The master's shapes after its group properties.
    pub(crate) master_tree: &'a str,
    /// Each layout's shapes (layout 1 first); layout 1 is "Blank", the others "Title Only".
    pub(crate) layouts: Vec<String>,
    pub(crate) slides: Vec<PptxSlide>,
    /// The media parts (`ppt/media/<name>`) the slides may name.
    pub(crate) media: Vec<(&'a str, Vec<u8>)>,
}

pub(crate) fn pptx_package(deck: &PptxDeck) -> Vec<Entry> {
    let ct = "application/vnd.openxmlformats-officedocument.presentationml";
    let mut types = format!(
        "{PPTX_DECL}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"jpg\" ContentType=\"image/jpeg\"/><Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"mp4\" ContentType=\"video/mp4\"/><Override PartName=\"/ppt/presentation.xml\" ContentType=\"{ct}.presentation.main+xml\"/><Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"{ct}.slideMaster+xml\"/>"
    );
    for n in 1..=deck.layouts.len() {
        let _ = write!(
            types,
            "<Override PartName=\"/ppt/slideLayouts/slideLayout{n}.xml\" ContentType=\"{ct}.slideLayout+xml\"/>"
        );
    }
    types.push_str("<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>");
    for n in 1..=deck.slides.len() {
        let _ = write!(
            types,
            "<Override PartName=\"/ppt/slides/slide{n}.xml\" ContentType=\"{ct}.slide+xml\"/>"
        );
    }
    types.push_str("</Types>");
    let mut slide_ids = String::new();
    let mut pres_rels = vec![
        (
            "rId1".to_owned(),
            "slideMaster",
            "slideMasters/slideMaster1.xml".to_owned(),
        ),
        ("rId2".to_owned(), "theme", "theme/theme1.xml".to_owned()),
    ];
    for n in 0..deck.slides.len() {
        let _ = write!(
            slide_ids,
            "<p:sldId id=\"{}\" r:id=\"rId{}\"/>",
            256 + n,
            3 + n
        );
        pres_rels.push((
            format!("rId{}", 3 + n),
            "slide",
            format!("slides/slide{}.xml", n + 1),
        ));
    }
    let mut layout_ids = String::new();
    let mut master_rels = Vec::new();
    for n in 1..=deck.layouts.len() {
        let _ = write!(
            layout_ids,
            "<p:sldLayoutId id=\"{}\" r:id=\"rId{n}\"/>",
            2_147_483_648u64 + n as u64
        );
        master_rels.push((
            format!("rId{n}"),
            "slideLayout",
            format!("../slideLayouts/slideLayout{n}.xml"),
        ));
    }
    master_rels.push((
        format!("rId{}", deck.layouts.len() + 1),
        "theme",
        "../theme/theme1.xml".to_owned(),
    ));
    let mut entries = vec![
        xml_entry("[Content_Types].xml", types),
        xml_entry(
            "_rels/.rels",
            rels(&[(
                "rId1".to_owned(),
                "officeDocument",
                "ppt/presentation.xml".to_owned(),
            )]),
        ),
        xml_entry(
            "ppt/presentation.xml",
            format!(
                "{PPTX_DECL}<p:presentation xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\"><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst><p:sldIdLst>{slide_ids}</p:sldIdLst><p:sldSz cx=\"12192000\" cy=\"6858000\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"
            ),
        ),
        xml_entry("ppt/_rels/presentation.xml.rels", rels(&pres_rels)),
        xml_entry(
            "ppt/slideMasters/slideMaster1.xml",
            format!(
                "{PPTX_DECL}<p:sldMaster xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\"><p:cSld><p:bg><p:bgPr>{}<a:effectLst/></p:bgPr></p:bg><p:spTree>{GROUP}{}</p:spTree></p:cSld><p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/><p:sldLayoutIdLst>{layout_ids}</p:sldLayoutIdLst></p:sldMaster>",
                solid(SLIDE, OPAQUE),
                deck.master_tree
            ),
        ),
        xml_entry(
            "ppt/slideMasters/_rels/slideMaster1.xml.rels",
            rels(&master_rels),
        ),
    ];
    for (n, tree) in deck.layouts.iter().enumerate() {
        let (kind, name) = if n == 0 {
            ("blank", "Blank")
        } else {
            ("titleOnly", "Title Only")
        };
        entries.push(xml_entry(
            &format!("ppt/slideLayouts/slideLayout{}.xml", n + 1),
            format!(
                "{PPTX_DECL}<p:sldLayout xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\" type=\"{kind}\" preserve=\"1\"><p:cSld name=\"{name}\"><p:spTree>{GROUP}{tree}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
            ),
        ));
        entries.push(xml_entry(
            &format!("ppt/slideLayouts/_rels/slideLayout{}.xml.rels", n + 1),
            rels(&[(
                "rId1".to_owned(),
                "slideMaster",
                "../slideMasters/slideMaster1.xml".to_owned(),
            )]),
        ));
    }
    entries.push(xml_entry("ppt/theme/theme1.xml", theme(deck.theme_effects)));
    for (n, slide) in deck.slides.iter().enumerate() {
        debug_assert!(slide.layout <= deck.layouts.len());
        let background = slide.background.map_or_else(String::new, |c| {
            format!(
                "<p:bg><p:bgPr>{}<a:effectLst/></p:bgPr></p:bg>",
                solid(c, OPAQUE)
            )
        });
        entries.push(xml_entry(
            &format!("ppt/slides/slide{}.xml", n + 1),
            format!(
                "{PPTX_DECL}<p:sld xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\"><p:cSld>{background}<p:spTree>{GROUP}{}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>",
                slide.tree
            ),
        ));
        entries.push(xml_entry(
            &format!("ppt/slides/_rels/slide{}.xml.rels", n + 1),
            rels(&slide.rels),
        ));
    }
    for (name, data) in &deck.media {
        entries.push(stored(&format!("ppt/media/{name}"), data.clone()));
    }
    entries
}

fn theme(effects: &str) -> String {
    let mut colors = String::new();
    for (name, value) in [
        ("dk1", "000000"),
        ("lt1", "FFFFFF"),
        ("dk2", "1F2937"),
        ("lt2", "F2F2F2"),
        ("accent1", "2F6FDF"),
        ("accent2", "F9A825"),
        ("accent3", "2E7D32"),
        ("accent4", "6A1B9A"),
        ("accent5", "C2410C"),
        ("accent6", "0E7490"),
        ("hlink", "1565C0"),
        ("folHlink", "6A1B9A"),
    ] {
        let _ = write!(colors, "<a:{name}><a:srgbClr val=\"{value}\"/></a:{name}>");
    }
    let font = "<a:latin typeface=\"Liberation Sans\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/>";
    let fill = "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>";
    let line = "<a:ln w=\"9525\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln>";
    format!(
        "{PPTX_DECL}<a:theme xmlns:a=\"{NS_A}\" name=\"Shadow probe\"><a:themeElements><a:clrScheme name=\"Shadow probe\">{colors}</a:clrScheme><a:fontScheme name=\"Shadow probe\"><a:majorFont>{font}</a:majorFont><a:minorFont>{font}</a:minorFont></a:fontScheme><a:fmtScheme name=\"Shadow probe\"><a:fillStyleLst>{fill}{fill}{fill}</a:fillStyleLst><a:lnStyleLst>{line}{line}{line}</a:lnStyleLst><a:effectStyleLst>{effects}</a:effectStyleLst><a:bgFillStyleLst>{fill}{fill}{fill}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"
    )
}

// ---------------------------------------------------------------------------------------------------------------------
// ODP

const ODF_DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";
const ODF_NAMESPACES: &str = concat!(
    " xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"",
    " xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\"",
    " xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"",
    " xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\"",
    " xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\"",
    " xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\"",
    " xmlns:xlink=\"http://www.w3.org/1999/xlink\"",
    " xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\"",
    " xmlns:presentation=\"urn:oasis:names:tc:opendocument:xmlns:presentation:1.0\"",
    " xmlns:loext=\"urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0\""
);

/// The automatic styles of a content part, named in the order they are asked for.
#[derive(Default)]
pub(crate) struct OdpStyles {
    pub(crate) xml: String,
    pub(crate) count: usize,
}

impl OdpStyles {
    /// A graphic style of these properties (and, when not empty, text properties), unparented unless `parent`.
    pub(crate) fn graphic(&mut self, parent: Option<&str>, props: &str, text: &str) -> String {
        self.count += 1;
        let name = format!("gr{}", self.count);
        let parent =
            parent.map_or_else(String::new, |p| format!(" style:parent-style-name=\"{p}\""));
        let text = if text.is_empty() {
            String::new()
        } else {
            format!("<style:text-properties {text}/>")
        };
        let _ = write!(
            self.xml,
            "<style:style style:name=\"{name}\" style:family=\"graphic\"{parent}><style:graphic-properties {props}/>{text}</style:style>"
        );
        name
    }

    /// A text style of these properties.
    pub(crate) fn text(&mut self, props: &str) -> String {
        self.count += 1;
        let name = format!("T{}", self.count);
        let _ = write!(
            self.xml,
            "<style:style style:name=\"{name}\" style:family=\"text\"><style:text-properties {props}/></style:style>"
        );
        name
    }

    pub(crate) fn raw(&mut self, xml: &str) {
        self.xml.push_str(xml);
    }
}

pub(crate) fn odf_shadow(shadow: Option<Shadow>) -> String {
    shadow.map_or_else(
        || "draw:shadow=\"hidden\"".to_owned(),
        |s| {
            let (dx, dy) = s.offset();
            format!(
                "draw:shadow=\"visible\" draw:shadow-offset-x=\"{}pt\" draw:shadow-offset-y=\"{}pt\" draw:shadow-color=\"{}\" draw:shadow-opacity=\"{}\" loext:shadow-blur=\"{}pt\"",
                num(dx),
                num(dy),
                odf_color(s.color),
                percent(s.alpha),
                num(s.blur)
            )
        },
    )
}

fn odf_fill(fill: Option<([u8; 3], u32)>) -> String {
    match fill {
        None => "draw:fill=\"none\"".to_owned(),
        Some((color, alpha)) if alpha >= OPAQUE => {
            format!(
                "draw:fill=\"solid\" draw:fill-color=\"{}\"",
                odf_color(color)
            )
        }
        Some((color, alpha)) => format!(
            "draw:fill=\"solid\" draw:fill-color=\"{}\" draw:opacity=\"{}\"",
            odf_color(color),
            percent(alpha)
        ),
    }
}

fn odf_stroke(stroke: Option<Stroke>) -> String {
    stroke.map_or_else(
        || "draw:stroke=\"none\"".to_owned(),
        |s| {
            format!(
                "draw:stroke=\"solid\" svg:stroke-width=\"{}pt\" svg:stroke-color=\"{}\"",
                num(s.width),
                odf_color(s.color)
            )
        },
    )
}

/// The placement of a box turned `rot` degrees clockwise about its centre: `svg:x`/`svg:y`, or LibreOffice's
/// `draw:transform` (turn the box about its origin, then move that corner).
pub(crate) fn placement((x, y, w, h): (f64, f64, f64, f64), rot: i32) -> String {
    let size = format!("svg:width=\"{}pt\" svg:height=\"{}pt\"", num(w), num(h));
    if rot == 0 {
        return format!("svg:x=\"{}pt\" svg:y=\"{}pt\" {size}", num(x), num(y));
    }
    // ODF's rotate(a) is anticlockwise on the page: x' = cos·x + sin·y, y' = −sin·x + cos·y for a = −rot.
    let (cos, sin) = cos_sin(rot);
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let turned_centre = (cos * w / 2.0 - sin * h / 2.0, sin * w / 2.0 + cos * h / 2.0);
    let (tx, ty) = (cx - turned_centre.0, cy - turned_centre.1);
    let radians = -f64::from(rot) * std::f64::consts::PI / 180.0;
    format!(
        "{size} draw:transform=\"rotate ({}) translate ({}pt {}pt)\"",
        format_radians(radians),
        num(tx),
        num(ty)
    )
}

fn format_radians(value: f64) -> String {
    let text = format!("{value:.12}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.to_owned()
}

/// A box turned `rot` degrees clockwise about `pivot`: where its own centre goes.
fn turn_about(
    (x, y, w, h): (f64, f64, f64, f64),
    rot: i32,
    pivot: (f64, f64),
) -> (f64, f64, f64, f64) {
    let (cos, sin) = cos_sin(rot);
    let (dx, dy) = (x + w / 2.0 - pivot.0, y + h / 2.0 - pivot.1);
    let (cx, cy) = (pivot.0 + cos * dx - sin * dy, pivot.1 + sin * dx + cos * dy);
    (cx - w / 2.0, cy - h / 2.0, w, h)
}

fn odf_text(styles: &mut OdpStyles, text: &Text) -> String {
    text_lines(styles, text.lines, text.size, text.color)
}

/// The text area of a shape that holds text: PowerPoint's default insets, anchored at the top, never grown or wrapped.
pub(crate) const TEXT_AREA: &str = "draw:textarea-vertical-align=\"top\" draw:textarea-horizontal-align=\"left\" draw:auto-grow-height=\"false\" draw:auto-grow-width=\"false\" fo:wrap-option=\"no-wrap\" fo:padding-left=\"7.2pt\" fo:padding-right=\"7.2pt\" fo:padding-top=\"3.6pt\" fo:padding-bottom=\"3.6pt\"";

fn custom_geometry(geom: Geom, flip_h: bool, flip_v: bool) -> String {
    let mut mirror = String::new();
    if flip_h {
        mirror.push_str(" draw:mirror-horizontal=\"true\"");
    }
    if flip_v {
        mirror.push_str(" draw:mirror-vertical=\"true\"");
    }
    match geom {
        Geom::Rect => format!(
            "<draw:enhanced-geometry svg:viewBox=\"0 0 21600 21600\" draw:type=\"rectangle\" draw:enhanced-path=\"M 0 0 L 21600 0 21600 21600 0 21600 0 0 Z N\"{mirror}/>"
        ),
        Geom::Ellipse => format!(
            "<draw:enhanced-geometry svg:viewBox=\"0 0 21600 21600\" draw:type=\"ellipse\" draw:enhanced-path=\"U 10800 10800 10800 10800 0 360 Z N\"{mirror}/>"
        ),
        Geom::RightTriangle => format!(
            "<draw:enhanced-geometry svg:viewBox=\"0 0 21600 21600\" draw:type=\"right-triangle\" draw:enhanced-path=\"M 0 0 L 21600 21600 0 21600 0 0 Z N\"{mirror}/>"
        ),
        Geom::Star => unreachable!("the star is written as a draw:polygon"),
    }
}

pub(crate) fn odf_label(styles: &mut OdpStyles, text: &str, color: [u8; 3]) -> String {
    let style = styles.graphic(
        None,
        "draw:fill=\"none\" draw:stroke=\"none\" draw:shadow=\"hidden\" draw:textarea-vertical-align=\"top\" draw:textarea-horizontal-align=\"left\" draw:auto-grow-height=\"false\" draw:auto-grow-width=\"false\" fo:wrap-option=\"no-wrap\" fo:padding-left=\"0pt\" fo:padding-right=\"0pt\" fo:padding-top=\"0pt\" fo:padding-bottom=\"0pt\"",
        "",
    );
    let lines = [text];
    let paragraphs = text_lines(styles, &lines, 12, color);
    format!(
        "<draw:custom-shape draw:style-name=\"{style}\" draw:text-style-name=\"Pfi\" draw:name=\"Label\" {}>{paragraphs}{}</draw:custom-shape>",
        placement((24.0, 12.0, 720.0, 20.0), 0),
        custom_geometry(Geom::Rect, false, false)
    )
}

pub(crate) fn text_lines(
    styles: &mut OdpStyles,
    lines: &[&str],
    size: u32,
    color: [u8; 3],
) -> String {
    let span = styles.text(&format!(
        "fo:font-family=\"'Liberation Sans'\" style:font-family-generic=\"swiss\" fo:font-size=\"{size}pt\" fo:color=\"{}\" style:letter-kerning=\"true\"",
        odf_color(color)
    ));
    lines
        .iter()
        .map(|line| format!("<text:p text:style-name=\"Pfi\"><text:span text:style-name=\"{span}\">{}</text:span></text:p>", escape(line)))
        .collect()
}

fn odp_item(styles: &mut OdpStyles, item: &Item) -> String {
    let name = escape(&item.name);
    let shadow = odf_shadow(item.shadow);
    let at = placement(item.frame, item.rot);
    match &item.kind {
        Kind::Shape {
            geom,
            fill,
            stroke,
            text,
        } => {
            let mut props = format!("{} {} {shadow}", odf_fill(*fill), odf_stroke(*stroke));
            if text.is_some() {
                let _ = write!(props, " {TEXT_AREA}");
            }
            let style = styles.graphic(None, &props, "");
            let paragraphs = text
                .as_ref()
                .map_or_else(String::new, |t| odf_text(styles, t));
            if *geom == Geom::Rect && text.is_none() && !item.flip_h && !item.flip_v {
                return format!(
                    "<draw:rect draw:style-name=\"{style}\" draw:name=\"{name}\" {at}/>"
                );
            }
            if *geom == Geom::Ellipse && text.is_none() {
                return format!(
                    "<draw:ellipse draw:style-name=\"{style}\" draw:name=\"{name}\" {at}/>"
                );
            }
            if *geom == Geom::Star && text.is_none() {
                let points: Vec<String> = STAR.iter().map(|(x, y)| format!("{x},{y}")).collect();
                return format!(
                    "<draw:polygon draw:style-name=\"{style}\" draw:name=\"{name}\" {at} svg:viewBox=\"0 0 20000 20000\" draw:points=\"{}\"/>",
                    points.join(" ")
                );
            }
            let text_style = if paragraphs.is_empty() {
                ""
            } else {
                " draw:text-style-name=\"Pfi\""
            };
            format!(
                "<draw:custom-shape draw:style-name=\"{style}\"{text_style} draw:name=\"{name}\" {at}>{paragraphs}{}</draw:custom-shape>",
                custom_geometry(*geom, item.flip_h, item.flip_v)
            )
        }
        Kind::Arrow(stroke) => {
            let style = styles.graphic(
                None,
                &format!(
                    "draw:fill=\"none\" {} draw:marker-end=\"msArrowEnd_20_5\" draw:marker-end-width=\"{}pt\" {shadow}",
                    odf_stroke(Some(*stroke)),
                    num(3.0 * stroke.width)
                ),
                "",
            );
            let (x, y, w, h) = item.frame;
            format!(
                "<draw:line draw:style-name=\"{style}\" draw:name=\"{name}\" svg:x1=\"{}pt\" svg:y1=\"{}pt\" svg:x2=\"{}pt\" svg:y2=\"{}pt\"/>",
                num(x),
                num(y),
                num(x + w),
                num(y + h)
            )
        }
        Kind::Picture {
            image,
            crop,
            border,
            round,
            alpha,
        } => {
            let mut props = format!("{} {shadow}", odf_stroke(*border));
            if let Some([l, t, r, b]) = crop {
                let (w, h) = image.size();
                let part = |share: &u32, of: f64| num(of * f64::from(*share) / 100_000.0);
                let _ = write!(
                    props,
                    " fo:clip=\"rect({}pt, {}pt, {}pt, {}pt)\"",
                    part(t, h),
                    part(r, w),
                    part(b, h),
                    part(l, w)
                );
            }
            if let Some(alpha) = alpha {
                let _ = write!(props, " draw:image-opacity=\"{}\"", percent(*alpha));
            }
            if *round {
                let style = styles.graphic(
                    None,
                    &format!(
                        "draw:fill=\"bitmap\" draw:fill-image-name=\"Photo\" style:repeat=\"stretch\" {props}"
                    ),
                    "",
                );
                return format!(
                    "<draw:custom-shape draw:style-name=\"{style}\" draw:name=\"{name}\" {at}><draw:enhanced-geometry svg:viewBox=\"0 0 21600 21600\" draw:type=\"round-rectangle\" draw:modifiers=\"3600\" draw:enhanced-path=\"M ?f7 0 X 0 ?f8 L 0 ?f9 Y ?f7 21600 L ?f10 21600 X 21600 ?f9 L 21600 ?f8 Y ?f10 0 Z N\"><draw:equation draw:name=\"f0\" draw:formula=\"45\"/><draw:equation draw:name=\"f1\" draw:formula=\"$0 *sin(?f0 *(pi/180))\"/><draw:equation draw:name=\"f2\" draw:formula=\"?f1 *3163/7636\"/><draw:equation draw:name=\"f3\" draw:formula=\"left+?f2 \"/><draw:equation draw:name=\"f4\" draw:formula=\"top+?f2 \"/><draw:equation draw:name=\"f5\" draw:formula=\"right-?f2 \"/><draw:equation draw:name=\"f6\" draw:formula=\"bottom-?f2 \"/><draw:equation draw:name=\"f7\" draw:formula=\"left+$0 \"/><draw:equation draw:name=\"f8\" draw:formula=\"top+$0 \"/><draw:equation draw:name=\"f9\" draw:formula=\"bottom-$0 \"/><draw:equation draw:name=\"f10\" draw:formula=\"right-$0 \"/><draw:handle draw:handle-position=\"$0 top\" draw:handle-switched=\"true\" draw:handle-range-x-minimum=\"0\" draw:handle-range-x-maximum=\"10800\"/></draw:enhanced-geometry></draw:custom-shape>"
                );
            }
            let style = styles.graphic(None, &format!("draw:fill=\"none\" {props}"), "");
            format!(
                "<draw:frame draw:style-name=\"{style}\" draw:name=\"{name}\" {at}><draw:image xlink:href=\"Pictures/{}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\" draw:mime-type=\"{}\"/></draw:frame>",
                image.file(),
                image.mime()
            )
        }
        Kind::Video => {
            let style = styles.graphic(
                None,
                &format!("draw:fill=\"none\" draw:stroke=\"none\" {shadow}"),
                "",
            );
            format!(
                "<draw:frame draw:style-name=\"{style}\" draw:name=\"{name}\" {at}><draw:plugin xlink:href=\"Media/clip.mp4\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\" draw:mime-type=\"video/mp4\"><draw:param draw:name=\"Loop\" draw:value=\"false\"/><draw:param draw:name=\"Mute\" draw:value=\"false\"/></draw:plugin><draw:image xlink:href=\"Pictures/poster.png\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\" draw:mime-type=\"image/png\"/></draw:frame>"
            )
        }
        Kind::Group(children) => {
            // ODF groups have no transform: each child is turned about the group's centre.
            let (x, y, w, h) = item.frame;
            let pivot = (x + w / 2.0, y + h / 2.0);
            let mut out = format!("<draw:g draw:name=\"{name}\">");
            for child in children {
                let turned = Item {
                    name: child.name.clone(),
                    frame: turn_about(child.frame, item.rot, pivot),
                    rot: child.rot + item.rot,
                    flip_h: child.flip_h,
                    flip_v: child.flip_v,
                    kind: match &child.kind {
                        Kind::Shape {
                            geom, fill, stroke, ..
                        } => Kind::Shape {
                            geom: *geom,
                            fill: *fill,
                            stroke: *stroke,
                            text: None,
                        },
                        _ => panic!("group children are plain shapes"),
                    },
                    shadow: child.shadow,
                };
                out.push_str(&odp_item(styles, &turned));
            }
            out.push_str("</draw:g>");
            out
        }
    }
}

/// The paragraphs of the ODP's labels and text boxes: font-independent line spacing, as LibreOffice reads every PPTX
/// paragraph, so the twins' text lines up.
pub(crate) const PARAGRAPH: &str = "<style:style style:name=\"Pfi\" style:family=\"paragraph\"><style:paragraph-properties fo:margin-top=\"0pt\" fo:margin-bottom=\"0pt\" fo:line-height=\"100%\" style:font-independent-line-spacing=\"true\"/></style:style>";

/// LibreOffice's triangle line end (only its name and outline are read).
const ARROW_MARKER: &str = "<draw:marker draw:name=\"msArrowEnd_20_5\" draw:display-name=\"msArrowEnd 5\" svg:viewBox=\"0 0 210 210\" svg:d=\"M105 0L210 210H0Z\"/>";

const FILL_IMAGE: &str = "<draw:fill-image draw:name=\"Photo\" xlink:href=\"Pictures/photo.jpg\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/>";

fn twin_odp(slides: &[TwinSlide]) -> Vec<Entry> {
    let mut styles = OdpStyles::default();
    styles.raw(PARAGRAPH);
    styles.raw(&page_style("dpSlide", SLIDE));
    styles.raw(&page_style("dpDark", DARK));
    let mut pages = String::new();
    for (k, slide) in slides.iter().enumerate() {
        let page_style = if slide.background == DARK {
            "dpDark"
        } else {
            "dpSlide"
        };
        let mut body = odf_label(&mut styles, &slide.label, label_color(slide.background));
        for item in &slide.items {
            body.push_str(&odp_item(&mut styles, item));
        }
        let _ = write!(
            pages,
            "<draw:page draw:name=\"A{:02}\" draw:style-name=\"{page_style}\" draw:master-page-name=\"Probe\">{body}</draw:page>",
            k + 1
        );
    }
    odp_package(&OdpDeck {
        automatic: &styles.xml,
        common: &format!("{}{ARROW_MARKER}{FILL_IMAGE}", standard_styles("")),
        master_automatic: "",
        masters: "",
        pages: &pages,
        media: vec![
            ("Pictures/photo.jpg", "image/jpeg", PHOTO.to_vec()),
            ("Pictures/disc.png", "image/png", disc_png()),
            ("Media/clip.mp4", "video/mp4", CLIP.to_vec()),
            ("Pictures/poster.png", "image/png", POSTER.to_vec()),
        ],
    })
}

pub(crate) fn page_style(name: &str, color: [u8; 3]) -> String {
    format!(
        "<style:style style:name=\"{name}\" style:family=\"drawing-page\"><style:drawing-page-properties draw:background-size=\"full\" draw:fill=\"solid\" draw:fill-color=\"{}\" presentation:background-visible=\"true\" presentation:background-objects-visible=\"true\"/></style:style>",
        odf_color(color)
    )
}

/// The file's "standard" graphic style (no fill, no line, no shadow unless `shadow` says otherwise) and
/// "objectwithshadow" as LibreOffice writes it.
pub(crate) fn standard_styles(shadow: &str) -> String {
    format!(
        "<style:default-style style:family=\"graphic\"><style:graphic-properties svg:stroke-color=\"#1f2937\" draw:fill-color=\"#ffffff\"/><style:text-properties fo:font-family=\"'Liberation Sans'\" style:font-family-generic=\"swiss\" fo:font-size=\"18pt\"/></style:default-style><style:style style:name=\"standard\" style:family=\"graphic\"><style:graphic-properties draw:stroke=\"none\" draw:fill=\"none\" draw:shadow=\"hidden\"{shadow}/><style:text-properties fo:font-family=\"'Liberation Sans'\" style:font-family-generic=\"swiss\" fo:font-size=\"18pt\" fo:text-shadow=\"none\"/></style:style><style:style style:name=\"objectwithshadow\" style:family=\"graphic\" style:parent-style-name=\"standard\"><style:graphic-properties draw:shadow=\"visible\" draw:shadow-offset-x=\"0.2cm\" draw:shadow-offset-y=\"0.2cm\" draw:shadow-color=\"#808080\"/></style:style>"
    )
}

pub(crate) struct OdpDeck<'a> {
    /// content.xml's automatic styles.
    pub(crate) automatic: &'a str,
    /// styles.xml's common styles.
    pub(crate) common: &'a str,
    /// styles.xml's automatic styles beside the page layout and the masters' page styles.
    pub(crate) master_automatic: &'a str,
    /// Master pages beside "Probe".
    pub(crate) masters: &'a str,
    pub(crate) pages: &'a str,
    /// (path in the package, media type, bytes).
    pub(crate) media: Vec<(&'a str, &'a str, Vec<u8>)>,
}

pub(crate) fn odp_package(deck: &OdpDeck) -> Vec<Entry> {
    let mut files = String::new();
    for (path, mime, _) in &deck.media {
        let _ = write!(
            files,
            "<manifest:file-entry manifest:full-path=\"{path}\" manifest:media-type=\"{mime}\"/>"
        );
    }
    let manifest = format!(
        "{ODF_DECL}<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\"><manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.3\" manifest:media-type=\"application/vnd.oasis.opendocument.presentation\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/><manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/>{files}</manifest:manifest>"
    );
    let content = format!(
        "{ODF_DECL}<office:document-content{ODF_NAMESPACES} office:version=\"1.3\"><office:automatic-styles>{}</office:automatic-styles><office:body><office:presentation>{}</office:presentation></office:body></office:document-content>",
        deck.automatic, deck.pages
    );
    let styles = format!(
        "{ODF_DECL}<office:document-styles{ODF_NAMESPACES} office:version=\"1.3\"><office:styles>{}</office:styles><office:automatic-styles><style:page-layout style:name=\"PM1\"><style:page-layout-properties fo:margin-top=\"0pt\" fo:margin-bottom=\"0pt\" fo:margin-left=\"0pt\" fo:margin-right=\"0pt\" fo:page-width=\"960pt\" fo:page-height=\"540pt\" style:print-orientation=\"landscape\"/></style:page-layout>{}{}</office:automatic-styles><office:master-styles><style:master-page style:name=\"Probe\" style:page-layout-name=\"PM1\" draw:style-name=\"Mdp1\"/>{}</office:master-styles></office:document-styles>",
        deck.common,
        page_style("Mdp1", SLIDE),
        deck.master_automatic,
        deck.masters
    );
    let mut entries = vec![
        stored(
            "mimetype",
            b"application/vnd.oasis.opendocument.presentation".to_vec(),
        ),
        xml_entry("META-INF/manifest.xml", manifest),
        xml_entry("content.xml", content),
        xml_entry("styles.xml", styles),
    ];
    for (path, _, data) in &deck.media {
        entries.push(stored(path, data.clone()));
    }
    entries
}

// ---------------------------------------------------------------------------------------------------------------------
// shadow-probe-odp.odp

fn odp_only() -> Vec<Entry> {
    let mut styles = OdpStyles::default();
    styles.raw(PARAGRAPH);
    styles.raw(&page_style("dpSlide", SLIDE));
    let mut pages = String::new();
    let mut page = |styles: &mut OdpStyles, label: &str, body: String, master: &str| {
        let number = pages.matches("<draw:page ").count() + 1;
        let label = odf_label(styles, label, LABEL);
        let _ = write!(
            pages,
            "<draw:page draw:name=\"B{number:02}\" draw:style-name=\"dpSlide\" draw:master-page-name=\"{master}\">{label}{body}</draw:page>"
        );
    };
    let base = odf_shadow(Some(BASE));
    let white = odf_fill(Some((WHITE, OPAQUE)));
    let at = placement(CENTRE, 0);
    let rect_of = |style: &str| {
        format!("<draw:rect draw:style-name=\"{style}\" draw:name=\"Rectangle\" {at}/>")
    };

    let s = styles.graphic(None, &format!("{white} draw:shadow=\"visible\""), "");
    page(
        &mut styles,
        "B01 unparented style, only draw:shadow visible",
        rect_of(&s),
        "Probe",
    );
    let s = styles.graphic(
        Some("standard"),
        &format!("{white} draw:shadow=\"visible\""),
        "",
    );
    page(
        &mut styles,
        "B02 the same, parented to standard",
        rect_of(&s),
        "Probe",
    );
    let s = styles.graphic(
        Some("objectwithshadow"),
        &format!("{white} draw:shadow-color=\"#0000ff\""),
        "",
    );
    page(
        &mut styles,
        "B03 parent objectwithshadow, colour blue",
        rect_of(&s),
        "Probe",
    );

    styles.raw("<style:style style:name=\"pr1\" style:family=\"presentation\" style:parent-style-name=\"Probe-title\"><style:graphic-properties draw:fill=\"solid\" draw:fill-color=\"#ffffff\" draw:shadow=\"visible\" loext:shadow-blur=\"0.5cm\"/></style:style>");
    page(
        &mut styles,
        "B04 title placeholder, only visible and blur 0.5 cm",
        format!(
            "<draw:frame presentation:style-name=\"pr1\" draw:layer=\"layout\" presentation:class=\"title\" draw:name=\"Title\" {}><draw:text-box><text:p>Title halo</text:p></draw:text-box></draw:frame>",
            placement((240.0, 190.0, 480.0, 160.0), 0)
        ),
        "Probe",
    );
    styles.raw("<style:style style:name=\"pr2\" style:family=\"presentation\" style:parent-style-name=\"Probe-subtitle\"><style:graphic-properties draw:fill=\"solid\" draw:fill-color=\"#ffffff\"/></style:style>");
    page(
        &mut styles,
        "B05 subtitle placeholder, master style blue 0.3 cm",
        format!(
            "<draw:frame presentation:style-name=\"pr2\" draw:layer=\"layout\" presentation:class=\"subtitle\" draw:name=\"Subtitle\" {}><draw:text-box><text:p>Subtitle</text:p></draw:text-box></draw:frame>",
            placement((240.0, 190.0, 480.0, 160.0), 0)
        ),
        "Probe",
    );
    for (label, shadow_alpha) in [
        ("B06 fill at 50 %, shadow at 50 %", 50_000),
        ("B07 fill at 50 %, shadow at 40 %", 40_000),
        ("B08 shadow at 0 %", 0),
    ] {
        let fill = if shadow_alpha == 0 {
            white.clone()
        } else {
            odf_fill(Some((BLUE, 50_000)))
        };
        let s = styles.graphic(
            None,
            &format!(
                "{fill} draw:stroke=\"none\" {}",
                odf_shadow(Some(Shadow {
                    alpha: shadow_alpha,
                    ..BASE
                }))
            ),
            "",
        );
        page(&mut styles, label, rect_of(&s), "Probe");
    }
    let plain = styles.graphic(
        None,
        &format!("draw:fill=\"none\" draw:stroke=\"none\" draw:shadow=\"hidden\" {TEXT_AREA}"),
        "",
    );
    for (label, color, value) in [
        (
            "B09 fo:text-shadow on black text at 12, 24, 48, 96 pt",
            "#000000",
            "1pt 1pt",
        ),
        (
            "B10 fo:text-shadow 5pt 5pt 3pt #00ff00 on red text",
            "#cc0000",
            "5pt 5pt 3pt #00ff00",
        ),
    ] {
        let mut body = String::new();
        let mut y = 50.0;
        for size in [12u32, 24, 48, 96] {
            let span = styles.text(&format!(
                "fo:font-family=\"'Liberation Sans'\" style:font-family-generic=\"swiss\" fo:font-size=\"{size}pt\" fo:color=\"{color}\" fo:text-shadow=\"{value}\""
            ));
            let height = f64::from(size) * 1.4 + 8.0;
            let _ = write!(
                body,
                "<draw:frame draw:style-name=\"{plain}\" draw:name=\"Text {size}\" {}><draw:text-box><text:p><text:span text:style-name=\"{span}\">Shadow {size}</text:span></text:p></draw:text-box></draw:frame>",
                placement((60.0, y, 840.0, height), 0)
            );
            y += height + 16.0;
        }
        page(&mut styles, label, body, "Probe");
    }
    let span = styles.text("fo:font-family=\"'Liberation Sans'\" style:font-family-generic=\"swiss\" fo:font-size=\"48pt\" fo:color=\"#000000\" fo:text-shadow=\"1pt 1pt\"");
    page(
        &mut styles,
        "B11 character shadow on text turned 30",
        format!(
            "<draw:frame draw:style-name=\"{plain}\" draw:name=\"Turned text\" {}><draw:text-box><text:p><text:span text:style-name=\"{span}\">Turned</text:span></text:p></draw:text-box></draw:frame>",
            placement((330.0, 220.0, 300.0, 100.0), 30)
        ),
        "Probe",
    );
    let table = styles.graphic(
        None,
        &format!("draw:fill=\"none\" draw:stroke=\"none\" {base}"),
        "",
    );
    styles.raw("<style:style style:name=\"ce1\" style:family=\"table-cell\"><style:graphic-properties draw:fill=\"solid\" draw:fill-color=\"#2f6fdf\"/></style:style><style:style style:name=\"ce2\" style:family=\"table-cell\"><style:graphic-properties draw:fill=\"none\"/></style:style><style:style style:name=\"co1\" style:family=\"table-column\"><style:table-column-properties style:column-width=\"120pt\"/></style:style><style:style style:name=\"ro1\" style:family=\"table-row\"><style:table-row-properties style:row-height=\"80pt\"/></style:style>");
    let cell = |style: &str| {
        format!("<table:table-cell table:style-name=\"{style}\"><text:p/></table:table-cell>")
    };
    page(
        &mut styles,
        "B12 2 x 2 table, diagonal fills, with a shadow",
        format!(
            "<draw:frame draw:style-name=\"{table}\" draw:name=\"Table\" {}><table:table><table:table-column table:style-name=\"co1\" table:number-columns-repeated=\"2\"/><table:table-row table:style-name=\"ro1\">{}{}</table:table-row><table:table-row table:style-name=\"ro1\">{}{}</table:table-row></table:table></draw:frame>",
            placement(CENTRE, 0),
            cell("ce1"),
            cell("ce2"),
            cell("ce2"),
            cell("ce1")
        ),
        "Probe",
    );
    let group = styles.graphic(
        None,
        &format!("draw:fill=\"none\" draw:stroke=\"none\" {base}"),
        "",
    );
    let inner = styles.graphic(
        None,
        &format!("{white} draw:stroke=\"none\" draw:shadow=\"hidden\""),
        "",
    );
    let inner_amber = styles.graphic(
        None,
        &format!(
            "{} draw:stroke=\"none\" draw:shadow=\"hidden\"",
            odf_fill(Some((AMBER, OPAQUE)))
        ),
        "",
    );
    page(
        &mut styles,
        "B13 draw:g with a shadow style",
        format!(
            "<draw:g draw:style-name=\"{group}\" draw:name=\"Group\"><draw:rect draw:style-name=\"{inner}\" draw:name=\"Left\" {}/><draw:rect draw:style-name=\"{inner_amber}\" draw:name=\"Right\" {}/></draw:g>",
            placement((300.0, 170.0, 160.0, 100.0), 0),
            placement((500.0, 270.0, 160.0, 100.0), 0)
        ),
        "Probe",
    );
    let connector = styles.graphic(
        None,
        &format!(
            "draw:fill=\"none\" {} draw:marker-end=\"msArrowEnd_20_5\" draw:marker-end-width=\"24pt\" {base}",
            odf_stroke(Some(Stroke {
                width: 8.0,
                color: INK
            }))
        ),
        "",
    );
    page(
        &mut styles,
        "B14 bent connector with an arrow",
        format!(
            "<draw:connector draw:style-name=\"{connector}\" draw:name=\"Connector\" draw:type=\"standard\" svg:x1=\"300pt\" svg:y1=\"190pt\" svg:x2=\"660pt\" svg:y2=\"350pt\" svg:viewBox=\"0 0 360 160\" svg:d=\"M300 190H480V350H660\"/>"
        ),
        "Probe",
    );
    let ramp = styles.graphic(
        None,
        &format!("draw:fill=\"none\" draw:stroke=\"none\" {base}"),
        "",
    );
    page(
        &mut styles,
        "B15 PNG with a horizontal alpha ramp",
        format!(
            "<draw:frame draw:style-name=\"{ramp}\" draw:name=\"Ramp\" {}><draw:image xlink:href=\"Pictures/ramp.png\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\" draw:mime-type=\"image/png\"/></draw:frame>",
            placement(CENTRE, 0)
        ),
        "Probe",
    );
    page(
        &mut styles,
        "B16 shadowed object on the master page",
        String::new(),
        "ProbeObject",
    );

    let common = format!(
        "{}{ARROW_MARKER}<style:style style:name=\"Probe-title\" style:family=\"presentation\"><style:graphic-properties draw:stroke=\"none\" draw:fill=\"none\" draw:textarea-vertical-align=\"middle\"/><style:paragraph-properties fo:text-align=\"center\"/><style:text-properties fo:font-family=\"'Liberation Sans'\" style:font-family-generic=\"swiss\" fo:font-size=\"44pt\" fo:color=\"#1f2937\"/></style:style><style:style style:name=\"Probe-subtitle\" style:family=\"presentation\"><style:graphic-properties draw:stroke=\"none\" draw:fill=\"none\" draw:shadow=\"visible\" draw:shadow-offset-x=\"0.3cm\" draw:shadow-offset-y=\"0.3cm\" draw:shadow-color=\"#0000ff\" draw:textarea-vertical-align=\"middle\"/><style:paragraph-properties fo:text-align=\"center\"/><style:text-properties fo:font-family=\"'Liberation Sans'\" style:font-family-generic=\"swiss\" fo:font-size=\"32pt\" fo:color=\"#1f2937\"/></style:style>",
        standard_styles(
            " draw:shadow-offset-x=\"0.5cm\" draw:shadow-offset-y=\"1cm\" draw:shadow-color=\"#ff0000\" draw:shadow-opacity=\"50%\""
        )
    );
    let master_automatic = format!(
        "{}<style:style style:name=\"Mgr1\" style:family=\"graphic\"><style:graphic-properties {white} draw:stroke=\"none\" {base}/></style:style>",
        page_style("Mdp2", SLIDE)
    );
    let masters = format!(
        "<style:master-page style:name=\"ProbeObject\" style:page-layout-name=\"PM1\" draw:style-name=\"Mdp2\"><draw:rect draw:style-name=\"Mgr1\" draw:name=\"Master rectangle\" {at}/></style:master-page>"
    );
    odp_package(&OdpDeck {
        automatic: &styles.xml,
        common: &common,
        master_automatic: &master_automatic,
        masters: &masters,
        pages: &pages,
        media: vec![("Pictures/ramp.png", "image/png", ramp_png())],
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// shadow-probe-ooxml.pptx

/// An `a:outerShdw` (or another shadow element) with these attributes around a colour.
fn shadow_element(element: &str, attrs: &str, color: &str) -> String {
    format!("<a:{element} {attrs}>{color}</a:{element}>")
}

fn ooxml_only() -> Vec<Entry> {
    let black = srgb(BLACK, 50_000);
    // The default shadow's polar part, at a blur.
    let polar = |blur: f64| {
        format!(
            "blurRad=\"{}\" dist=\"{}\" dir=\"2700000\"",
            emu(blur),
            emu(BASE.dist)
        )
    };
    let base_attrs = polar(0.0);
    let keynote = |extra: &str| {
        format!(
            "sx=\"100000\" sy=\"100000\" kx=\"0\" ky=\"0\" algn=\"b\" {extra}rotWithShape=\"0\" {base_attrs}"
        )
    };
    let outer_base = shadow_element("outerShdw", &keynote(""), &black);
    let effect_lst = |inner: &str| format!("<a:effectLst>{inner}</a:effectLst>");
    let theme_effects = format!(
        "<a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle>{}</a:effectStyle><a:effectStyle>{}</a:effectStyle>",
        effect_lst(&outer_base),
        effect_lst(&shadow_element(
            "outerShdw",
            &format!("{} rotWithShape=\"0\"", polar(10.0)),
            "<a:schemeClr val=\"phClr\"><a:alpha val=\"50000\"/></a:schemeClr>"
        ))
    );
    let mut slides: Vec<PptxSlide> = Vec::new();
    let mut add = |label: &str, layout: usize, body: &dyn Fn(&mut PptxSlide) -> String| {
        let mut slide = PptxSlide::new(layout, None);
        slide.label(label, LABEL);
        let xml = body(&mut slide);
        slide.tree.push_str(&xml);
        slides.push(slide);
    };
    let white = solid(WHITE, OPAQUE);
    let no_line = "<a:ln><a:noFill/></a:ln>";
    let rect_geom = geometry(Geom::Rect);
    let sp = |slide: &mut PptxSlide,
              name: &str,
              frame: String,
              geom: &str,
              fill: &str,
              effects: &str,
              style: &str| {
        let id = slide.id();
        format!(
            "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr>{frame}{geom}{fill}{no_line}{effects}</p:spPr>{style}</p:sp>"
        )
    };
    let centre = || xfrm(CENTRE, 0, false, false);
    let style_ref = |idx: u32, color: &str| {
        format!(
            "<p:style><a:lnRef idx=\"0\"><a:srgbClr val=\"000000\"/></a:lnRef><a:fillRef idx=\"0\"><a:srgbClr val=\"FFFFFF\"/></a:fillRef><a:effectRef idx=\"{idx}\">{color}</a:effectRef><a:fontRef idx=\"minor\"><a:schemeClr val=\"tx1\"/></a:fontRef></p:style>"
        )
    };
    let no_color = "<a:srgbClr val=\"000000\"/>";
    let blue_ref = "<a:srgbClr val=\"0000FF\"/>";
    let green = shadow_element("outerShdw", &keynote(""), &srgb([0, 0x99, 0], 50_000));

    add("C01 effectRef idx 2 (the theme's shadow)", 1, &|s| {
        sp(
            s,
            "Rectangle",
            centre(),
            &rect_geom,
            &white,
            "",
            &style_ref(2, no_color),
        )
    });
    add("C02 effectRef idx 2 with an empty own effectLst", 1, &|s| {
        sp(
            s,
            "Rectangle",
            centre(),
            &rect_geom,
            &white,
            "<a:effectLst/>",
            &style_ref(2, no_color),
        )
    });
    add("C03 effectRef idx 2 with an own green shadow", 1, &|s| {
        sp(
            s,
            "Rectangle",
            centre(),
            &rect_geom,
            &white,
            &effect_lst(&green),
            &style_ref(2, no_color),
        )
    });
    add(
        "C04 effectRef idx 3, phClr, blue reference colour",
        1,
        &|s| {
            sp(
                s,
                "Rectangle",
                centre(),
                &rect_geom,
                &white,
                "",
                &style_ref(3, blue_ref),
            )
        },
    );
    add(
        "C05 title placeholder inheriting its layout's shadow",
        2,
        &|s| {
            let id = s.id();
            format!(
                "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Title\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph type=\"title\"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang=\"en-US\" dirty=\"0\"/><a:t>Inherited shadow</a:t></a:r></a:p></p:txBody></p:sp>"
            )
        },
    );
    let inner = shadow_element("innerShdw", &polar(10.0), &black);
    add("C06 innerShdw", 1, &|s| {
        sp(
            s,
            "Rectangle",
            centre(),
            &rect_geom,
            &white,
            &effect_lst(&inner),
            "",
        )
    });
    for (k, preset) in ["shdw1", "shdw13", "shdw17"].into_iter().enumerate() {
        let element = shadow_element(
            "prstShdw",
            &format!(
                "prst=\"{preset}\" dist=\"{}\" dir=\"2700000\"",
                emu(BASE.dist)
            ),
            &black,
        );
        add(&format!("C{:02} prstShdw {preset}", k + 7), 1, &|s| {
            sp(
                s,
                "Rectangle",
                centre(),
                &rect_geom,
                &white,
                &effect_lst(&element),
                "",
            )
        });
    }
    add("C10 outer and inner shadow", 1, &|s| {
        sp(
            s,
            "Rectangle",
            centre(),
            &rect_geom,
            &white,
            &effect_lst(&format!("{inner}{outer_base}")),
            "",
        )
    });
    for (label, attrs) in [
        (
            "C11 sx/sy 50 % aligned tl",
            "sx=\"50000\" sy=\"50000\" kx=\"0\" ky=\"0\" algn=\"tl\"",
        ),
        (
            "C12 sx 150 % aligned t",
            "sx=\"150000\" sy=\"100000\" kx=\"0\" ky=\"0\" algn=\"t\"",
        ),
        (
            "C13 sy 50 % only",
            "sx=\"100000\" sy=\"50000\" kx=\"0\" ky=\"0\" algn=\"b\"",
        ),
        (
            "C14 kx 20",
            "sx=\"100000\" sy=\"100000\" kx=\"1200000\" ky=\"0\" algn=\"b\"",
        ),
    ] {
        let element = shadow_element(
            "outerShdw",
            &format!("{attrs} rotWithShape=\"0\" {base_attrs}"),
            &black,
        );
        add(label, 1, &|s| {
            sp(
                s,
                "Rectangle",
                centre(),
                &rect_geom,
                &white,
                &effect_lst(&element),
                "",
            )
        });
    }
    let turned = xfrm(CENTRE, 30, false, false);
    let with_rot = shadow_element(
        "outerShdw",
        &format!("{base_attrs} rotWithShape=\"1\""),
        &black,
    );
    let without = shadow_element("outerShdw", &base_attrs, &black);
    add("C15 rotWithShape 1 on a rectangle turned 30", 1, &|s| {
        sp(
            s,
            "Rectangle",
            turned.clone(),
            &rect_geom,
            &white,
            &effect_lst(&with_rot),
            "",
        )
    });
    add(
        "C16 rotWithShape absent on a rectangle turned 30",
        1,
        &|s| {
            sp(
                s,
                "Rectangle",
                turned.clone(),
                &rect_geom,
                &white,
                &effect_lst(&without),
                "",
            )
        },
    );
    add("C17 right triangle flipped H, rotWithShape 1", 1, &|s| {
        sp(
            s,
            "Triangle",
            xfrm(CENTRE, 0, true, false),
            &geometry(Geom::RightTriangle),
            &white,
            &effect_lst(&with_rot),
            "",
        )
    });
    add("C18 group shadow", 1, &|s| {
        let id = s.id();
        let left = sp(
            s,
            "Left",
            xfrm((300.0, 170.0, 160.0, 100.0), 0, false, false),
            &rect_geom,
            &white,
            "",
            "",
        );
        let right = sp(
            s,
            "Right",
            xfrm((500.0, 270.0, 160.0, 100.0), 0, false, false),
            &rect_geom,
            &solid(AMBER, OPAQUE),
            "",
            "",
        );
        let (x, y, w, h) = (emu(300.0), emu(170.0), emu(360.0), emu(200.0));
        format!(
            "<p:grpSp><p:nvGrpSpPr><p:cNvPr id=\"{id}\" name=\"Group\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{w}\" cy=\"{h}\"/><a:chOff x=\"{x}\" y=\"{y}\"/><a:chExt cx=\"{w}\" cy=\"{h}\"/></a:xfrm>{}</p:grpSpPr>{left}{right}</p:grpSp>",
            effect_lst(&outer_base)
        )
    });
    add("C19 table shadow", 1, &|s| {
        let id = s.id();
        let cell = |filled: bool| {
            let fill = if filled {
                solid(BLUE, OPAQUE)
            } else {
                "<a:noFill/>".to_owned()
            };
            format!(
                "<a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang=\"en-US\" dirty=\"0\"/></a:p></a:txBody><a:tcPr>{fill}</a:tcPr></a:tc>"
            )
        };
        let row = |first: bool| {
            format!(
                "<a:tr h=\"{}\">{}{}</a:tr>",
                emu(80.0),
                cell(first),
                cell(!first)
            )
        };
        format!(
            "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{id}\" name=\"Table\"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp=\"1\"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></p:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\"><a:tbl><a:tblPr>{}</a:tblPr><a:tblGrid><a:gridCol w=\"{}\"/><a:gridCol w=\"{}\"/></a:tblGrid>{}{}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>",
            emu(CENTRE.0),
            emu(CENTRE.1),
            emu(CENTRE.2),
            emu(CENTRE.3),
            effect_lst(&outer_base),
            emu(CENTRE.2 / 2.0),
            emu(CENTRE.2 / 2.0),
            row(true),
            row(false)
        )
    });
    let text_frame = (240.0, 200.0, 480.0, 120.0);
    add("C20 run shadow (a:rPr)", 1, &|s| {
        let id = s.id();
        let body = text_body(
            &Text {
                lines: &["Run shadow"],
                size: 72,
                color: INK,
            },
            &effect_lst(&outer_base),
        );
        format!(
            "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Text\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr>{}{rect_geom}<a:noFill/>{no_line}</p:spPr>{body}</p:sp>",
            xfrm(text_frame, 0, false, false)
        )
    });
    add("C21 run shadow from the body's lstStyle", 1, &|s| {
        let id = s.id();
        let body = text_body(
            &Text {
                lines: &["List shadow"],
                size: 72,
                color: INK,
            },
            "",
        )
        .replacen(
            "<a:lstStyle/>",
            &format!(
                "<a:lstStyle><a:lvl1pPr><a:defRPr>{}</a:defRPr></a:lvl1pPr></a:lstStyle>",
                effect_lst(&outer_base)
            ),
            1,
        );
        format!(
            "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Text\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr>{}{rect_geom}<a:noFill/>{no_line}</p:spPr>{body}</p:sp>",
            xfrm(text_frame, 0, false, false)
        )
    });
    add("C22 shadow in an effectDag", 1, &|s| {
        sp(
            s,
            "Rectangle",
            centre(),
            &rect_geom,
            &white,
            &format!("<a:effectDag type=\"sib\">{outer_base}</a:effectDag>"),
            "",
        )
    });
    add("C23 fill at 50 %, shadow at 50 %", 1, &|s| {
        sp(
            s,
            "Rectangle",
            centre(),
            &rect_geom,
            &solid(BLUE, 50_000),
            &effect_lst(&outer_base),
            "",
        )
    });
    add("C24 picture at alphaModFix 50 % with a shadow", 1, &|s| {
        let picture = Item::new(
            "Faded photo",
            (360.0, 198.0, 240.0, 144.0),
            Kind::Picture {
                image: Image::Photo,
                crop: None,
                border: None,
                round: false,
                alpha: Some(50_000),
            },
            Some(BASE),
        );
        pptx_item(s, &picture)
    });
    add("C25 no fill, no line, no text, with a shadow", 1, &|s| {
        sp(
            s,
            "Empty",
            centre(),
            &rect_geom,
            "<a:noFill/>",
            &effect_lst(&outer_base),
            "",
        )
    });

    let title = |effects: &str| {
        format!(
            "<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Title\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph type=\"title\"/></p:nvPr></p:nvSpPr><p:spPr>{}{rect_geom}{white}{no_line}{effects}</p:spPr><p:txBody><a:bodyPr lIns=\"91440\" tIns=\"45720\" rIns=\"91440\" bIns=\"45720\" anchor=\"ctr\"><a:noAutofit/></a:bodyPr><a:lstStyle><a:lvl1pPr algn=\"ctr\"><a:defRPr sz=\"4400\"><a:solidFill><a:srgbClr val=\"1F2937\"/></a:solidFill><a:latin typeface=\"Liberation Sans\"/></a:defRPr></a:lvl1pPr></a:lstStyle><a:p><a:r><a:rPr lang=\"en-US\" dirty=\"0\"/><a:t>Title</a:t></a:r></a:p></p:txBody></p:sp>",
            xfrm((240.0, 190.0, 480.0, 160.0), 0, false, false)
        )
    };
    pptx_package(&PptxDeck {
        theme_effects: &theme_effects,
        master_tree: &title(""),
        layouts: vec![String::new(), title(&effect_lst(&outer_base))],
        slides,
        media: vec![("photo.jpg", PHOTO.to_vec())],
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// Pictures drawn here

/// A 240 × 160 RGBA picture: an orange disc (radius 72) with a hole (radius 28) on a transparent ground.
fn disc_png() -> Vec<u8> {
    let (width, height) = (240u32, 160u32);
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            // Pixel centres against the picture's centre (120, 80), in quarter pixels to stay in integers.
            let dx = 4 * i64::from(x) + 2 - 480;
            let dy = 4 * i64::from(y) + 2 - 320;
            let d2 = dx * dx + dy * dy;
            let inside = d2 <= (4 * 72) * (4 * 72) && d2 > (4 * 28) * (4 * 28);
            if inside {
                pixels.extend_from_slice(&[0xFE, 0x78, 0x67, 0xFF]);
            } else {
                pixels.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    png_rgba(width, height, &pixels)
}

/// A 240 × 160 RGBA picture of one blue whose alpha ramps from 0 at the left edge to 255 at the right.
fn ramp_png() -> Vec<u8> {
    let (width, height) = (240u32, 160u32);
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..height {
        for x in 0..width {
            // 0 at x = 0, 255 at x = width − 1.
            #[allow(clippy::cast_possible_truncation)]
            let alpha = ((x * 255 + (width - 1) / 2) / (width - 1)) as u8;
            pixels.extend_from_slice(&[0x2F, 0x6F, 0xDF, alpha]);
        }
    }
    png_rgba(width, height, &pixels)
}

/// A PNG of RGBA pixels: 8 bits, each row unfiltered, zlib around the deflate stream.
fn png_rgba(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let stride = width as usize * 4;
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for row in pixels.chunks(stride) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
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
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_decks_are_the_generators() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../probes/out");
        assert_eq!(stale(&root), Vec::<String>::new());
    }

    #[test]
    fn the_twins_have_46_slides_of_one_labelled_case_each() {
        let slides = twin_slides();
        assert_eq!(slides.len(), 46);
        for (k, slide) in slides.iter().enumerate() {
            assert!(
                slide.label.starts_with(&format!("A{:02} ", k + 1)),
                "{}",
                slide.label
            );
        }
        let pptx = twin_pptx(&slides);
        let odp = twin_odp(&slides);
        let slide_parts = pptx
            .iter()
            .filter(|e| e.name.starts_with("ppt/slides/slide"))
            .count();
        assert_eq!(slide_parts, 46);
        let content = odp.iter().find(|e| e.name == "content.xml").unwrap();
        let content = std::str::from_utf8(&content.data).unwrap();
        assert_eq!(content.matches("<draw:page ").count(), 46);
        // Every shadow of the PPTX is in the ODP: one per slide, but two on A43 and A44 and 48 on A46.
        let shadows: usize = pptx
            .iter()
            .filter(|e| e.name.starts_with("ppt/slides/slide"))
            .map(|e| {
                std::str::from_utf8(&e.data)
                    .unwrap()
                    .matches("<a:outerShdw ")
                    .count()
            })
            .sum();
        assert_eq!(shadows, 43 + 2 + 2 + 48);
        assert_eq!(content.matches("draw:shadow=\"visible\"").count(), shadows);
    }

    #[test]
    fn the_other_decks_have_their_cases() {
        let content = odp_only();
        let content = content.iter().find(|e| e.name == "content.xml").unwrap();
        let content = std::str::from_utf8(&content.data).unwrap();
        assert_eq!(content.matches("<draw:page ").count(), 16);
        let ooxml = ooxml_only();
        let slides = ooxml
            .iter()
            .filter(|e| e.name.starts_with("ppt/slides/slide"))
            .count();
        assert_eq!(slides, 25);
    }

    #[test]
    fn offsets_are_exact_at_the_probes_angles() {
        assert_eq!(BASE.offset(), (14.1421, 14.1421));
        assert_eq!(
            Shadow {
                dir: 90,
                dist: 3.287,
                ..BASE
            }
            .offset(),
            (0.0, 3.287)
        );
        assert_eq!(Shadow { dir: 225, ..BASE }.offset(), (-14.1421, -14.1421));
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(14.14210), "14.1421");
        assert_eq!(percent(RETRO_ALPHA), "53.314%");
    }

    #[test]
    fn a_turned_box_keeps_its_centre() {
        // The 240 × 160 box turned 90° clockwise: its corner goes to the centre plus R(−90°)·(−w/2, −h/2) …
        let at = placement(CENTRE, 90);
        assert!(at.contains("rotate (-1.570796326795)"), "{at}");
        assert!(at.contains("translate (560pt 150pt)"), "{at}");
        assert_eq!(
            turn_about((0.0, 0.0, 2.0, 2.0), 90, (5.0, 5.0)),
            (8.0, 0.0, 2.0, 2.0)
        );
    }

    #[test]
    fn pngs_decode_to_their_size() {
        let disc = disc_png();
        assert_eq!(&disc[1..4], b"PNG");
        assert_eq!(&disc[16..24], &[0, 0, 0, 240, 0, 0, 0, 160]);
        assert_eq!(disc[25], 6, "RGBA");
    }
}
