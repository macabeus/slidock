//! The ODP gradient probes (`cargo xtask probes-gradients`: `probes/out/gradient-probe.odp` and
//! `gradient-probe-odp12.odp`) through the `slidock` binary: each slide reports exactly the
//! diagnostics `gradient-probe.cases.json` lists for it, and the resolved IR draws what LibreOffice 25.2's PDF of the
//! probe shows (unitless angles by ODF version and generator, defaults, the axial orientation, steps, stops and
//! transparency gradients folded into the stop alphas).
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

fn probes() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../probes/out")
}

/// The resolved IR and the report of a probe deck.
fn compile(dir: &Path, deck: &str) -> (Value, Value) {
    std::fs::copy(probes().join(deck), dir.join(deck)).unwrap();
    let read = |emit: &str, file: &str| -> Value {
        let out = format!("{deck}-{emit}");
        let run = Command::new(env!("CARGO_BIN_EXE_slidock"))
            .args(["compile", deck, "--out", &out, "--emit", emit])
            .current_dir(dir)
            .env("SLIDOCK_CACHE_DIR", dir.join(".cache"))
            .output()
            .unwrap();
        assert_eq!(
            run.status.code(),
            Some(0),
            "{deck}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        serde_json::from_slice(&std::fs::read(dir.join(&out).join(file)).unwrap()).unwrap()
    };
    (
        read("ir", "presentation-ir.json"),
        read("report", "report.json"),
    )
}

/// The probe slide's gradient fill: the one element that is not the caption, or the background.
fn fill(ir: &Value, slide: usize) -> &Value {
    let slide = &ir["slides"][slide - 1];
    let shapes: Vec<&Value> = slide["elements"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["name"] != "Label")
        .collect();
    match shapes.as_slice() {
        [] => &slide["background"],
        [shape] => &shape["fill"],
        _ => panic!("slide {slide}: more than one shape"),
    }
}

fn number(value: &Value) -> f64 {
    value.as_f64().unwrap()
}

/// Whether a lowered linear gradient on its 600 × 200 pt box runs along the ODF direction d = (sin α, cos α) in the
/// page, given as (sin α, cos α): t grows along d, so ∇ᵤt = (W·d.x, H·d.y)/E, which is parallel to `to − from`.
#[track_caller]
fn assert_odf_direction(fill: &Value, (sin, cos): (f64, f64), what: &str) {
    assert_eq!(fill["kind"], "linear", "{what}: {fill}");
    let dx = number(&fill["to"]["x"]) - number(&fill["from"]["x"]);
    let dy = number(&fill["to"]["y"]) - number(&fill["from"]["y"]);
    let (gx, gy) = (600.0 * sin, 200.0 * cos);
    let cross = (dx * gy - dy * gx) / ((dx * dx + dy * dy) * (gx * gx + gy * gy)).sqrt();
    assert!(
        cross.abs() < 1e-3 && dx * gx + dy * gy > 0.0,
        "{what}: {fill}"
    );
}

fn stops(fill: &Value) -> Vec<(f64, [f64; 4])> {
    fill["stops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|stop| {
            let c = &stop["color"];
            (
                number(&stop["pos"]),
                [
                    number(&c["r"]),
                    number(&c["g"]),
                    number(&c["b"]),
                    number(&c["a"]),
                ],
            )
        })
        .collect()
}

#[track_caller]
fn assert_near(got: f64, want: f64, what: &str) {
    assert!((got - want).abs() < 2e-3, "{what}: {got} against {want}");
}

#[test]
fn reports_on_each_odp_gradient_probe_slide_what_the_cases_file_lists() {
    let dir = tempfile::tempdir().unwrap();
    let cases: Value =
        serde_json::from_slice(&std::fs::read(probes().join("gradient-probe.cases.json")).unwrap())
            .unwrap();
    let decks: Vec<&Value> = cases["decks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|deck| deck["frontend"] == "odp")
        .collect();
    assert_eq!(decks.len(), 2);
    for deck in decks {
        let file = deck["file"].as_str().unwrap();
        let (ir, report) = compile(dir.path(), file);
        let diagnostics = report["diagnostics"].as_array().unwrap();
        for case in deck["slides"].as_array().unwrap() {
            let id = ir["slides"][case["slide"].as_u64().unwrap() as usize - 1]["id"].clone();
            let mut codes: Vec<&str> = diagnostics
                .iter()
                .filter(|d| d["container"] == id)
                .map(|d| d["code"].as_str().unwrap())
                .collect();
            codes.sort_unstable();
            let expected: Vec<&str> = case["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .map(|code| code.as_str().unwrap())
                .collect();
            assert_eq!(codes, expected, "{file} {}", case["id"]);
        }
    }
}

#[test]
fn lowers_the_odp_gradient_probes_as_libreoffice_draws_them() {
    let dir = tempfile::tempdir().unwrap();
    let (odp, _) = compile(dir.path(), "gradient-probe.odp");
    let (odp12, _) = compile(dir.path(), "gradient-probe-odp12.odp");
    let (black, white) = ([0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]);

    // Unitless "300": degrees in ODF 1.3 (O30), tenths in ODF 1.2 from LibreOfficeDev 6 (N01).
    let half_root3 = 3.0f64.sqrt() / 2.0;
    assert_odf_direction(fill(&odp, 30), (-half_root3, 0.5), "O30");
    assert_odf_direction(fill(&odp12, 1), (0.5, half_root3), "N01");
    assert_odf_direction(fill(&odp, 2), (0.5, half_root3), "O02");

    // No cx/cy: the centre is the box's corner (O28, N02), reaching half the diagonal; no colours: black (O29).
    for (ir, slide) in [(&odp, 28), (&odp12, 2)] {
        let radial = fill(ir, slide);
        assert_eq!(radial["center"], json!({ "x": 0, "y": 0 }), "{slide}");
        assert_near(number(&radial["radius"]), 632.456 / 2.0 / 600.0, "radius");
    }
    assert_eq!(stops(fill(&odp, 29)), [(0.0, black), (1.0, black)]);

    // Axial: the start colour on the axis, the end colour at both edges (O09).
    assert_eq!(
        stops(fill(&odp, 9)),
        [(0.0, white), (0.5, black), (1.0, white)]
    );

    // Steps: as written (O04), per pair of stops (O27), LibreOffice's automatic count for an ellipsoid (O17).
    assert_eq!(fill(&odp, 4)["steps"], 4);
    assert_eq!(fill(&odp, 27)["steps"], 2);
    assert_eq!(fill(&odp, 27)["stops"].as_array().unwrap().len(), 3);
    assert_eq!(fill(&odp, 17)["kind"], "ellipse");
    assert_eq!(fill(&odp, 17)["steps"], 128);

    // loext stops replace the start and end colours (O24, O26).
    assert_eq!(
        stops(fill(&odp, 24)),
        [
            (0.0, [1.0, 0.0, 0.0, 1.0]),
            (0.3, [0.0, 1.0, 0.0, 1.0]),
            (1.0, [0.0, 0.0, 1.0, 1.0])
        ]
    );
    assert_eq!(
        stops(fill(&odp, 26)),
        [(0.0, [1.0, 0.0, 0.0, 1.0]), (1.0, [0.0, 0.0, 1.0, 1.0])]
    );

    // Transparency: over a solid colour (O31), folded into a gradient of the same contours (O33), and a uniform
    // opacity winning over it (O35).
    let alphas = |fill: &Value| -> Vec<f64> { stops(fill).iter().map(|s| s.1[3]).collect() };
    assert_eq!(alphas(fill(&odp, 31)), [1.0, 0.0]);
    assert_eq!(alphas(fill(&odp, 33)), [1.0, 0.2]);
    assert_eq!(alphas(fill(&odp, 35)), [0.5, 0.5]);
    // A radial transparency's border also holds its end opacity around the centre (O32: border 20 %).
    let ring: Vec<(f64, f64)> = stops(fill(&odp, 32))
        .iter()
        .map(|s| (s.0, s.1[3]))
        .collect();
    assert_eq!(ring, [(0.2, 1.0), (1.0, 0.0)]);

    // A table cell's linear gradient is an angled CSS one: ODF 30° is 60° clockwise from x (O38).
    let table = odp["slides"][37]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "table")
        .unwrap();
    assert_eq!(table["rows"][0]["cells"][0]["fill"]["angle"], 60);
    assert_eq!(table["rows"][0]["cells"][1]["fill"]["kind"], "radial");
}
