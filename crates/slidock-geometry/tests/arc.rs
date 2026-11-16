//! `arcTo`, and the committed vectors: random arcs, whole-turn and huge sweeps, sweeps either side of the quarter-turn
//! piece tolerance, and inputs that draw nothing.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

mod support;

use std::f64::consts::PI;

use slidock_geometry::{Arc, Point, arc_to};
use support::{digest, js, num, rows};

/// `toBeCloseTo(b, digits)`: within half a unit of the `digits`th decimal.
fn close(a: f64, b: f64, digits: u8) -> bool {
    let half = match digits {
        3 => 5e-4,
        _ => 5e-10,
    };
    (a - b).abs() < half
}

fn round3(x: f64) -> f64 {
    slidock_canon::js_round(x * 1000.0) / 1000.0 + 0.0
}

#[test]
fn draws_a_circle_in_quarter_turns_and_comes_back_to_its_start() {
    let Arc { cubics, end } = arc_to(
        Point::new(0.0, 50.0),
        50.0,
        50.0,
        10_800_000.0,
        21_600_000.0,
    );
    assert_eq!(cubics.len(), 4);
    assert!(close(end.x, 0.0, 9) && close(end.y, 50.0, 9));
    // The first quarter ends at the top of the circle, a cubic of the standard 0.5523 handle.
    assert_eq!(cubics[0].map(round3), [0.0, 22.386, 22.386, 0.0, 50.0, 0.0]);
}

#[test]
fn reads_its_angles_as_visual_directions_on_an_ellipse() {
    // From the point seen at 45° on a 2:1 ellipse, a 90° sweep ends at the point seen at 135°.
    let t = slidock_math::atan2(2.0, 1.0);
    let from = Point::new(2.0 * slidock_math::cos(t), slidock_math::sin(t));
    let Arc { end, .. } = arc_to(from, 2.0, 1.0, 2_700_000.0, 5_400_000.0);
    assert!(close(slidock_math::atan2(end.y, end.x), 3.0 * PI / 4.0, 9));
    assert!(close((end.x / 2.0) * (end.x / 2.0) + end.y * end.y, 1.0, 9));
}

#[test]
fn sweeps_backwards_for_a_negative_angle_and_draws_nothing_for_a_zero_radius() {
    let Arc { end, .. } = arc_to(Point::new(10.0, 0.0), 10.0, 10.0, 0.0, -5_400_000.0);
    let round9 = |x: f64| slidock_canon::js_round(x * 1e9) / 1e9;
    assert_eq!([round9(end.x), round9(end.y)], [0.0, -10.0]);
    let nothing = Arc {
        cubics: vec![],
        end: Point::new(1.0, 2.0),
    };
    assert_eq!(
        arc_to(Point::new(1.0, 2.0), 0.0, 10.0, 0.0, 5_400_000.0),
        nothing
    );
}

#[test]
fn draws_a_sweep_past_a_whole_turn_as_one_turn_and_the_rest() {
    // A hostile guide: a hundred million turns and a quarter.
    let start = Point::new(0.0, 50.0);
    let huge = arc_to(
        start,
        50.0,
        50.0,
        10_800_000.0,
        21_600_000.0 * 1e8 + 5_400_000.0,
    );
    let quarter = arc_to(start, 50.0, 50.0, 10_800_000.0, 5_400_000.0);
    assert!(huge.cubics.len() <= 8);
    assert!(close(huge.end.x, quarter.end.x, 3) && close(huge.end.y, quarter.end.y, 3));
    assert_eq!(
        arc_to(start, 50.0, 50.0, 10_800_000.0, -21_600_000.0 * 3.0)
            .cubics
            .len(),
        4
    );
    let nothing = Arc {
        cubics: vec![],
        end: Point::new(1.0, 2.0),
    };
    assert_eq!(
        arc_to(Point::new(1.0, 2.0), 50.0, f64::NAN, 0.0, 5_400_000.0),
        nothing
    );
    assert_eq!(
        arc_to(Point::new(1.0, 2.0), 50.0, 50.0, 0.0, f64::INFINITY),
        nothing
    );
    assert_eq!(arc_to(Point::new(1.0, 2.0), 50.0, 50.0, 0.0, -0.0), nothing);
}

#[test]
fn snaps_a_sweep_a_hair_past_whole_turns_to_the_turns() {
    // 0.001 units past k turns is 1.5e-10 rad, under the 1e-9 snap: the arc is the k-turn arc, bit for bit.
    for k in 1..=3 {
        let turns = 21_600_000.0 * f64::from(k);
        let from = Point::new(3.5, -7.25);
        assert_eq!(
            arc_to(from, 50.0, 30.0, 1_234_567.0, turns + 0.001),
            arc_to(from, 50.0, 30.0, 1_234_567.0, turns),
            "{k} turns"
        );
    }
}

#[test]
fn takes_one_more_cubic_only_past_a_billionth_of_a_quarter_turn() {
    let from = Point::new(-2.0, 11.5);
    let pieces = |quarters: f64| {
        arc_to(from, 17.5, 17.5, 0.0, 5_400_000.0 * quarters)
            .cubics
            .len()
    };
    assert_eq!(
        [pieces(1.0), pieces(1.0 + 5e-10), pieces(1.0 + 2e-9)],
        [1, 1, 2]
    );
    assert_eq!(
        [pieces(-3.0), pieces(-3.0 - 5e-10), pieces(-3.0 - 2e-9)],
        [3, 3, 4]
    );
}

/// The arc as the generator prints it: each cubic's numbers joined by `,`, cubics by ` `, then `@x,y`.
fn arc_text(arc: &Arc) -> String {
    let cubics: Vec<String> = arc
        .cubics
        .iter()
        .map(|c| c.iter().map(|x| js(*x)).collect::<Vec<_>>().join(","))
        .collect();
    format!("{}@{},{}", cubics.join(" "), js(arc.end.x), js(arc.end.y))
}

#[test]
fn matches_the_committed_vectors() {
    let cases = rows(support::ARCS);
    let mut misses = Vec::new();
    for case in &cases {
        let args: Vec<f64> = case[0].as_array().unwrap().iter().map(num).collect();
        let arc = arc_to(
            Point::new(args[0], args[1]),
            args[2],
            args[3],
            args[4],
            args[5],
        );
        let (count, expected) = (case[1].as_u64().unwrap(), case[2].as_str().unwrap());
        if arc.cubics.len() as u64 != count || digest(&arc_text(&arc)) != expected {
            misses.push(format!(
                "{args:?}: {} cubics, expected {count}",
                arc.cubics.len()
            ));
        }
    }
    assert_eq!(misses, Vec::<String>::new());
    assert!(cases.len() >= 1000, "{}", cases.len());
}
