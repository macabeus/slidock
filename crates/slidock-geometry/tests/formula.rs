//! Guide formulas, and the committed vectors: every operator over guides and literals, `-0` and NaN included, the error
//! messages, and the built-in guides of boxes that are not finite or have signed zeros.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use indexmap::IndexMap;
use slidock_geometry::{BUILTIN_NAMES, FormulaError, Guides, builtin_values};
use support::{js, num, rows, same};

fn guides() -> Guides {
    Guides::builtin(200.0, 100.0)
        .with_guides(&[("a", "val 30"), ("b", "*/ w a 100")], &IndexMap::new())
        .unwrap()
}

#[test]
fn every_operator() {
    let guides = guides();
    for (formula, value) in [
        ("*/ w 3 4", 150.0),
        ("+- w 10 h", 110.0),
        ("+/ w h 3", 100.0),
        ("?: a 1 2", 1.0),
        ("?: 0 1 2", 2.0),
        ("abs -7", 7.0),
        ("at2 1 1", 2_700_000.0),
        ("cat2 10 3 4", 6.0),
        ("sat2 10 3 4", 8.0),
        ("cos 10 cd2", -10.0),
        ("sin 10 cd4", 10.0),
        ("tan 10 cd8", 10.0),
        ("max a b", 60.0),
        ("min a b", 30.0),
        ("mod 2 3 6", 7.0),
        ("pin 0 b 50", 50.0),
        ("sqrt 81", 9.0),
        ("val ss", 100.0),
    ] {
        let got = guides.evaluate(formula).unwrap();
        assert!(
            (got - value).abs() < 5e-10,
            "{formula} = {got}, not {value}"
        );
    }
}

#[test]
fn has_every_builtin_guide_and_refuses_unknown_names_and_wrong_arities() {
    let guides = guides();
    let values: Vec<Option<f64>> = ["3cd4", "wd32", "hd10", "ssd16", "ls"]
        .iter()
        .map(|name| guides.get(name))
        .collect();
    assert_eq!(
        values,
        [
            Some(16_200_000.0),
            Some(6.25),
            Some(10.0),
            Some(6.25),
            Some(200.0)
        ]
    );
    assert_eq!(
        guides.evaluate("val nope"),
        Err(FormulaError::UnknownGuide("nope".into()))
    );
    let arity = guides.evaluate("*/ 1 2").unwrap_err();
    assert_eq!(arity.to_string(), "*/ takes 3 arguments: */ 1 2");
    assert_eq!(
        guides.evaluate("frob 1").unwrap_err().to_string(),
        "Unknown operator frob"
    );
}

#[test]
fn min_and_max_return_nan_for_a_nan_argument() {
    let guides = guides()
        .with_guides(&[("n", "sqrt -1")], &IndexMap::new())
        .unwrap();
    for formula in ["min n 5", "min 5 n", "max n 5", "max 5 n"] {
        assert!(guides.evaluate(formula).unwrap().is_nan(), "{formula}");
    }
}

#[test]
fn ignores_extra_arguments() {
    // The standard's circular arrows write `+- xH 0 dxB 0` for xH − dxB.
    assert_eq!(guides().evaluate("+- a 0 b 0"), Ok(-30.0));
}

#[test]
fn guides_see_the_guides_before_them_and_overrides_win() {
    let overrides = IndexMap::from([("adj".to_owned(), 5.0)]);
    let out = Guides::builtin(100.0, 50.0)
        .with_guides(
            &[
                ("adj", "val 20"),
                ("x", "*/ w adj 100"),
                ("adj", "val 7"),
                ("y", "+- adj 0 0"),
            ],
            &overrides,
        )
        .unwrap();
    assert_eq!(out.get("x"), Some(5.0));
    assert_eq!(out.get("y"), Some(5.0));
    // A guide may not read a later one, and the error is the first one in order.
    let error = Guides::builtin(1.0, 1.0)
        .with_guides(&[("p", "val q"), ("q", "val 1")], &IndexMap::new())
        .unwrap_err();
    assert_eq!(error.to_string(), "Unknown guide q");
    // A guide can shadow a built-in.
    let shadow = Guides::builtin(10.0, 10.0)
        .with_guides(&[("w", "val 3"), ("z", "*/ w 2 1")], &IndexMap::new())
        .unwrap();
    assert_eq!(shadow.get("z"), Some(6.0));
}

#[test]
fn matches_the_committed_vectors() {
    let rows = rows(support::FORMULAS);
    let (context, cases) = rows.split_first().unwrap();
    let context: Vec<(String, String)> = context
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| {
            (
                pair[0].as_str().unwrap().to_owned(),
                pair[1].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let guides = Guides::builtin(200.0, 100.0)
        .with_guides(&context, &IndexMap::new())
        .unwrap();
    let mut misses = Vec::new();
    for case in cases {
        let formula = case[0].as_str().unwrap();
        let got = guides.evaluate(formula);
        let ok = match (&got, case[1].is_null()) {
            (Ok(value), false) => same(*value, num(&case[1])),
            (Err(error), true) => error.to_string() == case[2].as_str().unwrap(),
            _ => false,
        };
        if !ok {
            misses.push(format!(
                "{formula:?}: {:?}, expected {}",
                got.map(js),
                case.as_array().unwrap()[1..]
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
    }
    assert_eq!(misses, Vec::<String>::new());
    assert!(cases.len() > 1000, "{}", cases.len());
}

#[test]
fn builtins_match_the_committed_vectors() {
    let cases = rows(support::BUILTINS);
    let mut misses = Vec::new();
    for case in &cases {
        let (w, h) = (num(&case[0]), num(&case[1]));
        let expected: Vec<(&str, f64)> = case[2]
            .as_array()
            .unwrap()
            .iter()
            .map(|pair| (pair[0].as_str().unwrap(), num(&pair[1])))
            .collect();
        let ours: Vec<(&str, f64)> = BUILTIN_NAMES
            .iter()
            .copied()
            .zip(builtin_values(w, h))
            .collect();
        let equal = expected.len() == ours.len()
            && expected
                .iter()
                .zip(&ours)
                .all(|((a, x), (b, y))| a == b && same(*x, *y));
        if !equal {
            misses.push(format!("{} x {}", js(w), js(h)));
        }
    }
    assert_eq!(misses, Vec::<String>::new());
    assert!(cases.len() > 10, "{}", cases.len());
}
