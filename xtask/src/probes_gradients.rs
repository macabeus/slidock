//! The gradient probe decks: 960 × 540 pt slides, one case per slide, each named by a 12 pt
//! Liberation Sans caption in its top-left corner that gives the case's parameters. Unless a case says otherwise the
//! object is a 600 × 200 pt box at (180, 170) (3:1, so a circle and the box's ellipse differ) filled black → white, so
//! the drawn luminance is the ramp parameter t, on a #BFC9D4 slide; stops are opaque unless the case says otherwise.
//!
//! `gradient-probe.pptx` (G01–G43; LibreOffice is the oracle only of the slides marked so in the cases file, Keynote's
//! import of the deck is the oracle of all):
//!
//! - G01–G06: `lin` 0; 45 unscaled; 45 scaled; 30 on 150 × 450; 90 on 300 × 300; 315.
//! - G07–G11: 3 stops r/g/b at 0/40/100; a hard stop (two stops at 50); stops inset to 25/75; 10 stops; black at
//!   100 % → red at 20 % over stripes.
//! - G12–G16: `lin` 0 on a box turned 30° with `rotWithShape` 1; with 0; `lin` 45 flipped horizontally; vertically;
//!   horizontally with `rotWithShape` 0.
//! - G17: an ellipse filled by `lin` 0.
//! - G18–G19: a centred circle path on 3:1; on a square.
//! - G20–G23: a circle path with a point focus at (.75,.75), (.75,.25), (.25,.75), (.25,.25) on 400 × 200.
//! - G24–G27: a circle path with `fillToRect` l10 t30 r80 b20; PowerPoint's "from bottom-right corner" preset; its
//!   "from top-left corner"; a focus above the box (t −80 % b 180 %).
//! - G28–G30: a rect path with a centred point focus; l10 t30 r80 b20 with red@0, yellow@1 %, blue@100 %; the
//!   bottom-right corner preset.
//! - G31–G32: a shape path on an ellipse; on a star.
//! - G33–G34: a centred circle path on a box turned 30°; a rect path with its focus at (.25, .5) on a box flipped
//!   horizontally.
//! - G35: `lin` 0 with `tileRect` l 50 % and `flip="x"`.
//! - G36–G37: a background `lin` 90; a background centred circle path.
//! - G38: a 2 × 2 table: `lin` 90; `lin` 0 to a transparent stop; a centred circle path; no fill.
//! - G39: a group whose `grpFill` gradient fills its two children.
//! - G40: `fillRef` 2 and 3: the theme's vertical gradients (`scaled` 1 and 0) of `phClr`.
//! - G41: a gradient line and gradient text.
//! - G42: 600 × 20 bars, `lin` 0 and 90.
//! - G43: 48 small squares of every kind LibreOffice draws as PowerPoint does (cost).
//!
//! `gradient-probe.odp` (O01–O41, LibreOffice is the oracle of all): every ODF gradient style (linear, axial, radial,
//! ellipsoid, square, rectangular) with angle, border, centre, step count and intensity; rotated, mirrored and
//! flipped shapes; multi-colour stops (`loext:gradient-stop`); defaults; transparency gradients (`draw:opacity`);
//! backgrounds; a table; a rotated ellipse; a `draw:path` smaller than its frame; a group. `gradient-probe-odp12.odp`
//! (N01–N02): ODF 1.2 written by an old LibreOfficeDev, so a unitless angle is in tenths of a degree.
//!
//! `gradient-probe.cases.json` lists every slide: its id, caption, row id, parameters, whether LibreOffice is a valid
//! oracle, the diagnostics the compiler should report on it, and for the PPTX slides PowerPoint's analytic geometry
//! (box fractions: the linear ends, a circle's centre, radii and focus, a rect path's frame and inner
//! rectangle).

use std::fmt::Write as _;
use std::path::Path;

use crate::probes_pictures::probes_zip::{self, Entry};

#[path = "probes_json.rs"]
#[allow(
    dead_code,
    clippy::duplicate_mod,
    reason = "the gradient probes use the printer only"
)]
mod probes_json;

use probes_json::Json;

const USAGE: &str = "usage: cargo xtask probes-gradients [--check]";

/// EMU per point.
const EMU: f64 = 12_700.0;

type Rgb = [u8; 3];

const BLACK: Rgb = [0, 0, 0];
const WHITE: Rgb = [0xFF, 0xFF, 0xFF];
const RED: Rgb = [0xFF, 0, 0];
const GREEN: Rgb = [0, 0xFF, 0];
const BLUE: Rgb = [0, 0, 0xFF];
const YELLOW: Rgb = [0xFF, 0xFF, 0];
const SLIDE: Rgb = [0xBF, 0xC9, 0xD4];
const LABEL: Rgb = [0x33, 0x33, 0x33];
const STRIPE_A: Rgb = [0x2F, 0x6F, 0xDF];
const STRIPE_B: Rgb = [0xF9, 0xA8, 0x25];
const THEME_ACCENT: Rgb = [0x2F, 0x6F, 0xDF];

/// OOXML's whole: 100 % in thousandths of a percent.
const WHOLE: i32 = 100_000;

/// The default object.
const BOX: Frame = (180.0, 170.0, 600.0, 200.0);
const SLIDE_SIZE: (f64, f64) = (960.0, 540.0);

/// x, y, width, height in points.
type Frame = (f64, f64, f64, f64);

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
                println!("probes/out gradient files are up to date");
                Ok(())
            } else {
                Err(format!(
                    "stale (run cargo xtask probes-gradients): {}",
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
    let pptx = pptx_slides();
    let odp = odp_slides();
    let odp12 = odp12_slides();
    let cases = Json::obj()
        .with(
            "note",
            "Fractions are of the paint box (the shape's frame, the slide for a background, \
             the declared cell for a cell), x right and y down, before the frame's turn and flips. Angles are \
             degrees, clockwise from +x in OOXML, the ODF draw:angle as written.",
        )
        .with(
            "decks",
            Json::Arr(vec![
                deck_json(
                    "gradient-probe.pptx",
                    "pptx",
                    pptx.iter().enumerate().map(|(k, s)| s.json(k)).collect(),
                ),
                deck_json(
                    "gradient-probe.odp",
                    "odp",
                    odp.iter().enumerate().map(|(k, s)| s.json(k)).collect(),
                ),
                deck_json(
                    "gradient-probe-odp12.odp",
                    "odp",
                    odp12.iter().enumerate().map(|(k, s)| s.json(k)).collect(),
                ),
            ]),
        );
    vec![
        (
            "gradient-probe.pptx".to_owned(),
            probes_zip::write_zip(&pptx_package(&pptx)),
        ),
        (
            "gradient-probe.odp".to_owned(),
            probes_zip::write_zip(&odp_package(&odp, OdfVersion::V13)),
        ),
        (
            "gradient-probe-odp12.odp".to_owned(),
            probes_zip::write_zip(&odp_package(&odp12, OdfVersion::V12OldGenerator)),
        ),
        (
            "gradient-probe.cases.json".to_owned(),
            cases.pretty_until(8).into_bytes(),
        ),
    ]
}

fn deck_json(file: &str, frontend: &str, slides: Vec<Json>) -> Json {
    Json::obj()
        .with("file", file)
        .with("frontend", frontend)
        .with("slides", Json::Arr(slides))
}

// ---------------------------------------------------------------------------------------------------------------------
// Numbers

/// The cosine and sine of the probes' angles, exact to the last bit (no libm: the outputs are the same on every host).
fn cos_sin(degrees: i32) -> (f64, f64) {
    let half = std::f64::consts::FRAC_1_SQRT_2;
    let third = 3f64.sqrt() / 2.0;
    match degrees.rem_euclid(360) {
        0 => (1.0, 0.0),
        30 => (third, 0.5),
        45 => (half, half),
        60 => (0.5, third),
        90 => (0.0, 1.0),
        135 => (-half, half),
        180 => (-1.0, 0.0),
        225 => (-half, -half),
        270 => (0.0, -1.0),
        315 => (half, -half),
        330 => (third, -0.5),
        other => panic!("no exact cosine for {other} degrees"),
    }
}

fn round_to(value: f64, places: i32) -> f64 {
    let scale = match places {
        4 => 10_000.0,
        _ => 1_000_000.0,
    };
    let rounded = (value * scale).round() / scale;
    if rounded == 0.0 { 0.0 } else { rounded }
}

/// A number with at most `places` decimals (4 or 6) and no trailing zeros.
fn fixed(value: f64, places: i32) -> String {
    let text = if places == 4 {
        format!("{:.4}", round_to(value, 4))
    } else {
        format!("{:.6}", round_to(value, 6))
    };
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        &text
    };
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

fn num(value: f64) -> String {
    fixed(value, 4)
}

/// A JSON number of 6 decimals.
fn jnum(value: f64) -> Json {
    Json::Num(fixed(value, 6))
}

fn jpoint(x: f64, y: f64) -> Json {
    Json::Arr(vec![jnum(x), jnum(y)])
}

fn emu(points: f64) -> i64 {
    // Far inside i64.
    #[allow(clippy::cast_possible_truncation)]
    let value = (points * EMU).round() as i64;
    value
}

fn hex(color: Rgb) -> String {
    format!("{:02X}{:02X}{:02X}", color[0], color[1], color[2])
}

fn odf_color(color: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn jframe((x, y, w, h): Frame) -> Json {
    Json::Arr(vec![jnum(x), jnum(y), jnum(w), jnum(h)])
}

// ---------------------------------------------------------------------------------------------------------------------
// The PPTX model

/// A gradient stop: position and opacity in OOXML thousandths of a percent.
#[derive(Clone, Copy)]
struct Stop {
    pos: i32,
    color: Rgb,
    alpha: i32,
}

const fn stop(pos: i32, color: Rgb) -> Stop {
    Stop {
        pos,
        color,
        alpha: WHOLE,
    }
}

const fn faded(pos: i32, color: Rgb, alpha: i32) -> Stop {
    Stop { pos, color, alpha }
}

const BW: [Stop; 2] = [stop(0, BLACK), stop(WHOLE, WHITE)];

#[derive(Clone, Copy, PartialEq)]
enum PathKind {
    Circle,
    Rect,
    Shape,
}

impl PathKind {
    fn name(self) -> &'static str {
        match self {
            PathKind::Circle => "circle",
            PathKind::Rect => "rect",
            PathKind::Shape => "shape",
        }
    }
}

/// Insets l, t, r, b in OOXML thousandths of a percent of the box (negative = outside it).
type Insets = [i32; 4];

#[derive(Clone, Copy)]
enum Shade {
    /// `a:lin`: angle in degrees clockwise from +x, `scaled`.
    Lin { ang: i32, scaled: bool },
    /// `a:path` with its `a:fillToRect`.
    Path { kind: PathKind, fill_to: Insets },
}

#[derive(Clone)]
struct Grad {
    stops: Vec<Stop>,
    shade: Shade,
    /// `rotWithShape`, always written.
    rws: bool,
    tile: Option<Insets>,
    flip: Option<&'static str>,
}

fn lin(ang: i32) -> Grad {
    Grad {
        stops: BW.to_vec(),
        shade: Shade::Lin { ang, scaled: false },
        rws: true,
        tile: None,
        flip: None,
    }
}

fn path(kind: PathKind, fill_to: Insets) -> Grad {
    Grad {
        stops: BW.to_vec(),
        shade: Shade::Path { kind, fill_to },
        rws: true,
        tile: None,
        flip: None,
    }
}

const CENTRED: Insets = [50_000, 50_000, 50_000, 50_000];

impl Grad {
    fn with_stops(mut self, stops: &[Stop]) -> Grad {
        self.stops = stops.to_vec();
        self
    }

    fn scaled(mut self) -> Grad {
        if let Shade::Lin { ang, .. } = self.shade {
            self.shade = Shade::Lin { ang, scaled: true };
        }
        self
    }

    fn rws(mut self, rws: bool) -> Grad {
        self.rws = rws;
        self
    }

    fn tiled(mut self, tile: Insets) -> Grad {
        self.tile = Some(tile);
        self
    }

    fn flipped(mut self, flip: &'static str) -> Grad {
        self.flip = Some(flip);
        self
    }

    fn xml(&self) -> String {
        let mut out = String::from("<a:gradFill");
        if let Some(flip) = self.flip {
            let _ = write!(out, " flip=\"{flip}\"");
        }
        let _ = write!(out, " rotWithShape=\"{}\"><a:gsLst>", u8::from(self.rws));
        for s in &self.stops {
            let _ = write!(
                out,
                "<a:gs pos=\"{}\">{}</a:gs>",
                s.pos,
                srgb(s.color, s.alpha)
            );
        }
        out.push_str("</a:gsLst>");
        match self.shade {
            Shade::Lin { ang, scaled } => {
                let _ = write!(
                    out,
                    "<a:lin ang=\"{}\" scaled=\"{}\"/>",
                    ang.rem_euclid(360) * 60_000,
                    u8::from(scaled)
                );
            }
            Shade::Path { kind, fill_to } => {
                let _ = write!(
                    out,
                    "<a:path path=\"{}\"><a:fillToRect{}/></a:path>",
                    kind.name(),
                    insets_attrs(fill_to)
                );
            }
        }
        if let Some(tile) = self.tile {
            let _ = write!(out, "<a:tileRect{}/>", insets_attrs(tile));
        }
        out.push_str("</a:gradFill>");
        out
    }

    fn json(&self) -> Json {
        let stops = self
            .stops
            .iter()
            .map(|s| {
                Json::obj()
                    .with("pos", jnum(f64::from(s.pos) / f64::from(WHOLE)))
                    .with("color", hex(s.color))
                    .with("alpha", jnum(f64::from(s.alpha) / f64::from(WHOLE)))
            })
            .collect();
        let mut out = match self.shade {
            Shade::Lin { ang, scaled } => Json::obj()
                .with("type", "lin")
                .with("ang", i64::from(ang))
                .with("scaled", scaled),
            Shade::Path { kind, fill_to } => Json::obj()
                .with("type", "path")
                .with("path", kind.name())
                .with("fillToRect", insets_json(fill_to)),
        };
        out.push("rotWithShape", self.rws);
        if let Some(tile) = self.tile {
            out.push("tileRect", insets_json(tile));
        }
        if let Some(flip) = self.flip {
            out.push("flip", flip);
        }
        out.push("stops", Json::Arr(stops));
        out
    }
}

fn insets_attrs(insets: Insets) -> String {
    let mut out = String::new();
    for (name, value) in ["l", "t", "r", "b"].iter().zip(insets) {
        if value != 0 {
            let _ = write!(out, " {name}=\"{value}\"");
        }
    }
    out
}

fn insets_json(insets: Insets) -> Json {
    Json::Arr(
        insets
            .iter()
            .map(|v| jnum(f64::from(*v) / f64::from(WHOLE)))
            .collect(),
    )
}

fn srgb(color: Rgb, alpha: i32) -> String {
    if alpha >= WHOLE {
        format!("<a:srgbClr val=\"{}\"/>", hex(color))
    } else {
        format!(
            "<a:srgbClr val=\"{}\"><a:alpha val=\"{alpha}\"/></a:srgbClr>",
            hex(color)
        )
    }
}

fn solid(color: Rgb) -> String {
    format!("<a:solidFill>{}</a:solidFill>", srgb(color, WHOLE))
}

#[derive(Clone, Copy, PartialEq)]
enum Geom {
    Rect,
    Ellipse,
    Star,
}

impl Geom {
    fn name(self) -> &'static str {
        match self {
            Geom::Rect => "rect",
            Geom::Ellipse => "ellipse",
            Geom::Star => "star (custom)",
        }
    }
}

#[derive(Clone)]
enum Fill {
    Grad(Grad),
    Solid(Rgb),
    /// `a:grpFill`: the group's fill.
    Group,
}

#[derive(Clone)]
struct PShape {
    name: String,
    frame: Frame,
    rot: i32,
    flip_h: bool,
    flip_v: bool,
    geom: Geom,
    fill: Fill,
}

fn shape(name: &str, frame: Frame, fill: Fill) -> PShape {
    PShape {
        name: name.to_owned(),
        frame,
        rot: 0,
        flip_h: false,
        flip_v: false,
        geom: Geom::Rect,
        fill,
    }
}

fn boxed(grad: Grad) -> PShape {
    shape("Gradient box", BOX, Fill::Grad(grad))
}

impl PShape {
    fn turned(mut self, rot: i32) -> PShape {
        self.rot = rot;
        self
    }

    fn flip_h(mut self) -> PShape {
        self.flip_h = true;
        self
    }

    fn flip_v(mut self) -> PShape {
        self.flip_v = true;
        self
    }

    fn geom(mut self, geom: Geom) -> PShape {
        self.geom = geom;
        self
    }

    fn at(mut self, frame: Frame) -> PShape {
        self.frame = frame;
        self
    }
}

/// One cell of a 2 × 2 table: a gradient or no fill.
type Cell = Option<Grad>;

enum PItem {
    Shape(PShape),
    /// A group whose own fill is `fill`; its children fill with `a:grpFill`.
    Group {
        fill: Grad,
        children: Vec<PShape>,
    },
    /// A 2 × 2 table over the frame, rows first.
    Table {
        frame: Frame,
        cells: Box<[Cell; 4]>,
    },
    /// A straight line from the frame's left to its right edge, at its vertical middle.
    Line {
        frame: Frame,
        width: f64,
        fill: Grad,
    },
    /// A text box of one bold run filled by the gradient.
    Text {
        frame: Frame,
        text: &'static str,
        size: u32,
        fill: Grad,
    },
    /// A rectangle filled through `p:style`'s `fillRef idx`, with this reference colour.
    ThemeRef {
        frame: Frame,
        idx: u32,
        color: Rgb,
    },
}

struct PSlide {
    id: String,
    caption: String,
    background: Option<Grad>,
    label: Rgb,
    items: Vec<PItem>,
    lo_valid: bool,
    note: Option<&'static str>,
    diagnostics: Vec<&'static str>,
}

fn pptx_slides() -> Vec<PSlide> {
    let mut slides: Vec<PSlide> = Vec::new();
    let mut add = |caption: &str, items: Vec<PItem>, lo_valid: bool| {
        let id = format!("G{:02}", slides.len() + 1);
        slides.push(PSlide {
            caption: format!("{id} {caption}"),
            id,
            background: None,
            label: LABEL,
            items,
            lo_valid,
            note: None,
            diagnostics: Vec::new(),
        });
    };
    let one = |s: PShape| vec![PItem::Shape(s)];
    // G01–G06: angles and extents.
    add("lin ang 0 scaled 0", one(boxed(lin(0))), true);
    add("lin ang 45 scaled 0", one(boxed(lin(45))), true);
    add("lin ang 45 scaled 1", one(boxed(lin(45).scaled())), false);
    add(
        "lin ang 30 scaled 0 on 150 x 450",
        one(boxed(lin(30)).at((405.0, 45.0, 150.0, 450.0))),
        true,
    );
    add(
        "lin ang 90 scaled 0 on 300 x 300",
        one(boxed(lin(90)).at((330.0, 120.0, 300.0, 300.0))),
        true,
    );
    add("lin ang 315 scaled 0", one(boxed(lin(315))), true);
    // G07–G11: stops.
    add(
        "lin 0, 3 stops red 0, green 40, blue 100",
        one(boxed(lin(0).with_stops(&[
            stop(0, RED),
            stop(40_000, GREEN),
            stop(WHOLE, BLUE),
        ]))),
        true,
    );
    add(
        "lin 0, hard stop: black 0, black 50, white 50, white 100",
        one(boxed(lin(0).with_stops(&[
            stop(0, BLACK),
            stop(50_000, BLACK),
            stop(50_000, WHITE),
            stop(WHOLE, WHITE),
        ]))),
        true,
    );
    add(
        "lin 0, stops inset: black 25, white 75",
        one(boxed(
            lin(0).with_stops(&[stop(25_000, BLACK), stop(75_000, WHITE)]),
        )),
        true,
    );
    let ten: Vec<Stop> = (0..10)
        .map(|k| {
            let pos = (k * WHOLE + 4) / 9;
            stop(pos, if k % 2 == 0 { BLACK } else { WHITE })
        })
        .collect();
    add(
        "lin 0, 10 stops alternating black and white at k/9",
        one(boxed(lin(0).with_stops(&ten))),
        true,
    );
    let mut stripes: Vec<PItem> = (0..5)
        .map(|k| {
            let color = if k % 2 == 0 { STRIPE_A } else { STRIPE_B };
            PItem::Shape(shape(
                &format!("Stripe {}", k + 1),
                (160.0, 150.0 + 48.0 * f64::from(k), 640.0, 48.0),
                Fill::Solid(color),
            ))
        })
        .collect();
    stripes.push(PItem::Shape(boxed(
        lin(0).with_stops(&[stop(0, BLACK), faded(WHOLE, RED, 20_000)]),
    )));
    add(
        "lin 0, black at 100% to red at 20% over stripes",
        stripes,
        true,
    );
    // G12–G16: rotation and flips.
    add(
        "lin 0 on a box turned 30, rotWithShape 1",
        one(boxed(lin(0)).turned(30)),
        false,
    );
    add(
        "lin 0 on a box turned 30, rotWithShape 0",
        one(boxed(lin(0).rws(false)).turned(30)),
        true,
    );
    add(
        "lin 45 flipH, rotWithShape 1",
        one(boxed(lin(45)).flip_h()),
        true,
    );
    add(
        "lin 45 flipV, rotWithShape 1",
        one(boxed(lin(45)).flip_v()),
        true,
    );
    add(
        "lin 45 flipH, rotWithShape 0",
        one(boxed(lin(45).rws(false)).flip_h()),
        false,
    );
    // G17: the extent is the box, not the outline.
    add(
        "ellipse, lin 0",
        one(boxed(lin(0)).geom(Geom::Ellipse)),
        true,
    );
    // G18–G23: circles.
    add(
        "circle path, centred focus, on 600 x 200",
        one(boxed(path(PathKind::Circle, CENTRED))),
        true,
    );
    add(
        "circle path, centred focus, on 300 x 300",
        one(boxed(path(PathKind::Circle, CENTRED)).at((330.0, 120.0, 300.0, 300.0))),
        true,
    );
    for (fx, fy) in [(75, 75), (75, 25), (25, 75), (25, 25)] {
        let insets = [fx * 1000, fy * 1000, WHOLE - fx * 1000, WHOLE - fy * 1000];
        add(
            &format!("circle path, point focus (.{fx}, .{fy}), on 400 x 200"),
            one(boxed(path(PathKind::Circle, insets)).at((280.0, 170.0, 400.0, 200.0))),
            false,
        );
    }
    // G24–G27: focus rectangles and tiles.
    add(
        "circle path, fillToRect l10 t30 r80 b20",
        one(boxed(path(
            PathKind::Circle,
            [10_000, 30_000, 80_000, 20_000],
        ))),
        false,
    );
    add(
        "circle path, bottom-right corner preset (fillToRect l t 100, tileRect r b -100)",
        one(boxed(
            path(PathKind::Circle, [WHOLE, WHOLE, 0, 0]).tiled([0, 0, -WHOLE, -WHOLE]),
        )),
        false,
    );
    add(
        "circle path, top-left corner preset (fillToRect r b 100, tileRect l t -100)",
        one(boxed(
            path(PathKind::Circle, [0, 0, WHOLE, WHOLE]).tiled([-WHOLE, -WHOLE, 0, 0]),
        )),
        false,
    );
    add(
        "circle path, focus above the box (fillToRect l50 t-80 r50 b180)",
        one(boxed(path(
            PathKind::Circle,
            [50_000, -80_000, 50_000, 180_000],
        ))),
        false,
    );
    // G28–G30: rect paths.
    add(
        "rect path, centred point focus",
        one(boxed(path(PathKind::Rect, CENTRED))),
        false,
    );
    add(
        "rect path, fillToRect l10 t30 r80 b20, red 0, yellow 1, blue 100",
        one(boxed(
            path(PathKind::Rect, [10_000, 30_000, 80_000, 20_000]).with_stops(&[
                stop(0, RED),
                stop(1_000, YELLOW),
                stop(WHOLE, BLUE),
            ]),
        )),
        false,
    );
    add(
        "rect path, bottom-right corner preset (fillToRect l t 100, tileRect r b -100)",
        one(boxed(
            path(PathKind::Rect, [WHOLE, WHOLE, 0, 0]).tiled([0, 0, -WHOLE, -WHOLE]),
        )),
        false,
    );
    // G31–G32: shape paths.
    add(
        "shape path, centred focus, on an ellipse",
        one(boxed(path(PathKind::Shape, CENTRED)).geom(Geom::Ellipse)),
        false,
    );
    add(
        "shape path, centred focus, on a star",
        one(boxed(path(PathKind::Shape, CENTRED)).geom(Geom::Star)),
        false,
    );
    // G33–G35: paths on turned and flipped boxes, tiles.
    add(
        "circle path, centred focus, on a box turned 30",
        one(boxed(path(PathKind::Circle, CENTRED)).turned(30)),
        true,
    );
    add(
        "rect path, point focus (.25, .5), flipH",
        one(boxed(path(PathKind::Rect, [25_000, 50_000, 75_000, 50_000])).flip_h()),
        false,
    );
    add(
        "lin 0, tileRect l 50, flip x",
        one(boxed(lin(0).tiled([50_000, 0, 0, 0]).flipped("x"))),
        false,
    );
    // G36–G37: backgrounds.
    add("background lin 90", Vec::new(), true);
    add("background circle path, centred focus", Vec::new(), true);
    // G38: a table.
    add(
        "2 x 2 table: lin 90; lin 0 to a transparent stop; centred circle path; no fill",
        vec![PItem::Table {
            frame: BOX,
            cells: Box::new([
                Some(lin(90)),
                Some(lin(0).with_stops(&[stop(0, BLACK), faded(WHOLE, BLACK, 0)])),
                Some(path(PathKind::Circle, CENTRED)),
                None,
            ]),
        }],
        true,
    );
    // G39: a group's fill.
    add(
        "group of two boxes, grpFill of the group's lin 0",
        vec![PItem::Group {
            fill: lin(0),
            children: vec![
                shape("Left", (180.0, 170.0, 280.0, 200.0), Fill::Group),
                shape("Right", (500.0, 170.0, 280.0, 200.0), Fill::Group),
            ],
        }],
        false,
    );
    // G40: theme references.
    add(
        "fillRef 2 (theme lin 270 scaled 1, phClr tint 50 to shade 50); fillRef 3 (lin 90 scaled 0, phClr to white)",
        vec![
            PItem::ThemeRef {
                frame: (180.0, 170.0, 280.0, 200.0),
                idx: 2,
                color: THEME_ACCENT,
            },
            PItem::ThemeRef {
                frame: (500.0, 170.0, 280.0, 200.0),
                idx: 3,
                color: THEME_ACCENT,
            },
        ],
        true,
    );
    // G41: a line and text.
    let red_blue = lin(0).with_stops(&[stop(0, RED), stop(WHOLE, BLUE)]);
    add(
        "gradient line (20 pt, lin 0 red to blue) and gradient text (lin 0 red to blue)",
        vec![
            PItem::Line {
                frame: (180.0, 120.0, 600.0, 0.0),
                width: 20.0,
                fill: red_blue.clone(),
            },
            PItem::Text {
                frame: (180.0, 200.0, 600.0, 160.0),
                text: "Gradient",
                size: 96,
                fill: red_blue,
            },
        ],
        false,
    );
    // G42: thin bars.
    add(
        "600 x 20 bars: lin 0 (top), lin 90 (bottom)",
        vec![
            PItem::Shape(shape(
                "Bar lin 0",
                (180.0, 200.0, 600.0, 20.0),
                Fill::Grad(lin(0)),
            )),
            PItem::Shape(shape(
                "Bar lin 90",
                (180.0, 320.0, 600.0, 20.0),
                Fill::Grad(lin(90)),
            )),
        ],
        true,
    );
    // G43: cost.
    let mut many = Vec::new();
    for row in 0..6 {
        for col in 0..8 {
            let ang = 45 * col;
            let grad = match row {
                0 => lin(ang),
                1 => lin(ang).with_stops(&[stop(0, RED), stop(50_000, GREEN), stop(WHOLE, BLUE)]),
                2 => path(PathKind::Circle, CENTRED),
                3 => path(PathKind::Rect, CENTRED),
                4 => lin(ang).with_stops(&[stop(0, BLACK), faded(WHOLE, BLACK, 20_000)]),
                _ => lin(ang).rws(false),
            };
            let frame = (
                170.0 + 80.0 * f64::from(col),
                50.0 + 80.0 * f64::from(row),
                60.0,
                60.0,
            );
            let name = format!("Small {}", row * 8 + col + 1);
            many.push(PItem::Shape(shape(&name, frame, Fill::Grad(grad))));
        }
    }
    add(
        "48 squares 60 x 60: lin 0..315; 3 stops; centred circle; centred rect; alpha; rotWithShape 0",
        many,
        true,
    );
    // Per-slide extras.
    for slide in &mut slides {
        match slide.id.as_str() {
            "G36" => {
                slide.background = Some(lin(90));
                slide.label = WHITE;
            }
            "G37" => slide.background = Some(path(PathKind::Circle, CENTRED)),
            "G38" => {
                slide.note = Some(
                    "LibreOffice 25.2.3.2 draws the circle cell as PowerPoint's centred circle, t = d / (half the cell \
                     diagonal): RGB rms 0.0034, max 0.0079 on its 144 dpi raster, as on G18's shape",
                );
            }
            "G40" => {
                slide.note = Some(
                    "both theme gradients are vertical, where scaled does not matter, so LibreOffice is valid for both",
                );
            }
            "G43" => {
                slide.note = Some(
                    "a centred rect path on a square is the same contours under PowerPoint's max rule and \
                     LibreOffice's equal insets",
                );
            }
            "G31" | "G32" | "G35" => slide.diagnostics = vec!["PX2106"],
            "G41" => slide.diagnostics = vec!["PX2105", "PX2211"],
            _ => {}
        }
    }
    slides
}

// ---------------------------------------------------------------------------------------------------------------------
// PowerPoint's analytic geometry, in fractions of the paint box

/// A rectangle in box fractions.
#[derive(Clone, Copy)]
struct Span {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Span {
    fn of(insets: Insets) -> Span {
        let f = |v: i32| f64::from(v) / f64::from(WHOLE);
        Span {
            x0: f(insets[0]),
            y0: f(insets[1]),
            x1: 1.0 - f(insets[2]),
            y1: 1.0 - f(insets[3]),
        }
    }

    fn w(self) -> f64 {
        self.x1 - self.x0
    }

    fn h(self) -> f64 {
        self.y1 - self.y0
    }

    fn centre(self) -> (f64, f64) {
        ((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0)
    }
}

/// Bowen's focus on one axis: the focus rectangle F seen from the tile T (Microsoft Q&A 2247174).
fn focus_axis(f0: f64, fd: f64, t0: f64, td: f64) -> f64 {
    if td - fd <= 1e-6 {
        f0 + fd / 2.0
    } else {
        f0 + fd * (f0 - t0) / (td - fd)
    }
}

/// PowerPoint's drawing of a gradient on a `w` × `h` paint box, in box fractions.
fn powerpoint_geometry(grad: &Grad, w: f64, h: f64) -> Json {
    let tile = Span::of(grad.tile.unwrap_or([0; 4]));
    match grad.shade {
        Shade::Lin { ang, scaled } => {
            let (c, s) = cos_sin(ang);
            // The gradient of t in box fractions.
            let (gx, gy) = if scaled {
                let l = c.abs() + s.abs();
                (c / tile.w() / l, s / tile.h() / l)
            } else {
                let l = tile.w() * w * c.abs() + tile.h() * h * s.abs();
                (w * c / l, h * s / l)
            };
            let g2 = gx * gx + gy * gy;
            let (cx, cy) = tile.centre();
            let from = (cx - gx / g2 / 2.0, cy - gy / g2 / 2.0);
            let to = (from.0 + gx / g2, from.1 + gy / g2);
            Json::obj()
                .with("kind", "linear")
                .with("from", jpoint(from.0, from.1))
                .with("to", jpoint(to.0, to.1))
        }
        Shade::Path { kind, fill_to } => {
            let focus = Span::of(fill_to);
            let (cx, cy) = tile.centre();
            if kind == PathKind::Circle {
                let (tw, th) = (tile.w() * w, tile.h() * h);
                let r = (tw * tw + th * th).sqrt() / 2.0;
                let (rx, ry) = (r / w, r / h);
                let fx = focus_axis(focus.x0, focus.w(), tile.x0, tile.w());
                let fy = focus_axis(focus.y0, focus.h(), tile.y0, tile.h());
                let (ux, uy) = ((fx - cx) / rx, (fy - cy) / ry);
                let inside = ux * ux + uy * uy < 1.0;
                Json::obj()
                    .with("kind", "radial")
                    .with("center", jpoint(cx, cy))
                    .with("radius", jnum(rx))
                    .with("radiusY", jnum(ry))
                    .with("focus", jpoint(fx, fy))
                    .with("focusInside", inside)
            } else {
                let unit = |v: f64, t0: f64, td: f64| -1.0 + 2.0 * (v - t0) / td;
                Json::obj()
                    .with("kind", "rect")
                    .with("origin", jpoint(cx, cy))
                    .with("axisX", jpoint(tile.w() / 2.0, 0.0))
                    .with("axisY", jpoint(0.0, tile.h() / 2.0))
                    .with(
                        "inner",
                        Json::obj()
                            .with("l", jnum(unit(focus.x0, tile.x0, tile.w())))
                            .with("t", jnum(unit(focus.y0, tile.y0, tile.h())))
                            .with("r", jnum(unit(focus.x1, tile.x0, tile.w())))
                            .with("b", jnum(unit(focus.y1, tile.y0, tile.h()))),
                    )
                    .with_opt(
                        "drawnAs",
                        (kind == PathKind::Shape).then_some(
                            "a rect path (PowerPoint's outline contours are unmeasured)",
                        ),
                    )
            }
        }
    }
}

impl PSlide {
    fn json(&self, index: usize) -> Json {
        let mut objects = Vec::new();
        if let Some(bg) = &self.background {
            objects.push(
                Json::obj()
                    .with("element", "background")
                    .with("box", jframe((0.0, 0.0, SLIDE_SIZE.0, SLIDE_SIZE.1)))
                    .with("fill", bg.json())
                    .with(
                        "powerpoint",
                        powerpoint_geometry(bg, SLIDE_SIZE.0, SLIDE_SIZE.1),
                    ),
            );
        }
        for item in &self.items {
            item_json(item, &mut objects);
        }
        Json::obj()
            .with("slide", index + 1)
            .with("id", self.id.as_str())
            .with("caption", self.caption.as_str())
            .with("row", format!("pp-{}", 256 + index))
            .with("loValid", self.lo_valid)
            .with_opt("note", self.note)
            .with("diagnostics", self.diagnostics.clone())
            .with("objects", Json::Arr(objects))
    }
}

fn shape_json(s: &PShape, fill: Json, powerpoint: Option<Json>) -> Json {
    Json::obj()
        .with("element", s.name.as_str())
        .with("geometry", s.geom.name())
        .with("box", jframe(s.frame))
        .with("rot", i64::from(s.rot))
        .with("flipH", s.flip_h)
        .with("flipV", s.flip_v)
        .with("fill", fill)
        .with_opt("powerpoint", powerpoint)
}

fn item_json(item: &PItem, out: &mut Vec<Json>) {
    match item {
        PItem::Shape(s) => {
            let (fill, pp) = match &s.fill {
                Fill::Grad(g) => (g.json(), Some(powerpoint_geometry(g, s.frame.2, s.frame.3))),
                Fill::Solid(c) => (
                    Json::obj().with("type", "solid").with("color", hex(*c)),
                    None,
                ),
                Fill::Group => (Json::obj().with("type", "grpFill"), None),
            };
            out.push(shape_json(s, fill, pp));
        }
        PItem::Group { fill, children } => {
            for child in children {
                out.push(shape_json(
                    child,
                    Json::obj()
                        .with("type", "grpFill")
                        .with("group", fill.json()),
                    None,
                ));
            }
        }
        PItem::Table { frame, cells } => {
            let (w, h) = (frame.2 / 2.0, frame.3 / 2.0);
            let cells: Vec<Json> = cells
                .iter()
                .enumerate()
                .map(|(k, cell)| {
                    let base = Json::obj()
                        .with("row", k / 2)
                        .with("col", k % 2)
                        .with("size", Json::Arr(vec![jnum(w), jnum(h)]));
                    match cell {
                        Some(g) => base
                            .with("fill", g.json())
                            .with("powerpoint", powerpoint_geometry(g, w, h)),
                        None => base.with("fill", Json::obj().with("type", "none")),
                    }
                })
                .collect();
            out.push(
                Json::obj()
                    .with("element", "Table")
                    .with("box", jframe(*frame))
                    .with("cells", Json::Arr(cells)),
            );
        }
        PItem::Line { frame, width, fill } => out.push(
            Json::obj()
                .with("element", "Gradient line")
                .with("box", jframe(*frame))
                .with("width", jnum(*width))
                .with("stroke", fill.json()),
        ),
        PItem::Text {
            frame,
            text,
            size,
            fill,
        } => out.push(
            Json::obj()
                .with("element", "Gradient text")
                .with("box", jframe(*frame))
                .with("text", *text)
                .with("size", *size)
                .with("runFill", fill.json()),
        ),
        PItem::ThemeRef { frame, idx, color } => out.push(
            Json::obj()
                .with("element", format!("fillRef {idx}"))
                .with("box", jframe(*frame))
                .with(
                    "fill",
                    Json::obj()
                        .with("type", "fillRef")
                        .with("idx", *idx)
                        .with("color", hex(*color)),
                ),
        ),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// PPTX writer

const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PPTX_DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const GROUP: &str = "<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>";

/// A five-pointed star in a 20000 × 20000 path box.
const STAR: [(i32, i32); 10] = [
    (10_000, 0),
    (12_245, 6_910),
    (19_511, 6_910),
    (13_633, 11_180),
    (15_878, 18_090),
    (10_000, 13_820),
    (4_122, 18_090),
    (6_367, 11_180),
    (489, 6_910),
    (7_755, 6_910),
];

/// A table cell's four borders, none drawn (LibreOffice draws thin black ones when a cell names none).
const NO_BORDERS: &str = "<a:lnL w=\"0\"><a:noFill/></a:lnL><a:lnR w=\"0\"><a:noFill/></a:lnR><a:lnT w=\"0\"><a:noFill/></a:lnT><a:lnB w=\"0\"><a:noFill/></a:lnB>";

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
        let _ = write!(
            out,
            "<Relationship Id=\"{id}\" Type=\"{REL}/{kind}\" Target=\"{target}\"/>"
        );
    }
    out.push_str("</Relationships>");
    out
}

fn xfrm((x, y, w, h): Frame, rot: i32, flip_h: bool, flip_v: bool) -> String {
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

fn run_props(size: u32, fill: &str, bold: bool) -> String {
    format!(
        "<a:rPr lang=\"en-US\" sz=\"{}\"{} dirty=\"0\">{fill}<a:latin typeface=\"Liberation Sans\"/></a:rPr>",
        size * 100,
        if bold { " b=\"1\"" } else { "" }
    )
}

fn star_geometry() -> String {
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
    match geom {
        Geom::Rect => "<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>".to_owned(),
        Geom::Ellipse => "<a:prstGeom prst=\"ellipse\"><a:avLst/></a:prstGeom>".to_owned(),
        Geom::Star => star_geometry(),
    }
}

struct Ids(u32);

impl Ids {
    fn next(&mut self) -> u32 {
        self.0 += 1;
        self.0
    }
}

fn pptx_shape(ids: &mut Ids, s: &PShape) -> String {
    let id = ids.next();
    let fill = match &s.fill {
        Fill::Grad(g) => g.xml(),
        Fill::Solid(c) => solid(*c),
        Fill::Group => "<a:grpFill/>".to_owned(),
    };
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr>{}{}{fill}<a:ln><a:noFill/></a:ln></p:spPr></p:sp>",
        escape(&s.name),
        xfrm(s.frame, s.rot, s.flip_h, s.flip_v),
        geometry(s.geom)
    )
}

fn pptx_item(ids: &mut Ids, item: &PItem) -> String {
    match item {
        PItem::Shape(s) => pptx_shape(ids, s),
        PItem::Group { fill, children } => {
            let id = ids.next();
            let (x0, y0) = children.iter().fold((f64::MAX, f64::MAX), |(x, y), c| {
                (x.min(c.frame.0), y.min(c.frame.1))
            });
            let (x1, y1) = children.iter().fold((f64::MIN, f64::MIN), |(x, y), c| {
                (x.max(c.frame.0 + c.frame.2), y.max(c.frame.1 + c.frame.3))
            });
            let (x, y, w, h) = (emu(x0), emu(y0), emu(x1 - x0), emu(y1 - y0));
            let body: String = children.iter().map(|c| pptx_shape(ids, c)).collect();
            format!(
                "<p:grpSp><p:nvGrpSpPr><p:cNvPr id=\"{id}\" name=\"Group\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{w}\" cy=\"{h}\"/><a:chOff x=\"{x}\" y=\"{y}\"/><a:chExt cx=\"{w}\" cy=\"{h}\"/></a:xfrm>{}</p:grpSpPr>{body}</p:grpSp>",
                fill.xml()
            )
        }
        PItem::Table { frame, cells } => {
            let id = ids.next();
            let cell = |c: &Cell| {
                let fill = c
                    .as_ref()
                    .map_or_else(|| "<a:noFill/>".to_owned(), Grad::xml);
                format!(
                    "<a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang=\"en-US\" dirty=\"0\"/></a:p></a:txBody><a:tcPr>{NO_BORDERS}{fill}</a:tcPr></a:tc>"
                )
            };
            let row = |a: &Cell, b: &Cell| {
                format!(
                    "<a:tr h=\"{}\">{}{}</a:tr>",
                    emu(frame.3 / 2.0),
                    cell(a),
                    cell(b)
                )
            };
            format!(
                "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{id}\" name=\"Table\"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp=\"1\"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></p:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\"><a:tbl><a:tblPr/><a:tblGrid><a:gridCol w=\"{}\"/><a:gridCol w=\"{}\"/></a:tblGrid>{}{}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>",
                emu(frame.0),
                emu(frame.1),
                emu(frame.2),
                emu(frame.3),
                emu(frame.2 / 2.0),
                emu(frame.2 / 2.0),
                row(&cells[0], &cells[1]),
                row(&cells[2], &cells[3])
            )
        }
        PItem::Line { frame, width, fill } => {
            let id = ids.next();
            format!(
                "<p:cxnSp><p:nvCxnSpPr><p:cNvPr id=\"{id}\" name=\"Gradient line\"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr><p:spPr>{}<a:prstGeom prst=\"line\"><a:avLst/></a:prstGeom><a:ln w=\"{}\">{}</a:ln></p:spPr></p:cxnSp>",
                xfrm(*frame, 0, false, false),
                emu(*width),
                fill.xml()
            )
        }
        PItem::Text {
            frame,
            text,
            size,
            fill,
        } => {
            let id = ids.next();
            format!(
                "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Gradient text\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/><a:ln><a:noFill/></a:ln></p:spPr><p:txBody><a:bodyPr wrap=\"none\" lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\" anchor=\"t\" rtlCol=\"0\"><a:noAutofit/></a:bodyPr><a:lstStyle/><a:p><a:r>{}<a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>",
                xfrm(*frame, 0, false, false),
                run_props(*size, &fill.xml(), true),
                escape(text)
            )
        }
        PItem::ThemeRef { frame, idx, color } => {
            let id = ids.next();
            format!(
                "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"fillRef {idx}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:ln><a:noFill/></a:ln></p:spPr><p:style><a:lnRef idx=\"0\"><a:srgbClr val=\"000000\"/></a:lnRef><a:fillRef idx=\"{idx}\"><a:srgbClr val=\"{}\"/></a:fillRef><a:effectRef idx=\"0\"><a:srgbClr val=\"000000\"/></a:effectRef><a:fontRef idx=\"minor\"><a:srgbClr val=\"000000\"/></a:fontRef></p:style></p:sp>",
                xfrm(*frame, 0, false, false),
                hex(*color)
            )
        }
    }
}

fn pptx_label(ids: &mut Ids, text: &str, color: Rgb) -> String {
    let id = ids.next();
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Label\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/><a:ln><a:noFill/></a:ln></p:spPr><p:txBody><a:bodyPr wrap=\"none\" lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\" anchor=\"t\" rtlCol=\"0\"><a:noAutofit/></a:bodyPr><a:lstStyle/><a:p><a:r>{}<a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>",
        xfrm((24.0, 12.0, 912.0, 20.0), 0, false, false),
        run_props(12, &solid(color), false),
        escape(text)
    )
}

fn pptx_package(slides: &[PSlide]) -> Vec<Entry> {
    let ct = "application/vnd.openxmlformats-officedocument.presentationml";
    let mut types = format!(
        "{PPTX_DECL}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/ppt/presentation.xml\" ContentType=\"{ct}.presentation.main+xml\"/><Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"{ct}.slideMaster+xml\"/><Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"{ct}.slideLayout+xml\"/><Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>"
    );
    for n in 1..=slides.len() {
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
    for n in 0..slides.len() {
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
                "{PPTX_DECL}<p:sldMaster xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\"><p:cSld><p:bg><p:bgPr>{}<a:effectLst/></p:bgPr></p:bg><p:spTree>{GROUP}</p:spTree></p:cSld><p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/><p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst></p:sldMaster>",
                solid(SLIDE)
            ),
        ),
        xml_entry(
            "ppt/slideMasters/_rels/slideMaster1.xml.rels",
            rels(&[
                (
                    "rId1".to_owned(),
                    "slideLayout",
                    "../slideLayouts/slideLayout1.xml".to_owned(),
                ),
                ("rId2".to_owned(), "theme", "../theme/theme1.xml".to_owned()),
            ]),
        ),
        xml_entry(
            "ppt/slideLayouts/slideLayout1.xml",
            format!(
                "{PPTX_DECL}<p:sldLayout xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\" type=\"blank\" preserve=\"1\"><p:cSld name=\"Blank\"><p:spTree>{GROUP}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
            ),
        ),
        xml_entry(
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            rels(&[(
                "rId1".to_owned(),
                "slideMaster",
                "../slideMasters/slideMaster1.xml".to_owned(),
            )]),
        ),
        xml_entry("ppt/theme/theme1.xml", theme()),
    ];
    for (n, slide) in slides.iter().enumerate() {
        let mut ids = Ids(1);
        let mut tree = String::new();
        for item in &slide.items {
            tree.push_str(&pptx_item(&mut ids, item));
        }
        tree.push_str(&pptx_label(&mut ids, &slide.caption, slide.label));
        let background = slide.background.as_ref().map_or_else(String::new, |g| {
            format!("<p:bg><p:bgPr>{}<a:effectLst/></p:bgPr></p:bg>", g.xml())
        });
        entries.push(xml_entry(
            &format!("ppt/slides/slide{}.xml", n + 1),
            format!(
                "{PPTX_DECL}<p:sld xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\"><p:cSld>{background}<p:spTree>{GROUP}{tree}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
            ),
        ));
        entries.push(xml_entry(
            &format!("ppt/slides/_rels/slide{}.xml.rels", n + 1),
            rels(&[(
                "rId1".to_owned(),
                "slideLayout",
                "../slideLayouts/slideLayout1.xml".to_owned(),
            )]),
        ));
    }
    entries
}

/// The theme: fill styles 2 and 3 are vertical gradients of `phClr` (scaled 1 and 0), as Office themes write them.
fn theme() -> String {
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
    let scaled = "<a:gradFill rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:schemeClr val=\"phClr\"><a:tint val=\"50000\"/></a:schemeClr></a:gs><a:gs pos=\"100000\"><a:schemeClr val=\"phClr\"><a:shade val=\"50000\"/></a:schemeClr></a:gs></a:gsLst><a:lin ang=\"16200000\" scaled=\"1\"/></a:gradFill>";
    let unscaled = "<a:gradFill rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:schemeClr val=\"phClr\"/></a:gs><a:gs pos=\"100000\"><a:srgbClr val=\"FFFFFF\"/></a:gs></a:gsLst><a:lin ang=\"5400000\" scaled=\"0\"/></a:gradFill>";
    let line = "<a:ln w=\"9525\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln>";
    let effect = "<a:effectStyle><a:effectLst/></a:effectStyle>";
    format!(
        "{PPTX_DECL}<a:theme xmlns:a=\"{NS_A}\" name=\"Gradient probe\"><a:themeElements><a:clrScheme name=\"Gradient probe\">{colors}</a:clrScheme><a:fontScheme name=\"Gradient probe\"><a:majorFont>{font}</a:majorFont><a:minorFont>{font}</a:minorFont></a:fontScheme><a:fmtScheme name=\"Gradient probe\"><a:fillStyleLst>{fill}{scaled}{unscaled}</a:fillStyleLst><a:lnStyleLst>{line}{line}{line}</a:lnStyleLst><a:effectStyleLst>{effect}{effect}{effect}</a:effectStyleLst><a:bgFillStyleLst>{fill}{fill}{fill}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"
    )
}

// ---------------------------------------------------------------------------------------------------------------------
// The ODP model

/// A `draw:gradient`: every attribute optional, so the defaults can be probed.
#[derive(Clone, Default)]
struct OGrad {
    style: &'static str,
    angle: Option<&'static str>,
    border: Option<u32>,
    cx: Option<u32>,
    cy: Option<u32>,
    start: Option<Rgb>,
    end: Option<Rgb>,
    start_intensity: Option<u32>,
    end_intensity: Option<u32>,
    /// `loext:gradient-stop` (offset, colour).
    stops: Vec<(&'static str, Rgb)>,
}

/// A gradient of `style` from black to white with no border, at the angle (linear, axial, ellipsoid, square,
/// rectangular) or the centre (radial and the others) of the case.
fn ograd(style: &'static str) -> OGrad {
    let centred = style != "linear" && style != "axial";
    OGrad {
        style,
        angle: (style != "radial").then_some("0deg"),
        border: Some(0),
        cx: centred.then_some(50),
        cy: centred.then_some(50),
        start: Some(BLACK),
        end: Some(WHITE),
        ..OGrad::default()
    }
}

impl OGrad {
    fn angle(mut self, angle: &'static str) -> OGrad {
        self.angle = Some(angle);
        self
    }

    fn border(mut self, border: u32) -> OGrad {
        self.border = Some(border);
        self
    }

    fn centre(mut self, cx: u32, cy: u32) -> OGrad {
        self.cx = Some(cx);
        self.cy = Some(cy);
        self
    }

    fn colours(mut self, start: Rgb, end: Rgb) -> OGrad {
        self.start = Some(start);
        self.end = Some(end);
        self
    }

    fn stops(mut self, stops: &[(&'static str, Rgb)]) -> OGrad {
        self.stops = stops.to_vec();
        self
    }

    fn xml(&self, name: &str) -> String {
        let mut out = format!(
            "<draw:gradient draw:name=\"{name}\" draw:display-name=\"{name}\" draw:style=\"{}\"",
            self.style
        );
        if let Some(cx) = self.cx {
            let _ = write!(out, " draw:cx=\"{cx}%\"");
        }
        if let Some(cy) = self.cy {
            let _ = write!(out, " draw:cy=\"{cy}%\"");
        }
        if let Some(c) = self.start {
            let _ = write!(out, " draw:start-color=\"{}\"", odf_color(c));
        }
        if let Some(c) = self.end {
            let _ = write!(out, " draw:end-color=\"{}\"", odf_color(c));
        }
        if let Some(i) = self.start_intensity {
            let _ = write!(out, " draw:start-intensity=\"{i}%\"");
        }
        if let Some(i) = self.end_intensity {
            let _ = write!(out, " draw:end-intensity=\"{i}%\"");
        }
        if let Some(a) = self.angle {
            let _ = write!(out, " draw:angle=\"{a}\"");
        }
        if let Some(b) = self.border {
            let _ = write!(out, " draw:border=\"{b}%\"");
        }
        if self.stops.is_empty() {
            out.push_str("/>");
        } else {
            out.push('>');
            for (offset, color) in &self.stops {
                let _ = write!(
                    out,
                    "<loext:gradient-stop svg:offset=\"{offset}\" loext:color-type=\"rgb\" loext:color-value=\"{}\"/>",
                    odf_color(*color)
                );
            }
            out.push_str("</draw:gradient>");
        }
        out
    }

    fn json(&self) -> Json {
        let pct = |v: Option<u32>| v.map(|v| format!("{v}%"));
        Json::obj()
            .with("style", self.style)
            .with_opt("angle", self.angle)
            .with_opt("border", pct(self.border))
            .with_opt("cx", pct(self.cx))
            .with_opt("cy", pct(self.cy))
            .with_opt("startColor", self.start.map(odf_color))
            .with_opt("endColor", self.end.map(odf_color))
            .with_opt("startIntensity", pct(self.start_intensity))
            .with_opt("endIntensity", pct(self.end_intensity))
            .with_opt(
                "stops",
                (!self.stops.is_empty()).then(|| {
                    Json::Arr(
                        self.stops
                            .iter()
                            .map(|(o, c)| {
                                Json::Arr(vec![Json::Num((*o).to_owned()), odf_color(*c).into()])
                            })
                            .collect(),
                    )
                }),
            )
    }
}

/// A `draw:opacity` (transparency gradient): opacities in percent.
#[derive(Clone)]
struct OOpacity {
    style: &'static str,
    angle: Option<&'static str>,
    border: u32,
    centre: Option<(u32, u32)>,
    start: u32,
    end: u32,
}

impl OOpacity {
    fn xml(&self, name: &str) -> String {
        let mut out = format!(
            "<draw:opacity draw:name=\"{name}\" draw:display-name=\"{name}\" draw:style=\"{}\"",
            self.style
        );
        if let Some((cx, cy)) = self.centre {
            let _ = write!(out, " draw:cx=\"{cx}%\" draw:cy=\"{cy}%\"");
        }
        let _ = write!(
            out,
            " draw:start=\"{}%\" draw:end=\"{}%\"",
            self.start, self.end
        );
        if let Some(a) = self.angle {
            let _ = write!(out, " draw:angle=\"{a}\"");
        }
        let _ = write!(out, " draw:border=\"{}%\"/>", self.border);
        out
    }

    fn json(&self) -> Json {
        Json::obj()
            .with("style", self.style)
            .with_opt("angle", self.angle)
            .with("border", format!("{}%", self.border))
            .with_opt("cx", self.centre.map(|c| format!("{}%", c.0)))
            .with_opt("cy", self.centre.map(|c| format!("{}%", c.1)))
            .with("start", format!("{}%", self.start))
            .with("end", format!("{}%", self.end))
    }
}

/// A shape's area fill: a gradient or a solid colour, with its step count and transparency.
#[derive(Clone)]
struct OFill {
    grad: Option<OGrad>,
    solid: Rgb,
    steps: Option<u32>,
    opacity_grad: Option<OOpacity>,
    opacity: Option<u32>,
}

fn ofill(grad: OGrad) -> OFill {
    OFill {
        grad: Some(grad),
        solid: BLACK,
        steps: None,
        opacity_grad: None,
        opacity: None,
    }
}

impl OFill {
    fn steps(mut self, steps: u32) -> OFill {
        self.steps = Some(steps);
        self
    }

    fn transparency(mut self, opacity: OOpacity) -> OFill {
        self.opacity_grad = Some(opacity);
        self
    }

    fn json(&self) -> Json {
        let base = match &self.grad {
            Some(g) => Json::obj()
                .with("type", "gradient")
                .with("gradient", g.json()),
            None => Json::obj()
                .with("type", "solid")
                .with("color", odf_color(self.solid)),
        };
        base.with_opt("stepCount", self.steps)
            .with_opt("opacity", self.opacity.map(|o| format!("{o}%")))
            .with_opt(
                "transparency",
                self.opacity_grad.as_ref().map(OOpacity::json),
            )
    }
}

#[derive(Clone, Copy)]
enum OShape {
    Rect,
    Ellipse,
    /// A `draw:custom-shape` rectangle, mirrored as asked.
    Custom {
        mirror_h: bool,
        mirror_v: bool,
    },
    /// A `draw:path` drawing the top-left quarter of its frame (half its width and half its height). The path starts
    /// at the view box's origin: LibreOffice 25.2 moves a path whose bounds start elsewhere to its frame's corner.
    QuarterPath,
}

impl OShape {
    fn name(self) -> &'static str {
        match self {
            OShape::Rect => "draw:rect",
            OShape::Ellipse => "draw:ellipse",
            OShape::Custom { .. } => "draw:custom-shape rectangle",
            OShape::QuarterPath => "draw:path over the top-left quarter of its frame",
        }
    }
}

#[derive(Clone)]
struct OItem {
    name: &'static str,
    frame: Frame,
    /// Degrees anticlockwise on the page (ODF's `rotate`).
    rot: i32,
    shape: OShape,
    fill: OFill,
}

fn oitem(fill: OFill) -> OItem {
    OItem {
        name: "Gradient box",
        frame: BOX,
        rot: 0,
        shape: OShape::Rect,
        fill,
    }
}

impl OItem {
    fn turned(mut self, rot: i32) -> OItem {
        self.rot = rot;
        self
    }

    fn shape(mut self, shape: OShape) -> OItem {
        self.shape = shape;
        self
    }

    fn at(mut self, frame: Frame) -> OItem {
        self.frame = frame;
        self
    }

    fn named(mut self, name: &'static str) -> OItem {
        self.name = name;
        self
    }

    fn json(&self) -> Json {
        Json::obj()
            .with("element", self.name)
            .with("shape", self.shape.name())
            .with("box", jframe(self.frame))
            .with("rotCcw", i64::from(self.rot))
            .with("fill", self.fill.json())
    }
}

enum OKind {
    Item(OItem),
    Group(Vec<OItem>),
    /// A 2 × 2 table over the frame, rows first; a cell with no gradient has no fill.
    Table {
        frame: Frame,
        cells: Box<[Option<OGrad>; 4]>,
    },
}

struct OSlide {
    id: String,
    caption: String,
    background: Option<OFill>,
    label: Rgb,
    items: Vec<OKind>,
    diagnostics: Vec<&'static str>,
}

impl OSlide {
    fn json(&self, index: usize) -> Json {
        let mut objects = Vec::new();
        if let Some(bg) = &self.background {
            objects.push(
                Json::obj()
                    .with("element", "background")
                    .with("box", jframe((0.0, 0.0, SLIDE_SIZE.0, SLIDE_SIZE.1)))
                    .with("fill", bg.json()),
            );
        }
        for item in &self.items {
            match item {
                OKind::Item(i) => objects.push(i.json()),
                OKind::Group(children) => {
                    objects.extend(children.iter().map(|c| c.json().with("group", true)));
                }
                OKind::Table { frame, cells } => {
                    let cells: Vec<Json> = cells
                        .iter()
                        .enumerate()
                        .map(|(k, c)| {
                            let base = Json::obj().with("row", k / 2).with("col", k % 2);
                            match c {
                                Some(g) => base.with("fill", ofill(g.clone()).json()),
                                None => base.with("fill", Json::obj().with("type", "none")),
                            }
                        })
                        .collect();
                    objects.push(
                        Json::obj()
                            .with("element", "Table")
                            .with("box", jframe(*frame))
                            .with("cells", Json::Arr(cells)),
                    );
                }
            }
        }
        Json::obj()
            .with("slide", index + 1)
            .with("id", self.id.as_str())
            .with("caption", self.caption.as_str())
            .with("row", format!("od-p{}", index + 1))
            .with("loValid", true)
            .with("diagnostics", self.diagnostics.clone())
            .with("objects", Json::Arr(objects))
    }
}

const RGB3: [(&str, Rgb); 3] = [("0", RED), ("0.3", GREEN), ("1", BLUE)];

fn odp_slides() -> Vec<OSlide> {
    let mut slides: Vec<OSlide> = Vec::new();
    let mut add = |caption: &str, items: Vec<OKind>| {
        let id = format!("O{:02}", slides.len() + 1);
        slides.push(OSlide {
            caption: format!("{id} {caption}"),
            id,
            background: None,
            label: LABEL,
            items,
            diagnostics: Vec::new(),
        });
    };
    let one = |item: OItem| vec![OKind::Item(item)];
    let g = |grad: OGrad| one(oitem(ofill(grad)));
    // O01–O05: linear.
    add("linear 0deg, border 0%", g(ograd("linear")));
    add("linear 30deg", g(ograd("linear").angle("30deg")));
    add("linear 0deg, border 25%", g(ograd("linear").border(25)));
    add(
        "linear 0deg, step count 4",
        one(oitem(ofill(ograd("linear")).steps(4))),
    );
    add(
        "linear 0deg red to blue, start intensity 50%, end intensity 80%",
        g(OGrad {
            start_intensity: Some(50),
            end_intensity: Some(80),
            ..ograd("linear").colours(RED, BLUE)
        }),
    );
    // O06–O08: turned, mirrored, flipped.
    add(
        "linear 30deg on a rect turned 30 anticlockwise",
        one(oitem(ofill(ograd("linear").angle("30deg"))).turned(30)),
    );
    add(
        "linear 30deg on a custom shape mirrored horizontally",
        one(
            oitem(ofill(ograd("linear").angle("30deg"))).shape(OShape::Custom {
                mirror_h: true,
                mirror_v: false,
            }),
        ),
    );
    add(
        "linear 0deg on a custom shape mirrored vertically",
        one(oitem(ofill(ograd("linear"))).shape(OShape::Custom {
            mirror_h: false,
            mirror_v: true,
        })),
    );
    // O09–O11: axial.
    add("axial 0deg", g(ograd("axial")));
    add(
        "axial 30deg, border 20%",
        g(ograd("axial").angle("30deg").border(20)),
    );
    add(
        "axial 0deg, step count 5",
        one(oitem(ofill(ograd("axial")).steps(5))),
    );
    // O12–O16: radial.
    add(
        "radial cx 50% cy 50% on 400 x 200",
        one(oitem(ofill(ograd("radial"))).at((280.0, 170.0, 400.0, 200.0))),
    );
    add("radial cx 25% cy 75%", g(ograd("radial").centre(25, 75)));
    add("radial border 30%", g(ograd("radial").border(30)));
    add(
        "radial cx 25% cy 25% on a rect turned 30 anticlockwise",
        one(oitem(ofill(ograd("radial").centre(25, 25))).turned(30)),
    );
    add(
        "radial, step count 3",
        one(oitem(ofill(ograd("radial")).steps(3))),
    );
    // O17–O19: ellipsoid.
    add("ellipsoid 0deg", g(ograd("ellipsoid")));
    add("ellipsoid 30deg", g(ograd("ellipsoid").angle("30deg")));
    add(
        "ellipsoid 0deg, border 20%, cx 25% cy 75%",
        g(ograd("ellipsoid").border(20).centre(25, 75)),
    );
    // O20–O21: square.
    add("square 30deg", g(ograd("square").angle("30deg")));
    add(
        "square 0deg on a custom shape turned 30 anticlockwise",
        one(oitem(ofill(ograd("square")))
            .shape(OShape::Custom {
                mirror_h: false,
                mirror_v: false,
            })
            .turned(30)),
    );
    // O22–O23: rectangular.
    add("rectangular 0deg", g(ograd("rectangular")));
    add(
        "rectangular 30deg, border 20%",
        g(ograd("rectangular").angle("30deg").border(20)),
    );
    // O24–O27: multi-colour stops.
    add(
        "linear 0deg, loext stops red 0, green 0.3, blue 1",
        g(ograd("linear").colours(RED, BLUE).stops(&RGB3)),
    );
    add(
        "axial 0deg, loext stops red 0, green 0.3, blue 1",
        g(ograd("axial").colours(RED, BLUE).stops(&RGB3)),
    );
    add(
        "linear 0deg, start/end green, loext stops red 0, blue 1",
        g(ograd("linear")
            .colours(GREEN, GREEN)
            .stops(&[("0", RED), ("1", BLUE)])),
    );
    add(
        "linear 0deg, loext stops red 0, green 0.5, blue 1, step count 2",
        one(oitem(
            ofill(ograd("linear").colours(RED, BLUE).stops(&[
                ("0", RED),
                ("0.5", GREEN),
                ("1", BLUE),
            ]))
            .steps(2),
        )),
    );
    // O28–O30: defaults and units.
    add(
        "radial without cx, cy and border (cx cy 0%)",
        g(OGrad {
            cx: None,
            cy: None,
            border: None,
            ..ograd("radial")
        }),
    );
    add(
        "linear without colours, angle and border (black)",
        g(OGrad {
            style: "linear",
            ..OGrad::default()
        }),
    );
    add(
        "linear, unitless angle 300 in ODF 1.3 (300 degrees)",
        g(ograd("linear").angle("300")),
    );
    // O31–O35: transparency gradients.
    let linear_alpha = |start, end| OOpacity {
        style: "linear",
        angle: Some("0deg"),
        border: 0,
        centre: None,
        start,
        end,
    };
    let radial_alpha = OOpacity {
        style: "radial",
        angle: None,
        border: 20,
        centre: Some((25, 25)),
        start: 0,
        end: 100,
    };
    let solid_black = OFill {
        grad: None,
        solid: BLACK,
        steps: None,
        opacity_grad: None,
        opacity: None,
    };
    add(
        "solid black, transparency linear 0deg, opacity 100% to 0%",
        one(oitem(
            solid_black.clone().transparency(linear_alpha(100, 0)),
        )),
    );
    add(
        "solid black, transparency radial cx 25% cy 25%, border 20%, opacity 0% to 100%",
        one(oitem(solid_black.transparency(radial_alpha.clone()))),
    );
    add(
        "linear 90deg, transparency linear 90deg, opacity 100% to 20% (same geometry)",
        one(oitem(ofill(ograd("linear").angle("90deg")).transparency(
            OOpacity {
                angle: Some("90deg"),
                ..linear_alpha(100, 20)
            },
        ))),
    );
    add(
        "linear 90deg, transparency radial cx 25% cy 25%, border 20% (own geometry)",
        one(oitem(
            ofill(ograd("linear").angle("90deg")).transparency(radial_alpha),
        )),
    );
    add(
        "linear 90deg, opacity 50% and transparency linear 0deg 100% to 0%",
        one(oitem(OFill {
            opacity: Some(50),
            ..ofill(ograd("linear").angle("90deg")).transparency(linear_alpha(100, 0))
        })),
    );
    // O36–O37: backgrounds.
    add("background radial cx 25% cy 50%", Vec::new());
    add("background linear 0deg, step count 4", Vec::new());
    // O38: a table.
    add(
        "2 x 2 table: linear 30deg; radial; no fill; no fill",
        vec![OKind::Table {
            frame: BOX,
            cells: Box::new([
                Some(ograd("linear").angle("30deg")),
                Some(ograd("radial")),
                None,
                None,
            ]),
        }],
    );
    // O39–O41: outlines.
    add(
        "radial cx 25% cy 25% on an ellipse turned 30 anticlockwise",
        one(oitem(ofill(ograd("radial").centre(25, 25)))
            .shape(OShape::Ellipse)
            .turned(30)),
    );
    add(
        "linear 0deg on a draw:path over the top-left quarter of its 600 x 200 frame",
        one(oitem(ofill(ograd("linear"))).shape(OShape::QuarterPath)),
    );
    add(
        "group of two rects, each linear 0deg",
        vec![OKind::Group(vec![
            oitem(ofill(ograd("linear")))
                .at((180.0, 170.0, 280.0, 200.0))
                .named("Left"),
            oitem(ofill(ograd("linear")))
                .at((500.0, 170.0, 280.0, 200.0))
                .named("Right"),
        ])],
    );
    for slide in &mut slides {
        match slide.id.as_str() {
            "O34" => slide.diagnostics = vec!["OD9103"],
            "O36" => slide.background = Some(ofill(ograd("radial").centre(25, 50))),
            "O37" => {
                slide.background = Some(ofill(ograd("linear")).steps(4));
                slide.label = WHITE;
            }
            _ => {}
        }
    }
    slides
}

fn odp12_slides() -> Vec<OSlide> {
    let slide = |id: &str, caption: &str, grad: OGrad| OSlide {
        id: id.to_owned(),
        caption: format!("{id} {caption}"),
        background: None,
        label: LABEL,
        items: vec![OKind::Item(oitem(ofill(grad)))],
        diagnostics: Vec::new(),
    };
    vec![
        slide(
            "N01",
            "linear, unitless angle 300 in ODF 1.2 by LibreOfficeDev 6 (30 degrees)",
            ograd("linear").angle("300"),
        ),
        slide(
            "N02",
            "radial without cx, cy (cx cy 0%), ODF 1.2",
            OGrad {
                cx: None,
                cy: None,
                ..ograd("radial")
            },
        ),
    ]
}

// ---------------------------------------------------------------------------------------------------------------------
// ODP writer

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

/// The paragraphs of the captions: font-independent line spacing, as LibreOffice reads every PPTX paragraph.
const PARAGRAPH: &str = "<style:style style:name=\"Pfi\" style:family=\"paragraph\"><style:paragraph-properties fo:margin-top=\"0pt\" fo:margin-bottom=\"0pt\" fo:line-height=\"100%\" style:font-independent-line-spacing=\"true\"/></style:style>";

#[derive(Clone, Copy, PartialEq)]
enum OdfVersion {
    V13,
    /// ODF 1.2 written by LibreOfficeDev 6.0.5.2, as Google Slides' ODP export says (tdf#89475 reads its unitless
    /// angles in tenths of a degree).
    V12OldGenerator,
}

impl OdfVersion {
    fn number(self) -> &'static str {
        match self {
            OdfVersion::V13 => "1.3",
            OdfVersion::V12OldGenerator => "1.2",
        }
    }
}

/// The styles a deck writes as it goes: content.xml's automatic styles and styles.xml's named gradients.
#[derive(Default)]
struct OdpStyles {
    automatic: String,
    named: String,
    count: usize,
}

impl OdpStyles {
    fn next(&mut self, prefix: &str) -> String {
        self.count += 1;
        format!("{prefix}{}", self.count)
    }

    fn graphic(&mut self, props: &str) -> String {
        let name = self.next("gr");
        let _ = write!(
            self.automatic,
            "<style:style style:name=\"{name}\" style:family=\"graphic\"><style:graphic-properties {props}/></style:style>"
        );
        name
    }

    /// The fill attributes of `fill`, writing its named gradient and transparency as `name`.
    fn fill(&mut self, fill: &OFill, name: &str) -> String {
        let mut out = match &fill.grad {
            Some(grad) => {
                self.named.push_str(&grad.xml(name));
                format!("draw:fill=\"gradient\" draw:fill-gradient-name=\"{name}\"")
            }
            None => format!(
                "draw:fill=\"solid\" draw:fill-color=\"{}\"",
                odf_color(fill.solid)
            ),
        };
        if let Some(steps) = fill.steps {
            let _ = write!(out, " draw:gradient-step-count=\"{steps}\"");
        }
        if let Some(opacity) = fill.opacity {
            let _ = write!(out, " draw:opacity=\"{opacity}%\"");
        }
        if let Some(alpha) = &fill.opacity_grad {
            let alpha_name = format!("{name}t");
            self.named.push_str(&alpha.xml(&alpha_name));
            let _ = write!(out, " draw:opacity-name=\"{alpha_name}\"");
        }
        out
    }
}

/// The placement of a box turned `rot` degrees anticlockwise about its centre: `svg:x`/`svg:y`, or LibreOffice's
/// `draw:transform` (turn the box about its origin, then move that corner).
fn placement((x, y, w, h): Frame, rot: i32) -> String {
    let size = format!("svg:width=\"{}pt\" svg:height=\"{}pt\"", num(w), num(h));
    if rot == 0 {
        return format!("svg:x=\"{}pt\" svg:y=\"{}pt\" {size}", num(x), num(y));
    }
    // ODF's rotate(a) on the page (y down): x' = cos·x + sin·y, y' = −sin·x + cos·y.
    let (cos, sin) = cos_sin(rot);
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let turned_centre = (
        cos * w / 2.0 + sin * h / 2.0,
        -sin * w / 2.0 + cos * h / 2.0,
    );
    let radians = f64::from(rot) * std::f64::consts::PI / 180.0;
    let text = format!("{radians:.12}");
    format!(
        "{size} draw:transform=\"rotate ({}) translate ({}pt {}pt)\"",
        text.trim_end_matches('0').trim_end_matches('.'),
        num(cx - turned_centre.0),
        num(cy - turned_centre.1)
    )
}

fn odp_label(styles: &mut OdpStyles, text: &str, color: Rgb) -> String {
    let style = styles.graphic(
        "draw:fill=\"none\" draw:stroke=\"none\" draw:shadow=\"hidden\" draw:textarea-vertical-align=\"top\" draw:textarea-horizontal-align=\"left\" draw:auto-grow-height=\"false\" draw:auto-grow-width=\"false\" fo:wrap-option=\"no-wrap\" fo:padding-left=\"0pt\" fo:padding-right=\"0pt\" fo:padding-top=\"0pt\" fo:padding-bottom=\"0pt\"",
    );
    let span = styles.next("T");
    let _ = write!(
        styles.automatic,
        "<style:style style:name=\"{span}\" style:family=\"text\"><style:text-properties fo:font-family=\"'Liberation Sans'\" style:font-family-generic=\"swiss\" fo:font-size=\"12pt\" fo:color=\"{}\" style:letter-kerning=\"true\"/></style:style>",
        odf_color(color)
    );
    format!(
        "<draw:custom-shape draw:style-name=\"{style}\" draw:text-style-name=\"Pfi\" draw:name=\"Label\" {}><text:p text:style-name=\"Pfi\"><text:span text:style-name=\"{span}\">{}</text:span></text:p><draw:enhanced-geometry svg:viewBox=\"0 0 21600 21600\" draw:type=\"rectangle\" draw:enhanced-path=\"M 0 0 L 21600 0 21600 21600 0 21600 0 0 Z N\"/></draw:custom-shape>",
        placement((24.0, 12.0, 912.0, 20.0), 0),
        escape(text)
    )
}

fn odp_item(styles: &mut OdpStyles, item: &OItem, gradient_name: &str) -> String {
    let fill = styles.fill(&item.fill, gradient_name);
    let style = styles.graphic(&format!(
        "{fill} draw:stroke=\"none\" draw:shadow=\"hidden\""
    ));
    let at = placement(item.frame, item.rot);
    let name = escape(item.name);
    match item.shape {
        OShape::Rect => {
            format!("<draw:rect draw:style-name=\"{style}\" draw:name=\"{name}\" {at}/>")
        }
        OShape::Ellipse => {
            format!("<draw:ellipse draw:style-name=\"{style}\" draw:name=\"{name}\" {at}/>")
        }
        OShape::Custom { mirror_h, mirror_v } => {
            let mut mirror = String::new();
            if mirror_h {
                mirror.push_str(" draw:mirror-horizontal=\"true\"");
            }
            if mirror_v {
                mirror.push_str(" draw:mirror-vertical=\"true\"");
            }
            format!(
                "<draw:custom-shape draw:style-name=\"{style}\" draw:name=\"{name}\" {at}><draw:enhanced-geometry svg:viewBox=\"0 0 21600 21600\" draw:type=\"rectangle\"{mirror} draw:enhanced-path=\"M 0 0 L 21600 0 21600 21600 0 21600 0 0 Z N\"/></draw:custom-shape>"
            )
        }
        OShape::QuarterPath => {
            let (_, _, w, h) = item.frame;
            let (x1, y1) = (num(w / 2.0), num(h / 2.0));
            format!(
                "<draw:path draw:style-name=\"{style}\" draw:name=\"{name}\" {at} svg:viewBox=\"0 0 {} {}\" svg:d=\"M0 0L{x1} 0 {x1} {y1} 0 {y1}Z\"/>",
                num(w),
                num(h)
            )
        }
    }
}

fn odp_table(styles: &mut OdpStyles, id: &str, frame: Frame, cells: &[Option<OGrad>; 4]) -> String {
    let frame_style =
        styles.graphic("draw:fill=\"none\" draw:stroke=\"none\" draw:shadow=\"hidden\"");
    let column = styles.next("co");
    let row = styles.next("ro");
    let _ = write!(
        styles.automatic,
        "<style:style style:name=\"{column}\" style:family=\"table-column\"><style:table-column-properties style:column-width=\"{}pt\"/></style:style><style:style style:name=\"{row}\" style:family=\"table-row\"><style:table-row-properties style:row-height=\"{}pt\"/></style:style>",
        num(frame.2 / 2.0),
        num(frame.3 / 2.0)
    );
    let mut xml = Vec::new();
    for (k, cell) in cells.iter().enumerate() {
        let style = styles.next("ce");
        let props = match cell {
            Some(grad) => {
                let name = format!("{id}{}", char::from(b'a' + u8::try_from(k).unwrap_or(0)));
                styles.fill(&ofill(grad.clone()), &name)
            }
            None => "draw:fill=\"none\"".to_owned(),
        };
        let _ = write!(
            styles.automatic,
            "<style:style style:name=\"{style}\" style:family=\"table-cell\"><loext:graphic-properties {props}/></style:style>"
        );
        xml.push(format!(
            "<table:table-cell table:style-name=\"{style}\"><text:p/></table:table-cell>"
        ));
    }
    format!(
        "<draw:frame draw:style-name=\"{frame_style}\" draw:name=\"Table\" {}><table:table><table:table-column table:style-name=\"{column}\" table:number-columns-repeated=\"2\"/><table:table-row table:style-name=\"{row}\">{}{}</table:table-row><table:table-row table:style-name=\"{row}\">{}{}</table:table-row></table:table></draw:frame>",
        placement(frame, 0),
        xml[0],
        xml[1],
        xml[2],
        xml[3]
    )
}

fn page_style(styles: &mut OdpStyles, fill: &str) -> String {
    let name = styles.next("dp");
    let _ = write!(
        styles.automatic,
        "<style:style style:name=\"{name}\" style:family=\"drawing-page\"><style:drawing-page-properties draw:background-size=\"full\" {fill} presentation:background-visible=\"true\" presentation:background-objects-visible=\"true\"/></style:style>"
    );
    name
}

fn odp_package(slides: &[OSlide], version: OdfVersion) -> Vec<Entry> {
    let mut styles = OdpStyles::default();
    styles.automatic.push_str(PARAGRAPH);
    let plain = page_style(
        &mut styles,
        &format!(
            "draw:fill=\"solid\" draw:fill-color=\"{}\"",
            odf_color(SLIDE)
        ),
    );
    let mut pages = String::new();
    for slide in slides {
        let page = match &slide.background {
            Some(bg) => {
                let fill = styles.fill(bg, &slide.id);
                page_style(&mut styles, &fill)
            }
            None => plain.clone(),
        };
        let mut body = String::new();
        for (k, item) in slide.items.iter().enumerate() {
            match item {
                OKind::Item(i) => body.push_str(&odp_item(&mut styles, i, &slide.id)),
                OKind::Group(children) => {
                    body.push_str("<draw:g>");
                    for (n, child) in children.iter().enumerate() {
                        let name = format!(
                            "{}{}",
                            slide.id,
                            char::from(b'a' + u8::try_from(n).unwrap_or(0))
                        );
                        body.push_str(&odp_item(&mut styles, child, &name));
                    }
                    body.push_str("</draw:g>");
                }
                OKind::Table { frame, cells } => {
                    debug_assert_eq!(k, 0);
                    body.push_str(&odp_table(&mut styles, &slide.id, *frame, cells));
                }
            }
        }
        body.push_str(&odp_label(&mut styles, &slide.caption, slide.label));
        let _ = write!(
            pages,
            "<draw:page draw:name=\"{}\" draw:style-name=\"{page}\" draw:master-page-name=\"Probe\">{body}</draw:page>",
            slide.id
        );
    }
    let v = version.number();
    let meta = (version == OdfVersion::V12OldGenerator).then(|| {
        format!(
            "{ODF_DECL}<office:document-meta xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\" office:version=\"{v}\"><office:meta><meta:generator>LibreOfficeDev/6.0.5.2$Linux_X86_64 LibreOffice_project/</meta:generator></office:meta></office:document-meta>"
        )
    });
    let meta_entry = if meta.is_some() {
        "<manifest:file-entry manifest:full-path=\"meta.xml\" manifest:media-type=\"text/xml\"/>"
    } else {
        ""
    };
    let manifest = format!(
        "{ODF_DECL}<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"{v}\"><manifest:file-entry manifest:full-path=\"/\" manifest:version=\"{v}\" manifest:media-type=\"application/vnd.oasis.opendocument.presentation\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/><manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/>{meta_entry}</manifest:manifest>"
    );
    let content = format!(
        "{ODF_DECL}<office:document-content{ODF_NAMESPACES} office:version=\"{v}\"><office:automatic-styles>{}</office:automatic-styles><office:body><office:presentation>{pages}</office:presentation></office:body></office:document-content>",
        styles.automatic
    );
    let master_page = format!(
        "<style:style style:name=\"Mdp1\" style:family=\"drawing-page\"><style:drawing-page-properties draw:background-size=\"full\" draw:fill=\"solid\" draw:fill-color=\"{}\"/></style:style>",
        odf_color(SLIDE)
    );
    let styles_xml = format!(
        "{ODF_DECL}<office:document-styles{ODF_NAMESPACES} office:version=\"{v}\"><office:styles><style:default-style style:family=\"graphic\"><style:graphic-properties svg:stroke-color=\"#1f2937\" draw:fill-color=\"#ffffff\"/><style:text-properties fo:font-family=\"'Liberation Sans'\" style:font-family-generic=\"swiss\" fo:font-size=\"18pt\"/></style:default-style><style:style style:name=\"standard\" style:family=\"graphic\"><style:graphic-properties draw:stroke=\"none\" draw:fill=\"none\" draw:shadow=\"hidden\"/></style:style>{}</office:styles><office:automatic-styles><style:page-layout style:name=\"PM1\"><style:page-layout-properties fo:margin-top=\"0pt\" fo:margin-bottom=\"0pt\" fo:margin-left=\"0pt\" fo:margin-right=\"0pt\" fo:page-width=\"960pt\" fo:page-height=\"540pt\" style:print-orientation=\"landscape\"/></style:page-layout>{master_page}</office:automatic-styles><office:master-styles><style:master-page style:name=\"Probe\" style:page-layout-name=\"PM1\" draw:style-name=\"Mdp1\"/></office:master-styles></office:document-styles>",
        styles.named
    );
    let mut entries = vec![
        stored(
            "mimetype",
            b"application/vnd.oasis.opendocument.presentation".to_vec(),
        ),
        xml_entry("META-INF/manifest.xml", manifest),
    ];
    if let Some(meta) = meta {
        entries.push(xml_entry("meta.xml", meta));
    }
    entries.push(xml_entry("content.xml", content));
    entries.push(xml_entry("styles.xml", styles_xml));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(entries: &[Entry], name: &str) -> String {
        let entry = entries.iter().find(|e| e.name == name).unwrap();
        String::from_utf8(entry.data.clone()).unwrap()
    }

    fn slide(slides: &[PSlide], id: &str) -> Json {
        let k = slides.iter().position(|s| s.id == id).unwrap();
        slides[k].json(k)
    }

    fn geometry_of(slide: &Json) -> Json {
        let Some(Json::Arr(objects)) = slide.get("objects") else {
            panic!("no objects")
        };
        objects
            .iter()
            .find_map(|o| o.get("powerpoint").cloned())
            .unwrap()
    }

    fn get<'a>(json: &'a Json, key: &str) -> &'a Json {
        json.get(key).unwrap_or_else(|| panic!("no {key}"))
    }

    fn point(x: &str, y: &str) -> Json {
        Json::Arr(vec![Json::Num(x.to_owned()), Json::Num(y.to_owned())])
    }

    #[test]
    fn the_committed_files_are_the_generators() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../probes/out");
        assert_eq!(stale(&root), Vec::<String>::new());
    }

    #[test]
    fn every_deck_has_one_captioned_case_per_slide() {
        let pptx = pptx_slides();
        assert_eq!(pptx.len(), 43);
        for (k, s) in pptx.iter().enumerate() {
            assert_eq!(s.id, format!("G{:02}", k + 1));
            assert!(
                s.caption.starts_with(&format!("{} ", s.id)),
                "{}",
                s.caption
            );
        }
        let odp = odp_slides();
        assert_eq!(odp.len(), 41);
        for (k, s) in odp.iter().enumerate() {
            assert_eq!(s.id, format!("O{:02}", k + 1));
        }
        let package = pptx_package(&pptx);
        let parts = package
            .iter()
            .filter(|e| e.name.starts_with("ppt/slides/slide"))
            .count();
        assert_eq!(parts, 43);
        for (k, s) in pptx.iter().enumerate() {
            let xml = text(&package, &format!("ppt/slides/slide{}.xml", k + 1));
            assert!(xml.contains(&escape(&s.caption)), "{}", s.id);
        }
        let content = text(&odp_package(&odp, OdfVersion::V13), "content.xml");
        assert_eq!(content.matches("<draw:page ").count(), 41);
        assert!(!content.contains("non-primitive"));
        let old = odp_package(&odp12_slides(), OdfVersion::V12OldGenerator);
        assert!(text(&old, "meta.xml").contains("LibreOfficeDev/6.0.5.2"));
        assert!(text(&old, "content.xml").contains("office:version=\"1.2\""));
    }

    #[test]
    fn the_libreoffice_valid_slides_are_the_designs() {
        let valid: Vec<String> = pptx_slides()
            .into_iter()
            .filter(|s| s.lo_valid)
            .map(|s| s.id)
            .collect();
        assert_eq!(
            valid,
            [
                "G01", "G02", "G04", "G05", "G06", "G07", "G08", "G09", "G10", "G11", "G13", "G14",
                "G15", "G17", "G18", "G19", "G33", "G36", "G37", "G38", "G40", "G42", "G43"
            ]
        );
    }

    #[test]
    fn powerpoint_geometry_follows_post_039() {
        let slides = pptx_slides();
        // Bowen's focus for fillToRect l10 t30 r80 b20, and R = ½·hypot(W, H).
        let g24 = geometry_of(&slide(&slides, "G24"));
        assert_eq!(get(&g24, "focus"), &point("0.111111", "0.6"));
        assert_eq!(get(&g24, "radius"), &Json::Num("0.527046".into()));
        assert_eq!(get(&g24, "radiusY"), &Json::Num("1.581139".into()));
        // The corner preset: the circle around the corner, R = hypot(W, H).
        let g25 = geometry_of(&slide(&slides, "G25"));
        assert_eq!(get(&g25, "center"), &point("1", "1"));
        assert_eq!(get(&g25, "focus"), &point("1", "1"));
        assert_eq!(get(&g25, "radius"), &Json::Num("1.054093".into()));
        // A rect path's inner rectangle is the focus rectangle in [−1, 1].
        let g29 = geometry_of(&slide(&slides, "G29"));
        let inner = get(&g29, "inner");
        assert_eq!(get(inner, "l"), &Json::Num("-0.8".into()));
        assert_eq!(get(inner, "t"), &Json::Num("-0.4".into()));
        assert_eq!(get(inner, "r"), &Json::Num("-0.6".into()));
        assert_eq!(get(inner, "b"), &Json::Num("0.6".into()));
        // Linear ends: unscaled 45 on 3:1 and scaled 45 (corner to corner).
        let g02 = geometry_of(&slide(&slides, "G02"));
        assert_eq!(get(&g02, "from"), &point("-0.1", "0.3"));
        assert_eq!(get(&g02, "to"), &point("1.1", "0.7"));
        let g03 = geometry_of(&slide(&slides, "G03"));
        assert_eq!(get(&g03, "from"), &point("0", "0"));
        assert_eq!(get(&g03, "to"), &point("1", "1"));
        // The focus above the box is still inside the circle.
        let g27 = geometry_of(&slide(&slides, "G27"));
        assert_eq!(get(&g27, "focus"), &point("0.5", "-0.8"));
        assert_eq!(get(&g27, "focusInside"), &Json::Bool(true));
    }

    #[test]
    fn a_turned_box_keeps_its_centre() {
        let at = placement(BOX, 30);
        assert!(at.contains("rotate (0.523598775598)"), "{at}");
        // The centre (480, 270) minus the turned half-diagonal (259.8076 + 50, −150 + 86.6025).
        assert!(at.contains("translate (170.1924pt 333.3975pt)"), "{at}");
    }

    #[test]
    fn gradients_are_written_as_each_format_spells_them() {
        let xml = lin(45)
            .scaled()
            .tiled([50_000, 0, 0, -WHOLE])
            .flipped("x")
            .xml();
        assert_eq!(
            xml,
            "<a:gradFill flip=\"x\" rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:srgbClr val=\"000000\"/></a:gs><a:gs pos=\"100000\"><a:srgbClr val=\"FFFFFF\"/></a:gs></a:gsLst><a:lin ang=\"2700000\" scaled=\"1\"/><a:tileRect l=\"50000\" b=\"-100000\"/></a:gradFill>"
        );
        let odf = ograd("radial").centre(25, 75).border(30).xml("X");
        assert_eq!(
            odf,
            "<draw:gradient draw:name=\"X\" draw:display-name=\"X\" draw:style=\"radial\" draw:cx=\"25%\" draw:cy=\"75%\" draw:start-color=\"#000000\" draw:end-color=\"#ffffff\" draw:border=\"30%\"/>"
        );
    }
}
